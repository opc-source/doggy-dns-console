use doggy_dns_console::config::{OpenObserveAuth, OpenObserveConfig};
use doggy_dns_console::datasource::openobserve::OpenObserveClient;

fn test_config(base_url: String) -> OpenObserveConfig {
    OpenObserveConfig {
        base_url,
        org: "default".to_string(),
        auth: OpenObserveAuth::Basic {
            username: "u".to_string(),
            password: "p".to_string(),
        },
        log_stream: "doggy-dns".to_string(),
        trace_stream: "doggy-dns".to_string(),
        metric_queries: "dns_queries".to_string(),
        metric_duration: "dns_query_duration_bucket".to_string(),
    }
}

#[tokio::test]
async fn search_logs_posts_sql_to_org_endpoint_with_basic_auth() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/default/_search"))
        .and(wiremock::matchers::query_param("type", "logs"))
        .and(wiremock::matchers::header("authorization", "Basic dTpw"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"hits":[{"body":"x","trace_id":"t1"}]})),
        )
        .mount(&server)
        .await;

    let client = OpenObserveClient::new(test_config(server.uri())).unwrap();
    let rows = client
        .search_logs("SELECT * FROM \"doggy-dns\"", 1_000, 2_000, 10)
        .await
        .unwrap();
    assert_eq!(rows[0]["trace_id"], "t1");
}

#[tokio::test]
async fn query_promql_parses_result_matrix() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path(
            "/api/default/prometheus/api/v1/query_range",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_prom.json"),
            "application/json",
        ))
        .mount(&server)
        .await;

    let client = OpenObserveClient::new(test_config(server.uri())).unwrap();
    let series = client
        .query_promql("sum(rate(dns_queries[5m]))", 1_000_000, 1_120_000, 60)
        .await
        .unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].labels["host_name"], "dns-node-1");
    assert_eq!(series[0].points, vec![(1_000_000, 12.0), (1_060_000, 15.0)]);
}

#[tokio::test]
async fn promql_error_envelope_is_mapped_to_upstream_error() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path(
            "/api/default/prometheus/api/v1/query_range",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "error",
                "errorType": "bad_data",
                "error": "invalid parameter \"query\": parse error"
            })),
        )
        .mount(&server)
        .await;

    let client = OpenObserveClient::new(test_config(server.uri())).unwrap();
    let err = client
        .query_promql("not a query", 1_000_000, 1_120_000, 60)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("parse error"), "{err}");
}

#[tokio::test]
async fn non_2xx_is_mapped_to_upstream_error() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let client = OpenObserveClient::new(test_config(server.uri())).unwrap();
    let err = client
        .search_logs("SELECT 1", 1_000, 2_000, 10)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("openobserve"));
}
