use std::sync::Arc;

use doggy_dns_console::config::{OpenObserveAuth, OpenObserveConfig};
use doggy_dns_console::datasource::openobserve::client::OpenObserveClient;
use doggy_dns_console::datasource::openobserve::overview::{
    failure_query, failure_series_query, p95_query, scoped_failure_query, scoped_total_query,
    series_query, total_query,
};
use doggy_dns_console::datasource::openobserve::seen_query;
use doggy_dns_console::datasource::openobserve::telemetry::OpenObserveTelemetry;
use doggy_dns_console::datasource::{FixedClock, TelemetrySource};
use doggy_dns_console::domain::{MetricCounts, Window};
use doggy_dns_console::error::DataError;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(base_url: String) -> OpenObserveConfig {
    OpenObserveConfig {
        base_url,
        org: "default".to_string(),
        auth: OpenObserveAuth::None,
        log_stream: "doggy_dns".to_string(),
        trace_stream: "doggy_dns".to_string(),
        metric_queries: "dns_queries".to_string(),
        metric_duration: "dns_query_duration_bucket".to_string(),
    }
}

fn telemetry(server: &MockServer) -> OpenObserveTelemetry {
    OpenObserveTelemetry::new(
        Arc::new(OpenObserveClient::new(config(server.uri())).unwrap()),
        Arc::new(FixedClock(1_761_436_800_000)),
    )
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
        &total_query("dns_queries", window),
        matrix(
            serde_json::json!([[ts(now), "1000"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &failure_query("dns_queries", window),
        matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &p95_query("dns_query_duration_bucket", window),
        matrix(
            serde_json::json!([[ts(now), "12.4"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &series_query("dns_queries", false),
        matrix(
            serde_json::json!([[ts(now - 60_000), "10"], [ts(now), "20"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &failure_series_query("dns_queries"),
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
        &total_query("dns_queries", window),
        matrix(serde_json::json!([[ts(now), "0"]]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &failure_query("dns_queries", window),
        matrix(serde_json::json!([[ts(now), "0"]]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &p95_query("dns_query_duration_bucket", window),
        matrix(serde_json::json!([]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &series_query("dns_queries", false),
        matrix(serde_json::json!([]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &failure_series_query("dns_queries"),
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
async fn seen_instances_tracks_newest_sample_and_builds_control_endpoint() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    Mock::given(method("GET"))
        .and(path("/api/default/prometheus/api/v1/query_range"))
        .and(query_param("query", seen_query("dns_queries")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "data": { "resultType": "matrix", "result": [
                { "metric": {"host_name": "dns-node-1", "doggy_dns_ip_addr": "10.0.0.9", "doggy_dns_control_port": "19090"}, "values": [[ts(now - 300_000), "1"]] },
                { "metric": {"host_name": "dns-node-1", "doggy_dns_ip_addr": "127.0.0.1", "doggy_dns_control_port": "19090"}, "values": [[ts(now), "2"]] },
                { "metric": {"host_name": "dns-node-2"}, "values": [[ts(now - 900_000), "1"], [ts(now - 300_000), "0"]] },
                { "metric": {"host_name": "dns-node-3", "doggy_dns_ip_addr": "not-an-ip", "doggy_dns_control_port": "19090"}, "values": [[ts(now - 300_000), "1"]] },
                { "metric": {"host_name": "dns-node-4", "doggy_dns_ip_addr": "2001:db8::1", "doggy_dns_control_port": "19091"}, "values": [[ts(now - 300_000), "1"]] },
                { "metric": {"host_name": "dns-node-5", "doggy_dns_ip_addr": "10.0.0.5", "doggy_dns_control_port": "19090"}, "values": [[ts(now - 60_000), "0"], [ts(now), "0"]] },
                { "metric": {"host_name": "dns-node-6", "doggy_dns_ip_addr": "0.0.0.0", "doggy_dns_control_port": "19090"}, "values": [[ts(now - 300_000), "1"]] }
            ]}
        })))
        .mount(&server)
        .await;

    let telemetry = OpenObserveTelemetry::new(
        Arc::new(OpenObserveClient::new(config(server.uri())).unwrap()),
        Arc::new(FixedClock(now)),
    );
    let seen = telemetry.seen_instances(Window::new(1440)).await.unwrap();
    assert_eq!(seen.len(), 6);
    let node1 = seen.iter().find(|item| item.host == "dns-node-1").unwrap();
    assert_eq!(node1.last_seen, now);
    assert_eq!(
        node1.endpoint.as_deref(),
        Some("http://127.0.0.1:19090/mcp")
    );
    let node2 = seen.iter().find(|item| item.host == "dns-node-2").unwrap();
    assert_eq!(node2.last_seen, now - 300_000);
    assert_eq!(node2.endpoint, None);
    let node3 = seen.iter().find(|item| item.host == "dns-node-3").unwrap();
    assert_eq!(node3.endpoint, None);
    let node4 = seen.iter().find(|item| item.host == "dns-node-4").unwrap();
    assert_eq!(
        node4.endpoint.as_deref(),
        Some("http://[2001:db8::1]:19091/mcp")
    );
    // A freshly restarted node has no positive `increase` samples yet, but
    // its advertised listener must still be discovered.
    let node5 = seen.iter().find(|item| item.host == "dns-node-5").unwrap();
    assert_eq!(node5.last_seen, now);
    assert_eq!(node5.endpoint.as_deref(), Some("http://10.0.0.5:19090/mcp"));
    let node6 = seen.iter().find(|item| item.host == "dns-node-6").unwrap();
    assert_eq!(node6.endpoint, None);
}

#[tokio::test]
async fn failure_metrics_queries_host_and_global_queries_and_failures() {
    let server = MockServer::start().await;
    let start = 1_761_436_800_000_i64;
    let end = start + 30 * 60_000;
    mount(
        &server,
        &scoped_total_query("dns_queries", 30, None),
        matrix(
            serde_json::json!([[ts(start), "100"], [ts(end), "120"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &scoped_failure_query("dns_queries", 30, None),
        matrix(
            serde_json::json!([[ts(start), "5"], [ts(end), "7"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &scoped_total_query("dns_queries", 30, Some("dns-node-1")),
        matrix(
            serde_json::json!([[ts(start), "10"], [ts(end), "12"]]),
            serde_json::json!({ "host_name": "dns-node-1" }),
        ),
    )
    .await;
    mount(
        &server,
        &scoped_failure_query("dns_queries", 30, Some("dns-node-1")),
        matrix(
            serde_json::json!([[ts(start), "1"], [ts(end), "2"]]),
            serde_json::json!({ "host_name": "dns-node-1" }),
        ),
    )
    .await;

    let metrics = telemetry(&server)
        .failure_metrics(Some("dns-node-1"), start as u64, end as u64)
        .await
        .unwrap();

    // Only the sample at the end of the range counts: summing every point
    // would double-count the same trailing `increase` window.
    assert_eq!(
        metrics.global,
        MetricCounts {
            queries: 120,
            failures: 7
        }
    );
    assert_eq!(
        metrics.host,
        Some(MetricCounts {
            queries: 12,
            failures: 2
        })
    );
}

#[tokio::test]
async fn failure_metrics_without_a_host_queries_global_scope_only() {
    let server = MockServer::start().await;
    let start = 1_761_436_800_000_i64;
    let end = start + 30 * 60_000;
    mount(
        &server,
        &scoped_total_query("dns_queries", 30, None),
        matrix(serde_json::json!([[ts(end), "120"]]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &scoped_failure_query("dns_queries", 30, None),
        matrix(serde_json::json!([[ts(end), "7"]]), serde_json::json!({})),
    )
    .await;

    let metrics = telemetry(&server)
        .failure_metrics(None, start as u64, end as u64)
        .await
        .unwrap();

    assert_eq!(
        metrics.global,
        MetricCounts {
            queries: 120,
            failures: 7
        }
    );
    assert_eq!(metrics.host, None);
}

#[tokio::test]
async fn failure_metrics_propagates_promql_failures() {
    let server = MockServer::start().await;
    let start = 1_761_436_800_000_i64;
    let end = start + 30 * 60_000;
    Mock::given(method("GET"))
        .and(path("/api/default/prometheus/api/v1/query_range"))
        .and(query_param(
            "query",
            scoped_total_query("dns_queries", 30, None),
        ))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let error = telemetry(&server)
        .failure_metrics(None, start as u64, end as u64)
        .await
        .unwrap_err();

    assert!(matches!(error, DataError::Upstream(_)));
}

#[test]
fn scoped_metric_queries_escape_host_label_values() {
    let query = scoped_total_query("dns_queries", 30, Some("node\"a\\b"));

    assert_eq!(
        query,
        r#"sum(increase(dns_queries{host_name="node\"a\\b"}[30m]))"#
    );
}
