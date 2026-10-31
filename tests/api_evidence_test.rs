use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use doggy_dns_console::api::state::{ApiState, Probe};
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{
    FailureDetail, FailureFilter, FailureGroup, Overview, SeenInstance, Window,
};
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

/// Telemetry stub whose only configured behavior is an upstream failure, used
/// to prove the evidence endpoint maps `DataError::Upstream` to 502.
struct FailingTelemetry;

#[async_trait]
impl TelemetrySource for FailingTelemetry {
    async fn overview(&self, _window: Window) -> Result<Overview, DataError> {
        Err(DataError::Upstream("connection refused".to_string()))
    }

    async fn failures(&self, _filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        Err(DataError::Upstream("connection refused".to_string()))
    }

    async fn failure_detail(
        &self,
        _trace_id: &str,
        _filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
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
        openobserve_mcp_health: None,
        model_configured: true,
    })
}

async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
    app.oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn evidence_endpoint_returns_bundle() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = get(
        app,
        "/api/failures/4bf92f3577b34da6/evidence?windowMinutes=60",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["traceId"], "4bf92f3577b34da6");
    assert!(
        body["fingerprint"]
            .as_str()
            .is_some_and(|fingerprint| fingerprint.starts_with("sha256:"))
    );
    assert_eq!(body["filter"]["windowMinutes"], 60);
    assert!(
        body["logs"]["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );

    // The fixture failure row carries `service_instance_id = dns-node-1`, so
    // the server-derived identity must resolve the exact instance.
    let instance = &body["instance"];
    assert_eq!(instance["identity"]["serviceInstanceId"], "dns-node-1");
    assert_eq!(instance["identity"]["host"], "dns-node-1");
    assert_eq!(instance["reachable"], true);

    let object = body.as_object().unwrap();
    assert!(object.contains_key("generatedAt"));
    assert!(object.contains_key("missing"));
    assert!(!object.contains_key("generated_at"));
}

#[tokio::test]
async fn evidence_endpoint_returns_404_for_unknown_trace() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = get(app, "/api/failures/does-not-exist/evidence").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn evidence_endpoint_returns_404_for_non_failure_trace() {
    // The fixture's NXDOMAIN row is excluded from the failure list.
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = get(app, "/api/failures/6df92f3577b34da8/evidence").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn evidence_endpoint_applies_the_list_filter() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = get(
        app,
        "/api/failures/4bf92f3577b34da6/evidence?responseCode=REFUSED",
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = get(
        app,
        "/api/failures/4bf92f3577b34da6/evidence?responseCode=SERVFAIL&domain=nacos",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["filter"]["responseCode"], "SERVFAIL");
    assert_eq!(body["filter"]["domain"], "nacos");
}

#[tokio::test]
async fn evidence_endpoint_rejects_malformed_query() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = get(
        app,
        "/api/failures/4bf92f3577b34da6/evidence?windowMinutes=abc",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn evidence_endpoint_maps_upstream_failure_to_502() {
    let app = app_with(Arc::new(FailingTelemetry));
    let response = get(app, "/api/failures/4bf92f3577b34da6/evidence").await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "openobserve_unavailable");
}
