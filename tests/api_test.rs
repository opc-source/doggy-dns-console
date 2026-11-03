use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use doggy_dns_console::api::state::{ApiState, Probe};
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{
    FailureDetail, FailureFilter, FailureGroup, Overview, Range, SeenInstance, Window,
};
use doggy_dns_console::error::DataError;
use doggy_dns_console::service::ConsoleService;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

struct AlwaysHealthy;

#[async_trait]
impl Probe for AlwaysHealthy {
    async fn probe(&self) -> Result<(), DataError> {
        Ok(())
    }
}

struct RecordingTelemetry {
    inner: FixtureSource,
    last_filter: Arc<Mutex<Option<FailureFilter>>>,
}

#[async_trait]
impl TelemetrySource for RecordingTelemetry {
    async fn overview(&self, range: Range) -> Result<Overview, DataError> {
        self.inner.overview(range).await
    }
    async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        *self.last_filter.lock().unwrap() = Some(filter.clone());
        self.inner.failures(filter).await
    }
    async fn failure_detail(
        &self,
        trace_id: &str,
        filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
        self.inner.failure_detail(trace_id, filter).await
    }
    async fn unlabeled_failures(&self, filter: FailureFilter) -> Result<u64, DataError> {
        // Record the resolved filter, then delegate: since Task 7 the fixture
        // answers this on the demo path, so the count is the real one.
        *self.last_filter.lock().unwrap() = Some(filter.clone());
        self.inner.unlabeled_failures(filter).await
    }
    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError> {
        self.inner.seen_instances(window).await
    }
}

struct FailingTelemetry;

#[async_trait]
impl TelemetrySource for FailingTelemetry {
    async fn overview(&self, _range: Range) -> Result<Overview, DataError> {
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
    let service = Arc::new(ConsoleService::new(
        telemetry,
        instances.clone(),
        Arc::new(FixedClock(1_761_436_800_000)),
    ));
    doggy_dns_console::api::business_router(ApiState {
        service,
        instances,
        openobserve_health: Arc::new(AlwaysHealthy),
        openobserve_mcp_health: None,
        model_configured: true,
        analysis: None,
    })
}

/// Same fixed `1_761_436_800_000` clock as [`app_with`], so `now_ms` is
/// deterministic and the resolver's future tolerance is testable, plus the
/// recorder handle to read the resolved filter back from.
async fn app_with_recorder() -> (axum::Router, Arc<RecordingTelemetry>) {
    let recorder = Arc::new(RecordingTelemetry {
        inner: FixtureSource::new(Arc::new(FixedClock(1_761_436_800_000))),
        last_filter: Arc::new(Mutex::new(None)),
    });
    let app = app_with(recorder.clone());
    (app, recorder)
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn instance_detail_exposes_runtime_signals_and_redactions() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get("/api/instances/dns-node-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["pid"], 4242);
    assert_eq!(body["status"], "running");
    assert_eq!(
        body["runtime"]["signals"]["nacos.nacos.local"]["cachedServices"],
        42
    );
    assert!(
        body["redactedKeys"]
            .as_array()
            .is_some_and(|keys| !keys.is_empty())
    );
}

#[tokio::test]
async fn overview_returns_camel_case_json() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get("/api/overview?windowMinutes=60")
                .body(Body::empty())
                .unwrap(),
        )
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
    let (app, recorder) = app_with_recorder().await;
    let response = app
        .oneshot(
            Request::get("/api/failures?limit=9999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body.as_array().unwrap().len() <= 200);
    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(filter.limit, 200);
}

#[tokio::test]
async fn failures_limit_defaults_to_50() {
    let (app, recorder) = app_with_recorder().await;
    let response = app
        .oneshot(Request::get("/api/failures").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        recorder.last_filter.lock().unwrap().clone().unwrap().limit,
        50
    );
}

#[tokio::test]
async fn connections_reports_sources() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get("/api/connections")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["openobserve"]["ok"], true);
    assert_eq!(body["openobserveMcp"]["ok"], false);
    assert_eq!(body["model"]["configured"], true);
}

#[tokio::test]
async fn failures_from_and_to_win_over_window_minutes() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures?from=1761433200000&to=1761436800000&windowMinutes=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(
        filter.range,
        Range::new(1_761_433_200_000, 1_761_436_800_000)
    );
}

#[tokio::test]
async fn failures_window_minutes_expands_backwards_from_to() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures?to=1761436800000&windowMinutes=120")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(
        filter.range,
        Range::new(1_761_429_600_000, 1_761_436_800_000)
    );
}

#[tokio::test]
async fn failures_reject_the_four_bracket_rules_with_400() {
    let now_ms: i64 = 1_761_436_800_000;
    let cases = [
        // start >= end
        format!("/api/failures?from={now_ms}&to={now_ms}").leak() as &str,
        // below the 5-minute floor
        format!("/api/failures?from={}&to={}", now_ms - 4 * 60_000, now_ms).leak() as &str,
        // above the 7-day ceiling
        format!(
            "/api/failures?from={}&to={}",
            now_ms - 7 * 1440 * 60_000 - 1,
            now_ms
        )
        .leak() as &str,
        // `to` beyond the 60-second future tolerance
        format!("/api/failures?from={}&to={}", now_ms, now_ms + 60_001).leak() as &str,
    ];

    for uri in cases {
        let (app, _recorder) = app_with_recorder().await;
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "expected 400 for {uri}"
        );
        let body = json_body(response).await;
        assert_eq!(
            body["error"]["code"], "invalid_request",
            "expected the JSON envelope for {uri}"
        );
    }
}

#[tokio::test]
async fn failures_reject_a_non_numeric_bound_with_400() {
    // This one never reaches the handler: the `Query` extractor rejects the
    // string, so the body is axum's plain text, not our JSON envelope.
    let (app, _recorder) = app_with_recorder().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures?from=abc&to=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn failures_reject_extreme_bounds_without_panicking() {
    for uri in [
        "/api/failures?from=-9223372036854775808&to=0",
        "/api/failures?to=-9223372036854775808",
    ] {
        let (app, _recorder) = app_with_recorder().await;
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], "invalid_request", "{uri}");
    }
}

#[tokio::test]
async fn failures_treat_a_blank_instance_as_no_filter() {
    let (app, recorder) = app_with_recorder().await;
    let response = app
        .oneshot(
            Request::get("/api/failures?instance=%20%20")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // The service sees no host filter, so labelled rows are not hidden.
    assert_eq!(
        recorder
            .last_filter
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .instance,
        None
    );
}

#[tokio::test]
async fn failures_accept_the_seven_day_edge_and_the_future_tolerance() {
    let now_ms: i64 = 1_761_436_800_000;
    for uri in [
        format!(
            "/api/failures?from={}&to={}",
            now_ms - 7 * 1440 * 60_000,
            now_ms
        )
        .leak() as &str,
        format!(
            "/api/failures?from={}&to={}",
            now_ms - 60 * 60_000,
            now_ms + 60_000
        )
        .leak() as &str,
        // exactly the 5-minute floor
        format!("/api/failures?from={}&to={}", now_ms - 5 * 60_000, now_ms).leak() as &str,
    ] {
        let (app, _recorder) = app_with_recorder().await;
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "expected 200 for {uri}");
    }
}

#[tokio::test]
async fn failures_pass_the_instance_and_default_a_relative_hour() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures?instance=dns-node-2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(filter.instance.as_deref(), Some("dns-node-2"));
    assert_eq!(
        filter.range.end_ms - filter.range.start_ms,
        60 * 60_000,
        "the default bracket is one hour"
    );
}

#[tokio::test]
async fn uncovered_reports_the_count_for_the_requested_bracket() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures/uncovered?instance=dns-node-2&to=1761436800000&windowMinutes=120")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    // The fixture's one host-less group (`b1f9…`, 20 minutes old) carries
    // `count: 5`; `uncovered` counts rows like the production `COUNT(*)`, so the
    // demo path answers 5 instead of the trait default's zero.
    assert_eq!(body["unlabeled"], 5);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(filter.instance.as_deref(), Some("dns-node-2"));
    assert_eq!(
        filter.range,
        Range::new(1_761_429_600_000, 1_761_436_800_000)
    );
}

#[tokio::test]
async fn uncovered_is_not_matched_by_the_trace_route() {
    // `/api/failures/{trace_id}` must not answer this literal path: the detail
    // handler would look up a trace named "uncovered" and miss.
    let (app, _recorder) = app_with_recorder().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures/uncovered")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.get("unlabeled").is_some(), "unexpected body: {body}");
}

#[tokio::test]
async fn uncovered_rejects_a_too_long_bracket() {
    let now_ms: i64 = 1_761_436_800_000;
    let (app, _recorder) = app_with_recorder().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/failures/uncovered?from={}&to={}",
                    now_ms - 7 * 1440 * 60_000 - 1,
                    now_ms
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

const FIXED_NOW_MS: i64 = 1_761_436_800_000;

#[tokio::test]
async fn overview_takes_an_absolute_bracket_without_clamping_it() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get(format!(
                "/api/overview?from={}&to={}",
                FIXED_NOW_MS - 1_000 * 60_000,
                FIXED_NOW_MS
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    // 1000 分钟在 10080 上限之内,却远在旧的 5-1440 clamp 之外:
    // 桥接实现(Task 2 留的)会在这里答 60,断言 `windowMinutes` 才抓得住它。
    assert_eq!(body["windowMinutes"], 1_000);
}

#[tokio::test]
async fn overview_takes_a_week_from_the_relative_shortcut() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get("/api/overview?windowMinutes=10080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["windowMinutes"], 10_080);
}

#[tokio::test]
async fn overview_refuses_a_bracket_the_investigation_chain_refuses() {
    // 四条规则、同一个 400,而且最重要的是:不 clamp。
    // `windowMinutes=20000` 过去会悄悄变成 1440。
    for uri in [
        "/api/overview?windowMinutes=20000",
        "/api/overview?from=1000&to=2000",
        "/api/overview?from=2000&to=1000",
        &format!("/api/overview?to={}", FIXED_NOW_MS + 61_000),
    ] {
        let app = app_with(fixture() as Arc<dyn TelemetrySource>);
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = json_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert_eq!(body["error"]["code"], "invalid_request", "{uri}: {body}");
    }
}

#[tokio::test]
async fn a_negative_window_minutes_is_refused_at_the_parameter_layer() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get("/api/overview?windowMinutes=-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // `window_minutes` 是 `u32`,负数在 axum 的反序列化层就 400,压根进不到
    // `Range::resolve`;因此只钉状态码,不钉 `{error:{code,message}}` 信封。
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_overview_and_the_failure_list_refuse_identically() {
    let overview_app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let overview = json_body(
        overview_app
            .oneshot(
                Request::get("/api/overview?windowMinutes=20000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let failures_app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let failures = json_body(
        failures_app
            .oneshot(
                Request::get("/api/failures?windowMinutes=20000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        overview["error"]["message"],
        "invalid request: range is longer than the 10080-minute ceiling"
    );
    assert_eq!(overview, failures);
}
