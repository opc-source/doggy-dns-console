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
