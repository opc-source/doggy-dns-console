//! Result validation against the evidence an attempt may cite.
//!
//! Validation is the server-side guard behind the prompt. It runs after every
//! model submission and rejects citations that do not exist, observed findings
//! without evidence, NXDOMAIN attributions and over-confident verdicts. It
//! never rewrites the result: downgrading confidence is the caller's decision
//! (spec §6.2, §6.3).

use std::collections::HashSet;

use crate::analysis::{AnalysisResult, AnalysisStatus, Confidence, FindingKind};
use crate::evidence::model::EvidenceBundle;

/// Why a submitted analysis result is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("unknown evidence ID: {0}")]
    UnknownEvidenceId(String),
    #[error("observed finding has no evidence ID: {0}")]
    ObservedFindingWithoutEvidence(String),
    #[error(
        "observed finding cites no log, span, config, metric or plugin ID: {0}. Cite bundle evidence or mark the finding inferred"
    )]
    ObservedFindingWithoutBundleEvidence(String),
    #[error("an ok result requires at least one observed finding")]
    NoObservedFindings,
    #[error("insufficient_evidence must list notDeterminable gaps")]
    InsufficientEvidenceWithoutGaps,
    #[error("NXDOMAIN is not a failure cause")]
    NxdomainNotFailure,
    #[error("high confidence requires a reachable instance")]
    HighConfidenceWithoutReachableInstance,
    #[error("high confidence requires an observed finding")]
    HighConfidenceWithoutObservedFinding,
}

/// IDs that a finding may cite: the bundle plus any evidence returned by this
/// attempt's drill-down calls.
///
/// The index also retains whether the bundle's matched instance was reachable,
/// because the high-confidence rule is enforced here but the rule depends on
/// instance evidence rather than on a citable ID.
#[derive(Debug, Clone, Default)]
pub struct EvidenceIndex {
    ids: HashSet<String>,
    instance_reachable: bool,
}

impl EvidenceIndex {
    /// Collects every citable ID from `bundle`.
    ///
    /// That is every log and span ID, every plugin ID plus the config ID when
    /// present, and the global (plus host-scoped) metric IDs.
    pub fn from_bundle(bundle: &EvidenceBundle) -> Self {
        let mut index = Self::default();

        for log in &bundle.logs.items {
            index.insert(log.id.as_str());
        }
        for span in &bundle.spans.items {
            index.insert(span.id.as_str());
        }
        if let Some(instance) = &bundle.instance {
            index.instance_reachable = instance.reachable;
            for plugin in &instance.plugins {
                index.insert(plugin.id.as_str());
            }
            if let Some(config) = &instance.config {
                index.insert(config.id.as_str());
            }
        }
        if let Some(metrics) = &bundle.metrics {
            index.insert(metrics.queries_id.as_str());
            index.insert(metrics.failures_id.as_str());
            if let Some(host) = &metrics.host {
                index.insert(host.queries_id.as_str());
                index.insert(host.failures_id.as_str());
            }
        }

        index
    }

    /// Adds an ID produced by a drill-down call (Task 8/10). Ignores empties.
    pub fn insert(&mut self, id: impl Into<String>) {
        let id = id.into();
        if !id.is_empty() {
            self.ids.insert(id);
        }
    }

    /// Whether `id` was collected from the bundle or a drill-down result.
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }
}

/// Validates `result` against `evidence` in a fixed order.
///
/// Rules, in order (later rules may assume earlier ones passed):
/// 1. every cited evidence ID exists;
/// 2. every observed finding cites at least one evidence ID, and that ID comes
///    from the bundle (log/span/config/metric/plugin), not only a tool result;
/// 3. NXDOMAIN never appears as a failure cause;
/// 4. `insufficient_evidence` lists gaps;
/// 5. `ok` has at least one observed finding;
/// 6. high confidence needs a reachable instance and an observed finding.
pub fn validate_result(
    result: &AnalysisResult,
    evidence: &EvidenceIndex,
) -> Result<(), ValidationError> {
    for finding in &result.findings {
        for id in &finding.evidence_ids {
            if !evidence.contains(id) {
                return Err(ValidationError::UnknownEvidenceId(id.clone()));
            }
        }
    }

    for finding in &result.findings {
        if finding.kind == FindingKind::Observed && finding.evidence_ids.is_empty() {
            return Err(ValidationError::ObservedFindingWithoutEvidence(
                finding.claim.clone(),
            ));
        }
        if finding.kind == FindingKind::Observed
            && !finding
                .evidence_ids
                .iter()
                .any(|id| is_bundle_reference(id))
        {
            return Err(ValidationError::ObservedFindingWithoutBundleEvidence(
                finding.claim.clone(),
            ));
        }
    }

    let nxdomain = mentions_nxdomain(&result.verdict.summary)
        || result
            .findings
            .iter()
            .any(|finding| mentions_nxdomain(&finding.claim))
        || result
            .next_steps
            .iter()
            .any(|step| mentions_nxdomain(&step.action) || mentions_nxdomain(&step.reason));
    if nxdomain {
        return Err(ValidationError::NxdomainNotFailure);
    }

    if result.status == AnalysisStatus::InsufficientEvidence && result.not_determinable.is_empty() {
        return Err(ValidationError::InsufficientEvidenceWithoutGaps);
    }

    let has_observed = result
        .findings
        .iter()
        .any(|finding| finding.kind == FindingKind::Observed);

    if result.status == AnalysisStatus::Ok && !has_observed {
        return Err(ValidationError::NoObservedFindings);
    }

    if result.verdict.confidence == Confidence::High {
        if !evidence.instance_reachable {
            return Err(ValidationError::HighConfidenceWithoutReachableInstance);
        }
        if !has_observed {
            return Err(ValidationError::HighConfidenceWithoutObservedFinding);
        }
    }

    Ok(())
}

fn mentions_nxdomain(text: &str) -> bool {
    text.to_ascii_lowercase().contains("nxdomain")
}

/// Whether `id` names evidence from the bundle rather than a tool result.
/// `tool:` IDs are citable but cannot alone support an `observed` finding
/// (spec §6.2).
fn is_bundle_reference(id: &str) -> bool {
    ["log:", "span:", "config:", "metric:", "plugin:"]
        .iter()
        .any(|prefix| id.starts_with(prefix))
}
