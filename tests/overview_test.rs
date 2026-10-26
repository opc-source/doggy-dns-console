use std::sync::Arc;

use doggy_dns_console::config::{OoAuth, OpenObserveConfig};
use doggy_dns_console::datasource::openobserve::client::OpenObserveClient;
use doggy_dns_console::datasource::openobserve::overview::{
    failure_query, failure_series_query, p95_query, series_query, total_query,
};
use doggy_dns_console::datasource::openobserve::seen_query;
use doggy_dns_console::datasource::openobserve::telemetry::OpenObserveTelemetry;
use doggy_dns_console::datasource::{FixedClock, TelemetrySource};
use doggy_dns_console::domain::Window;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(base_url: String) -> OpenObserveConfig {
    OpenObserveConfig {
        base_url,
        org: "default".to_string(),
        auth: OoAuth::None,
        log_stream: "doggy-dns".to_string(),
        trace_stream: "doggy-dns".to_string(),
        metric_queries: "dns_queries_total".to_string(),
        metric_duration: "dns_query_duration_milliseconds_bucket".to_string(),
    }
}

fn matrix(values: serde_json::Value, labels: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "status": "success",
        "data": { "resultType": "matrix", "result": [{ "metric": labels, "values": values }] }
    }))
}

async fn mount(server: &MockServer, query: &str, body: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/api/default/prometheus/api/v1/query_range"))
        .and(query_param("query", query))
        .respond_with(body)
        .mount(server)
        .await;
}

fn ts(base: i64) -> i64 {
    base / 1000
}

#[tokio::test]
async fn overview_computes_success_rate_and_series_from_promql() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    let window = Window::new(60);
    mount(
        &server,
        &total_query("dns_queries_total", window),
        matrix(
            serde_json::json!([[ts(now), "1000"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &failure_query("dns_queries_total", window),
        matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &p95_query("dns_query_duration_milliseconds_bucket", window),
        matrix(
            serde_json::json!([[ts(now), "12.4"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &series_query("dns_queries_total", false),
        matrix(
            serde_json::json!([[ts(now - 60_000), "10"], [ts(now), "20"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &failure_series_query("dns_queries_total"),
        matrix(
            serde_json::json!([[ts(now - 60_000), "1"], [ts(now), "2"]]),
            serde_json::json!({}),
        ),
    )
    .await;

    let telemetry = OpenObserveTelemetry::new(
        Arc::new(OpenObserveClient::new(config(server.uri())).unwrap()),
        Arc::new(FixedClock(now)),
    );
    let overview = telemetry.overview(window).await.unwrap();
    assert_eq!(overview.queries, 1000);
    assert_eq!(overview.failures, 12);
    assert_eq!(overview.success_rate, Some(0.988));
    assert_eq!(overview.p95_latency_ms, Some(12.4));
    assert_eq!(overview.series.len(), 2);
}

#[tokio::test]
async fn overview_returns_none_success_rate_when_counter_is_zero() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    let window = Window::new(60);
    mount(
        &server,
        &total_query("dns_queries_total", window),
        matrix(serde_json::json!([[ts(now), "0"]]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &failure_query("dns_queries_total", window),
        matrix(serde_json::json!([[ts(now), "0"]]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &p95_query("dns_query_duration_milliseconds_bucket", window),
        matrix(serde_json::json!([]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &series_query("dns_queries_total", false),
        matrix(serde_json::json!([]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &failure_series_query("dns_queries_total"),
        matrix(serde_json::json!([]), serde_json::json!({})),
    )
    .await;

    let telemetry = OpenObserveTelemetry::new(
        Arc::new(OpenObserveClient::new(config(server.uri())).unwrap()),
        Arc::new(FixedClock(now)),
    );
    let overview = telemetry.overview(window).await.unwrap();
    assert_eq!(overview.success_rate, None);
    assert_eq!(overview.queries, 0);
}

#[tokio::test]
async fn seen_instances_uses_last_nonzero_point() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    Mock::given(method("GET"))
        .and(path("/api/default/prometheus/api/v1/query_range"))
        .and(query_param("query", seen_query("dns_queries_total")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": { "resultType": "matrix", "result": [
                { "metric": {"host_name": "dns-node-1"}, "values": [[ts(now - 300_000), "1"], [ts(now), "2"]] },
                { "metric": {"host_name": "dns-node-2"}, "values": [[ts(now - 900_000), "1"], [ts(now - 300_000), "0"]] }
            ]}
        })))
        .mount(&server)
        .await;

    let telemetry = OpenObserveTelemetry::new(
        Arc::new(OpenObserveClient::new(config(server.uri())).unwrap()),
        Arc::new(FixedClock(now)),
    );
    let seen = telemetry.seen_instances(Window::new(1440)).await.unwrap();
    assert_eq!(seen.len(), 2);
    let node1 = seen.iter().find(|item| item.host == "dns-node-1").unwrap();
    assert_eq!(node1.last_seen, now);
    let node2 = seen.iter().find(|item| item.host == "dns-node-2").unwrap();
    assert_eq!(node2.last_seen, now - 900_000);
}
