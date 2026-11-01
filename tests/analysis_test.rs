use std::sync::Arc;

use doggy_dns_console::analysis::prompt::{PromptFacts, system_prompt};
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
async fn validation_rejects_observed_finding_citing_only_a_tool_result() {
    let bundle = fixture_bundle().await;
    let mut index = EvidenceIndex::from_bundle(&bundle);
    index.insert("tool:1:result");

    let mut result = valid_result(&bundle.logs.items[0].id);
    result.findings[0].evidence_ids = vec!["tool:1:result".to_string()];
    result.findings[0].claim = "the drill-down returned an upstream timeout".to_string();

    assert_eq!(
        validate_result(&result, &index),
        Err(ValidationError::ObservedFindingWithoutBundleEvidence(
            "the drill-down returned an upstream timeout".to_string()
        ))
    );
}

#[tokio::test]
async fn validation_accepts_observed_finding_citing_span_evidence() {
    let bundle = fixture_bundle().await;
    let span_id = bundle.spans.items[0].id.clone();

    let mut result = valid_result(&bundle.logs.items[0].id);
    result.findings[0].evidence_ids = vec![span_id.clone()];

    let index = EvidenceIndex::from_bundle(&bundle);

    assert_eq!(validate_result(&result, &index), Ok(()));
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

/// The OpenObserve tool names the tests render as declared drill-downs, so the
/// prompt can be asserted on exactly what the drill-down advertised.
const SCRIPTED_TOOLS: [&str; 3] = [
    "openobserve_search_sql",
    "openobserve_stream_schema",
    "openobserve_promql_query",
];

/// The limits a test prompt is rendered with. Deliberately different from every
/// default so a hardcoded value in the prompt cannot pass by accident: the
/// unproductive allowance and the result cap have to differ from
/// `MAX_UNPRODUCTIVE_ATTEMPTS` and `MAX_TOOL_RESULT_BYTES` too.
fn prompt_facts() -> PromptFacts {
    PromptFacts {
        max_tool_calls: 9,
        max_unproductive_attempts: 7,
        timeout: Duration::from_secs(150),
        max_result_bytes: 4096,
        log_stream: Some("app_logs".to_string()),
        trace_stream: Some("app_traces".to_string()),
        tool_names: SCRIPTED_TOOLS
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    }
}

#[test]
fn prompt_is_versioned_and_states_the_contract() {
    assert_eq!(PROMPT_VERSION, "failure-analyst/v2");

    let prompt = system_prompt(&prompt_facts());
    for needle in [
        "notDeterminable",
        "evidenceIds",
        "insufficient_evidence",
        "NXDOMAIN",
        "submit_failure_analysis",
        "openobserve_search_sql",
    ] {
        assert!(prompt.contains(needle), "prompt is missing {needle:?}");
    }
}

#[test]
fn prompt_states_the_limits_the_loop_enforces() {
    let prompt = system_prompt(&prompt_facts());

    assert!(
        prompt.contains("9 drill-down"),
        "prompt must state the configured drill-down budget"
    );
    assert!(
        prompt.contains("7 attempts that return no evidence"),
        "prompt must state the cap on attempts that return no evidence, and call \
         them what they are: a tool error is executed, not refused"
    );
    assert!(
        prompt.contains("150 seconds"),
        "prompt must state the configured deadline"
    );
    assert!(
        prompt.contains("4096 bytes"),
        "prompt must state the configured result cap"
    );
    assert!(
        !prompt.contains("4 refused"),
        "the prompt must not restate the constant it was handed"
    );
    assert!(
        !prompt.contains("60 seconds") && !prompt.contains("at most 6"),
        "the prompt must not carry a hardcoded deadline or budget: it then lies \
         about the limits the loop enforces"
    );
}

#[test]
fn prompt_names_the_streams_the_drilldown_can_read() {
    let prompt = system_prompt(&prompt_facts());
    assert!(
        prompt.contains("app_logs") && prompt.contains("app_traces"),
        "the model must be told the stream names, otherwise it guesses `logs` \
         or `traces` and every SQL drill-down fails"
    );

    let mut without = prompt_facts();
    without.log_stream = None;
    without.trace_stream = None;
    let prompt = system_prompt(&without);
    assert!(
        !prompt.contains("app_logs") && !prompt.contains("app_traces"),
        "no stream names may be claimed when no drill-down source is configured"
    );
    assert!(
        prompt.contains("no stream is configured"),
        "the prompt must say the streams are missing instead of staying silent"
    );
}

#[test]
fn prompt_names_only_the_stream_that_exists() {
    let mut half = prompt_facts();
    half.trace_stream = None;
    let prompt = system_prompt(&half);
    assert!(
        prompt.contains("app_logs"),
        "a configured log stream stays usable when the trace stream is missing"
    );
    assert!(
        !prompt.contains("app_traces"),
        "the prompt must not name a stream the drill-down cannot read"
    );
    assert!(
        !prompt.contains("no drill-down tools are available"),
        "tools are available here; only one of the two streams is"
    );
    assert!(
        paragraph_containing(&prompt, "take the stream as an argument").contains("the name above"),
        "with a single configured stream the prompt must not offer a choice of names — \
         a trace tool handed two names picks one at random"
    );
}

#[test]
fn prompt_places_the_span_stream_when_only_a_trace_stream_exists() {
    // The mirror of the log-only case: the SQL guidance and the argument sentence
    // are built from the two streams separately, so a trace-only console has its
    // own rendering path, and the one thing that must not happen is the prompt
    // naming a log stream this drill-down cannot read.
    let mut trace_only = prompt_facts();
    trace_only.log_stream = None;
    let prompt = system_prompt(&trace_only);
    assert!(
        prompt.contains("app_traces"),
        "a configured trace stream has to be named even when no log stream is"
    );
    assert!(
        !prompt.contains("app_logs"),
        "the prompt must not name a log stream the drill-down cannot read"
    );
    assert!(
        paragraph_containing(&prompt, "the stream goes in the SQL itself")
            .contains("FROM app_traces"),
        "with only a trace stream, the SQL guidance has to place that name"
    );
    assert!(
        paragraph_containing(&prompt, "take the stream as an argument").contains("the name above"),
        "one configured stream means one possible argument, whoever the tool is"
    );
}

#[test]
fn prompt_scopes_the_stream_names_to_the_tools_that_take_them() {
    let prompt = system_prompt(&prompt_facts());
    assert!(
        prompt.contains("FROM app_logs"),
        "openobserve_search_sql puts the stream in the SQL, so the prompt has to \
         show where the name goes instead of telling the model to pass it"
    );
    assert!(
        prompt.contains("FROM app_traces"),
        "the span stream has to be placed in the SQL the same way the log stream is"
    );
    assert!(
        prompt.contains("no stream"),
        "PromQL and stream-list tools take no stream at all; telling the model \
         otherwise produces calls rejected for an unknown field"
    );
    assert!(
        paragraph_containing(&prompt, "take no stream argument")
            .contains("openobserve_promql_query"),
        "the no-stream sentence has to name the tool, not just contain the words \
         \"no stream\" — which `openobserve_search_sql has no stream parameter` also satisfies"
    );
    assert!(
        paragraph_containing(&prompt, "take the stream as an argument")
            .contains("one of the names above"),
        "with both streams configured the model has to be told to pick one of them"
    );
}

/// The parameter shape a live OpenObserve MCP answered `tools/list` with on
/// 2026-11-02, kept in `tests/fixtures/openobserve_tools_list_2026-11-02.json`.
/// The prompt has to place each stream the way that record says, because the
/// classification lists in `prompt.rs` and the expectations below would else be
/// the same hand-written guess and could agree while both were wrong.
const LIVE_TOOLS_RECORD: &str = include_str!("fixtures/openobserve_tools_list_2026-11-02.json");

#[test]
fn prompt_places_the_stream_the_way_the_recorded_upstream_declares_it() {
    let record: serde_json::Value =
        serde_json::from_str(LIVE_TOOLS_RECORD).expect("the recorded tools/list must parse");
    let tools = record["tools"]
        .as_array()
        .expect("the record must carry a tools array");

    let mut recorded_names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    let mut menu = REAL_UPSTREAM_TOOLS.to_vec();
    recorded_names.sort();
    menu.sort();
    assert_eq!(
        recorded_names, menu,
        "the record and the live tool menu the prompt is tested against must be the same set"
    );

    let mut facts = prompt_facts();
    facts.tool_names = recorded_names
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let prompt = system_prompt(&facts);

    let as_argument = paragraph_containing(&prompt, "take the stream as an argument");
    let no_stream = paragraph_containing(&prompt, "take no stream argument");
    let in_sql = paragraph_containing(&prompt, "the stream goes in the SQL itself");

    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        if tool["requiresStreamName"].as_bool().unwrap() {
            assert!(
                as_argument.contains(name) && !no_stream.contains(name),
                "{name} requires `stream_name` in the recorded upstream schema, so the prompt \
                 has to place it with the tools that pass the stream as an argument; it reads: \
                 {as_argument}"
            );
        } else if name == "openobserve_search_sql" {
            assert!(
                in_sql.contains(name) && !as_argument.contains(name),
                "{name} carries the stream inside its SQL body rather than as an argument, so the \
                 prompt has to say where the name goes: {in_sql}"
            );
        } else {
            assert!(
                no_stream.contains(name) && !as_argument.contains(name),
                "{name} has no required `stream_name` in the recorded upstream schema, so the \
                 prompt must not tell the model to pass one: {no_stream}"
            );
        }
    }
}

#[test]
fn prompt_promises_only_what_the_console_itself_enforces() {
    let prompt = system_prompt(&prompt_facts());

    // `Drilldown::call_tool` forwards the model's arguments to the transport after
    // checking the tool name, so an extra `stream_name` on a streamless tool is
    // rejected by the upstream or ignored by it — the console cannot know which.
    // A promise about that is the same class of lie as a wrong budget.
    for (needle, what) in [
        (
            "take no stream argument",
            "whether the upstream rejects the call",
        ),
        (
            "take the stream as an argument",
            "what a wrong stream costs",
        ),
    ] {
        let line = paragraph_containing(&prompt, needle);
        assert!(
            !line.contains("rejected") && !line.contains("refused"),
            "the prompt must not promise how the upstream validates an argument it does not \
             validate itself ({what}); the line reads: {line}"
        );
    }

    // What the console does enforce may be stated: the duplicate-call refusal is
    // its own check in the loop.
    assert!(
        prompt.contains("Repeating the same tool with the same canonical arguments is refused"),
        "duplicate calls are refused by this loop, so the prompt may say so"
    );
}

/// The OpenObserve MCP tool set as a live `tools/list` answered on 2026-11-02:
/// four tools require `stream_name`, `openobserve_search_sql` names the stream
/// inside its SQL, and three have no stream to name at all. A declared tool the
/// prompt gives no instruction for is a tool the model has to guess about, and
/// guessing is what spent drill-down calls in the 2026-11-01 round.
const REAL_UPSTREAM_TOOLS: [&str; 8] = [
    "openobserve_latest_traces",
    "openobserve_promql_query",
    "openobserve_promql_range",
    "openobserve_search_around",
    "openobserve_search_sql",
    "openobserve_stream_list",
    "openobserve_stream_schema",
    "openobserve_trace_dag",
];

/// The one prompt line that carries the given instruction, so an assertion can
/// not be satisfied by the same words appearing in a different sentence.
fn paragraph_containing(prompt: &str, needle: &str) -> String {
    prompt
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no prompt line contains {needle:?}:\n{prompt}"))
        .to_string()
}

#[test]
fn prompt_places_the_stream_for_every_tool_the_upstream_declares() {
    let mut facts = prompt_facts();
    facts.tool_names = REAL_UPSTREAM_TOOLS
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let prompt = system_prompt(&facts);

    let as_argument = paragraph_containing(&prompt, "take the stream as an argument");
    for tool in [
        "openobserve_latest_traces",
        "openobserve_search_around",
        "openobserve_stream_schema",
        "openobserve_trace_dag",
    ] {
        assert!(
            as_argument.contains(tool),
            "{tool} requires `stream_name` upstream, so the prompt has to list it among \
             the tools that pass the stream as an argument; that line reads: {as_argument}"
        );
    }

    let no_stream = paragraph_containing(&prompt, "take no stream argument");
    for tool in [
        "openobserve_promql_query",
        "openobserve_promql_range",
        "openobserve_stream_list",
    ] {
        assert!(
            no_stream.contains(tool),
            "{tool} has no stream parameter upstream, so the prompt has to say so; that \
             line reads: {no_stream}"
        );
    }
    for tool in ["openobserve_latest_traces", "openobserve_search_sql"] {
        assert!(
            !no_stream.contains(tool),
            "{tool} does take a stream upstream; telling the model otherwise makes it \
             drop the only required parameter"
        );
    }

    assert!(
        paragraph_containing(&prompt, "the stream goes in the SQL itself")
            .contains("openobserve_search_sql"),
        "the SQL guidance has to be attached to the tool it applies to"
    );
}

#[test]
fn prompt_offers_only_the_tools_the_drilldown_declared() {
    let mut facts = prompt_facts();
    facts
        .tool_names
        .retain(|name| name != "openobserve_promql_query");
    let prompt = system_prompt(&facts);
    assert!(
        !prompt.contains("openobserve_promql_query"),
        "a tool the drill-down never declared must not appear in the prompt: the \
         model would call it and spend a refusal on an unknown tool"
    );
    assert!(
        prompt.contains("openobserve_search_sql"),
        "declared tools still have to be listed"
    );

    let mut none = prompt_facts();
    none.tool_names.clear();
    let prompt = system_prompt(&none);
    assert!(
        !prompt.contains("app_logs"),
        "with no declared tools the prompt cannot claim any stream is readable"
    );
    assert!(
        prompt.contains("no drill-down tools are available"),
        "no declared tools must be stated, not left for the model to discover"
    );
    assert!(
        !prompt.contains("openobserve_search_sql"),
        "the tool list must be built from the declarations, not hardcoded"
    );
}

#[test]
fn prompt_states_a_deadline_of_at_least_one_second() {
    let mut facts = prompt_facts();
    facts.timeout = Duration::from_millis(500);
    let prompt = system_prompt(&facts);
    assert!(
        prompt.contains("1 second") && !prompt.contains("0 seconds"),
        "a sub-second deadline must not be rendered as zero, which the loop does \
         not enforce as zero"
    );
}

// ------------------------------------------------------ forensic agent loop

use std::sync::Mutex;
use std::time::Duration;

use adk_rust::model::{ScriptedLlm, ScriptedResponse};
use async_trait::async_trait;
use doggy_dns_console::analysis::agent::{MAX_TOOL_RESULT_BYTES, SUBMIT_TOOL, run_forensic_loop};
use doggy_dns_console::analysis::drilldown::{Drilldown, DrilldownStreams, upstream_tool_name};
use serde_json::Value;

struct ScriptedDrilldown {
    calls: Mutex<Vec<(String, Value)>>,
    result: Value,
    error: Option<String>,
    delay: Option<Duration>,
    /// Number of leading calls that report an error before `result` is returned.
    fail_first: usize,
    streams: DrilldownStreams,
    declarations: Vec<Value>,
}

impl ScriptedDrilldown {
    fn new(result: Value) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            result,
            error: None,
            delay: None,
            fail_first: 0,
            streams: DrilldownStreams::default(),
            declarations: default_declarations(),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            result: json!({}),
            error: Some(message.to_string()),
            delay: None,
            fail_first: usize::MAX,
            streams: DrilldownStreams::default(),
            declarations: default_declarations(),
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Fails the first `error_first` calls, then returns `result`. Models hit
    /// real argument and stream-name errors before a query succeeds, so the loop
    /// has to distinguish those attempts from ones that returned evidence.
    fn flaky(error_first: usize, result: Value) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            result,
            error: None,
            delay: None,
            fail_first: error_first,
            streams: DrilldownStreams::default(),
            declarations: default_declarations(),
        }
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    fn with_streams(mut self, log_stream: &str, trace_stream: &str) -> Self {
        self.streams = DrilldownStreams {
            log_stream: Some(log_stream.to_string()),
            trace_stream: Some(trace_stream.to_string()),
        };
        self
    }

    /// Simulates tool discovery failing: the streams are configured but no tool
    /// declaration reached the loop.
    fn without_tools(mut self) -> Self {
        self.declarations = Vec::new();
        self
    }
}

#[async_trait]
impl Drilldown for ScriptedDrilldown {
    fn tool_declarations(&self) -> Vec<Value> {
        self.declarations.clone()
    }

    fn streams(&self) -> DrilldownStreams {
        self.streams.clone()
    }

    async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value, String> {
        let attempt = {
            let mut calls = self.calls.lock().unwrap();
            calls.push((tool_name.to_string(), args));
            calls.len()
        };
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if attempt <= self.fail_first {
            return Err("Search stream not found: logs".to_string());
        }
        Ok(self.result.clone())
    }
}

fn default_declarations() -> Vec<Value> {
    SCRIPTED_TOOLS
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "description": format!("read-only drill-down: {name}"),
                "parameters": {"type": "object"}
            })
        })
        .collect()
}

fn submit(result: &AnalysisResult) -> ScriptedResponse {
    ScriptedResponse::function_call(SUBMIT_TOOL, serde_json::to_value(result).unwrap())
}

fn first_log_id(bundle: &EvidenceBundle) -> String {
    bundle.logs.items.first().unwrap().id.clone()
}

/// A valid result that cites a bundle log plus a drill-down result. An
/// `observed` finding needs the bundle citation (spec §6.2); the tool ID proves
/// drill-down IDs are accepted when bundled evidence backs them.
fn drilled_result(bundle: &EvidenceBundle, tool_id: &str) -> AnalysisResult {
    let mut result = valid_result(&first_log_id(bundle));
    result.findings[0].evidence_ids.push(tool_id.to_string());
    result
}

fn function_responses(request: &adk_rust::LlmRequest) -> Vec<Value> {
    request
        .contents
        .iter()
        .flat_map(|content| &content.parts)
        .filter_map(|part| match part {
            adk_rust::Part::FunctionResponse {
                function_response, ..
            } => Some(function_response.response.clone()),
            _ => None,
        })
        .collect()
}

fn system_text(request: &adk_rust::LlmRequest) -> String {
    request
        .contents
        .iter()
        .filter(|content| content.role == "system")
        .flat_map(|content| &content.parts)
        .filter_map(|part| match part {
            adk_rust::Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
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
async fn forensic_loop_puts_the_drilldown_streams_in_the_prompt() {
    let bundle = fixture_bundle().await;
    let drilldown =
        ScriptedDrilldown::new(json!({"rows": []})).with_streams("doggy_dns", "doggy_dns_trace");
    let llm = ScriptedLlm::new("test", vec![submit(&valid_result(&first_log_id(&bundle)))]);

    run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    let prompt = system_text(&llm.requests()[0]);
    assert!(
        prompt.contains("doggy_dns") && prompt.contains("doggy_dns_trace"),
        "the model has to be told the stream names the drill-down reads"
    );

    let without = ScriptedDrilldown::new(json!({"rows": []}));
    let llm = ScriptedLlm::new("test", vec![submit(&valid_result(&first_log_id(&bundle)))]);
    run_forensic_loop(&llm, &bundle, &without, 6, Duration::from_secs(60))
        .await
        .unwrap();
    assert!(
        system_text(&llm.requests()[0]).contains("no stream is configured"),
        "a drill-down that reports no streams must be stated, not left to guesswork"
    );
}

#[tokio::test]
async fn forensic_loop_does_not_promise_tools_the_drilldown_never_discovered() {
    // Tool discovery fails when the MCP server is unreachable, and the drilldown
    // then holds an empty declaration list while the streams stay configured. A
    // prompt that still advertises them turns every call into an unknown tool.
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": []}))
        .with_streams("app_logs", "app_traces")
        .without_tools();
    let llm = ScriptedLlm::new("test", vec![submit(&valid_result(&first_log_id(&bundle)))]);

    run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    let prompt = system_text(&llm.requests()[0]);
    assert!(
        !prompt.contains("app_logs") && prompt.contains("no drill-down tools are available"),
        "the prompt must follow the tool list it was given, not the config alone"
    );
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
            submit(&drilled_result(&bundle, "tool:1:result")),
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
    script.push(submit(&drilled_result(&bundle, "tool:6:result")));
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
    assert!(
        outcome
            .result
            .not_determinable
            .iter()
            .any(|gap| gap.contains("budget exhausted"))
    );
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
            submit(&drilled_result(&bundle, "tool:1:result")),
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
async fn forensic_loop_does_not_make_a_failed_drilldown_citable() {
    let bundle = fixture_bundle().await;
    let log_id = first_log_id(&bundle);
    // Call 1 errors, call 2 succeeds: only the second produced evidence, so only
    // `tool:2:result` may be cited.
    let drilldown = ScriptedDrilldown::flaky(1, json!({"rows": []}));
    let mut bad = drilled_result(&bundle, "tool:1:result");
    bad.findings[0].evidence_ids = vec![log_id.clone(), "tool:1:result".to_string()];
    let good = drilled_result(&bundle, "tool:2:result");
    let llm = ScriptedLlm::new(
        "test",
        vec![
            ScriptedResponse::function_call("openobserve_search_sql", json!({"q": 1})),
            ScriptedResponse::function_call("openobserve_search_sql", json!({"q": 2})),
            submit(&bad),
            submit(&good),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    let after_failed_call = serde_json::to_string(&function_responses(&llm.requests()[1])).unwrap();
    assert!(
        !after_failed_call.contains("tool:1:result"),
        "an attempt that returned only an error must not be handed back a citable \
         evidence ID: {after_failed_call}"
    );
    let after_good_call = serde_json::to_string(&function_responses(&llm.requests()[2])).unwrap();
    assert!(
        after_good_call.contains("tool:2:result"),
        "the call that did return evidence stays citable: {after_good_call}"
    );

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(
        outcome.result.findings[0].evidence_ids,
        vec![log_id, "tool:2:result".to_string()],
        "the submission that cited the failed attempt must have been refused"
    );
}

#[tokio::test]
async fn forensic_loop_gives_failed_drilldowns_their_own_cap() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::failing("Search stream not found: logs");
    let mut script: Vec<ScriptedResponse> = (0..6)
        .map(|index| {
            ScriptedResponse::function_call(
                "openobserve_search_sql",
                json!({"query": format!("SELECT {index}")}),
            )
        })
        .collect();
    // Nothing this attempt ran returned evidence, so the verdict can only rest on
    // the bundle.
    script.push(submit(&valid_result(&first_log_id(&bundle))));
    let llm = ScriptedLlm::new("test", script);

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    // Four attempts that return no evidence are allowed; the fifth is refused, so
    // a model stuck on a bad stream name stops reaching the telemetry source. The
    // refusal still answers every later call, so only the deadline bounds how many
    // turns it can keep burning.
    assert_eq!(drilldown.calls().len(), 4);
    assert_eq!(outcome.tool_calls.len(), 4);
    let refusal = llm
        .requests()
        .iter()
        .rev()
        .flat_map(|request| request.contents.iter().rev())
        .flat_map(|content| content.parts.iter().rev())
        .find_map(|part| match part {
            adk_rust::Part::FunctionResponse {
                function_response, ..
            } => Some(function_response.response.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        refusal.to_string().contains("produced no evidence"),
        "the refusal must name the reason: {refusal}"
    );
    assert!(
        outcome
            .result
            .not_determinable
            .iter()
            .any(|gap| gap.contains("produced no evidence")),
        "a submission after a refusal must disclose the gap"
    );
}

#[tokio::test]
async fn forensic_loop_does_not_bill_the_budget_for_failed_drilldowns() {
    let bundle = fixture_bundle().await;
    // Three argument errors (the stream name the model guessed does not exist)
    // followed by four queries that do return evidence.
    let drilldown = ScriptedDrilldown::flaky(3, json!({"rows": []}));
    let mut script: Vec<ScriptedResponse> = (0..7)
        .map(|index| {
            ScriptedResponse::function_call(
                "openobserve_search_sql",
                json!({"query": format!("SELECT {index}")}),
            )
        })
        .collect();
    script.push(submit(&drilled_result(&bundle, "tool:7:result")));
    let llm = ScriptedLlm::new("test", script);

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(
        drilldown.calls().len(),
        7,
        "only the four calls that returned evidence may spend the 6-call budget"
    );
    assert_eq!(outcome.tool_calls.len(), 7);
    assert!(
        !outcome
            .result
            .not_determinable
            .iter()
            .any(|gap| gap.contains("budget exhausted")),
        "the verdict must not claim an exhausted budget it never hit: {:?}",
        outcome.result.not_determinable
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
    assert!(
        function_responses(&llm.requests()[1])
            .iter()
            .any(|response| response.to_string().contains("unknown evidence ID"))
    );
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
    assert!(
        outcome
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("log:missing-two")
    );
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
            // The cap shortens the preview, not the call: it returned evidence, so
            // citing `tool:1:result` has to stay valid even though the model only
            // saw the first 8 KB of it.
            submit(&drilled_result(&bundle, "tool:1:result")),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert!(outcome.tool_calls[0].summary.len() <= MAX_TOOL_RESULT_BYTES);
    assert!(outcome.tool_calls[0].summary.contains("\"truncated\":true"));
    assert!(!outcome.tool_calls[0].summary.contains("xxxxxxxxxx"));
    let model_payload = function_responses(&llm.requests()[1])
        .into_iter()
        .next()
        .unwrap();
    assert!(serde_json::to_string(&model_payload).unwrap().len() < MAX_TOOL_RESULT_BYTES + 256);
}

#[tokio::test]
async fn forensic_loop_audits_tool_errors() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::failing("openobserve unavailable");
    let llm = ScriptedLlm::new(
        "test",
        vec![
            ScriptedResponse::function_call("openobserve_search_sql", json!({"query": "SELECT 1"})),
            // The one call errored, so the verdict rests on the bundle alone.
            submit(&valid_result(&first_log_id(&bundle))),
        ],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_secs(60))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Ok);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert!(
        outcome.tool_calls[0]
            .summary
            .contains("openobserve unavailable")
    );
    assert_eq!(drilldown.calls().len(), 1);
}

#[tokio::test]
async fn forensic_loop_audits_a_timed_out_drilldown() {
    let bundle = fixture_bundle().await;
    let drilldown = ScriptedDrilldown::new(json!({"rows": []})).with_delay(Duration::from_secs(2));
    let llm = ScriptedLlm::new(
        "test",
        vec![ScriptedResponse::function_call(
            "openobserve_search_sql",
            json!({"query": "SELECT 1"}),
        )],
    );

    let outcome = run_forensic_loop(&llm, &bundle, &drilldown, 6, Duration::from_millis(300))
        .await
        .unwrap();

    assert_eq!(outcome.result.status, AnalysisStatus::Unavailable);
    assert_eq!(drilldown.calls().len(), 1);
    assert_eq!(outcome.tool_calls.len(), 1);
    assert!(
        outcome
            .result
            .not_determinable
            .iter()
            .any(|gap| gap.contains("timed out"))
    );
}

// ---------------------------------------------------------- investigation store

use doggy_dns_console::analysis::ToolCallAudit;
use doggy_dns_console::analysis::store::{
    CacheKey, InvestigationMarker, InvestigationOutcome, InvestigationRecord, InvestigationStatus,
    InvestigationStore, NewInvestigation, SqliteInvestigationStore,
};
use doggy_dns_console::domain::Window;
use sqlx::Row;

const EVIDENCE_FINGERPRINT: &str = "sha256:evidence-fingerprint";
const MODEL: &str = "test-model";

fn sample_filter() -> FailureFilter {
    FailureFilter {
        window: Window::new(30),
        domain: Some("example.com".to_string()),
        response_code: Some("SERVFAIL".to_string()),
        limit: 20,
    }
}

fn sample_audit() -> ToolCallAudit {
    ToolCallAudit {
        name: "openobserve_search_sql".to_string(),
        canonical_args: json!({"query": "SELECT * FROM logs LIMIT 5"}),
        duration_ms: 42,
        result_hash: "sha256:deadbeef".to_string(),
        summary: "{\"rows\":2}".to_string(),
    }
}

/// A terminal record that is internally consistent: the scalar verdict columns
/// mirror the full result.
fn succeeded_record(
    id: &str,
    trace_id: &str,
    attempt: u32,
    created_at: i64,
) -> InvestigationRecord {
    InvestigationRecord {
        id: id.to_string(),
        trace_id: trace_id.to_string(),
        filter: sample_filter(),
        evidence_fingerprint: EVIDENCE_FINGERPRINT.to_string(),
        model: MODEL.to_string(),
        prompt_version: PROMPT_VERSION.to_string(),
        status: InvestigationStatus::Succeeded,
        verdict_category: Some(VerdictCategory::UpstreamTimeout),
        confidence: Some(Confidence::Medium),
        result: Some(valid_result("log:1761436740000:00f067aa0ba902b7:deadbeef")),
        tool_calls: vec![sample_audit()],
        error: None,
        attempt,
        created_at,
        finished_at: Some(created_at + 5),
    }
}

/// An in-flight record that is internally consistent: no verdict, result or
/// completion time yet.
fn running_record(id: &str, trace_id: &str, attempt: u32, created_at: i64) -> InvestigationRecord {
    let mut record = succeeded_record(id, trace_id, attempt, created_at);
    record.status = InvestigationStatus::Running;
    record.verdict_category = None;
    record.confidence = None;
    record.result = None;
    record.tool_calls.clear();
    record.finished_at = None;
    record
}

fn cache_key<'a>(trace_id: &'a str, fingerprint: &'a str, model: &'a str) -> CacheKey<'a> {
    CacheKey {
        trace_id,
        evidence_fingerprint: fingerprint,
        model,
        prompt_version: PROMPT_VERSION,
    }
}

fn temp_db_url(dir: &tempfile::TempDir) -> String {
    format!("sqlite:{}", dir.path().join("console.db").display())
}

#[tokio::test]
async fn store_inserts_and_reads_an_investigation() {
    // The trait must stay object-safe: the service layer holds an
    // `Arc<dyn InvestigationStore>`.
    let store: Arc<dyn InvestigationStore> =
        Arc::new(SqliteInvestigationStore::memory().await.unwrap());
    let record = succeeded_record("inv-1", "trace-a", 1, 1_761_436_800);

    store.insert(&record).await.unwrap();

    let stored = store.get("inv-1").await.unwrap().unwrap();
    assert_eq!(stored, record);
    assert_eq!(stored.filter, sample_filter());
    assert_eq!(
        stored.verdict_category,
        Some(VerdictCategory::UpstreamTimeout)
    );
    assert_eq!(stored.confidence, Some(Confidence::Medium));
    assert_eq!(stored.tool_calls, vec![sample_audit()]);
    assert_eq!(stored.created_at, 1_761_436_800);
    assert_eq!(stored.finished_at, Some(1_761_436_805));

    assert_eq!(store.latest("trace-a").await.unwrap(), Some(record));
    assert_eq!(store.get("missing").await.unwrap(), None);
    assert_eq!(store.latest("trace-missing").await.unwrap(), None);
}

#[tokio::test]
async fn store_schema_matches_the_spec() {
    let dir = tempfile::TempDir::new().unwrap();
    let url = temp_db_url(&dir);
    let _store = SqliteInvestigationStore::new(&url).await.unwrap();

    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let columns: Vec<String> = sqlx::query("PRAGMA table_info(failure_investigations)")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert_eq!(
        columns,
        vec![
            "id",
            "trace_id",
            "filter_json",
            "evidence_fingerprint",
            "model",
            "prompt_version",
            "status",
            "verdict_category",
            "confidence",
            "result_json",
            "tool_calls_json",
            "error",
            "attempt",
            "created_at",
            "finished_at",
        ]
    );

    let indexes: Vec<String> = sqlx::query("PRAGMA index_list(failure_investigations)")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(indexes.contains(&"idx_investigations_trace".to_string()));
    assert!(indexes.contains(&"idx_investigations_created".to_string()));
}

#[tokio::test]
async fn store_cache_hit_requires_the_full_key() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    let record = succeeded_record("inv-1", "trace-a", 1, 1_761_436_800);
    store.insert(&record).await.unwrap();

    let key = cache_key("trace-a", EVIDENCE_FINGERPRINT, MODEL);
    assert_eq!(store.cache_hit(&key).await.unwrap(), Some(record.clone()));

    assert_eq!(
        store
            .cache_hit(&cache_key("trace-a", EVIDENCE_FINGERPRINT, "other-model"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .cache_hit(&cache_key("trace-a", "sha256:other", MODEL))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .cache_hit(&cache_key("trace-b", EVIDENCE_FINGERPRINT, MODEL))
            .await
            .unwrap(),
        None
    );

    // An in-flight attempt for the same key never shadows the cached success.
    let mut in_flight = succeeded_record("inv-2", "trace-a", 2, 1_761_436_900);
    in_flight.status = InvestigationStatus::Running;
    in_flight.verdict_category = None;
    in_flight.confidence = None;
    in_flight.result = None;
    in_flight.tool_calls.clear();
    in_flight.finished_at = None;
    store.insert(&in_flight).await.unwrap();
    assert_eq!(store.cache_hit(&key).await.unwrap(), Some(record));
}

#[tokio::test]
async fn store_create_attempt_increments_attempt_and_tracks_running() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    let filter = sample_filter();
    let input = NewInvestigation {
        trace_id: "trace-a",
        filter: &filter,
        evidence_fingerprint: EVIDENCE_FINGERPRINT,
        model: MODEL,
        prompt_version: PROMPT_VERSION,
    };

    let first = store.create_attempt(&input).await.unwrap();
    let second = store.create_attempt(&input).await.unwrap();

    assert_eq!(first.attempt, 1);
    assert_eq!(second.attempt, 2);
    assert_eq!(first.status, InvestigationStatus::Queued);
    assert_eq!(second.status, InvestigationStatus::Queued);
    assert_ne!(first.id, second.id);
    assert_eq!(first.filter, filter);
    assert!(first.finished_at.is_none());

    let key = cache_key("trace-a", EVIDENCE_FINGERPRINT, MODEL);
    assert_eq!(store.running(&key).await.unwrap().unwrap().id, second.id);

    // Retire the superseded attempt so only one remains in flight per key.
    store
        .finish(
            &first.id,
            &InvestigationOutcome {
                status: InvestigationStatus::Failed,
                result: None,
                tool_calls: Vec::new(),
                error: Some("superseded by a newer attempt".to_string()),
            },
        )
        .await
        .unwrap();
    store.mark_running(&second.id).await.unwrap();
    assert!(store.mark_running(&second.id).await.is_err());

    let result = valid_result("log:1761436740000:00f067aa0ba902b7:deadbeef");
    store
        .finish(
            &second.id,
            &InvestigationOutcome {
                status: InvestigationStatus::Succeeded,
                result: Some(result.clone()),
                tool_calls: vec![sample_audit()],
                error: None,
            },
        )
        .await
        .unwrap();

    let finished = store.get(&second.id).await.unwrap().unwrap();
    assert_eq!(finished.status, InvestigationStatus::Succeeded);
    assert_eq!(finished.result, Some(result));
    assert_eq!(finished.tool_calls, vec![sample_audit()]);
    assert_eq!(
        finished.verdict_category,
        Some(VerdictCategory::UpstreamTimeout)
    );
    assert_eq!(finished.confidence, Some(Confidence::Medium));
    assert!(finished.finished_at.is_some());
    assert_eq!(store.running(&key).await.unwrap(), None);
}

#[tokio::test]
async fn store_markers_return_the_latest_attempt_per_trace() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    store
        .insert(&succeeded_record("a-1", "trace-a", 1, 1_761_436_800))
        .await
        .unwrap();
    store
        .insert(&succeeded_record("a-2", "trace-a", 2, 1_761_436_900))
        .await
        .unwrap();
    store
        .insert(&succeeded_record("b-1", "trace-b", 1, 1_761_436_850))
        .await
        .unwrap();
    store
        .insert(&succeeded_record("c-1", "trace-c", 1, 1_761_436_860))
        .await
        .unwrap();

    let markers: Vec<InvestigationMarker> = store
        .markers(&["trace-a".to_string(), "trace-b".to_string()])
        .await
        .unwrap();

    assert_eq!(markers.len(), 2);
    let a = markers
        .iter()
        .find(|marker| marker.trace_id == "trace-a")
        .unwrap();
    assert_eq!(a.id, "a-2");
    assert_eq!(a.attempt, 2);
    assert_eq!(a.status, InvestigationStatus::Succeeded);
    assert_eq!(a.verdict_category, Some(VerdictCategory::UpstreamTimeout));
    assert_eq!(a.confidence, Some(Confidence::Medium));
    let b = markers
        .iter()
        .find(|marker| marker.trace_id == "trace-b")
        .unwrap();
    assert_eq!(b.id, "b-1");

    assert!(store.markers(&[]).await.unwrap().is_empty());
}

#[tokio::test]
async fn store_fail_interrupted_fails_only_in_flight_rows() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    let filter = sample_filter();
    let input = NewInvestigation {
        trace_id: "trace-a",
        filter: &filter,
        evidence_fingerprint: EVIDENCE_FINGERPRINT,
        model: MODEL,
        prompt_version: PROMPT_VERSION,
    };
    let queued = store.create_attempt(&input).await.unwrap();
    let succeeded = succeeded_record("done-1", "trace-b", 1, 1_761_436_800);
    store.insert(&succeeded).await.unwrap();

    let affected = store
        .fail_interrupted("interrupted by a previous console process")
        .await
        .unwrap();

    assert_eq!(affected, 1);
    let interrupted = store.get(&queued.id).await.unwrap().unwrap();
    assert_eq!(interrupted.status, InvestigationStatus::Failed);
    assert_eq!(
        interrupted.error.as_deref(),
        Some("interrupted by a previous console process")
    );
    assert!(interrupted.finished_at.is_some());
    assert_eq!(store.get("done-1").await.unwrap(), Some(succeeded));

    // A second sweep has nothing left to fail.
    assert_eq!(store.fail_interrupted("again").await.unwrap(), 0);
}

#[tokio::test]
async fn store_finish_rejects_an_already_terminal_row() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    let record = succeeded_record("inv-1", "trace-a", 1, 1_761_436_800);
    store.insert(&record).await.unwrap();

    let outcome = InvestigationOutcome {
        status: InvestigationStatus::Failed,
        result: None,
        tool_calls: Vec::new(),
        error: Some("late failure".to_string()),
    };
    let error = store.finish("inv-1", &outcome).await.unwrap_err();

    assert!(
        error.contains("already terminal") || error.contains("unknown"),
        "unexpected error: {error}"
    );
    assert_eq!(store.get("inv-1").await.unwrap(), Some(record));

    // An unknown id produces the same guard.
    assert!(store.finish("missing", &outcome).await.is_err());
}

#[tokio::test]
async fn store_prunes_rows_older_than_retention() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    store
        .insert(&succeeded_record("old", "trace-old", 1, now - 31 * 86_400))
        .await
        .unwrap();
    store
        .insert(&succeeded_record("fresh", "trace-fresh", 1, now))
        .await
        .unwrap();

    assert_eq!(store.prune_old(0).await.unwrap(), 0);
    assert_eq!(store.prune_old(30).await.unwrap(), 1);
    assert_eq!(store.get("old").await.unwrap(), None);
    assert!(store.get("fresh").await.unwrap().is_some());
}

#[tokio::test]
async fn store_survives_reopen() {
    let dir = tempfile::TempDir::new().unwrap();
    let url = temp_db_url(&dir);
    let record = succeeded_record("inv-1", "trace-a", 1, 1_761_436_800);

    {
        let store = SqliteInvestigationStore::new(&url).await.unwrap();
        store.insert(&record).await.unwrap();
    }

    let reopened = SqliteInvestigationStore::new(&url).await.unwrap();
    assert_eq!(reopened.get("inv-1").await.unwrap(), Some(record));
}

#[tokio::test]
async fn store_persists_only_structured_tool_audits() {
    let dir = tempfile::TempDir::new().unwrap();
    let url = temp_db_url(&dir);
    let store = SqliteInvestigationStore::new(&url).await.unwrap();

    let filter = sample_filter();
    let input = NewInvestigation {
        trace_id: "trace-a",
        filter: &filter,
        evidence_fingerprint: EVIDENCE_FINGERPRINT,
        model: MODEL,
        prompt_version: PROMPT_VERSION,
    };
    let attempt = store.create_attempt(&input).await.unwrap();
    store.mark_running(&attempt.id).await.unwrap();

    let audits = vec![sample_audit()];
    let result = valid_result("log:1761436740000:00f067aa0ba902b7:deadbeef");
    store
        .finish(
            &attempt.id,
            &InvestigationOutcome {
                status: InvestigationStatus::Succeeded,
                result: Some(result.clone()),
                tool_calls: audits.clone(),
                error: None,
            },
        )
        .await
        .unwrap();

    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let row =
        sqlx::query("SELECT tool_calls_json, result_json FROM failure_investigations WHERE id = ?")
            .bind(&attempt.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let tool_calls_json: Option<String> = row.get("tool_calls_json");
    assert_eq!(
        tool_calls_json.unwrap(),
        serde_json::to_string(&audits).unwrap()
    );

    let result_json: Option<String> = row.get("result_json");
    let stored: AnalysisResult = serde_json::from_str(&result_json.unwrap()).unwrap();
    assert_eq!(stored, result);
}

#[tokio::test]
async fn store_latest_prefers_the_newer_insert_when_timestamps_tie() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    // `created_at` only has second resolution and ids are random UUID v4, so
    // the tie has to be broken by insertion order. The older row carries a
    // lexicographically larger id to prove the id no longer decides.
    let created_at = 1_761_436_800;
    let older = succeeded_record("zzzz-older", "trace-a", 1, created_at);
    let newer = succeeded_record("aaaa-newer", "trace-a", 2, created_at);

    store.insert(&older).await.unwrap();
    store.insert(&newer).await.unwrap();

    assert_eq!(
        store.latest("trace-a").await.unwrap().unwrap().id,
        "aaaa-newer"
    );

    let markers = store.markers(&["trace-a".to_string()]).await.unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].id, "aaaa-newer");
    assert_eq!(markers[0].attempt, 2);
}

#[tokio::test]
async fn store_running_prefers_the_newer_insert_when_timestamps_tie() {
    let store = SqliteInvestigationStore::memory().await.unwrap();
    let created_at = 1_761_436_800;
    let older = running_record("zzzz-older", "trace-a", 1, created_at);
    let newer = running_record("aaaa-newer", "trace-a", 2, created_at);

    store.insert(&older).await.unwrap();
    store.insert(&newer).await.unwrap();

    let key = cache_key("trace-a", EVIDENCE_FINGERPRINT, MODEL);
    assert_eq!(store.running(&key).await.unwrap().unwrap().id, "aaaa-newer");
}

// --------------------------------------------------------- analysis service

use adk_rust::AdkError;
use doggy_dns_console::analysis::service::{AnalysisService, AnalysisStart};
use doggy_dns_console::config::AnalysisConfig;
use doggy_dns_console::error::DataError;

fn analysis_config(rate_per_minute: u64) -> AnalysisConfig {
    AnalysisConfig {
        enabled: true,
        retention_days: 30,
        rate_per_minute,
        timeout: Duration::from_secs(5),
        max_tool_calls: 6,
    }
}

/// A bundle that keeps the fixture evidence but answers to another trace, so
/// one service can exercise several cache keys without them colliding.
fn bundle_named(bundle: &EvidenceBundle, trace_id: &str) -> EvidenceBundle {
    let mut clone = bundle.clone();
    clone.trace_id = trace_id.to_string();
    clone
}

fn succeeded_seed(
    id: &str,
    trace_id: &str,
    fingerprint: &str,
    model: &str,
    prompt_version: &str,
    attempt: u32,
    created_at: i64,
) -> InvestigationRecord {
    InvestigationRecord {
        id: id.to_string(),
        trace_id: trace_id.to_string(),
        filter: sample_filter(),
        evidence_fingerprint: fingerprint.to_string(),
        model: model.to_string(),
        prompt_version: prompt_version.to_string(),
        status: InvestigationStatus::Succeeded,
        verdict_category: Some(VerdictCategory::UpstreamTimeout),
        confidence: Some(Confidence::Medium),
        result: Some(valid_result("log:1761436740000:00f067aa0ba902b7:deadbeef")),
        tool_calls: vec![sample_audit()],
        error: None,
        attempt,
        created_at,
        finished_at: Some(created_at + 5),
    }
}

async fn wait_terminal(service: &AnalysisService, trace_id: &str, id: &str) -> InvestigationRecord {
    for _ in 0..300 {
        let record = service.attempt(trace_id, Some(id)).await.unwrap();
        if matches!(
            record.status,
            InvestigationStatus::Succeeded
                | InvestigationStatus::Failed
                | InvestigationStatus::Invalid
        ) {
            return record;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("analysis did not reach a terminal state");
}

async fn memory_store() -> Arc<dyn InvestigationStore> {
    Arc::new(SqliteInvestigationStore::memory().await.unwrap())
}

#[tokio::test]
async fn service_runs_an_analysis_to_completion() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store = Arc::new(SqliteInvestigationStore::memory().await.unwrap());
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![submit(&valid_result(&first_log_id(&bundle)))],
    ));
    let service = Arc::new(AnalysisService::new(
        store.clone(),
        analysis_config(10),
        Some(llm),
        None,
    ));

    let start = service.start(&bundle, &filter, false).await.unwrap();
    let queued = start.record().clone();
    assert!(matches!(start, AnalysisStart::Started(_)));
    assert!(!queued.id.is_empty());
    assert_eq!(queued.status, InvestigationStatus::Queued);
    assert_eq!(queued.attempt, 1);

    let finished = wait_terminal(&service, &bundle.trace_id, &queued.id).await;
    assert_eq!(finished.status, InvestigationStatus::Succeeded);
    let result = finished.result.as_ref().unwrap();
    assert_eq!(result.status, AnalysisStatus::Ok);
    assert_eq!(finished.attempt, 1);
    assert!(finished.finished_at.is_some());
    assert_eq!(finished.error, None);

    let fetched = service
        .attempt(&bundle.trace_id, Some(&queued.id))
        .await
        .unwrap();
    assert_eq!(fetched, finished);
    let latest = service.attempt(&bundle.trace_id, None).await.unwrap();
    assert_eq!(latest, finished);
}

#[tokio::test]
async fn service_returns_a_cache_hit_without_calling_the_model() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store: Arc<dyn InvestigationStore> = memory_store().await;
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![submit(&valid_result(&first_log_id(&bundle)))],
    ));
    let service = Arc::new(AnalysisService::new(
        store,
        analysis_config(10),
        Some(llm.clone()),
        None,
    ));

    let first = service.start(&bundle, &filter, false).await.unwrap();
    let first_id = first.analysis_id().to_string();
    wait_terminal(&service, &bundle.trace_id, &first_id).await;
    let calls = llm.requests().len();
    assert_eq!(calls, 1);

    let second = service.start(&bundle, &filter, false).await.unwrap();
    assert!(matches!(second, AnalysisStart::Cached(_)));
    assert_eq!(second.analysis_id(), first_id);
    assert_eq!(llm.requests().len(), calls);
}

#[tokio::test]
async fn service_force_creates_a_new_attempt() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store: Arc<dyn InvestigationStore> = memory_store().await;
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![
            submit(&valid_result(&first_log_id(&bundle))),
            submit(&valid_result(&first_log_id(&bundle))),
        ],
    ));
    let service = Arc::new(AnalysisService::new(
        store,
        analysis_config(10),
        Some(llm.clone()),
        None,
    ));

    let first = service.start(&bundle, &filter, false).await.unwrap();
    let first_id = first.analysis_id().to_string();
    wait_terminal(&service, &bundle.trace_id, &first_id).await;

    let forced = service.start(&bundle, &filter, true).await.unwrap();
    let forced_record = forced.record().clone();
    assert!(matches!(forced, AnalysisStart::Started(_)));
    assert_ne!(forced_record.id, first_id);
    assert_eq!(forced_record.attempt, 2);
    assert_eq!(forced_record.status, InvestigationStatus::Queued);
    wait_terminal(&service, &bundle.trace_id, &forced_record.id).await;
    assert_eq!(llm.requests().len(), 2);

    let cached = service.start(&bundle, &filter, false).await.unwrap();
    assert!(matches!(cached, AnalysisStart::Cached(_)));
    assert_eq!(cached.record().attempt, 2);
    assert_eq!(cached.analysis_id(), forced_record.id);
}

#[tokio::test]
async fn service_keys_the_cache_by_fingerprint_model_and_prompt_version() {
    let base = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store: Arc<dyn InvestigationStore> = memory_store().await;
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![
            submit(&valid_result(&first_log_id(&base))),
            submit(&valid_result(&first_log_id(&base))),
            submit(&valid_result(&first_log_id(&base))),
        ],
    ));
    let service = Arc::new(AnalysisService::new(
        store.clone(),
        analysis_config(10),
        Some(llm.clone()),
        None,
    ));

    // Exact key: cached.
    let exact = bundle_named(&base, "trace-exact-key");
    store
        .insert(&succeeded_seed(
            "seed-exact",
            &exact.trace_id,
            &exact.fingerprint,
            "test",
            PROMPT_VERSION,
            1,
            1_761_436_800,
        ))
        .await
        .unwrap();
    let cached = service.start(&exact, &filter, false).await.unwrap();
    assert!(matches!(cached, AnalysisStart::Cached(_)));
    assert_eq!(cached.analysis_id(), "seed-exact");

    // One mismatched component at a time must miss the cache.
    let fingerprint = bundle_named(&base, "trace-fingerprint-key");
    store
        .insert(&succeeded_seed(
            "seed-fingerprint",
            &fingerprint.trace_id,
            "sha256:some-other-evidence",
            "test",
            PROMPT_VERSION,
            1,
            1_761_436_800,
        ))
        .await
        .unwrap();
    let started = service.start(&fingerprint, &filter, false).await.unwrap();
    assert!(matches!(started, AnalysisStart::Started(_)));
    assert_ne!(started.analysis_id(), "seed-fingerprint");
    wait_terminal(&service, &fingerprint.trace_id, started.analysis_id()).await;

    let model = bundle_named(&base, "trace-model-key");
    store
        .insert(&succeeded_seed(
            "seed-model",
            &model.trace_id,
            &model.fingerprint,
            "some-other-model",
            PROMPT_VERSION,
            1,
            1_761_436_800,
        ))
        .await
        .unwrap();
    let started = service.start(&model, &filter, false).await.unwrap();
    assert!(matches!(started, AnalysisStart::Started(_)));
    assert_ne!(started.analysis_id(), "seed-model");
    wait_terminal(&service, &model.trace_id, started.analysis_id()).await;

    let prompt = bundle_named(&base, "trace-prompt-key");
    store
        .insert(&succeeded_seed(
            "seed-prompt",
            &prompt.trace_id,
            &prompt.fingerprint,
            "test",
            "failure-analyst/v0",
            1,
            1_761_436_800,
        ))
        .await
        .unwrap();
    let started = service.start(&prompt, &filter, false).await.unwrap();
    assert!(matches!(started, AnalysisStart::Started(_)));
    assert_ne!(started.analysis_id(), "seed-prompt");
    wait_terminal(&service, &prompt.trace_id, started.analysis_id()).await;

    assert_eq!(llm.requests().len(), 3);
}

#[tokio::test]
async fn service_reuses_the_in_flight_attempt_for_the_same_key() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store: Arc<dyn InvestigationStore> = memory_store().await;
    let drilldown = Arc::new(
        ScriptedDrilldown::new(json!({"rows": []})).with_delay(Duration::from_millis(300)),
    );
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![
            ScriptedResponse::function_call("openobserve_search_sql", json!({"query": "SELECT 1"})),
            submit(&drilled_result(&bundle, "tool:1:result")),
        ],
    ));
    let service = Arc::new(AnalysisService::new(
        store.clone(),
        analysis_config(10),
        Some(llm.clone()),
        Some(drilldown),
    ));

    let first = service.start(&bundle, &filter, false).await.unwrap();
    let first_id = first.analysis_id().to_string();
    assert!(matches!(first, AnalysisStart::Started(_)));

    let second = service.start(&bundle, &filter, false).await.unwrap();
    assert!(matches!(second, AnalysisStart::InFlight(_)));
    assert_eq!(second.analysis_id(), first_id);
    assert!(matches!(
        second.record().status,
        InvestigationStatus::Queued | InvestigationStatus::Running
    ));

    let finished = wait_terminal(&service, &bundle.trace_id, &first_id).await;
    assert_eq!(finished.status, InvestigationStatus::Succeeded);
    assert_eq!(
        store
            .latest(&bundle.trace_id)
            .await
            .unwrap()
            .unwrap()
            .attempt,
        1
    );
    // One drill-down turn plus one submission turn: no second job ran.
    assert_eq!(llm.requests().len(), 2);
}

#[tokio::test]
async fn service_rate_limits_new_jobs_but_not_cache_hits() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store: Arc<dyn InvestigationStore> = memory_store().await;
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![submit(&valid_result(&first_log_id(&bundle)))],
    ));
    let service = Arc::new(AnalysisService::new(
        store,
        analysis_config(1),
        Some(llm),
        None,
    ));

    let first = service.start(&bundle, &filter, false).await.unwrap();
    let first_id = first.analysis_id().to_string();
    wait_terminal(&service, &bundle.trace_id, &first_id).await;

    let cached = service.start(&bundle, &filter, false).await.unwrap();
    assert!(matches!(cached, AnalysisStart::Cached(_)));

    let mut other = bundle.clone();
    other.trace_id = "trace-rate-other".to_string();
    other.fingerprint = "sha256:rate-other".to_string();
    match service.start(&other, &filter, false).await {
        Err(DataError::RateLimited(message)) => {
            assert!(message.contains("rate limit"), "{message}");
        }
        other => panic!("expected a rate limit, got {other:?}"),
    }
}

#[tokio::test]
async fn service_requires_enabled_analysis_and_a_model() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();

    let mut disabled = analysis_config(10);
    disabled.enabled = false;
    let service = Arc::new(AnalysisService::new(
        memory_store().await,
        disabled,
        Some(Arc::new(ScriptedLlm::new(
            "test",
            Vec::<ScriptedResponse>::new(),
        ))),
        None,
    ));
    match service.start(&bundle, &filter, false).await {
        Err(DataError::NotConfigured(message)) => {
            assert!(message.contains("disabled"), "{message}");
        }
        other => panic!("expected NotConfigured for disabled analysis, got {other:?}"),
    }

    let service = Arc::new(AnalysisService::new(
        memory_store().await,
        analysis_config(10),
        None,
        None,
    ));
    match service.start(&bundle, &filter, false).await {
        Err(DataError::NotConfigured(message)) => {
            assert!(message.contains("model"), "{message}");
        }
        other => panic!("expected NotConfigured for a missing model, got {other:?}"),
    }
}

#[tokio::test]
async fn service_reports_invalid_submissions_as_invalid() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store: Arc<dyn InvestigationStore> = memory_store().await;
    let drilldown = Arc::new(ScriptedDrilldown::new(json!({"rows": []})));
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![
            ScriptedResponse::function_call("openobserve_search_sql", json!({"query": "SELECT 1"})),
            submit(&valid_result("log:missing-id")),
            submit(&valid_result("log:missing-id")),
        ],
    ));
    let service = Arc::new(AnalysisService::new(
        store,
        analysis_config(10),
        Some(llm),
        Some(drilldown),
    ));

    let start = service.start(&bundle, &filter, false).await.unwrap();
    let id = start.analysis_id().to_string();
    let record = wait_terminal(&service, &bundle.trace_id, &id).await;

    assert_eq!(record.status, InvestigationStatus::Invalid);
    assert!(record.result.is_none());
    let error = record.error.as_deref().unwrap();
    assert!(error.contains("missing-id"), "{error}");
    // Tool audits survive an invalid terminal job.
    assert_eq!(record.tool_calls.len(), 1);
    assert_eq!(record.tool_calls[0].name, "openobserve_search_sql");
}

#[tokio::test]
async fn service_keeps_a_missing_model_result_as_a_successful_unavailable() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store: Arc<dyn InvestigationStore> = memory_store().await;
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![ScriptedResponse::error(AdkError::model("upstream 500"))],
    ));
    let service = Arc::new(AnalysisService::new(
        store,
        analysis_config(10),
        Some(llm),
        None,
    ));

    let start = service.start(&bundle, &filter, false).await.unwrap();
    let id = start.analysis_id().to_string();
    let record = wait_terminal(&service, &bundle.trace_id, &id).await;

    assert_eq!(record.status, InvestigationStatus::Succeeded);
    let result = record.result.as_ref().unwrap();
    assert_eq!(result.status, AnalysisStatus::Unavailable);
    assert!(!result.not_determinable.is_empty());
    assert_eq!(record.error, None);
}

#[tokio::test]
async fn service_recovers_a_stale_running_attempt() {
    let bundle = fixture_bundle().await;
    let filter = FailureFilter::default();
    let store = Arc::new(SqliteInvestigationStore::memory().await.unwrap());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut stale = succeeded_seed(
        "inv-stale",
        &bundle.trace_id,
        &bundle.fingerprint,
        "test",
        PROMPT_VERSION,
        1,
        now - 10_000,
    );
    stale.status = InvestigationStatus::Running;
    stale.verdict_category = None;
    stale.confidence = None;
    stale.result = None;
    stale.tool_calls.clear();
    stale.finished_at = None;
    store.insert(&stale).await.unwrap();

    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![submit(&valid_result(&first_log_id(&bundle)))],
    ));
    let service = Arc::new(AnalysisService::new(
        store.clone(),
        analysis_config(10),
        Some(llm),
        None,
    ));

    let start = service.start(&bundle, &filter, false).await.unwrap();
    let fresh = start.record().clone();
    assert!(matches!(start, AnalysisStart::Started(_)));
    assert_eq!(fresh.attempt, 2);

    let recovered = store.get("inv-stale").await.unwrap().unwrap();
    assert_eq!(recovered.status, InvestigationStatus::Failed);
    assert_eq!(
        recovered.error.as_deref(),
        Some("stale running attempt recovered")
    );

    let finished = wait_terminal(&service, &bundle.trace_id, &fresh.id).await;
    assert_eq!(finished.status, InvestigationStatus::Succeeded);
}
