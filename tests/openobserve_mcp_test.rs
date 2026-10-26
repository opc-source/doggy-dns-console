use std::sync::Arc;

use doggy_dns_console::agent::openobserve_mcp::OpenObserveMcpClient;
use doggy_dns_console::config::{OpenObserveAuth, OpenObserveMcpConfig};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_cfg(base_url: String) -> OpenObserveMcpConfig {
    OpenObserveMcpConfig {
        url: base_url,
        org: "default".to_string(),
        auth: OpenObserveAuth::None,
        enabled: true,
    }
}

async fn mount_tools_list(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({"method": "tools/list"})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/openobserve_mcp_tools.json"),
            "application/json",
        ))
        .mount(server)
        .await;
}

#[tokio::test]
async fn filters_tools_list_to_read_only_allowlist() {
    let server = MockServer::start().await;
    mount_tools_list(&server).await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({"method": "tools/call", "params": {"name": "tool_search"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"content": [{"type": "text", "text": serde_json::to_string(&json!({"tools": [
                {"name":"GetTraceDAG","description":"Trace DAG","inputSchema":{"type":"object","properties":{"trace_id":{"type":"string"}},"required":["trace_id"]}},
                {"name":"PrometheusQuery","description":"PromQL instant","inputSchema":{"type":"object","properties":{"org_id":{"type":"string"},"query":{"type":"string"}},"required":["org_id","query"]}},
                {"name":"SearchAround","description":"Search around","inputSchema":{"type":"object","properties":{"stream":{"type":"string"}},"required":["stream"]}}
            ]})).unwrap()}]}
        })))
        .mount(&server)
        .await;

    let client = Arc::new(OpenObserveMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let tools = client.allowed_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(names.len(), 8);
    assert!(names.contains(&"openobserve_search_sql"));
    assert!(names.contains(&"openobserve_promql_range"));
    assert!(names.contains(&"openobserve_trace_dag"));
    assert!(!names.contains(&"SearchSQL"));
    assert!(!names.contains(&"DeleteStream"));
    assert!(!names.contains(&"tool_search"));
    assert!(!names.contains(&"tools_call"));
}

#[tokio::test]
async fn call_tool_uses_tools_call_with_full_detail() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({
            "method": "tools/call",
            "params": {
                "name": "tools_call",
                "arguments": {
                    "tool": "SearchSQL",
                    "args": {"sql": "SELECT 1", "org_id": "default"},
                    "detail": "full"
                }
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"content": [{"type": "text", "text": "{\"hits\":[]}"}]}
        })))
        .mount(&server)
        .await;

    let client = Arc::new(OpenObserveMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let value = client
        .call_tool("SearchSQL", json!({"sql": "SELECT 1"}))
        .await
        .unwrap();
    assert_eq!(value["hits"], json!([]));
}

#[tokio::test]
async fn tools_list_failure_degrades_without_scanning_every_tool() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(500).set_body_string("down"))
        .mount(&server)
        .await;

    let client = Arc::new(OpenObserveMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let tools = client.allowed_tools().await.unwrap();
    assert!(tools.is_empty());
    assert!(server.received_requests().await.unwrap().len() <= 2);
}

#[tokio::test]
async fn call_tool_parses_an_sse_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({"method": "tools/call"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"{\\\"hits\\\":[]}\"}]}}\n\n",
                "text/event-stream",
            ),
        )
        .mount(&server)
        .await;

    let client = Arc::new(OpenObserveMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let value = client
        .call_tool("SearchSQL", json!({"sql": "SELECT 1"}))
        .await
        .unwrap();
    assert_eq!(value["hits"], json!([]));
}

#[tokio::test]
async fn attempts_initialize_when_tools_list_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({"method": "tools/list"})))
        .respond_with(ResponseTemplate::new(400).set_body_string("need initialize"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({"method": "initialize"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "result": {"protocolVersion": "2025-11-25", "capabilities": {}}
        })))
        .mount(&server)
        .await;

    let client = Arc::new(OpenObserveMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let tools = client.allowed_tools().await.unwrap();
    assert!(tools.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert!(requests.iter().any(|request| {
        String::from_utf8_lossy(&request.body).contains("\"method\":\"initialize\"")
    }));
}

#[tokio::test]
async fn rejects_tool_outside_allowlist_before_network() {
    let client = Arc::new(
        OpenObserveMcpClient::new(test_cfg("http://127.0.0.1:1/mcp".to_string())).unwrap(),
    );
    let err = client
        .call_tool("DeleteStream", json!({}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not allowed"));
}

#[tokio::test]
async fn tool_error_is_explicit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
        .mount(&server)
        .await;
    let client = Arc::new(OpenObserveMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let err = client
        .call_tool("SearchSQL", json!({"sql": "SELECT 1"}))
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("openobserve mcp"));
    assert!(!message.contains("Authorization"));
}
