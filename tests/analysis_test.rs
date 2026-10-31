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

// ------------------------------------------------------ forensic agent loop

use std::sync::Mutex;
use std::time::Duration;

use adk_rust::model::{ScriptedLlm, ScriptedResponse};
use async_trait::async_trait;
use doggy_dns_console::analysis::agent::{MAX_TOOL_RESULT_BYTES, SUBMIT_TOOL, run_forensic_loop};
use doggy_dns_console::analysis::drilldown::{Drilldown, upstream_tool_name};
use serde_json::Value;

struct ScriptedDrilldown {
    calls: Mutex<Vec<(String, Value)>>,
    result: Value,
}

impl ScriptedDrilldown {
    fn new(result: Value) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            result,
        }
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Drilldown for ScriptedDrilldown {
    fn tool_declarations(&self) -> Vec<Value> {
        vec![json!({
            "name": "openobserve_search_sql",
            "description": "run a read-only SQL query",
            "parameters": {"type": "object"}
        })]
    }

    async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push((tool_name.to_string(), args));
        Ok(self.result.clone())
    }
}

fn submit(result: &AnalysisResult) -> ScriptedResponse {
    ScriptedResponse::function_call(SUBMIT_TOOL, serde_json::to_value(result).unwrap())
}

fn first_log_id(bundle: &EvidenceBundle) -> String {
    bundle.logs.items.first().unwrap().id.clone()
}

#[test]
fn exposed_drilldown_names_map_to_upstream_names() {
    assert_eq!(
        upstream_tool_name("openobserve_search_sql"),
        Some("SearchSQL")
    );
    assert_eq!(upstream_tool_name("SearchSQL"), None);
    assert_eq!(upstream_tool_name("unknown"), None);
}

#[tokio::test]
async fn forensic_loop_accepts_a_direct_submission() {
    let bundle = fixture_bundle().await;
    let id = first_log_id(&bundle);
    let drilldown = ScriptedDrilldown::new(json!({"rows": []}));
    let llm = ScriptedLlm::new("test", vec![submit(&valid_result(&id))]);

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert!(outcome.tool_calls.is_empty());
    assert!(drilldown.calls().is_empty());
    assert_eq!(llm.requests().len(), 1);
    assert!(
        llm.requests()[0]
            .tools
            .contains_key("submit_failure_analysis")
    );
    assert!(
        llm.requests()[0]
            .tools
            .contains_key("openobserve_search_sql")
    );
}

#[tokio::test]
async fn forensic_loop_runs_a_drilldown_then_submits() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": [{"message": "timeout"}]}));
    let llm = ScriptedLlm::new(
        "test",
        vec![
            ScriptedResponse::function_call(
                "openobserve_search_sql",
                json!({"query": "SELECT * FROM logs"}),
            ),
            submit(&valid_result("tool:1:result")),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert_eq!(outcome.tool_calls[0].name, "openobserve_search_sql");
    assert_eq!(
        outcome.tool_calls[0].canonical_args,
        json!({"query": "SELECT * FROM logs"})
    );
    assert!(outcome.tool_calls[0].result_hash.starts_with("sha256:"));
    assert_eq!(drilldown.calls().len(), 1);
    assert_eq!(llm.requests().len(), 2);
    assert!(
        llm.requests()[1]
            .contents
            .iter()
            .any(|content| content.role == "function")
    );
}

#[tokio::test]
async fn forensic_loop_rejects_the_seventh_drilldown_call() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": []}));
    let mut script: Vec<ScriptedResponse> = (0..7)
        .map(|index| {
            ScriptedResponse::function_call(
                "openobserve_search_sql",
                json!({"query": format!("SELECT {index}")}),
            )
        })
        .collect();
    script.push(submit(&valid_result("tool:6:result")));
    let llm = ScriptedLlm::new("test", script);

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(drilldown.calls().len(), 6);
    assert_eq!(outcome.tool_calls.len(), 6);
    assert_eq!(llm.requests().len(), 8);
    // The history accumulates, so the response to the rejected seventh call is
    // the newest function response in the final request, not the first.
    let seventh_response = llm.requests()[7]
        .contents
        .iter()
        .rev()
        .flat_map(|content| content.parts.iter().rev())
        .find_map(|part| match part {
            adk_rust::Part::FunctionResponse {
                function_response, ..
            } => Some(function_response.response.clone()),
            _ => None,
        })
        .unwrap();
    assert!(seventh_response.to_string().contains("budget exhausted"));
}

#[tokio::test]
async fn forensic_loop_rejects_duplicate_canonical_arguments() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": []}));
    let llm = ScriptedLlm::new(
        "test",
        vec![
            ScriptedResponse::function_call(
                "openobserve_search_sql",
                json!({"query": "SELECT 1", "limit": 5}),
            ),
            ScriptedResponse::function_call(
                "openobserve_search_sql",
                json!({"limit": 5, "query": "SELECT 1"}),
            ),
            submit(&valid_result("tool:1:result")),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(drilldown.calls().len(), 1);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert_eq!(
        outcome.tool_calls[0].canonical_args,
        json!({"limit": 5, "query": "SELECT 1"})
    );
}

#[tokio::test]
async fn forensic_loop_allows_one_repair() {
    let bundle = fixture_bundle().await;
    let id = first_log_id(&bundle);
    let drilldown = ScriptedDrilldown::new(json!({"rows": []}));
    let llm = ScriptedLlm::new(
        "test",
        vec![
            submit(&valid_result("log:does-not-exist")),
            submit(&valid_result(&id)),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(llm.requests().len(), 2);
    let repair = llm.requests()[1]
        .contents
        .iter()
        .filter(|content| content.role == "user")
        .flat_map(|content| &content.parts)
        .filter_map(|part| match part {
            adk_rust::Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(repair.contains("unknown evidence ID"));
}

#[tokio::test]
async fn forensic_loop_is_invalid_after_two_rejections() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": []}));
    let llm = ScriptedLlm::new(
        "test",
        vec![
            submit(&valid_result("log:missing-one")),
            submit(&valid_result("log:missing-two")),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Invalid);
    assert_eq!(llm.requests().len(), 2);
    assert_eq!(outcome.result.findings, Vec::new());
    assert_eq!(outcome.result.not_determinable.len(), 1);
    assert!(outcome.result.not_determinable[0].contains("unknown evidence ID"));
}

#[tokio::test]
async fn forensic_loop_degrades_to_unavailable_on_timeout() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": []}));
    let llm = ScriptedLlm::new("test", Vec::<ScriptedResponse>::new());

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::ZERO)
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Unavailable);
    assert_eq!(outcome.result.verdict.category, VerdictCategory::Unknown);
    assert_eq!(outcome.result.not_determinable.len(), 1);
    assert!(outcome.result.not_determinable[0].contains("timed out"));
    assert!(drilldown.calls().is_empty());
    assert_eq!(llm.requests().len(), 0);
    assert!(outcome.tool_calls.is_empty());
}

#[tokio::test]
async fn forensic_loop_caps_tool_result_summaries() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": "x".repeat(20_000)}));
    let llm = ScriptedLlm::new(
        "test",
        vec![
            ScriptedResponse::function_call("openobserve_search_sql", json!({"query": "SELECT 1"})),
            submit(&valid_result("tool:1:result")),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert!(outcome.tool_calls[0].summary.len() <= MAX_TOOL_RESULT_BYTES);
    assert!(outcome.tool_calls[0].summary.contains("\"truncated\":true"));
}
