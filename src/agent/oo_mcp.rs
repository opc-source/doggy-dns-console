use std::sync::Arc;

use adk_rust::prelude::{AdkError, Tool, ToolContext};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

use crate::config::{OoAuth, OoMcpConfig};
use crate::error::DataError;

/// (upstream MCP tool name, exposed tool name). The exposed name is what the model sees.
pub const ALLOWED_OO_TOOLS: [(&str, &str); 8] = [
    ("StreamList", "oo_stream_list"),
    ("StreamSchema", "oo_stream_schema"),
    ("SearchSQL", "oo_search_sql"),
    ("PrometheusQuery", "oo_promql_query"),
    ("PrometheusRangeQuery", "oo_promql_range"),
    ("GetLatestTraces", "oo_latest_traces"),
    ("GetTraceDAG", "oo_trace_dag"),
    ("SearchAround", "oo_search_around"),
];

pub struct OoMcpClient {
    http: Client,
    cfg: OoMcpConfig,
}

impl OoMcpClient {
    pub fn new(cfg: OoMcpConfig) -> Result<Self, DataError> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|error| DataError::Upstream(format!("openobserve mcp client: {error}")))?;
        Ok(Self { http, cfg })
    }

    pub async fn allowed_tools(self: &Arc<Self>) -> Result<Vec<Arc<dyn Tool>>, DataError> {
        let list = self
            .rpc("tools/list", json!({}))
            .await
            .map(|value| tools_from_list(&value))
            .unwrap_or_default();

        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for (upstream, exposed) in ALLOWED_OO_TOOLS {
            let definition = list
                .iter()
                .find(|tool| tool.name == upstream)
                .cloned()
                .or_else(|| {
                    // OO only exposes pinned tools in tools/list; search for the rest.
                    None
                });
            let definition = match definition {
                Some(definition) => definition,
                None => match self.search_tool(upstream).await {
                    Ok(Some(definition)) => definition,
                    Ok(None) => {
                        tracing::warn!(tool = upstream, "openobserve mcp tool not found");
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(tool = upstream, error = %error, "openobserve mcp tool search failed");
                        continue;
                    }
                },
            };
            let schema = strip_org_id(definition.schema);
            tools.push(Arc::new(OoMcpTool {
                exposed_name: exposed.to_string(),
                upstream_name: upstream.to_string(),
                description: definition.description,
                schema,
                client: Arc::clone(self),
            }));
        }
        Ok(tools)
    }

    async fn search_tool(&self, name: &str) -> Result<Option<ToolDefinition>, DataError> {
        let result = self
            .rpc(
                "tools/call",
                json!({
                    "name": "tool_search",
                    "arguments": { "query": name, "limit": 5 }
                }),
            )
            .await?;
        Ok(tools_from_list(&result)
            .into_iter()
            .find(|tool| tool.name == name))
    }

    pub async fn call_tool(&self, tool: &str, args: Value) -> Result<Value, DataError> {
        if !ALLOWED_OO_TOOLS
            .iter()
            .any(|(upstream, _)| *upstream == tool)
        {
            return Err(DataError::Invalid(format!("tool not allowed: {tool}")));
        }
        let mut arguments = args.as_object().cloned().unwrap_or_default();
        arguments
            .entry("org_id")
            .or_insert_with(|| Value::String(self.cfg.org.clone()));
        let result = self
            .rpc(
                "tools/call",
                json!({
                    "name": "tools_call",
                    "arguments": {
                        "tool": tool,
                        "args": Value::Object(arguments),
                        "detail": "full"
                    }
                }),
            )
            .await?;
        extract_tool_result(&result)
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, DataError> {
        let request = self
            .authorize(self.http.post(&self.cfg.url))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            }));
        let response = request
            .send()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve mcp: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DataError::Upstream(format!(
                "openobserve mcp returned {status}: {}",
                preview(&body)
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve mcp json: {error}")))?;
        if let Some(error) = value.get("error") {
            return Err(DataError::Upstream(format!(
                "openobserve mcp error: {error}"
            )));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }

    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.cfg.auth {
            OoAuth::None => request,
            OoAuth::Basic { username, password } => {
                request.basic_auth(username, Some(password))
            }
            OoAuth::Bearer(token) => request.bearer_auth(token),
        }
    }
}

pub struct OoMcpTool {
    exposed_name: String,
    upstream_name: String,
    description: String,
    schema: Value,
    client: Arc<OoMcpClient>,
}

#[async_trait]
impl Tool for OoMcpTool {
    fn name(&self) -> &str {
        &self.exposed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(self.schema.clone())
    }

    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContext>,
        args: Value,
    ) -> Result<Value, AdkError> {
        self.client
            .call_tool(&self.upstream_name, args)
            .await
            .map_err(|error| AdkError::tool(error.to_string()))
    }
}

#[async_trait]
impl crate::api::state::Probe for OoMcpClient {
    async fn probe(&self) -> Result<(), DataError> {
        self.rpc("tools/list", json!({})).await.map(|_| ())
    }
}

#[derive(Clone)]
struct ToolDefinition {
    name: String,
    description: String,
    schema: Value,
}

fn tools_from_list(result: &Value) -> Vec<ToolDefinition> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item.get("name").and_then(Value::as_str)?;
                    Some(ToolDefinition {
                        name: name.to_string(),
                        description: item
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("OpenObserve tool")
                            .to_string(),
                        schema: item
                            .get("inputSchema")
                            .or_else(|| item.get("input_schema"))
                            .cloned()
                            .unwrap_or_else(|| json!({"type": "object"})),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strip_org_id(mut schema: Value) -> Value {
    if let Some(object) = schema.as_object_mut() {
        let mut empty = false;
        if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
            properties.remove("org_id");
        }
        if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|value| value.as_str() != Some("org_id"));
            empty = required.is_empty();
        }
        if empty {
            object.remove("required");
        }
    }
    schema
}

fn extract_tool_result(result: &Value) -> Result<Value, DataError> {
    if result.get("isError").and_then(Value::as_bool) == Some(true)
        || result.get("is_error").and_then(Value::as_bool) == Some(true)
    {
        return Err(DataError::Upstream(
            result
                .get("content")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("openobserve mcp tool error")
                .to_string(),
        ));
    }
    if let Some(structured) = result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"))
    {
        return Ok(structured.clone());
    }
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| DataError::Upstream("openobserve mcp returned no content".to_string()))?;
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(_) => Ok(Value::String(text.to_string())),
    }
}

fn preview(value: &str) -> String {
    let mut chars = value.chars();
    let preview: String = chars.by_ref().take(200).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}