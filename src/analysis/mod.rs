//! Analysis contract: the structured verdict returned for one evidence bundle.
//!
//! The result is a deterministic, citable contract. Every claim either cites
//! evidence IDs from the bundle (or from this attempt's drill-down calls) or is
//! explicitly marked as inferred; gaps are listed in `notDeterminable` instead
//! of being filled with guesses. The wire shape is fixed by spec §6.2, so the
//! field names and enum values here must not drift.

pub mod agent;
pub mod drilldown;
pub mod prompt;
pub mod service;
pub mod store;
pub mod validate;

pub use prompt::{PROMPT_VERSION, PromptFacts, system_prompt};

use serde::{Deserialize, Serialize};

/// Inputs recorded beside one analysis attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRequest {
    pub trace_id: String,
    pub fingerprint: String,
    pub model: String,
    pub prompt_version: String,
}

/// Structured conclusion for one evidence bundle (spec §6.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    pub status: AnalysisStatus,
    pub verdict: AnalysisVerdict,
    pub findings: Vec<Finding>,
    pub next_steps: Vec<NextStep>,
    pub not_determinable: Vec<String>,
}

/// Result-level status. The task lifecycle (`queued`/`running`/...) is a
/// separate concern and must not be modeled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatus {
    Ok,
    InsufficientEvidence,
    Unavailable,
    Invalid,
}

/// The conclusion itself: a fixed category, a short summary and confidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisVerdict {
    pub category: VerdictCategory,
    pub summary: String,
    pub confidence: Confidence,
}

/// Fixed failure categories. Unknown strings are rejected by serde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictCategory {
    UpstreamUnreachable,
    UpstreamTimeout,
    UpstreamRefused,
    PolicyRefused,
    ConfigInvalid,
    NacosStale,
    XdpPath,
    Unknown,
}

impl VerdictCategory {
    /// The serde wire string, used for the persisted scalar verdict column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UpstreamUnreachable => "upstream_unreachable",
            Self::UpstreamTimeout => "upstream_timeout",
            Self::UpstreamRefused => "upstream_refused",
            Self::PolicyRefused => "policy_refused",
            Self::ConfigInvalid => "config_invalid",
            Self::NacosStale => "nacos_stale",
            Self::XdpPath => "xdp_path",
            Self::Unknown => "unknown",
        }
    }
}

/// Confidence in the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    /// The serde wire string, used for the persisted scalar verdict column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// One claim with the evidence IDs that support it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub claim: String,
    pub evidence_ids: Vec<String>,
    pub kind: FindingKind,
}

/// Whether a finding comes straight from the evidence or is reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Observed,
    Inferred,
}

/// One read-only follow-up action with its rationale and risk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextStep {
    pub action: String,
    pub reason: String,
    pub risk: RiskLevel,
}

/// Risk of a proposed next step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

/// One drill-down invocation, persisted beside the conclusion (§6.3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallAudit {
    pub name: String,
    pub canonical_args: serde_json::Value,
    pub duration_ms: u64,
    pub result_hash: String,
    pub summary: String,
}
