use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use doggy_dns_console::api::state::{ApiState, Probe};
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{FailureDetail, FailureFilter, FailureGroup, Overview, SeenInstance, Window};
use doggy_dns_console::error::DataError;
use doggy_dns_console::service::ConsoleService;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct AlwaysHealthy;

#[async_trait]
impl Probe for AlwaysHealthy {
    async fn probe(&self) -> Result<(), DataError> {
        Ok(())
    }
}

struct FailingTelemetry;

#[async_trait]
impl TelemetrySource for FailingTelemetry {
    async fn overview(&self, _window: Window) -> Result<Overview, DataError> {
        Err(DataError::Upstream("connection refused".to_string()))
    }

    async fn failures(&self, _filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        Err(DataError::Upstream("connection refused".to_string()))
    }

    async fn failure_detail(&self, _trace_id: &str) -> Result<FailureDetail, DataError> {
        Err(DataError::Upstream("connection refused".to_string()))
    }

    async fn seen_instances(&self, _window: Window) -> Result<Vec<SeenInstance>, DataError> {
        Err(DataError::Upstream("connection refused".to_string()))
    }
}

fn fixture() -> Arc<FixtureSource> {
    Arc::new(FixtureSource::new(Arc::new(FixedClock(1_761_436_800_000))))
}

fn app_with(telemetry: Arc<dyn TelemetrySource>) -> axum::Router {
    let fixture = fixture();
    let instances = fixture.clone() as Arc<dyn InstanceSource>;
    let service = Arc::new(ConsoleService::new(telemetry, instances.clone()));
    doggy_dns_console::api::business_router(ApiState {
        service,
        instances,
        openobserve_health: Arc::new(AlwaysHealthy),
        oo_mcp_health: None,
        model_configured: true,
    })
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn overview_returns_camel_case_json() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(Request::get("/api/overview?windowMinutes=60").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["windowMinutes"], 60);
    assert!(body["successRate"].is_number());
    assert_eq!(body["instancesOnline"], 3);
}

#[tokio::test]
async fn overview_maps_upstream_failure_to_502() {
    let app = app_with(Arc::new(FailingTelemetry));
    let response = app
        .oneshot(Request::get("/api/overview").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "openobserve_unavailable");
}

#[tokio::test]
async fn failures_limit_is_clamped_and_returns_list() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(Request::get("/api/failures?limit=9999").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body.as_array().unwrap().len() <= 200);
}

#[tokio::test]
async fn connections_reports_sources() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(Request::get("/api/connections").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["openobserve"]["ok"], true);
    assert_eq!(body["openobserveMcp"]["ok"], false);
    assert_eq!(body["model"]["configured"], true);
}
