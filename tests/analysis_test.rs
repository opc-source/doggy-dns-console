use std::sync::Arc;

use doggy_dns_console::analysis::prompt::system_prompt;
use doggy_dns_console::analysis::validate::{EvidenceIndex, ValidationError, validate_result};
use doggy_dns_console::analysis::{
    AnalysisResult, AnalysisStatus, AnalysisVerdict, Confidence, Finding, FindingKind, NextStep,
    PROMPT_VERSION, RiskLevel, VerdictCategory,
};
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::FailureFilter;
use doggy_dns_console::evidence::assemble::assemble;
use doggy_dns_console::evidence::model::EvidenceBundle;
use serde_json::json;

const NOW_MS: i64 = 1_761_436_800_000;

async fn fixture_bundle() -> EvidenceBundle {
    let clock = Arc::new(FixedClock(NOW_MS));
    let source = Arc::new(FixtureSource::new(clock));
    let telemetry: Arc<dyn TelemetrySource> = source.clone();
    let instances: Arc<dyn InstanceSource> = source.clone();
    assemble(
        &*telemetry,
        &*instances,
        "4bf92f3577b34da6",
        &FailureFilter::default(),
        &FixedClock(NOW_MS),
    )
    .await
    .unwrap()
}

/// A minimal result that passes every validation rule for a reachable bundle.
fn valid_result(evidence_id: &str) -> AnalysisResult {
    AnalysisResult {
        status: AnalysisStatus::Ok,
        verdict: AnalysisVerdict {
            category: VerdictCategory::UpstreamTimeout,
            summary: "the upstream timed out while resolving the query".to_string(),
            confidence: Confidence::Medium,
        },
        findings: vec![Finding {
            claim: "the upstream connection timed out after 5s".to_string(),
            evidence_ids: vec![evidence_id.to_string()],
            kind: FindingKind::Observed,
        }],
        next_steps: vec![NextStep {
            action: "check connectivity to the upstream resolver".to_string(),
            reason: "the trace ended with an upstream timeout".to_string(),
            risk: RiskLevel::Low,
        }],
        not_determinable: Vec::new(),
    }
}

// ------------------------------------------------------------- serialization

#[test]
fn analysis_result_serializes_the_spec_contract() {
    let mut result = valid_result("log:1761436740000:00f067aa0ba902b7:deadbeef");
    result.status = AnalysisStatus::InsufficientEvidence;
    result.verdict.category = VerdictCategory::UpstreamUnreachable;
    result.verdict.confidence = Confidence::Medium;
    result.not_determinable = vec!["instance config is unavailable".to_string()];

    let value = serde_json::to_value(&result).unwrap();

    assert_eq!(value["status"], "insufficient_evidence");
    assert_eq!(value["verdict"]["category"], "upstream_unreachable");
    assert_eq!(value["verdict"]["confidence"], "medium");
    assert_eq!(value["findings"][0]["kind"], "observed");
    assert_eq!(
        value["findings"][0]["evidenceIds"][0],
        "log:1761436740000:00f067aa0ba902b7:deadbeef"
    );
    assert_eq!(value["nextSteps"][0]["risk"], "low");
    assert_eq!(
        value["notDeterminable"][0],
        "instance config is unavailable"
    );

    // The structured contract replaced the old flat observed/inferred arrays.
    assert!(value.get("observed").is_none());
    assert!(value.get("inferred").is_none());
    assert!(value.get("next_steps").is_none());
    assert!(value.get("not_determinable").is_none());
    assert!(value["findings"][0].get("evidence_ids").is_none());
}

#[test]
fn analysis_result_rejects_unknown_category() {
    let value = json!({
        "status": "ok",
        "verdict": {
            "category": "bogus",
            "summary": "an unknown category",
            "confidence": "high"
        },
        "findings": [],
        "nextSteps": [],
        "notDeterminable": []
    });

    assert!(serde_json::from_value::<AnalysisResult>(value).is_err());
}

// ---------------------------------------------------------------- validation

#[tokio::test]
async fn validation_accepts_ok_result_with_observed_evidence() {
    let bundle = fixture_bundle().await;
    let evidence_id = bundle.logs.items[0].id.clone();
    let result = valid_result(&evidence_id);
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(validate_result(&result, &index), Ok(()));
}

#[tokio::test]
async fn validation_accepts_tool_evidence_ids() {
    let bundle = fixture_bundle().await;
    let mut index = EvidenceIndex::from_bundle(&bundle);
    index.insert("tool:call-1:result");

    let mut result = valid_result(&bundle.logs.items[0].id);
    result.findings[0]
        .evidence_ids
        .push("tool:call-1:result".to_string());

    assert_eq!(validate_result(&result, &index), Ok(()));
}

#[tokio::test]
async fn validation_rejects_unknown_evidence_id() {
    let bundle = fixture_bundle().await;
    let result = valid_result("log:not-in-this-bundle");
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::UnknownEvidenceId(
            "log:not-in-this-bundle".to_string()
        ))
    );
}

#[tokio::test]
async fn validation_rejects_observed_finding_without_evidence() {
    let bundle = fixture_bundle().await;
    let mut result = valid_result(&bundle.logs.items[0].id);
    result.findings = vec![
        Finding {
            claim: "the upstream refused the connection".to_string(),
            evidence_ids: Vec::new(),
            kind: FindingKind::Observed,
        },
        Finding {
            claim: "the upstream timed out".to_string(),
            evidence_ids: vec![bundle.logs.items[0].id.clone()],
            kind: FindingKind::Inferred,
        },
    ];
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::ObservedFindingWithoutEvidence(
            "the upstream refused the connection".to_string()
        ))
    );
}

#[tokio::test]
async fn validation_rejects_nxdomain_in_summary() {
    let bundle = fixture_bundle().await;
    let mut result = valid_result(&bundle.logs.items[0].id);
    result.verdict.summary = "the upstream answered NXDOMAIN".to_string();
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::NxdomainNotFailure)
    );
}

#[tokio::test]
async fn validation_rejects_nxdomain_in_finding_claim() {
    let bundle = fixture_bundle().await;
    let mut result = valid_result(&bundle.logs.items[0].id);
    result.findings[0].claim = "the upstream returned nxdomain".to_string();
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::NxdomainNotFailure)
    );
}

#[tokio::test]
async fn validation_requires_gaps_for_insufficient_evidence() {
    let bundle = fixture_bundle().await;
    let mut result = valid_result(&bundle.logs.items[0].id);
    result.status = AnalysisStatus::InsufficientEvidence;
    result.not_determinable.clear();
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::InsufficientEvidenceWithoutGaps)
    );
}

#[tokio::test]
async fn validation_rejects_ok_without_observed_finding() {
    let bundle = fixture_bundle().await;
    let mut result = valid_result(&bundle.logs.items[0].id);
    result.findings[0].kind = FindingKind::Inferred;
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::NoObservedFindings)
    );
}

#[tokio::test]
async fn validation_requires_reachable_instance_for_high_confidence() {
    let mut bundle = fixture_bundle().await;
    bundle.instance.as_mut().unwrap().reachable = false;

    let mut result = valid_result(&bundle.logs.items[0].id);
    result.verdict.confidence = Confidence::High;
    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::HighConfidenceWithoutReachableInstance)
    );
}

#[tokio::test]
async fn validation_requires_observed_finding_for_high_confidence() {
    let bundle = fixture_bundle().await;
    assert!(bundle.instance.as_ref().unwrap().reachable);

    let mut result = valid_result(&bundle.logs.items[0].id);
    result.verdict.confidence = Confidence::High;
    result.findings[0].kind = FindingKind::Inferred;
    // Keep rule 5 (ok needs an observed finding) from firing first so this
    // test isolates the high-confidence/observed rule.
    result.status = AnalysisStatus::InsufficientEvidence;
    result.not_determinable = vec!["no observed evidence for the verdict".to_string()];

    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::HighConfidenceWithoutObservedFinding)
    );
}

// -------------------------------------------------------------------- prompt

#[test]
fn prompt_is_versioned_and_states_the_contract() {
    assert_eq!(PROMPT_VERSION, "failure-analyst/v1");

    let prompt = system_prompt();
    for needle in [
        "notDeterminable",
        "evidenceIds",
        "insufficient_evidence",
        "NXDOMAIN",
        "submit_failure_analysis",
        "6",
        "openobserve_search_sql",
    ] {
        assert!(prompt.contains(needle), "prompt is missing {needle:?}");
    }
}
