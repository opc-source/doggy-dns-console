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

pub const MIN_RANGE_MINUTES: u32 = 5;
/// The investigation chain may look back a full week; the live class stays at
/// 1440 minutes and keeps clamping through [`Window::new`].
pub const MAX_RANGE_MINUTES: u32 = 7 * 1440;
/// Clock skew between the console and the browser, in milliseconds.
pub const FUTURE_TOLERANCE_MS: i64 = 60_000;
pub const DEFAULT_WINDOW_MINUTES: u32 = 60;

/// An absolute `[start, end]` bracket in epoch milliseconds.
///
/// `Default` is the deliberately inert `0..0`: legacy `filter_json` blobs stored
/// before the range was introduced decode into it, and nothing reads the range
/// off the evidence echo any more (see the fingerprint decoupling).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Range {
    pub fn new(start_ms: i64, end_ms: i64) -> Self {
        Self { start_ms, end_ms }
    }

    /// Expand a relative window into `[now - minutes, now]`.
    pub fn from_window(window: Window, now_ms: i64) -> Self {
        let span = i64::from(window.minutes) * 60_000;
        Self::new(now_ms - span, now_ms)
    }

    /// Apply the precedence rule: an explicit `from`/`to` pair wins, otherwise
    /// `windowMinutes` (default 60) is measured backwards from `to` (default
    /// `now`). A lone `from` or lone `to` still participates, which is why the
    /// fallback anchors on the resolved `end`.
    pub fn resolve(
        from: Option<i64>,
        to: Option<i64>,
        window_minutes: Option<u32>,
        now_ms: i64,
    ) -> Result<Self, String> {
        let end_ms = to.unwrap_or(now_ms);
        let minutes = i64::from(window_minutes.unwrap_or(DEFAULT_WINDOW_MINUTES));
        // `saturating_sub` keeps a hostile `to = i64::MIN` inside `checked`'s
        // rejection path instead of panicking in a debug build.
        let start_ms = from.unwrap_or(end_ms.saturating_sub(minutes * 60_000));
        Self::checked(start_ms, end_ms, now_ms)
    }

    /// The three rejection rules plus the future tolerance, and nothing else.
    pub fn checked(start_ms: i64, end_ms: i64, now_ms: i64) -> Result<Self, String> {
        if start_ms >= end_ms {
            return Err("range start must be before range end".to_owned());
        }
        if end_ms > now_ms.saturating_add(FUTURE_TOLERANCE_MS) {
            return Err("range end is too far in the future".to_owned());
        }
        let span_ms = end_ms.saturating_sub(start_ms);
        if span_ms < i64::from(MIN_RANGE_MINUTES) * 60_000 {
            return Err(format!(
                "range is shorter than the {MIN_RANGE_MINUTES}-minute floor"
            ));
        }
        if span_ms > i64::from(MAX_RANGE_MINUTES) * 60_000 {
            return Err(format!(
                "range is longer than the {MAX_RANGE_MINUTES}-minute ceiling"
            ));
        }
        Ok(Self::new(start_ms, end_ms))
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
    /// The absolute bracket the investigation chain queries. `#[serde(default)]`
    /// keeps rows written before this field existed decodable.
    #[serde(default)]
    pub range: Range,
    pub domain: Option<String>,
    pub response_code: Option<String>,
    /// Case-insensitive equality against the telemetry `host_name` column.
    pub instance: Option<String>,
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

/// The `instance` filter as the telemetry layer means it: a trimmed host label,
/// or none. `?instance=` and `?instance=%20` must mean "no host filter", never
/// "only the rows that have no host".
pub fn host_filter(value: Option<String>) -> Option<String> {
    value
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty())
}

impl FailureFilter {
    pub fn new(range: Range) -> Self {
        Self {
            range,
            domain: None,
            response_code: None,
            instance: None,
            limit: 50,
        }
    }
}

impl Default for FailureFilter {
    fn default() -> Self {
        Self::new(Range::default())
    }
}
