use std::sync::Arc;

use doggy_dns_console::agent::oo_mcp::OoMcpClient;
use doggy_dns_console::config::{OoAuth, OoMcpConfig};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_cfg(base_url: String) -> OoMcpConfig {
    OoMcpConfig {
        url: base_url,
        org: "default".to_string(),
        auth: OoAuth::None,
        enabled: true,
    }
}

async fn mount_tools_list(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({"method": "tools/list"})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/oo_mcp_tools.json"),
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
            "result": {"tools": [
                {"name":"GetTraceDAG","description":"Trace DAG","inputSchema":{"type":"object","properties":{"trace_id":{"type":"string"}},"required":["trace_id"]}},
                {"name":"PrometheusQuery","description":"PromQL instant","inputSchema":{"type":"object","properties":{"org_id":{"type":"string"},"query":{"type":"string"}},"required":["org_id","query"]}},
                {"name":"SearchAround","description":"Search around","inputSchema":{"type":"object","properties":{"stream":{"type":"string"}},"required":["stream"]}}
            ]}
        })))
        .mount(&server)
        .await;

    let client = Arc::new(OoMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let tools = client.allowed_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(names.len(), 8);
    assert!(names.contains(&"oo_search_sql"));
    assert!(names.contains(&"oo_promql_range"));
    assert!(names.contains(&"oo_trace_dag"));
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

    let client = Arc::new(OoMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let value = client
        .call_tool("SearchSQL", json!({"sql": "SELECT 1"}))
        .await
        .unwrap();
    assert_eq!(value["hits"], json!([]));
}

#[tokio::test]
async fn rejects_tool_outside_allowlist_before_network() {
    let client =
        Arc::new(OoMcpClient::new(test_cfg("http://127.0.0.1:1/mcp".to_string())).unwrap());
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
    let client = Arc::new(OoMcpClient::new(test_cfg(server.uri() + "/mcp")).unwrap());
    let err = client
        .call_tool("SearchSQL", json!({"sql": "SELECT 1"}))
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("openobserve mcp"));
    assert!(!message.contains("Authorization"));
}
