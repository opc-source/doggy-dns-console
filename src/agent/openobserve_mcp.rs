use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Mutex;

use adk_rust::prelude::{AdkError, Tool, ToolContext};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

use crate::config::{OpenObserveAuth, OpenObserveMcpConfig};
use crate::error::DataError;

/// (upstream MCP tool name, exposed tool name). The exposed name is what the model sees.
pub const ALLOWED_OPENOBSERVE_TOOLS: [(&str, &str); 8] = [
    ("StreamList", "openobserve_stream_list"),
    ("StreamSchema", "openobserve_stream_schema"),
    ("SearchSQL", "openobserve_search_sql"),
    ("PrometheusQuery", "openobserve_promql_query"),
    ("PrometheusRangeQuery", "openobserve_promql_range"),
    ("GetLatestTraces", "openobserve_latest_traces"),
    ("GetTraceDAG", "openobserve_trace_dag"),
    ("SearchAround", "openobserve_search_around"),
];

pub struct OpenObserveMcpClient {
    http: Client,
    cfg: OpenObserveMcpConfig,
    session: Mutex<Option<String>>,
    initialized: AtomicBool,
}

impl OpenObserveMcpClient {
    pub fn new(cfg: OpenObserveMcpConfig) -> Result<Self, DataError> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|error| DataError::Upstream(format!("openobserve mcp client: {error}")))?;
        Ok(Self {
            http,
            cfg,
            session: Mutex::new(None),
            initialized: AtomicBool::new(false),
        })
    }

    pub async fn allowed_tools(self: &Arc<Self>) -> Result<Vec<Arc<dyn Tool>>, DataError> {
        let list = match self.rpc_with_handshake("tools/list", json!({})).await {
            Ok(value) => tools_from_list(&value),
            Err(error) => {
                tracing::warn!(error = %error, "openobserve mcp tools/list failed; disabling raw tools");
                return Ok(Vec::new());
            }
        };

        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for (upstream, exposed) in ALLOWED_OPENOBSERVE_TOOLS {
            let definition = list.iter().find(|tool| tool.name == upstream).cloned();
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
            tools.push(Arc::new(OpenObserveMcpTool {
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
            .rpc_with_handshake(
                "tools/call",
                json!({
                    "name": "tool_search",
                    "arguments": { "query": name, "limit": 5 }
                }),
            )
            .await?;
        let result = extract_tool_result(&result)?;
        Ok(tools_from_list(&result)
            .into_iter()
            .find(|tool| tool.name == name))
    }

    pub async fn call_tool(&self, tool: &str, args: Value) -> Result<Value, DataError> {
        if !ALLOWED_OPENOBSERVE_TOOLS
            .iter()
            .any(|(upstream, _)| *upstream == tool)
        {
            return Err(DataError::Invalid(format!("tool not allowed: {tool}")));
        }
        let mut arguments = args.as_object().cloned().unwrap_or_default();
        arguments.insert("org_id".to_string(), Value::String(self.cfg.org.clone()));
        let result = self
            .rpc_with_handshake(
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

    async fn rpc_with_handshake(&self, method: &str, params: Value) -> Result<Value, DataError> {
        match self.rpc(method, params.clone()).await {
            Ok(value) => Ok(value),
            Err(first) => {
                if self.ensure_initialized().await.is_ok() {
                    self.rpc(method, params).await
                } else {
                    Err(first)
                }
            }
        }
    }

    async fn ensure_initialized(&self) -> Result<(), DataError> {
        if self.initialized.load(Ordering::SeqCst) {
            return Ok(());
        }
        let (value, session) = self
            .post(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "doggy-dns-console",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }))
            .await?;
        if let Some(error) = value.get("error") {
            return Err(DataError::Upstream(format!(
                "openobserve mcp initialize: {error}"
            )));
        }
        if let Some(session) = session {
            *self.session.lock().await = Some(session);
        }
        let _ = self
            .post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        self.initialized.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, DataError> {
        let (value, _) = self
            .post(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            }))
            .await?;
        if let Some(error) = value.get("error") {
            return Err(DataError::Upstream(format!(
                "openobserve mcp error: {error}"
            )));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn post(&self, body: Value) -> Result<(Value, Option<String>), DataError> {
        let mut request = self
            .authorize(self.http.post(&self.cfg.url))
            .header("accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(session) = self.session.lock().await.as_deref() {
            request = request.header("mcp-session-id", session);
        }
        let response = request
            .send()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve mcp: {error}")))?;
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(DataError::Upstream(format!(
                "openobserve mcp returned {status}: {}",
                preview(&text)
            )));
        }
        let value = if content_type.contains("text/event-stream") {
            parse_sse_json(&text).ok_or_else(|| {
                DataError::Upstream("openobserve mcp returned an invalid SSE stream".to_string())
            })?
        } else {
            serde_json::from_str(&text)
                .map_err(|error| DataError::Upstream(format!("openobserve mcp json: {error}")))?
        };
        Ok((value, session))
    }

    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.cfg.auth {
            OpenObserveAuth::None => request,
            OpenObserveAuth::Basic { username, password } => {
                request.basic_auth(username, Some(password))
            }
            OpenObserveAuth::Bearer(token) => request.bearer_auth(token),
        }
    }
}

pub struct OpenObserveMcpTool {
    exposed_name: String,
    upstream_name: String,
    description: String,
    schema: Value,
    client: Arc<OpenObserveMcpClient>,
}

#[async_trait]
impl Tool for OpenObserveMcpTool {
    fn name(&self) -> &str {
        &self.exposed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(self.schema.clone())
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value, AdkError> {
        self.client
            .call_tool(&self.upstream_name, args)
            .await
            .map_err(|error| AdkError::tool(error.to_string()))
    }
}

#[async_trait]
impl crate::api::state::Probe for OpenObserveMcpClient {
    async fn probe(&self) -> Result<(), DataError> {
        self.rpc_with_handshake("tools/list", json!({}))
            .await
            .map(|_| ())
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

fn parse_sse_json(text: &str) -> Option<Value> {
    let mut last = None;
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let data = block
            .lines()
            .filter(|line| line.starts_with("data:"))
            .map(|line| line[5..].trim_start())
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&data)
            && (value.get("result").is_some() || value.get("error").is_some())
        {
            last = Some(value);
        }
    }
    last
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
