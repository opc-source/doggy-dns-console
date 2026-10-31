use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub minutes: u32,
}

impl Window {
    pub fn new(minutes: u32) -> Self {
        Self {
            minutes: minutes.clamp(5, 1440),
        }
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new(60)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPoint {
    pub ts: i64,
    pub queries: u64,
    pub failures: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub window_minutes: u32,
    pub instances_online: u32,
    pub instances_total: u32,
    pub queries: u64,
    pub failures: u64,
    pub success_rate: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub series: Vec<SeriesPoint>,
}

impl Overview {
    pub fn success_rate(queries: u64, failures: u64) -> Option<f64> {
        if queries == 0 {
            return None;
        }
        let rate = (1.0 - failures as f64 / queries as f64).clamp(0.0, 1.0);
        Some((rate * 1_000_000.0).round() / 1_000_000.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailureFilter {
    pub window: Window,
    pub domain: Option<String>,
    pub response_code: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailureGroup {
    pub key: String,
    pub trace_id: Option<String>,
    pub query: String,
    pub qtype: String,
    pub instance: Option<String>,
    pub authority: Option<String>,
    pub response_code: String,
    pub error: Option<String>,
    pub count: u64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    pub ts: i64,
    pub severity: String,
    pub message: String,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub fields: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpanRecord {
    pub span_id: String,
    pub name: String,
    pub duration_ms: Option<f64>,
    pub status: String,
    pub attributes: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailureDetail {
    pub group: FailureGroup,
    pub logs: Vec<LogRecord>,
    pub spans: Vec<SpanRecord>,
    /// True when the span lookup failed. The detail remains usable: logs and
    /// instance evidence are still valid, but spans must not be read as absent.
    #[serde(default)]
    pub spans_unavailable: bool,
}

/// How a telemetry identity was matched to a seen instance, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchQuality {
    Exact,
    HostIp,
    Host,
    None,
}

/// Instance identity carried by failure telemetry rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

/// A seen instance matched to a telemetry identity. `detail` carries the
/// telemetry identity even when the probe failed; `reachable` says whether
/// config/runtime/plugin evidence was actually collected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedInstance {
    pub quality: MatchQuality,
    pub reachable: bool,
    pub detail: InstanceDetail,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricCounts {
    pub queries: u64,
    pub failures: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowMetrics {
    /// Per-host counts; absent when the host is unknown or has no series.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<MetricCounts>,
    pub global: MetricCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub id: String,
    pub host: String,
    pub version: Option<String>,
    pub online: bool,
    pub last_seen: i64,
    pub mcp_reachable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeenInstance {
    pub id: String,
    pub host: String,
    pub last_seen: i64,
    /// MCP endpoint joined from the node's published address and control port
    /// (`doggy_dns.ip_addr`/`doggy_dns.control.port`). `None` for older nodes
    /// or when the listener was not exported.
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub order: usize,
    pub kind: String,
    pub enabled: bool,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstanceDetail {
    pub instance: Instance,
    pub pid: Option<u32>,
    pub status: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub started_at: Option<String>,
    pub listen_addr: Option<String>,
    pub protocols: Vec<String>,
    pub plugins: Vec<PluginInfo>,
    pub config_toml: Option<String>,
    pub redacted_keys: Vec<String>,
    /// Raw `get_runtime_status` payload (`{"signals": {...}}`); absent when
    /// the instance's MCP build predates the tool or the call failed.
    pub runtime: Option<Value>,
    pub recent_failures: u64,
}

impl Default for FailureFilter {
    fn default() -> Self {
        Self {
            window: Window::default(),
            domain: None,
            response_code: None,
            limit: 50,
        }
    }
}
