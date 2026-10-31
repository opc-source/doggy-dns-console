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
    /// `logs.truncated` is true.
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
    pub uptime_seconds: Option<u64>,
    pub listen_addr: Option<String>,
    pub plugins: Vec<EvidencePlugin>,
    pub config: Option<EvidenceConfig>,
    pub runtime: Option<Value>,
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
}

/// Inclusive metric window bounds in epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRange {
    pub start: i64,
    pub end: i64,
}

/// Host-scoped metric counts, flattened to `{ name, queries, failures }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceHostMetrics {
    pub name: String,
    #[serde(flatten)]
    pub counts: MetricCounts,
}

/// Stable reasons why part of the evidence is absent or incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingEvidence {
    InstanceUnreachable,
    InstanceNotFound,
    SpansUnavailable,
    MetricsUnavailable,
    LogsTruncated,
}
