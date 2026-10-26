use std::sync::Arc;

use doggy_dns_console::config::{OoAuth, OpenObserveConfig};
use doggy_dns_console::datasource::openobserve::client::OpenObserveClient;
use doggy_dns_console::datasource::openobserve::normalize::normalize_log_row;
use doggy_dns_console::datasource::openobserve::telemetry::OpenObserveTelemetry;
use doggy_dns_console::datasource::{FixedClock, TelemetrySource};
use doggy_dns_console::domain::{FailureFilter, Window};
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

#[test]
fn normalizes_flat_and_nested_tracing_fields() {
    let flat = normalize_log_row(&serde_json::json!({
        "body": "upstream forward lookup failed",
        "severity_text": "ERROR",
        "trace_id": "t1",
        "query": "a.example.",
        "qtype": "A",
        "error": "timeout"
    }))
    .unwrap();
    assert_eq!(flat.record.severity, "ERROR");
    assert_eq!(flat.query.as_deref(), Some("a.example."));

    let nested = normalize_log_row(&serde_json::json!({
        "body": "authority returned error",
        "severity_text": "WARN",
        "trace_id": "t2",
        "attributes": {"query": "b.example.", "qtype": "AAAA"}
    }))
    .unwrap();
    assert_eq!(nested.query.as_deref(), Some("b.example."));
    assert_eq!(nested.qtype.as_deref(), Some("AAAA"));
}

#[tokio::test]
async fn failure_list_excludes_nxdomain_and_dedupes_by_trace() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/oo_failures_flat.json"),
            "application/json",
        ))
        .mount(&server)
        .await;

    let telemetry = OpenObserveTelemetry::new(
        Arc::new(OpenObserveClient::new(config(server.uri())).unwrap()),
        Arc::new(FixedClock(1_761_436_800_000)),
    );
    let groups = telemetry
        .failures(FailureFilter {
            window: Window::new(60),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(groups.iter().all(|group| group.response_code != "NXDOMAIN"));
    let t1 = groups
        .iter()
        .find(|group| group.trace_id.as_deref() == Some("t1"))
        .unwrap();
    assert_eq!(t1.count, 2);
    assert_eq!(groups[0].trace_id.as_deref(), Some("t1"));
    assert_eq!(groups.len(), 3);
    assert!(
        !groups
            .iter()
            .any(|group| group.trace_id.as_deref() == Some("t5"))
    );
}

#[tokio::test]
async fn failure_detail_joins_trace_spans() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "logs"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/oo_failures_flat.json"),
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/default/_search"))
        .and(query_param("type", "traces"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(include_str!("fixtures/oo_trace.json"), "application/json"),
        )
        .mount(&server)
        .await;

    let telemetry = OpenObserveTelemetry::new(
        Arc::new(OpenObserveClient::new(config(server.uri())).unwrap()),
        Arc::new(FixedClock(1_761_436_800_000)),
    );
    let detail = telemetry.failure_detail("t1").await.unwrap();
    assert_eq!(detail.group.trace_id.as_deref(), Some("t1"));
    assert!(!detail.logs.is_empty());
    assert_eq!(detail.spans.len(), 2);
    assert_eq!(detail.spans[1].name, "dns.error");
}
