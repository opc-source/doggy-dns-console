use std::sync::Arc;
use std::time::Duration;

use adk_rust::model::{ScriptedLlm, ScriptedResponse};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use doggy_dns_console::analysis::agent::SUBMIT_TOOL;
use doggy_dns_console::analysis::service::AnalysisService;
use doggy_dns_console::analysis::store::{
    CacheKey, InvestigationMarker, InvestigationOutcome, InvestigationRecord, InvestigationStore,
    NewInvestigation, SqliteInvestigationStore,
};
use doggy_dns_console::analysis::{
    AnalysisResult, AnalysisStatus, AnalysisVerdict, Confidence, Finding, FindingKind, NextStep,
    PROMPT_VERSION, RiskLevel, VerdictCategory,
};
use doggy_dns_console::api::error::ApiError;
use doggy_dns_console::api::state::{ApiState, Probe};
use doggy_dns_console::config::AnalysisConfig;
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{FailureFilter, Window};
use doggy_dns_console::error::DataError;
use doggy_dns_console::evidence::assemble::assemble;
use doggy_dns_console::evidence::model::EvidenceBundle;
use doggy_dns_console::service::ConsoleService;
use http_body_util::BodyExt;
use tower::ServiceExt;

const NOW_MS: i64 = 1_761_436_800_000;
const DEMO_TRACE: &str = "4bf92f3577b34da6";

struct AlwaysHealthy;

#[async_trait]
impl Probe for AlwaysHealthy {
    async fn probe(&self) -> Result<(), DataError> {
        Ok(())
    }
}

#[tokio::test]
async fn rate_limited_error_maps_to_429() {
    let response = ApiError(DataError::RateLimited("too many requests".into())).into_response();
    assert_eq!(response.status(), 429);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"]["code"], "rate_limited");
}

// --------------------------------------------------------------- harness

fn filter() -> FailureFilter {
    // The same defaults `filter_from(..., 1)` produces for the analysis routes.
    FailureFilter {
        window: Window::new(60),
        domain: None,
        response_code: None,
        limit: 1,
    }
}

fn analysis_config(rate_per_minute: u64) -> AnalysisConfig {
    AnalysisConfig {
        enabled: true,
        retention_days: 30,
        rate_per_minute,
        timeout: Duration::from_secs(5),
        max_tool_calls: 6,
    }
}

async fn fixture_bundle() -> EvidenceBundle {
    let fixture = Arc::new(FixtureSource::new(Arc::new(FixedClock(NOW_MS))));
    let telemetry: Arc<dyn TelemetrySource> = fixture.clone();
    let instances: Arc<dyn InstanceSource> = fixture;
    assemble(
        &*telemetry,
        &*instances,
        DEMO_TRACE,
        &filter(),
        &FixedClock(NOW_MS),
    )
    .await
    .unwrap()
}

/// A minimal result that passes every validation rule for the fixture bundle.
/// Copied from `tests/analysis_test.rs` because integration test files are
/// separate crates.
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

fn submit(result: &AnalysisResult) -> ScriptedResponse {
    ScriptedResponse::function_call(SUBMIT_TOOL, serde_json::to_value(result).unwrap())
}

fn first_log_id(bundle: &EvidenceBundle) -> String {
    bundle.logs.items.first().unwrap().id.clone()
}

fn build_app(analysis: Option<Arc<AnalysisService>>) -> Router {
    let fixture = Arc::new(FixtureSource::new(Arc::new(FixedClock(NOW_MS))));
    let instances: Arc<dyn InstanceSource> = fixture.clone();
    let telemetry: Arc<dyn TelemetrySource> = fixture;
    let service = Arc::new(ConsoleService::new(telemetry, instances.clone()));
    doggy_dns_console::api::business_router(ApiState {
        service,
        instances,
        openobserve_health: Arc::new(AlwaysHealthy),
        openobserve_mcp_health: None,
        model_configured: true,
        analysis,
    })
}

struct Harness {
    app: Router,
    llm: Arc<ScriptedLlm>,
    store: Arc<dyn InvestigationStore>,
    trace: String,
    bundle: EvidenceBundle,
}

/// Builds the fixture-backed app with a real in-memory investigation store.
/// The default script always submits a valid result citing the fixture's first
/// log; `script` appends further model turns.
async fn harness(rate_per_minute: u64, script: Vec<ScriptedResponse>) -> Harness {
    let bundle = fixture_bundle().await;
    let store: Arc<dyn InvestigationStore> =
        Arc::new(SqliteInvestigationStore::memory().await.unwrap());
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![submit(&valid_result(&first_log_id(&bundle)))],
    ));
    for response in script {
        llm.push(response);
    }
    let analysis = Arc::new(AnalysisService::new(
        store.clone(),
        analysis_config(rate_per_minute),
        Some(llm.clone()),
        None,
    ));
    Harness {
        app: build_app(Some(analysis)),
        llm,
        store,
        trace: bundle.trace_id.clone(),
        bundle,
    }
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn request(app: &Router, method: Method, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, json_body(response).await)
}

async fn post(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    request(app, Method::POST, uri).await
}

async fn get(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    request(app, Method::GET, uri).await
}

/// Polls the latest analysis for `trace` until it succeeds. The loop is
/// bounded; the 10 ms steps only pace the check, they are not correctness.
async fn poll_succeeded(app: &Router, trace: &str) -> serde_json::Value {
    let uri = format!("/api/failures/{trace}/analysis?windowMinutes=60");
    poll_uri(app, &uri).await
}

/// Polls one explicit attempt until it succeeds. Used when several attempts
/// share a creation second and "latest" is not a stable discriminator.
async fn poll_analysis_succeeded(
    app: &Router,
    trace: &str,
    analysis_id: &str,
) -> serde_json::Value {
    let uri = format!("/api/failures/{trace}/analysis?analysis={analysis_id}");
    poll_uri(app, &uri).await
}

async fn poll_uri(app: &Router, uri: &str) -> serde_json::Value {
    for _ in 0..500 {
        let (status, body) = get(app, uri).await;
        if status == StatusCode::OK && body["status"] == "succeeded" {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("analysis did not reach a succeeded state");
}

fn start_uri(h: &Harness) -> String {
    format!("/api/failures/{}/analysis?windowMinutes=60", h.trace)
}

// ------------------------------------------------------------------ tests

#[tokio::test]
async fn post_starts_a_job_and_get_reports_the_snapshot() {
    let h = harness(10, Vec::new()).await;
    let (status, body) = post(&h.app, &start_uri(&h)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let object = body.as_object().unwrap();
    assert_eq!(object.len(), 1, "202 must return exactly analysisId");
    let analysis_id = body["analysisId"].as_str().unwrap().to_string();

    let snapshot = poll_succeeded(&h.app, &h.trace).await;
    assert_eq!(snapshot["analysisId"], analysis_id);
    assert_eq!(snapshot["traceId"], h.trace);
    assert_eq!(snapshot["status"], "succeeded");
    assert_eq!(snapshot["attempt"], 1);
    assert_eq!(snapshot["toolCallCount"], 0);
    assert_eq!(snapshot["model"], "test");
    assert_eq!(snapshot["promptVersion"], PROMPT_VERSION);
    let started_at = snapshot["startedAt"].as_i64().unwrap();
    let finished_at = snapshot["finishedAt"].as_i64().unwrap();
    // The wire unit is epoch milliseconds, like evidence timestamps.
    assert!(
        started_at >= 1_000_000_000_000,
        "startedAt must be milliseconds, got {started_at}"
    );
    assert_eq!(finished_at % 1000, 0, "seconds leaked onto the wire");
    assert!(finished_at >= started_at);
    assert_eq!(snapshot["filter"]["windowMinutes"], 60);
    assert_eq!(snapshot["error"], serde_json::Value::Null);
    assert_eq!(snapshot["result"]["status"], "ok");
    assert!(
        snapshot["evidenceFingerprint"]
            .as_str()
            .is_some_and(|fingerprint| fingerprint.starts_with("sha256:"))
    );
    assert_eq!(snapshot["toolCalls"], serde_json::json!([]));
}

#[tokio::test]
async fn post_returns_200_and_the_same_snapshot_for_a_cache_hit() {
    let h = harness(10, Vec::new()).await;
    let (status, body) = post(&h.app, &start_uri(&h)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let analysis_id = body["analysisId"].as_str().unwrap().to_string();
    poll_succeeded(&h.app, &h.trace).await;
    let calls = h.llm.requests().len();

    let (status, body) = post(&h.app, &start_uri(&h)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["analysisId"], analysis_id);
    assert_eq!(body["status"], "succeeded");
    assert_eq!(h.llm.requests().len(), calls);
}

#[tokio::test]
async fn post_force_creates_a_new_attempt() {
    let h = harness(10, Vec::new()).await;
    let (status, body) = post(&h.app, &start_uri(&h)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let first_id = body["analysisId"].as_str().unwrap().to_string();
    poll_succeeded(&h.app, &h.trace).await;

    h.llm.push(submit(&valid_result(&first_log_id(&h.bundle))));
    let (status, body) = post(&h.app, &format!("{}&force=true", start_uri(&h))).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let second_id = body["analysisId"].as_str().unwrap().to_string();
    assert_ne!(second_id, first_id);

    let snapshot = poll_analysis_succeeded(&h.app, &h.trace, &second_id).await;
    assert_eq!(snapshot["analysisId"], second_id);
    assert_eq!(snapshot["attempt"], 2);
    assert_eq!(h.store.get(&second_id).await.unwrap().unwrap().attempt, 2);
}

#[tokio::test]
async fn post_is_503_without_an_analysis_service() {
    let app = build_app(None);
    let (status, body) = post(&app, &format!("/api/failures/{DEMO_TRACE}/analysis")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "not_configured");
}

#[tokio::test]
async fn post_is_429_when_the_rate_limit_is_exhausted() {
    let h = harness(1, Vec::new()).await;
    let (status, _) = post(&h.app, &start_uri(&h)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    poll_succeeded(&h.app, &h.trace).await;

    let (status, body) = post(&h.app, &format!("{}&force=true", start_uri(&h))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
}

#[tokio::test]
async fn post_unknown_trace_is_404() {
    let h = harness(10, Vec::new()).await;
    let (status, body) = post(&h.app, "/api/failures/does-not-exist/analysis").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn get_unknown_analysis_is_404() {
    let h = harness(10, Vec::new()).await;
    let (status, body) = get(
        &h.app,
        &format!("/api/failures/{}/analysis?analysis=missing", h.trace),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn markers_return_latest_attempts_and_skip_unknown_traces() {
    let h = harness(10, Vec::new()).await;
    let (status, _) = post(&h.app, &start_uri(&h)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    poll_succeeded(&h.app, &h.trace).await;

    let (status, body) = get(
        &h.app,
        &format!("/api/analysis/markers?traceIds={},missing", h.trace),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let markers = body.as_array().unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0]["traceId"], h.trace);
    assert_eq!(markers[0]["status"], "succeeded");
    assert_eq!(markers[0]["attempt"], 1);
    assert!(markers[0].get("result").is_none());
    // Markers use the same millisecond unit as the snapshot API.
    let created_at = markers[0]["createdAt"].as_i64().unwrap();
    let finished_at = markers[0]["finishedAt"].as_i64().unwrap();
    assert!(
        created_at >= 1_000_000_000_000,
        "createdAt must be milliseconds, got {created_at}"
    );
    assert_eq!(finished_at % 1000, 0, "seconds leaked onto the wire");
}

#[tokio::test]
async fn markers_reject_more_than_200_ids() {
    let h = harness(10, Vec::new()).await;
    let ids: Vec<String> = (0..201).map(|index| format!("trace-{index}")).collect();
    let (status, body) = get(
        &h.app,
        &format!("/api/analysis/markers?traceIds={}", ids.join(",")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");
}

// ------------------------------------------------------------ failing store

struct FailingStore;

fn unavailable() -> String {
    "database unavailable".to_string()
}

#[async_trait]
impl InvestigationStore for FailingStore {
    async fn insert(&self, _record: &InvestigationRecord) -> Result<(), String> {
        Err(unavailable())
    }

    async fn create_attempt(
        &self,
        _input: &NewInvestigation<'_>,
    ) -> Result<InvestigationRecord, String> {
        Err(unavailable())
    }

    async fn mark_running(&self, _id: &str) -> Result<(), String> {
        Err(unavailable())
    }

    async fn finish(&self, _id: &str, _outcome: &InvestigationOutcome) -> Result<(), String> {
        Err(unavailable())
    }

    async fn get(&self, _id: &str) -> Result<Option<InvestigationRecord>, String> {
        Err(unavailable())
    }

    async fn latest(&self, _trace_id: &str) -> Result<Option<InvestigationRecord>, String> {
        Err(unavailable())
    }

    async fn cache_hit(&self, _key: &CacheKey<'_>) -> Result<Option<InvestigationRecord>, String> {
        Err(unavailable())
    }

    async fn running(&self, _key: &CacheKey<'_>) -> Result<Option<InvestigationRecord>, String> {
        Err(unavailable())
    }

    async fn markers(&self, _trace_ids: &[String]) -> Result<Vec<InvestigationMarker>, String> {
        Err(unavailable())
    }

    async fn prune_old(&self, _retention_days: u32) -> Result<u64, String> {
        Err(unavailable())
    }
}

#[tokio::test]
async fn store_failures_map_to_500_internal_error() {
    let bundle = fixture_bundle().await;
    let llm = Arc::new(ScriptedLlm::new(
        "test",
        vec![submit(&valid_result(&first_log_id(&bundle)))],
    ));
    let analysis = Arc::new(AnalysisService::new(
        Arc::new(FailingStore),
        analysis_config(10),
        Some(llm),
        None,
    ));
    let app = build_app(Some(analysis));

    let (status, body) = post(&app, &format!("/api/failures/{DEMO_TRACE}/analysis")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"]["code"], "internal_error");
}
