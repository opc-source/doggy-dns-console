//! Assembles the evidence contract from telemetry, instance and metric
//! sources.
//!
//! Assembly is a deterministic projection: the failure detail is fetched
//! exactly once with the caller's filter, logs and spans keep their stable
//! order, instance resolution happens at most once, and the metric window is
//! centred on the trace rather than on "now". Every degradation is recorded
//! as a [`MissingEvidence`] value instead of being silently filled in.

use crate::datasource::{Clock, InstanceSource, TelemetrySource};
use crate::domain::{
    FailureFilter, FailureGroup, InstanceIdentity, LogRecord, MatchQuality, MatchedInstance,
};
use crate::error::DataError;
use crate::evidence::fingerprint::compute_fingerprint;
use crate::evidence::hash::{log_hash, normalized_log_fields};
use crate::evidence::ids::IdAllocator;
use crate::evidence::model::{
    EvidenceBundle, EvidenceConfig, EvidenceFilter, EvidenceHostMetrics, EvidenceInstance,
    EvidenceLog, EvidenceLogs, EvidenceMetrics, EvidencePlugin, EvidenceRange, EvidenceSpan,
    EvidenceSpans, MissingEvidence,
};
use crate::evidence::select::select_logs;

/// Maximum number of spans kept in an evidence bundle.
pub const MAX_SPANS: usize = 200;

/// The metric window is the trace time ±15 minutes.
pub const METRICS_RADIUS_MS: i64 = 15 * 60_000;

/// Raw log field aliases that identify the process that emitted the row.
///
/// These mirror the shapes produced by
/// [`crate::datasource::openobserve::normalize`] plus the flattened resource
/// attributes that real doggy-dns telemetry exports.
const SERVICE_INSTANCE_ALIASES: &[&str] = &[
    "service_instance_id",
    "service.instance.id",
    "serviceInstanceId",
];
const HOST_ALIASES: &[&str] = &["host_name", "host.name", "host", "hostName"];
const IP_ALIASES: &[&str] = &[
    "doggy_dns_ip_addr",
    "doggy_dns.ip_addr",
    "doggy_dns_ip",
    "host.ip",
];

/// Builds the complete evidence bundle for one failure trace.
///
/// The failure detail is fetched once with `filter`, the same filter the list
/// and detail endpoints use, so the bundle's group cannot drift from the list
/// entry. A missing trace propagates as [`DataError::NotFound`]. Instance or
/// metric failures degrade the bundle instead of failing it.
pub async fn assemble(
    telemetry: &dyn TelemetrySource,
    instances: &dyn InstanceSource,
    trace_id: &str,
    filter: &FailureFilter,
    clock: &dyn Clock,
) -> Result<EvidenceBundle, DataError> {
    let generated_at = clock.now_ms();
    let detail = telemetry.failure_detail(trace_id, filter.clone()).await?;

    let identity = derive_identity(&detail.logs, &detail.group);
    let center = trace_center(&detail.logs, detail.group.last_seen, generated_at);

    let mut allocator = IdAllocator::new();
    let mut missing = Vec::new();

    let selection = select_logs(detail.logs);
    let logs = EvidenceLogs {
        items: selection
            .items
            .iter()
            .map(|record| evidence_log(&mut allocator, record))
            .collect(),
        total: selection.total,
        truncated: selection.truncated,
        omitted: selection.omitted,
    };
    if logs.truncated {
        missing.push(MissingEvidence::LogsTruncated);
    }

    let spans_total = detail.spans.len();
    let spans = EvidenceSpans {
        items: detail
            .spans
            .into_iter()
            .take(MAX_SPANS)
            .map(|span| EvidenceSpan {
                id: allocator.span(&span.span_id),
                span_id: span.span_id,
                name: span.name,
                duration_ms: span.duration_ms,
                status: span.status,
            })
            .collect(),
        total: spans_total,
        truncated: spans_total > MAX_SPANS,
    };
    if detail.spans_unavailable {
        missing.push(MissingEvidence::SpansUnavailable);
    }

    let instance =
        assemble_instance(instances, identity.as_ref(), &mut allocator, &mut missing).await;

    let metric_host = identity
        .as_ref()
        .and_then(|identity| identity.host.as_deref());
    let start = (center - METRICS_RADIUS_MS).max(0);
    let end = (center + METRICS_RADIUS_MS).max(0);
    let metrics = match telemetry
        .failure_metrics(metric_host, start as u64, end as u64)
        .await
    {
        Ok(window) => {
            let range = EvidenceRange { start, end };
            let host = window
                .host
                .zip(metric_host)
                .map(|(counts, name)| EvidenceHostMetrics::new(name, counts, &range));
            Some(EvidenceMetrics::new(range, host, window.global))
        }
        Err(error) => {
            tracing::warn!(
                trace_id = trace_id,
                error = %error,
                "failure metric lookup failed"
            );
            missing.push(MissingEvidence::MetricsUnavailable);
            None
        }
    };

    let mut bundle = EvidenceBundle {
        trace_id: trace_id.to_string(),
        filter: EvidenceFilter::from(filter),
        generated_at,
        fingerprint: String::new(),
        group: detail.group,
        instance,
        logs,
        spans,
        metrics,
        missing,
    };
    bundle.fingerprint = compute_fingerprint(&bundle);
    Ok(bundle)
}

/// Resolves the telemetry identity at most once. A resolve error or an
/// unreachable match keeps the telemetry identity and degrades the instance
/// evidence instead of failing the bundle.
async fn assemble_instance(
    instances: &dyn InstanceSource,
    identity: Option<&InstanceIdentity>,
    allocator: &mut IdAllocator,
    missing: &mut Vec<MissingEvidence>,
) -> Option<EvidenceInstance> {
    let Some(identity) = identity else {
        missing.push(MissingEvidence::InstanceNotFound);
        return None;
    };

    match instances.resolve(identity).await {
        Ok(Some(matched)) => Some(evidence_instance(allocator, identity, matched, missing)),
        Ok(None) => {
            missing.push(MissingEvidence::InstanceNotFound);
            None
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "instance resolution failed; keeping telemetry identity"
            );
            missing.push(MissingEvidence::InstanceUnreachable);
            Some(telemetry_only_instance(identity))
        }
    }
}

/// Projects a matched instance. Unreachable matches keep only the telemetry
/// identity: config, plugins and runtime from the detail are not evidence that
/// the process is currently running with that state.
fn evidence_instance(
    allocator: &mut IdAllocator,
    identity: &InstanceIdentity,
    matched: MatchedInstance,
    missing: &mut Vec<MissingEvidence>,
) -> EvidenceInstance {
    let MatchedInstance {
        quality,
        reachable,
        detail,
    } = matched;

    if !reachable {
        missing.push(MissingEvidence::InstanceUnreachable);
        return EvidenceInstance {
            match_quality: quality,
            identity: identity.clone(),
            reachable: false,
            status: None,
            pid: None,
            uptime_seconds: None,
            listen_addr: None,
            plugins: Vec::new(),
            config: None,
            runtime: None,
            missing: vec![MissingEvidence::InstanceUnreachable],
        };
    }

    let instance_id = detail.instance.id.clone();
    let plugins = detail
        .plugins
        .into_iter()
        .map(|plugin| EvidencePlugin {
            id: allocator.plugin(&instance_id, plugin.order),
            order: plugin.order,
            kind: plugin.kind,
            enabled: plugin.enabled,
            details: plugin.details,
        })
        .collect();
    let config = detail.config_toml.map(|content| EvidenceConfig {
        id: allocator.config(&instance_id),
        content,
        redacted_keys: detail.redacted_keys,
    });

    EvidenceInstance {
        match_quality: quality,
        identity: identity.clone(),
        reachable: true,
        status: detail.status,
        pid: detail.pid,
        uptime_seconds: detail.uptime_seconds,
        listen_addr: detail.listen_addr,
        plugins,
        config,
        runtime: detail.runtime,
        missing: Vec::new(),
    }
}

fn telemetry_only_instance(identity: &InstanceIdentity) -> EvidenceInstance {
    EvidenceInstance {
        match_quality: MatchQuality::None,
        identity: identity.clone(),
        reachable: false,
        status: None,
        pid: None,
        uptime_seconds: None,
        listen_addr: None,
        plugins: Vec::new(),
        config: None,
        runtime: None,
        missing: vec![MissingEvidence::InstanceUnreachable],
    }
}

fn evidence_log(allocator: &mut IdAllocator, record: &LogRecord) -> EvidenceLog {
    let fields = normalized_log_fields(record);
    EvidenceLog {
        id: allocator.log(record.ts, record.span_id.as_deref(), &log_hash(record)),
        ts: record.ts,
        severity: record.severity.clone(),
        message: record.message.clone(),
        span_id: record.span_id.clone(),
        query: fields.query,
        qtype: fields.qtype,
        response_code: fields.response_code,
        error: fields.error,
    }
}

/// Derives the process identity from the trace's log resource fields.
///
/// The first non-empty value for each field wins (logs arrive in chronological
/// order); distinct fields may come from different rows so a trace whose error
/// row only carries `service_instance_id` still gets the host from a sibling
/// row. When no row names the host, the failure group's `instance` — the
/// `host_name` that produced the aggregate — is a justified fallback.
fn derive_identity(logs: &[LogRecord], group: &FailureGroup) -> Option<InstanceIdentity> {
    let mut service_instance_id = None;
    let mut host = None;
    let mut ip = None;

    for log in logs {
        service_instance_id =
            service_instance_id.or_else(|| field_text(&log.fields, SERVICE_INSTANCE_ALIASES));
        host = host.or_else(|| field_text(&log.fields, HOST_ALIASES));
        ip = ip.or_else(|| field_text(&log.fields, IP_ALIASES));
    }

    if host.is_none() {
        host = group
            .instance
            .as_deref()
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(ToString::to_string);
    }

    if service_instance_id.is_none() && host.is_none() && ip.is_none() {
        return None;
    }

    Some(InstanceIdentity {
        service_instance_id,
        host,
        ip,
    })
}

/// Picks a deterministic centre for the trace's metric window.
///
/// Preference order:
/// 1. the lower median of the trace's positive log timestamps — representative
///    of when the failure actually happened and stable for equal-sized input;
/// 2. the failure group's `last_seen`;
/// 3. the bundle's generation time.
fn trace_center(logs: &[LogRecord], last_seen: i64, generated_at: i64) -> i64 {
    let mut timestamps: Vec<i64> = logs.iter().map(|log| log.ts).filter(|ts| *ts > 0).collect();
    if !timestamps.is_empty() {
        timestamps.sort_unstable();
        return timestamps[(timestamps.len() - 1) / 2];
    }
    if last_seen > 0 {
        return last_seen;
    }
    generated_at
}

fn field_text(fields: &serde_json::Value, aliases: &[&str]) -> Option<String> {
    let object = fields.as_object()?;
    aliases.iter().find_map(|alias| {
        let value = object.get(*alias)?;
        let text = match value {
            serde_json::Value::String(text) => text.trim().to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::Bool(flag) => flag.to_string(),
            _ => return None,
        };
        (!text.is_empty()).then_some(text)
    })
}
