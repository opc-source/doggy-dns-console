use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use doggy_dns_console::api::state::{ApiState, Probe};
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceControl, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{LogLevelChange, LogLevelState};
use doggy_dns_console::error::DataError;
use doggy_dns_console::service::ConsoleService;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const NOW: i64 = 1_761_436_800_000;

struct AlwaysHealthy;

#[async_trait]
impl Probe for AlwaysHealthy {
    async fn probe(&self) -> Result<(), DataError> {
        Ok(())
    }
}

fn fixture() -> Arc<FixtureSource> {
    Arc::new(FixtureSource::new(Arc::new(FixedClock(NOW))))
}

fn router(service: ConsoleService, instances: Arc<dyn InstanceSource>) -> axum::Router {
    doggy_dns_console::api::business_router(ApiState {
        service: Arc::new(service),
        instances,
        openobserve_health: Arc::new(AlwaysHealthy),
        openobserve_mcp_health: None,
        model_configured: true,
        analysis: None,
    })
}

fn app_with_control() -> axum::Router {
    let fixture = fixture();
    let service = ConsoleService::new(
        fixture.clone() as Arc<dyn TelemetrySource>,
        fixture.clone() as Arc<dyn InstanceSource>,
        Arc::new(FixedClock(NOW)),
    )
    .with_control(fixture.clone());
    router(service, fixture as Arc<dyn InstanceSource>)
}

fn app_without_control() -> axum::Router {
    let fixture = fixture();
    let service = ConsoleService::new(
        fixture.clone() as Arc<dyn TelemetrySource>,
        fixture.clone() as Arc<dyn InstanceSource>,
        Arc::new(FixedClock(NOW)),
    );
    router(service, fixture as Arc<dyn InstanceSource>)
}

struct FailingControl;

#[async_trait]
impl InstanceControl for FailingControl {
    async fn set_log_level(
        &self,
        _id: &str,
        _change: LogLevelChange,
    ) -> Result<LogLevelState, DataError> {
        Err(DataError::Control(
            "upstream rejected the change".to_string(),
        ))
    }
}

fn app_with_failing_control() -> axum::Router {
    let fixture = fixture();
    let service = ConsoleService::new(
        fixture.clone() as Arc<dyn TelemetrySource>,
        fixture.clone() as Arc<dyn InstanceSource>,
        Arc::new(FixedClock(NOW)),
    )
    .with_control(Arc::new(FailingControl));
    router(service, fixture as Arc<dyn InstanceSource>)
}

async fn status_body(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn post_log_level(app: axum::Router, id: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::post(format!("/api/instances/{id}/log-level"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    status_body(response).await
}

async fn detail(app: axum::Router, id: &str) -> Value {
    let response = app
        .oneshot(
            Request::get(format!("/api/instances/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    status_body(response).await.1
}

#[tokio::test]
async fn detail_reports_whether_runtime_control_is_enabled() {
    assert_eq!(
        detail(app_with_control(), "dns-node-1").await["controlEnabled"],
        true
    );
    assert_eq!(
        detail(app_without_control(), "dns-node-1").await["controlEnabled"],
        false
    );
}

#[tokio::test]
async fn log_level_sets_directives_and_the_detail_shows_them() {
    let app = app_with_control();
    let (status, body) = post_log_level(
        app.clone(),
        "dns-node-1",
        json!({ "directives": "doggy_dns_nacos=debug" }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "stdout": "doggy_dns_nacos=debug", "otlp": "doggy_dns_nacos=debug" })
    );
    assert_eq!(
        detail(app, "dns-node-1").await["runtime"]["logs"]["stdout"],
        "doggy_dns_nacos=debug"
    );
}

#[tokio::test]
async fn log_level_resets_to_the_startup_filters() {
    let app = app_with_control();
    let (status, first) = post_log_level(
        app.clone(),
        "dns-node-1",
        json!({ "directives": "doggy_dns_nacos=debug" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");

    let (status, body) = post_log_level(app, "dns-node-1", json!({ "reset": true })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "stdout": "info", "otlp": "info" }));
}

#[tokio::test]
async fn log_level_rejects_both_and_neither() {
    for body in [json!({ "directives": "debug", "reset": true }), json!({})] {
        let (status, answer) = post_log_level(app_with_control(), "dns-node-1", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(answer["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn log_level_rejects_empty_directives() {
    let (status, answer) = post_log_level(
        app_with_control(),
        "dns-node-1",
        json!({ "directives": "   " }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn log_level_is_not_configured_without_a_control_handle() {
    let (status, body) = post_log_level(
        app_without_control(),
        "dns-node-1",
        json!({ "reset": true }),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"]["code"], "not_configured");
}

#[tokio::test]
async fn log_level_returns_not_found_for_an_unknown_instance() {
    let (status, body) =
        post_log_level(app_with_control(), "missing", json!({ "reset": true })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn log_level_maps_a_control_failure_to_bad_gateway() {
    let (status, body) = post_log_level(
        app_with_failing_control(),
        "dns-node-1",
        json!({ "reset": true }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["error"]["code"], "mcp_control_failed");
}

async fn post_raw(app: axum::Router, id: &str, body: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::post(format!("/api/instances/{id}/log-level"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn log_level_rejects_a_malformed_body_as_invalid_request() {
    for body in ["{\"directives\": 5}", "{\"reset\": \"yes\"}", "not json"] {
        let (status, answer) = post_raw(app_with_control(), "dns-node-1", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
        assert_eq!(
            answer["error"]["code"], "invalid_request",
            "{body}: {answer}"
        );
    }
}
