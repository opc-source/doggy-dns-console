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
use doggy_dns_console::domain::{MetricCounts, Range, SeriesPoint, Window};
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

fn empty_matrix() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "status": "success",
        "data": { "resultType": "matrix", "result": [] }
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

/// `query_promql` is a GET with `query`/`start`/`end`/`step`; matching on
/// `step` too is the only way to pin D15's other half — the same query text
/// replayed every 60s over a week is still 10080 samples. `start`/`end` are
/// pinned as well, because a totals query whose step is LONGER than the real
/// span evaluates its single sample at `start`: right query text, right step,
/// and a bracket nobody asked for. Without the window matchers no test in this
/// file can see which bracket the console shipped.
async fn mount_at_step(
    server: &MockServer,
    query: &str,
    step_seconds: u64,
    start_ms: i64,
    end_ms: i64,
    body: ResponseTemplate,
) {
    Mock::given(method("GET"))
        .and(path("/api/default/prometheus/api/v1/query_range"))
        .and(query_param("query", query))
        .and(query_param("step", step_seconds.to_string()))
        .and(query_param("start", ts(start_ms).to_string()))
        .and(query_param("end", ts(end_ms).to_string()))
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
    let window = Range::new(now - 3_600_000, now);
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
        &p95_query("dns_query_duration_bucket", 5),
        matrix(
            serde_json::json!([[ts(now), "12.4"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &series_query("dns_queries", 5),
        matrix(
            serde_json::json!([[ts(now - 60_000), "10"], [ts(now), "20"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount(
        &server,
        &failure_series_query("dns_queries", 5),
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
    assert_eq!(overview.window_minutes, 60);
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
    let window = Range::new(now - 3_600_000, now);
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
        &p95_query("dns_query_duration_bucket", 5),
        matrix(serde_json::json!([]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &series_query("dns_queries", 5),
        matrix(serde_json::json!([]), serde_json::json!({})),
    )
    .await;
    mount(
        &server,
        &failure_series_query("dns_queries", 5),
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
async fn a_one_hour_bracket_keeps_the_existing_trend_text_and_step() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    // The default view must not change shape: `rate[5m]` at a 60s step, and
    // totals still one `increase(m[60m])` covering the bracket.
    let rate = series_query("dns_queries", 5);
    let failure_rate = failure_series_query("dns_queries", 5);
    let p95 = p95_query("dns_query_duration_bucket", 5);
    assert_eq!(rate, "sum(rate(dns_queries[5m])) * 60");
    assert_eq!(
        p95,
        "histogram_quantile(0.95, sum by (le) (rate(dns_query_duration_bucket[5m])))"
    );
    let window = Range::new(now - 3_600_000, now);
    mount_at_step(
        &server,
        &total_query("dns_queries", window),
        3_600,
        window.start_ms,
        window.end_ms,
        matrix(
            serde_json::json!([[ts(now), "1000"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &failure_query("dns_queries", window),
        3_600,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &p95,
        60,
        window.start_ms,
        window.end_ms,
        matrix(
            serde_json::json!([[ts(now), "12.4"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &rate,
        60,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "20"]]), serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &failure_rate,
        60,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "2"]]), serde_json::json!({})),
    )
    .await;

    let overview = telemetry(&server).overview(window).await.unwrap();
    assert_eq!(overview.window_minutes, 60);
    assert_eq!(overview.queries, 1000);
    assert_eq!(overview.p95_latency_ms, Some(12.4));
}

#[tokio::test]
async fn a_week_bracket_widens_the_rate_window_and_the_step() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    // 10080 / 168 = 60-minute buckets: hourly points, and the totals still
    // sampled once for the whole week instead of every minute.
    let window = Range::new(now - 10_080 * 60_000, now);
    mount_at_step(
        &server,
        &series_query("dns_queries", 60),
        3_600,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "20"]]), serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &failure_series_query("dns_queries", 60),
        3_600,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "2"]]), serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &p95_query("dns_query_duration_bucket", 60),
        3_600,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "9"]]), serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &total_query("dns_queries", window),
        604_800,
        window.start_ms,
        window.end_ms,
        matrix(
            serde_json::json!([[ts(now), "1000"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &failure_query("dns_queries", window),
        604_800,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({})),
    )
    .await;

    let overview = telemetry(&server).overview(window).await.unwrap();
    assert_eq!(overview.window_minutes, 10_080);
    assert_eq!(
        total_query("dns_queries", window),
        "sum(increase(dns_queries[10080m]))"
    );
}

#[tokio::test]
async fn p95_reports_the_worst_finite_bucket_not_the_last_one() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    let window = Range::new(now - 3_600_000, now);
    // Peaks in the middle and ends on NaN: `histogram_quantile` over an empty
    // bucket is NaN, so the old "last point" rule would have reported it.
    let points = serde_json::json!([
        [ts(now - 180_000), "12.5"],
        [ts(now - 120_000), "42.5"],
        [ts(now - 60_000), "7.5"],
        [ts(now), "NaN"]
    ]);
    mount_at_step(
        &server,
        &series_query("dns_queries", 5),
        60,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "20"]]), serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &failure_series_query("dns_queries", 5),
        60,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "2"]]), serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &p95_query("dns_query_duration_bucket", 5),
        60,
        window.start_ms,
        window.end_ms,
        matrix(points, serde_json::json!({})),
    )
    .await;
    mount_at_step(
        &server,
        &total_query("dns_queries", window),
        3_600,
        window.start_ms,
        window.end_ms,
        matrix(
            serde_json::json!([[ts(now), "1000"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &failure_query("dns_queries", window),
        3_600,
        window.start_ms,
        window.end_ms,
        matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({})),
    )
    .await;

    let overview = telemetry(&server).overview(window).await.unwrap();
    assert_eq!(overview.p95_latency_ms, Some(42.5));
}

#[tokio::test]
async fn totals_on_a_sub_minute_bracket_read_the_sample_at_the_end() {
    let server = MockServer::start().await;
    let end = 1_761_436_800_000_i64;
    // 59.5 minutes — the bracket the drawer gets when it resolves its range
    // from `Date.now()`. `span_minutes` rounds up to 60, so the whole-bracket
    // step (3600s) is LONGER than the real span (3570s). A `query_range` steps
    // forward from `start`, so the real start yields ONE sample, at
    // 1761433230, whose `increase[60m]` covers 1761429630..1761433230: the
    // period entirely BEFORE what the operator asked for.
    let start = end - 3_570_000;
    let window = Range::new(start, end);
    assert_eq!(window.span_minutes(), 60);
    assert_eq!(window.whole_step_seconds(), 3_600);

    // Totals start is aligned to its own step, so the LAST evaluation lands
    // exactly on `end`. Both samples carry distinct values: reading the KPI
    // from `end` has to be provable, not lucky.
    let aligned_start = end - 3_600_000;
    assert_ne!(aligned_start, start);
    mount_at_step(
        &server,
        &total_query("dns_queries", window),
        3_600,
        aligned_start,
        end,
        matrix(
            serde_json::json!([[ts(aligned_start), "7777"], [ts(end), "1000"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &failure_query("dns_queries", window),
        3_600,
        aligned_start,
        end,
        matrix(
            serde_json::json!([[ts(aligned_start), "77"], [ts(end), "12"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    // Trend, failure series and p95 keep tiling from the real start: there the
    // bracket is the contract, so shifting it would drop the first bucket.
    mount_at_step(
        &server,
        &series_query("dns_queries", 5),
        60,
        start,
        end,
        matrix(
            serde_json::json!([[ts(start), "10"], [ts(end), "20"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &failure_series_query("dns_queries", 5),
        60,
        start,
        end,
        matrix(
            serde_json::json!([[ts(start), "1"], [ts(end), "2"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &p95_query("dns_query_duration_bucket", 5),
        60,
        start,
        end,
        matrix(
            serde_json::json!([[ts(end), "12.4"]]),
            serde_json::json!({}),
        ),
    )
    .await;

    let overview = telemetry(&server).overview(window).await.unwrap();
    assert_eq!(overview.window_minutes, 60);
    // The KPI is the sample at `end` — not the aligned-start sample, and not
    // the increase window that ended before the bracket opened.
    assert_eq!(overview.queries, 1000);
    assert_eq!(overview.failures, 12);
    assert_eq!(overview.success_rate, Some(0.988));
    assert_eq!(overview.p95_latency_ms, Some(12.4));
    assert_eq!(
        overview.series,
        vec![
            SeriesPoint {
                ts: start,
                queries: 10,
                failures: 1
            },
            SeriesPoint {
                ts: end,
                queries: 20,
                failures: 2
            },
        ]
    );
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
async fn failure_metrics_on_a_sub_minute_span_reads_the_sample_at_the_end() {
    let server = MockServer::start().await;
    // 59.5 minutes: `minutes` rounds up to 60, so the step is wider than the span
    // and `query_range` — stepping FORWARD from `start` — yields exactly one
    // evaluation, at `start`. The two samples differ on purpose: reading the
    // evidence counts from `start` instead of `end` answers 7777/77, and a request
    // anchored anywhere other than the aligned start matches no mock at all.
    let end = 1_761_436_800_000_i64;
    let start = end - 3_570_000;
    let aligned_start = end - 3_600_000;
    assert_ne!(aligned_start, start);

    mount_at_step(
        &server,
        &scoped_total_query("dns_queries", 60, None),
        3_600,
        aligned_start,
        end,
        matrix(
            serde_json::json!([[ts(aligned_start), "7777"], [ts(end), "120"]]),
            serde_json::json!({}),
        ),
    )
    .await;
    mount_at_step(
        &server,
        &scoped_failure_query("dns_queries", 60, None),
        3_600,
        aligned_start,
        end,
        matrix(
            serde_json::json!([[ts(aligned_start), "77"], [ts(end), "7"]]),
            serde_json::json!({}),
        ),
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

#[tokio::test]
async fn failure_metrics_reports_unavailable_when_global_metric_is_absent() {
    let server = MockServer::start().await;
    let start = 1_761_436_800_000_i64;
    let end = start + 30 * 60_000;
    mount(
        &server,
        &scoped_total_query("dns_queries", 30, None),
        empty_matrix(),
    )
    .await;
    mount(
        &server,
        &scoped_failure_query("dns_queries", 30, None),
        empty_matrix(),
    )
    .await;

    let error = telemetry(&server)
        .failure_metrics(None, start as u64, end as u64)
        .await
        .unwrap_err();

    // An absent unfiltered series means the metric is not scraped; reporting
    // 0/0 from that state would invent data.
    assert!(matches!(error, DataError::Upstream(_)));
}

#[tokio::test]
async fn failure_metrics_keeps_measured_zero_when_only_failure_series_is_empty() {
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
        empty_matrix(),
    )
    .await;

    let metrics = telemetry(&server)
        .failure_metrics(None, start as u64, end as u64)
        .await
        .unwrap();

    // A live total with no failure-coded series is a measured zero, not a
    // missing metric.
    assert_eq!(
        metrics.global,
        MetricCounts {
            queries: 120,
            failures: 0
        }
    );
    assert_eq!(metrics.host, None);
}

#[tokio::test]
async fn failure_metrics_omits_host_counts_when_host_series_is_absent() {
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
    mount(
        &server,
        &scoped_total_query("dns_queries", 30, Some("dns-node-1")),
        empty_matrix(),
    )
    .await;
    mount(
        &server,
        &scoped_failure_query("dns_queries", 30, Some("dns-node-1")),
        empty_matrix(),
    )
    .await;

    let metrics = telemetry(&server)
        .failure_metrics(Some("dns-node-1"), start as u64, end as u64)
        .await
        .unwrap();

    assert_eq!(metrics.host, None);
    assert_eq!(
        metrics.global,
        MetricCounts {
            queries: 120,
            failures: 7
        }
    );
}

#[tokio::test]
async fn failure_metrics_omits_host_counts_when_host_total_is_absent() {
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
    mount(
        &server,
        &scoped_total_query("dns_queries", 30, Some("dns-node-1")),
        empty_matrix(),
    )
    .await;
    mount(
        &server,
        &scoped_failure_query("dns_queries", 30, Some("dns-node-1")),
        matrix(
            serde_json::json!([[ts(end), "2"]]),
            serde_json::json!({ "host_name": "dns-node-1" }),
        ),
    )
    .await;

    let metrics = telemetry(&server)
        .failure_metrics(Some("dns-node-1"), start as u64, end as u64)
        .await
        .unwrap();

    // An inconsistent backend must not produce a host count from a missing
    // total series.
    assert_eq!(metrics.host, None);
    assert_eq!(
        metrics.global,
        MetricCounts {
            queries: 120,
            failures: 7
        }
    );
}

#[test]
fn scoped_metric_queries_escape_host_label_values() {
    let query = scoped_total_query("dns_queries", 30, Some("node\"a\\b\r\nx\ty"));

    assert_eq!(
        query,
        r#"sum(increase(dns_queries{host_name="node\"a\\b\r\nx\ty"}[30m]))"#
    );
}
