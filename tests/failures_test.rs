use std::sync::Arc;

use doggy_dns_console::config::{OpenObserveAuth, OpenObserveConfig};
use doggy_dns_console::datasource::openobserve::client::OpenObserveClient;
use doggy_dns_console::datasource::openobserve::failures::{
    failure_group_query, failure_groups_query, failure_logs_query, trace_spans_query,
};
use doggy_dns_console::datasource::openobserve::normalize::{
    normalize_failure_group_row, normalize_log_row,
};
use doggy_dns_console::datasource::openobserve::telemetry::OpenObserveTelemetry;
use doggy_dns_console::datasource::{FixedClock, TelemetrySource};
use doggy_dns_console::domain::{FailureFilter, Window};
use doggy_dns_console::error::DataError;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const NOW_MS: i64 = 1_761_436_800_000;

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
        Arc::new(FixedClock(NOW_MS)),
    )
}

fn window_filter(limit: usize) -> FailureFilter {
    FailureFilter {
        window: Window::new(60),
        limit,
        ..Default::default()
    }
}

#[test]
fn normalizes_flat_and_nested_tracing_fields() {
    let flat = normalize_log_row(&serde_json::json!({
        "body": "upstream forward lookup failed",
        "severity": "ERROR",
        "trace_id": "t1",
        "query": "a.example.",
        "qtype": "A",
        "error": "timeout"
    }))
    .unwrap();
    assert_eq!(flat.record.severity, "ERROR");
    assert_eq!(flat.query.as_deref(), Some("a.example."));
    assert_eq!(flat.record.fields["query"], "a.example.");
    assert!(flat.record.fields.get("severity").is_none());
    assert!(flat.record.fields.get("body").is_none());

    let named = normalize_log_row(&serde_json::json!({
        "body": "dns query received",
        "severity": "INFO",
        "trace_id": "t6",
        "query_name": "c.example.",
        "query_type": "AAAA"
    }))
    .unwrap();
    assert_eq!(named.query.as_deref(), Some("c.example."));
    assert_eq!(named.qtype.as_deref(), Some("AAAA"));

    let nested = normalize_log_row(&serde_json::json!({
        "body": "authority returned error",
        "severity": "WARN",
        "trace_id": "t2",
        "attributes": {"query": "b.example.", "qtype": "AAAA"}
    }))
    .unwrap();
    assert_eq!(nested.query.as_deref(), Some("b.example."));
    assert_eq!(nested.qtype.as_deref(), Some("AAAA"));
    assert_eq!(nested.record.fields["query"], "b.example.");

    // Flat values are authoritative and neither shape may re-introduce a core
    // field into the raw drawer map.
    let mixed = normalize_log_row(&serde_json::json!({
        "body": "mixed",
        "severity": "ERROR",
        "trace_id": "t7",
        "query": "flat.example.",
        "attributes": {
            "query": "nested.example.",
            "severity": "FATAL",
            "extra": "kept"
        }
    }))
    .unwrap();
    assert_eq!(mixed.query.as_deref(), Some("flat.example."));
    assert_eq!(mixed.record.severity, "ERROR");
    assert_eq!(mixed.record.fields["query"], "flat.example.");
    assert_eq!(mixed.record.fields["extra"], "kept");
    assert!(mixed.record.fields.get("severity").is_none());

    assert!(
        normalize_log_row(&serde_json::json!({
            "body": "missing severity",
            "trace_id": "t3"
        }))
        .is_none()
    );
}

#[test]
fn failure_group_row_applies_newest_non_empty_fallbacks() {
    let group = normalize_failure_group_row(&serde_json::json!({
        "trace_id": "t9",
        "count": 1.0,
        "first_seen": 1761436800000000_i64,
        "last_seen": 1761436860000000_i64,
        "queries": [null, "", "older.example."],
        "qtype": null,
        "host_name": null,
        "authority": null,
        "response_code": null,
        "errors": [null, "fallback error"]
    }))
    .unwrap();
    assert_eq!(group.key, "trace:t9");
    assert_eq!(group.trace_id.as_deref(), Some("t9"));
    assert_eq!(group.query, "older.example.");
    assert_eq!(group.qtype, "A");
    assert_eq!(group.instance, None);
    assert_eq!(group.authority, None);
    assert_eq!(group.response_code, "UNKNOWN");
    assert_eq!(group.error.as_deref(), Some("fallback error"));
    assert_eq!(group.count, 1);
    assert_eq!(group.first_seen, 1_761_436_800_000);
    assert_eq!(group.last_seen, 1_761_436_860_000);

    assert!(normalize_failure_group_row(&serde_json::json!({"trace_id": ""})).is_none());
    assert!(normalize_failure_group_row(&serde_json::json!({"count": 1})).is_none());
}

#[test]
fn failure_queries_share_the_predicate_and_escape_filters() {
    let filter = FailureFilter {
        window: Window::new(60),
        domain: Some("O'Reilly.example".to_string()),
        response_code: Some("ServFail".to_string()),
        limit: 50,
    };
    let list = failure_groups_query("doggy_dns", &filter, 1_000, 2_000, 50);
    assert!(list.contains("FROM \"doggy_dns\""));
    assert!(list.contains("GROUP BY trace_id"));
    assert!(list.contains("upper(response_code) != 'NXDOMAIN'"));
    assert!(list.contains("trace_id IS NOT NULL"));
    assert!(list.contains("lower(response_code) = lower('ServFail')"));
    assert!(list.contains(
        "position(lower('O''Reilly.example') in lower(coalesce(query, query_name))) > 0"
    ));
    assert!(list.contains("_timestamp >= 1000000"));
    assert!(list.contains("_timestamp <= 2000000"));
    assert!(list.contains("LIMIT 50"));

    // The detail shares the aggregate shape, so its group metadata cannot
    // drift from the list entry for the same trace.
    let detail = failure_group_query("doggy_dns", "t'1", &filter, 1_000, 2_000);
    assert!(detail.contains("trace_id = 't''1'"));
    assert!(detail.contains("GROUP BY trace_id LIMIT 1"));
    assert!(detail.contains("first_value(qtype ORDER BY _timestamp DESC)"));

    let logs = failure_logs_query("doggy_dns", "t'1", 1_000, 2_000, 500);
    assert!(logs.starts_with("SELECT * FROM \"doggy_dns\""));
    assert!(logs.contains("ORDER BY _timestamp DESC LIMIT 500"));

    let spans = trace_spans_query("doggy_dns", "t'1", 200);
    assert!(spans.contains("ORDER BY start_time ASC LIMIT 200"));
}

#[tokio::test]
async fn failure_list_maps_aggregate_rows() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .and(body_string_contains("GROUP BY trace_id"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_failure_groups.json"),
            "application/json",
        ))
        .mount(&server)
        .await;

    let groups = telemetry(&server)
        .failures(window_filter(50))
        .await
        .unwrap();
    assert_eq!(groups.len(), 3);
    let t1 = &groups[0];
    assert_eq!(t1.trace_id.as_deref(), Some("t1"));
    assert_eq!(t1.query, "a.example.");
    assert_eq!(t1.qtype, "A");
    assert_eq!(t1.instance.as_deref(), Some("dns-node-1"));
    assert_eq!(t1.authority.as_deref(), Some("forward[8.8.8.8:53]"));
    assert_eq!(t1.response_code, "SERVFAIL");
    assert_eq!(t1.error.as_deref(), Some("timeout"));
    assert_eq!(t1.count, 2);
    assert_eq!(t1.first_seen, 1_761_436_800_000);
    assert_eq!(t1.last_seen, 1_761_436_860_000);
    assert!(groups.iter().all(|group| group.response_code != "NXDOMAIN"));
}

#[tokio::test]
async fn failure_detail_group_matches_the_list_entry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .and(body_string_contains("GROUP BY trace_id"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_failure_groups.json"),
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .and(body_string_contains("SELECT * FROM"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_failures_flat.json"),
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "traces"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_trace.json"),
            "application/json",
        ))
        .mount(&server)
        .await;

    let telemetry = telemetry(&server);
    let filter = window_filter(50);
    let groups = telemetry.failures(filter.clone()).await.unwrap();
    let detail = telemetry.failure_detail("t1", filter).await.unwrap();
    assert_eq!(detail.group, groups[0]);

    assert_eq!(detail.logs.len(), 3);
    assert!(detail.logs.windows(2).all(|pair| pair[0].ts <= pair[1].ts));
    assert_eq!(detail.logs[2].severity, "INFO");
    assert_eq!(detail.logs[2].message, "dns query completed");
    assert_eq!(detail.spans.len(), 2);
    assert_eq!(detail.spans[1].name, "dns.error");
    assert!(!detail.spans_unavailable);
}

#[tokio::test]
async fn failure_detail_marks_spans_unavailable_when_trace_search_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .and(body_string_contains("GROUP BY trace_id"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_failure_groups.json"),
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .and(body_string_contains("SELECT * FROM"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_failures_flat.json"),
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "traces"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let detail = telemetry(&server)
        .failure_detail("t1", window_filter(50))
        .await
        .unwrap();
    assert!(detail.spans.is_empty());
    assert!(detail.spans_unavailable);
}

#[tokio::test]
async fn failure_detail_returns_not_found_without_a_failure_group() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .and(body_string_contains("GROUP BY trace_id"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"hits":[]}"#, "application/json"))
        .mount(&server)
        .await;

    // NXDOMAIN traces and traces without a response code are filtered by the
    // shared predicate, so their aggregate is empty.
    let result = telemetry(&server)
        .failure_detail("t3", window_filter(1))
        .await;
    assert!(
        matches!(result, Err(DataError::NotFound(_))),
        "expected NotFound, got {result:?}"
    );
}
