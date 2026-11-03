use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::datasource::{Clock, InstanceSource, TelemetrySource};
use crate::domain::{
    FailureDetail, FailureFilter, FailureGroup, Instance, InstanceDetail, LogRecord, MetricCounts,
    Overview, PluginInfo, SeenInstance, SeriesPoint, SpanRecord, Window, WindowMetrics,
};
use crate::error::DataError;

pub struct FixtureSource {
    clock: Arc<dyn Clock>,
}

impl FixtureSource {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }

    /// Eight rows spread from 10 minutes to 8 days across three hosts plus one
    /// host-less row, so "1h" and "7d" answers genuinely differ (D10). Ages are
    /// relative to the fixture clock: `first_seen = now - (age + 10) minutes`,
    /// `last_seen = now - age`; the ten-minute span keeps each group a window,
    /// not a point.
    fn groups(&self) -> Vec<FailureGroup> {
        let now = self.clock.now_ms();
        let rows = [
            (
                "4bf92f3577b34da6",
                "user-service.DEFAULT_GROUP.public.nacos.local.",
                "SERVFAIL",
                "upstream timed out",
                12usize,
                Some("dns-node-1"),
                "nacos[nacos.local]",
                15i64,
            ),
            (
                "5cf92f3577b34da7",
                "payment-service.DEFAULT_GROUP.public.nacos.local.",
                "SERVFAIL",
                "upstream connection refused",
                7,
                Some("dns-node-2"),
                "forward[8.8.8.8:53]",
                180,
            ),
            (
                "6df92f3577b34da8",
                "missing.example.com.",
                "NXDOMAIN",
                "no authority handled query",
                31,
                Some("dns-node-1"),
                "none",
                45,
            ),
            (
                "7ef92f3577b34da9",
                "api.internal.",
                "REFUSED",
                "query refused by policy",
                3,
                Some("dns-node-3"),
                "native",
                3 * 1440,
            ),
            (
                "8ff92f3577b34da0",
                "slow.example.com.",
                "SERVFAIL",
                "upstream timeout after 5s",
                2,
                Some("dns-node-2"),
                "forward[1.1.1.1:53]",
                10,
            ),
            (
                "9af92f3577b34db1",
                "batch.internal.",
                "SERVFAIL",
                "upstream reset",
                4,
                Some("dns-node-2"),
                "forward[9.9.9.9:53]",
                6 * 1440,
            ),
            (
                "a0f92f3577b34db2",
                "old.internal.",
                "SERVFAIL",
                "upstream timed out",
                9,
                Some("dns-node-1"),
                "nacos[nacos.local]",
                8 * 1440,
            ),
            (
                "b1f92f3577b34db3",
                "unlabeled.example.com.",
                "SERVFAIL",
                "upstream timed out",
                5,
                None,
                "forward[8.8.8.8:53]",
                20,
            ),
        ];

        rows.into_iter()
            .map(
                |(trace, query, code, error, count, instance, authority, age_minutes)| {
                    FailureGroup {
                        key: format!("trace:{trace}"),
                        trace_id: Some(trace.to_string()),
                        query: query.to_string(),
                        qtype: "A".to_string(),
                        instance: instance.map(str::to_string),
                        authority: Some(authority.to_string()),
                        response_code: code.to_string(),
                        error: Some(error.to_string()),
                        count: count as u64,
                        first_seen: now - (age_minutes + 10) * 60_000,
                        last_seen: now - age_minutes * 60_000,
                        duration_ms: if code == "SERVFAIL" {
                            Some(5012)
                        } else {
                            Some(2)
                        },
                    }
                },
            )
            .collect()
    }

    /// Demo-side stand-in for the OpenObserve WHERE clause: overlap on the bracket,
    /// substring on the domain, equality on the response code, host equality on the
    /// instance. An unlabeled group is never reachable by the host filter.
    fn matches(&self, group: &FailureGroup, filter: &FailureFilter) -> bool {
        if group.response_code.eq_ignore_ascii_case("NXDOMAIN") {
            return false;
        }
        if group.last_seen < filter.range.start_ms || group.first_seen > filter.range.end_ms {
            return false;
        }
        if let Some(domain) = &filter.domain
            && !group.query.contains(domain.as_str())
        {
            return false;
        }
        if let Some(code) = &filter.response_code
            && !group.response_code.eq_ignore_ascii_case(code)
        {
            return false;
        }
        if let Some(instance) = &filter.instance {
            let Some(host) = &group.instance else {
                return false;
            };
            if !host.eq_ignore_ascii_case(instance) {
                return false;
            }
        }
        true
    }
}

#[async_trait]
impl TelemetrySource for FixtureSource {
    async fn overview(&self, window: Window) -> Result<Overview, DataError> {
        let now = self.clock.now_ms();
        let series = (0..24)
            .map(|index| SeriesPoint {
                ts: now - (23 - index) * 60 * 60 * 1000,
                queries: 4_800 + (index as u64 * 137) % 1_600,
                failures: 8 + (index as u64 * 7) % 31,
            })
            .collect();
        let queries = 128_934;
        let failures = 421;
        Ok(Overview {
            window_minutes: window.minutes,
            instances_online: 0,
            instances_total: 0,
            queries,
            failures,
            success_rate: Overview::success_rate(queries, failures),
            p95_latency_ms: Some(12.4),
            series,
        })
    }

    async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        let mut groups = self.groups();
        groups.retain(|group| self.matches(group, &filter));
        groups.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| right.last_seen.cmp(&left.last_seen))
        });
        groups.truncate(filter.limit.clamp(1, 200));
        Ok(groups)
    }

    async fn failure_detail(
        &self,
        trace_id: &str,
        filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
        let group = self
            .groups()
            .into_iter()
            .find(|group| {
                group.trace_id.as_deref() == Some(trace_id) && self.matches(group, &filter)
            })
            .ok_or_else(|| DataError::NotFound(format!("failure trace {trace_id}")))?;
        let logs = vec![LogRecord {
            // Inside the group's own window: the drawer's log row must not sit
            // in the future relative to `first_seen`, and it must move with
            // rows that spread across days.
            ts: group.last_seen + 5 * 60_000,
            severity: "ERROR".to_string(),
            message: group
                .error
                .clone()
                .unwrap_or_else(|| "query failed".to_string()),
            trace_id: Some(trace_id.to_string()),
            span_id: Some("00f067aa0ba902b7".to_string()),
            fields: json!({
                "query": group.query,
                "qtype": group.qtype,
                "authority": group.authority,
                // Real telemetry carries the process identity on the failure
                // row; the demo path must exercise the same extraction.
                "host_name": group.instance,
                "service_instance_id": group.instance,
                "doggy_dns_ip_addr": "127.0.0.1",
                "doggy_dns_control_port": "19090",
            }),
        }];
        let spans = vec![SpanRecord {
            span_id: "00f067aa0ba902b7".to_string(),
            name: "dns.error".to_string(),
            duration_ms: Some(5012.0),
            status: "ERROR".to_string(),
            attributes: json!({
                "error.message": group.error,
                "dns.question.name": group.query,
            }),
        }];
        Ok(FailureDetail {
            group,
            logs,
            spans,
            spans_unavailable: false,
        })
    }

    async fn unlabeled_failures(&self, filter: FailureFilter) -> Result<u64, DataError> {
        let unlabeled = self
            .groups()
            .into_iter()
            .filter(|group| group.instance.is_none())
            .filter(|group| {
                // Same scope as the list, minus the host equality that cannot match.
                let mut scope = filter.clone();
                scope.instance = None;
                self.matches(group, &scope)
            })
            .count();
        Ok(unlabeled as u64)
    }

    async fn seen_instances(&self, _window: Window) -> Result<Vec<SeenInstance>, DataError> {
        let now = self.clock.now_ms();
        Ok(vec![
            SeenInstance {
                id: "dns-node-1".to_string(),
                host: "dns-node-1".to_string(),
                last_seen: now,
                endpoint: None,
            },
            SeenInstance {
                id: "dns-node-2".to_string(),
                host: "dns-node-2".to_string(),
                last_seen: now - 30_000,
                endpoint: None,
            },
            SeenInstance {
                id: "dns-node-3".to_string(),
                host: "dns-node-3".to_string(),
                last_seen: now - 4 * 60 * 1000,
                endpoint: None,
            },
        ])
    }

    async fn failure_metrics(
        &self,
        host: Option<&str>,
        _start_ms: u64,
        _end_ms: u64,
    ) -> Result<WindowMetrics, DataError> {
        Ok(WindowMetrics {
            host: host.map(|_| MetricCounts {
                queries: 120,
                failures: 12,
            }),
            global: MetricCounts {
                queries: 4800,
                failures: 421,
            },
        })
    }
}

#[async_trait]
impl InstanceSource for FixtureSource {
    async fn list(&self) -> Result<Vec<Instance>, DataError> {
        let now = self.clock.now_ms();
        Ok(vec![
            instance("dns-node-1", true, true, now, "0.1.0"),
            instance("dns-node-2", true, true, now - 30_000, "0.1.0"),
            instance("dns-node-3", true, false, now - 4 * 60 * 1000, "0.0.9"),
            instance(
                "dns-node-4",
                false,
                false,
                now - 3 * 60 * 60 * 1000,
                "0.0.8",
            ),
        ])
    }

    async fn detail(&self, id: &str) -> Result<InstanceDetail, DataError> {
        let instance = self
            .list()
            .await?
            .into_iter()
            .find(|instance| instance.id == id || instance.host == id)
            .ok_or_else(|| DataError::NotFound(format!("instance {id}")))?;
        Ok(InstanceDetail {
            instance,
            pid: Some(4242),
            status: Some("running".to_string()),
            uptime_seconds: Some(86_400),
            started_at: Some("2026-10-25T09:00:00Z".to_string()),
            listen_addr: Some("0.0.0.0:15353".to_string()),
            protocols: vec!["udp".to_string(), "tcp".to_string()],
            plugins: vec![
                PluginInfo {
                    order: 0,
                    kind: "nacos".to_string(),
                    enabled: false,
                    details: json!({ "dnsZone": "nacos.local", "namespace": "public" }),
                },
                PluginInfo {
                    order: 1,
                    kind: "native".to_string(),
                    enabled: true,
                    details: json!({ "cacheSize": 4096, "minTtl": 10, "maxTtl": 300 }),
                },
                PluginInfo {
                    order: 2,
                    kind: "forward".to_string(),
                    enabled: true,
                    details: json!({
                        "upstream": ["8.8.8.8:53", "1.1.1.1:53"],
                        "cacheSize": 1024,
                    }),
                },
            ],
            config_toml: Some(
                "[server]\nlisten_addr = \"0.0.0.0\"\nport = 15353\n\n[mcp]\nenabled = true\nport = 19090\n"
                    .to_string(),
            ),
            redacted_keys: vec!["observability.basic_auth.password".to_string()],
            runtime: Some(json!({
                "signals": {
                    "nacos.nacos.local": {"cachedServices": 42, "lastPushSecondsAgo": 3},
                    "xdp": {"attached": true}
                }
            })),
            recent_failures: 0,
        })
    }
}

fn instance(
    id: &str,
    online: bool,
    mcp_reachable: bool,
    last_seen: i64,
    version: &str,
) -> Instance {
    Instance {
        id: id.to_string(),
        host: id.to_string(),
        version: Some(version.to_string()),
        online,
        last_seen,
        mcp_reachable,
    }
}
