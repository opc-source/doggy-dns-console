//! Evidence contract returned by `GET /api/failures/{traceId}/evidence`.
//!
//! The bundle is a deterministic projection of the failure detail, the matched
//! instance and the trace's metric window. Every object that an analysis may
//! cite carries a stable ID so findings can reference it by ID instead of
//! repeating raw telemetry. All fields serialize as `camelCase`; only the
//! [`MissingEvidence`] values use `snake_case` strings.
//!
//! The bundle deliberately contains no model name, prompt version or tool-call
//! metadata: those live in the analysis layer and must never affect the
//! evidence fingerprint.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{FailureFilter, FailureGroup, InstanceIdentity, MatchQuality, MetricCounts};

/// Complete evidence for one failure trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceBundle {
    pub trace_id: String,
    pub filter: EvidenceFilter,
    /// Wall-clock generation time. Excluded from the fingerprint.
    pub generated_at: i64,
    /// `sha256:<64 lowercase hex>` over the canonical JSON of this bundle.
    pub fingerprint: String,
    /// The same group the failure list and detail return.
    pub group: FailureGroup,
    /// `None` when no instance could be matched.
    pub instance: Option<EvidenceInstance>,
    pub logs: EvidenceLogs,
    pub spans: EvidenceSpans,
    /// `None` when metrics are unavailable for the trace window.
    pub metrics: Option<EvidenceMetrics>,
    /// Top-level degradations. `LogsTruncated` is present whenever
    /// `logs.truncated` is true. Order and duplicates are not significant and
    /// are normalized before fingerprinting.
    pub missing: Vec<MissingEvidence>,
}

/// Filter echo so an analysis can state the exact evidence window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceFilter {
    pub window_minutes: u32,
    pub domain: Option<String>,
    pub response_code: Option<String>,
}

impl From<&FailureFilter> for EvidenceFilter {
    fn from(filter: &FailureFilter) -> Self {
        Self {
            window_minutes: filter.window.minutes,
            domain: filter.domain.clone(),
            response_code: filter.response_code.clone(),
        }
    }
}

/// Selected logs plus the metadata required by the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceLogs {
    pub items: Vec<EvidenceLog>,
    /// Number of logs supplied before selection.
    pub total: usize,
    pub truncated: bool,
    /// Number of logs left out (`total - items.len()`).
    pub omitted: usize,
}

/// One selected log with its deterministic ID and normalized fields.
///
/// `query`, `qtype`, `response_code` and `error` are the normalized telemetry
/// fields used for the log's content hash; arbitrary raw fields are not
/// repeated here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceLog {
    /// `log:<ts>:<spanId|none>:<hash8>`
    pub id: String,
    pub ts: i64,
    pub severity: String,
    pub message: String,
    pub span_id: Option<String>,
    pub query: Option<String>,
    pub qtype: Option<String>,
    pub response_code: Option<String>,
    pub error: Option<String>,
}

/// Selected spans plus truncation metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSpans {
    pub items: Vec<EvidenceSpan>,
    pub total: usize,
    pub truncated: bool,
}

/// One span with its deterministic ID and the fields the UI renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSpan {
    /// `span:<spanId>`
    pub id: String,
    pub span_id: String,
    pub name: String,
    pub duration_ms: Option<f64>,
    pub status: String,
}

/// Matched instance evidence. When the instance is unreachable, `identity`
/// still carries the telemetry identity, `reachable` is false, and
/// config/runtime/plugins are empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceInstance {
    pub match_quality: MatchQuality,
    pub identity: InstanceIdentity,
    pub reachable: bool,
    pub status: Option<String>,
    pub pid: Option<u32>,
    /// Seconds since the process started, sampled by the control plane at call
    /// time. Excluded from the fingerprint.
    pub uptime_seconds: Option<u64>,
    pub listen_addr: Option<String>,
    pub plugins: Vec<EvidencePlugin>,
    pub config: Option<EvidenceConfig>,
    /// Raw `get_runtime_status` snapshot, sampled at call time (counters and
    /// ages move on every read). Excluded from the fingerprint.
    pub runtime: Option<Value>,
    /// Instance-level degradations. Order and duplicates are not significant
    /// and are normalized before fingerprinting.
    pub missing: Vec<MissingEvidence>,
}

/// One plugin in the instance's ordered plugin chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePlugin {
    /// `plugin:<instanceId>:<order>`
    pub id: String,
    pub order: usize,
    pub kind: String,
    pub enabled: bool,
    pub details: Value,
}

/// Redacted instance configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceConfig {
    /// `config:<instanceId>`
    pub id: String,
    pub content: String,
    pub redacted_keys: Vec<String>,
}

/// Metrics for the trace's `±15` minute window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceMetrics {
    pub range: EvidenceRange,
    /// Per-host counts; absent when the host is unknown or has no series.
    pub host: Option<EvidenceHostMetrics>,
    pub global: MetricCounts,
    /// `metric:global:queries:<start>-<end>`
    pub queries_id: String,
    /// `metric:global:failures:<start>-<end>`
    pub failures_id: String,
}

impl EvidenceMetrics {
    /// Builds global metric evidence with IDs derived from `range`.
    ///
    /// Host evidence, when present, must be built through
    /// [`EvidenceHostMetrics::new`] with the same `range`.
    pub fn new(
        range: EvidenceRange,
        host: Option<EvidenceHostMetrics>,
        global: MetricCounts,
    ) -> Self {
        Self {
            queries_id: range.metric_id("global", "queries"),
            failures_id: range.metric_id("global", "failures"),
            range,
            host,
            global,
        }
    }
}

/// Inclusive metric window bounds in epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRange {
    pub start: i64,
    pub end: i64,
}

impl EvidenceRange {
    /// `metric:<host|global>:<name>:<start>-<end>`
    ///
    /// Host IDs use `host:<host name>:<queries|failures>` as the name so they
    /// remain unique across hosts while retaining the scope discriminator.
    pub fn metric_id(&self, scope: &str, name: &str) -> String {
        format!("metric:{scope}:{name}:{}-{}", self.start, self.end)
    }
}

/// Host-scoped metric counts, flattened to
/// `{ name, queries, failures, queriesId, failuresId }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceHostMetrics {
    pub name: String,
    #[serde(flatten)]
    pub counts: MetricCounts,
    /// `metric:host:<name>:queries:<start>-<end>`
    pub queries_id: String,
    /// `metric:host:<name>:failures:<start>-<end>`
    pub failures_id: String,
}

impl EvidenceHostMetrics {
    /// Builds host metric evidence with IDs derived from `name` and `range`.
    pub fn new(name: impl Into<String>, counts: MetricCounts, range: &EvidenceRange) -> Self {
        let name = name.into();
        Self {
            queries_id: range.metric_id("host", &format!("{name}:queries")),
            failures_id: range.metric_id("host", &format!("{name}:failures")),
            name,
            counts,
        }
    }
}

/// Stable reasons why part of the evidence is absent or incomplete.
///
/// This is a set value: the fingerprint sorts and deduplicates `missing`
/// before hashing. The ordering is by the serialized name so it remains
/// stable if the variants are reordered in source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingEvidence {
    InstanceUnreachable,
    InstanceNotFound,
    SpansUnavailable,
    MetricsUnavailable,
    LogsTruncated,
}

impl MissingEvidence {
    /// Serialized name, used as the canonical sort key.
    fn canonical_key(self) -> &'static str {
        match self {
            Self::InstanceUnreachable => "instance_unreachable",
            Self::InstanceNotFound => "instance_not_found",
            Self::SpansUnavailable => "spans_unavailable",
            Self::MetricsUnavailable => "metrics_unavailable",
            Self::LogsTruncated => "logs_truncated",
        }
    }
}

impl PartialOrd for MissingEvidence {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MissingEvidence {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.canonical_key().cmp(other.canonical_key())
    }
}
