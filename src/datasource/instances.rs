use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

use async_trait::async_trait;

use crate::datasource::mcp::{McpInstanceSource, Probe};
use crate::datasource::{Clock, InstanceSource, TelemetrySource};
use crate::domain::{
    Instance, InstanceDetail, InstanceIdentity, MatchQuality, MatchedInstance, SeenInstance, Window,
};
use crate::error::DataError;

/// A node whose newest telemetry sample is older than this is offline.
const ONLINE_WINDOW_MS: i64 = 180_000;
/// Telemetry lookback used to discover instances.
const DISCOVERY_WINDOW_MINUTES: u32 = 1440;

pub struct MergedInstanceSource {
    mcp: Arc<McpInstanceSource>,
    telemetry: Arc<dyn TelemetrySource>,
    control_port: u16,
    clock: Arc<dyn Clock>,
}

impl MergedInstanceSource {
    pub fn new(
        mcp: Arc<McpInstanceSource>,
        telemetry: Arc<dyn TelemetrySource>,
        control_port: u16,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            mcp,
            telemetry,
            control_port,
            clock,
        }
    }

    async fn seen(&self) -> Vec<SeenInstance> {
        match self
            .telemetry
            .seen_instances(Window::new(DISCOVERY_WINDOW_MINUTES))
            .await
        {
            Ok(seen) => seen,
            Err(error) => {
                tracing::warn!(error = %error, "telemetry instance list failed");
                Vec::new()
            }
        }
    }

    fn static_probes(&self) -> Vec<Probe> {
        self.mcp
            .endpoints()
            .iter()
            .cloned()
            .map(Probe::new)
            .collect()
    }

    /// Endpoints to probe: the statically configured ones (explicit override)
    /// followed by every endpoint discovered from telemetry — joined from the
    /// published `doggy_dns.ip_addr`/`control_port`, otherwise `host:control_port`.
    fn probes(&self, seen: &[SeenInstance]) -> Vec<Probe> {
        let mut probes = self.static_probes();
        for instance in seen {
            if let Some(endpoint) = endpoint_for(instance, self.control_port) {
                push_probe(&mut probes, Probe::discovered(endpoint, &instance.host));
            }
        }
        probes
    }

    /// Probes for a detail view: statics (their identity is unknown until
    /// answered) plus the discovered endpoint of the requested instance only.
    fn detail_probes(&self, seen: &[SeenInstance], id: &str) -> Vec<Probe> {
        let mut probes = self.static_probes();
        for instance in seen
            .iter()
            .filter(|instance| matches_instance(instance, id))
        {
            if let Some(endpoint) = endpoint_for(instance, self.control_port) {
                push_probe(&mut probes, Probe::discovered(endpoint, &instance.host));
            }
        }
        probes
    }
}

#[async_trait]
impl InstanceSource for MergedInstanceSource {
    async fn list(&self) -> Result<Vec<Instance>, DataError> {
        let seen = self.seen().await;
        let mcp = self.mcp.list_endpoints(&self.probes(&seen)).await;

        let now = self.clock.now_ms();
        let cutoff = now - ONLINE_WINDOW_MS;
        let mut merged: BTreeMap<String, Instance> = BTreeMap::new();
        for instance in mcp {
            merged
                .entry(host_key(&instance.host))
                .and_modify(|existing| {
                    // A reachable probe knows more about the same host than an
                    // unreachable one.
                    if instance.mcp_reachable && !existing.mcp_reachable {
                        *existing = instance.clone();
                    }
                })
                .or_insert(instance);
        }
        for seen_instance in seen {
            match merged.entry(host_key(&seen_instance.host)) {
                Entry::Occupied(mut entry) => {
                    // Reachability and telemetry recency are independent: a
                    // failed probe must not hide fresh samples.
                    let instance = entry.get_mut();
                    instance.last_seen = instance.last_seen.max(seen_instance.last_seen);
                    instance.online = instance.online || seen_instance.last_seen >= cutoff;
                }
                Entry::Vacant(entry) => {
                    entry.insert(instance_from_seen(&seen_instance, cutoff));
                }
            }
        }
        let mut instances: Vec<_> = merged.into_values().collect();
        instances.sort_by(|left, right| {
            right
                .online
                .cmp(&left.online)
                .then_with(|| left.host.cmp(&right.host))
        });
        Ok(instances)
    }

    async fn detail(&self, id: &str) -> Result<InstanceDetail, DataError> {
        let seen = self.seen().await;
        match self.mcp.detail_at(&self.detail_probes(&seen, id), id).await {
            Ok(detail) => Ok(detail),
            Err(DataError::NotFound(_)) => {
                let cutoff = self.clock.now_ms() - ONLINE_WINDOW_MS;
                let instance = seen
                    .iter()
                    .find(|instance| matches_instance(instance, id))
                    .map(|instance| instance_from_seen(instance, cutoff))
                    .or_else(|| instance_for_static_id(self.mcp.endpoints(), id))
                    .ok_or_else(|| DataError::NotFound(format!("instance {id}")))?;
                Ok(degraded_detail(instance))
            }
            Err(error) => Err(error),
        }
    }

    async fn resolve(
        &self,
        wanted: &InstanceIdentity,
    ) -> Result<Option<MatchedInstance>, DataError> {
        let seen = self.seen().await;
        let Some((instance, quality)) = match_seen(&seen, wanted) else {
            return Ok(None);
        };
        let sid = wanted
            .service_instance_id
            .as_deref()
            .map(str::trim)
            .filter(|sid| !sid.is_empty());

        let Some(endpoint) = endpoint_for(&instance, self.control_port) else {
            // No dialable endpoint: keep the telemetry identity and let
            // config/runtime remain unavailable downstream.
            return Ok(Some(MatchedInstance {
                quality: verify_exact(quality, sid, &instance.id),
                reachable: false,
                detail: degraded_detail(instance_from_seen(
                    &instance,
                    self.clock.now_ms() - ONLINE_WINDOW_MS,
                )),
            }));
        };

        // Probe the matched instance only, exactly once.
        let probes = [Probe::discovered(endpoint, &instance.host)];
        match self.mcp.detail_at(&probes, &instance.id).await {
            Ok(detail) => {
                let quality = verify_exact(quality, sid, &detail.instance.id);
                Ok(Some(MatchedInstance {
                    quality,
                    reachable: true,
                    detail,
                }))
            }
            Err(DataError::NotFound(_)) => Ok(Some(MatchedInstance {
                quality: verify_exact(quality, sid, &instance.id),
                reachable: false,
                detail: degraded_detail(instance_from_seen(
                    &instance,
                    self.clock.now_ms() - ONLINE_WINDOW_MS,
                )),
            })),
            Err(error) => Err(error),
        }
    }
}

/// Exact is only credible once the detail identifies the requested process.
/// Until then the strongest verified fact is the shared host.
fn verify_exact(quality: MatchQuality, sid: Option<&str>, detail_id: &str) -> MatchQuality {
    if quality != MatchQuality::Exact {
        return quality;
    }
    match sid {
        Some(sid) if detail_id.eq_ignore_ascii_case(sid) => MatchQuality::Exact,
        _ => MatchQuality::Host,
    }
}

/// Match a telemetry identity to a seen instance using the documented
/// strongest-first chain: exact `service_instance_id`, host + ip, host, none.
pub fn match_seen(
    seen: &[SeenInstance],
    identity: &InstanceIdentity,
) -> Option<(SeenInstance, MatchQuality)> {
    if let Some(sid) = identity
        .service_instance_id
        .as_deref()
        .map(str::trim)
        .filter(|sid| !sid.is_empty())
    {
        // Exact id, or `<host>-<pid>` whose host published the telemetry row.
        for instance in seen {
            if instance.id.eq_ignore_ascii_case(sid) {
                return Some((instance.clone(), MatchQuality::Exact));
            }
        }
        if let Some((host, pid)) = sid.rsplit_once('-')
            && pid.parse::<u32>().is_ok()
        {
            for instance in seen {
                if instance.host.eq_ignore_ascii_case(host.trim()) {
                    return Some((instance.clone(), MatchQuality::Exact));
                }
            }
        }
    }

    if let (Some(host), Some(ip)) = (
        identity
            .host
            .as_deref()
            .map(str::trim)
            .filter(|host| !host.is_empty()),
        identity
            .ip
            .as_deref()
            .map(str::trim)
            .filter(|ip| !ip.is_empty()),
    ) {
        for instance in seen {
            if instance.host.eq_ignore_ascii_case(host) && endpoint_matches_ip(instance, ip) {
                return Some((instance.clone(), MatchQuality::HostIp));
            }
        }
    }

    if let Some(host) = identity
        .host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
    {
        for instance in seen {
            if instance.host.eq_ignore_ascii_case(host) {
                return Some((instance.clone(), MatchQuality::Host));
            }
        }
    }

    None
}

fn endpoint_matches_ip(instance: &SeenInstance, ip: &str) -> bool {
    let Some(url) = instance
        .endpoint
        .as_deref()
        .and_then(|endpoint| url::Url::parse(endpoint).ok())
    else {
        return false;
    };
    url.host_str().is_some_and(|endpoint_ip| {
        trim_brackets(endpoint_ip).eq_ignore_ascii_case(trim_brackets(ip))
    })
}

fn trim_brackets(value: &str) -> &str {
    value.trim_matches(['[', ']'])
}

fn endpoint_for(instance: &SeenInstance, control_port: u16) -> Option<String> {
    instance
        .endpoint
        .clone()
        .or_else(|| fallback_endpoint(&instance.host, control_port))
}

fn push_probe(probes: &mut Vec<Probe>, probe: Probe) {
    if let Some(existing) = probes
        .iter_mut()
        .find(|candidate| candidate.endpoint.eq_ignore_ascii_case(&probe.endpoint))
    {
        // A static endpoint that is also published by a node keeps the
        // telemetry identity, so a failed probe still merges.
        if existing.host.is_none() {
            existing.host = probe.host;
        }
    } else {
        probes.push(probe);
    }
}

/// `http://{host}:{port}/mcp`, or `None` when the host cannot be dialed.
fn fallback_endpoint(host: &str, port: u16) -> Option<String> {
    let host = host.trim();
    if host.is_empty() || host.eq_ignore_ascii_case("unknown") || host.contains('/') {
        return None;
    }
    if host
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_unspecified())
    {
        return None;
    }
    Some(format!(
        "http://{}:{port}/mcp",
        crate::datasource::bracketed(host)
    ))
}

fn instance_from_seen(seen: &SeenInstance, cutoff: i64) -> Instance {
    Instance {
        id: seen.id.clone(),
        host: seen.host.clone(),
        version: None,
        online: seen.last_seen >= cutoff,
        last_seen: seen.last_seen,
        mcp_reachable: false,
    }
}

/// Match a listed instance id (either a telemetry host or the control plane's
/// `<host>-<pid>` instance id) against the host that published it.
fn matches_instance(seen: &SeenInstance, id: &str) -> bool {
    let id = id.trim();
    if seen.id.eq_ignore_ascii_case(id) || seen.host.eq_ignore_ascii_case(id) {
        return true;
    }
    id.rsplit_once('-').is_some_and(|(host, pid)| {
        host.eq_ignore_ascii_case(&seen.host) && pid.parse::<u32>().is_ok()
    })
}

fn host_key(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

/// A static endpoint that never answered is still listed by its URL host; its
/// detail view degrades to that identity.
fn instance_for_static_id(endpoints: &[String], id: &str) -> Option<Instance> {
    let host = endpoints.iter().find_map(|endpoint| {
        let host = url::Url::parse(endpoint).ok()?.host_str()?.to_string();
        host.eq_ignore_ascii_case(id.trim()).then_some(host)
    })?;
    Some(Instance {
        id: host.clone(),
        host,
        version: None,
        online: false,
        last_seen: 0,
        mcp_reachable: false,
    })
}

fn degraded_detail(instance: Instance) -> InstanceDetail {
    InstanceDetail {
        instance,
        pid: None,
        status: None,
        uptime_seconds: None,
        started_at: None,
        listen_addr: None,
        protocols: Vec::new(),
        plugins: Vec::new(),
        config_toml: None,
        redacted_keys: Vec::new(),
        runtime: None,
        recent_failures: 0,
    }
}
