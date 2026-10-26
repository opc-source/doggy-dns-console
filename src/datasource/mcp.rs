use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::Value;

use crate::datasource::InstanceSource;
use crate::domain::{Instance, InstanceDetail, PluginInfo};
use crate::error::DataError;

#[async_trait]
pub trait InstanceTransport: Send + Sync {
    async fn call(&self, endpoint: &str, tool: &str, args: Value) -> Result<Value, DataError>;
}

pub struct RmcpTransport {
    token: Option<String>,
    timeout: Duration,
}

impl RmcpTransport {
    pub fn new(token: Option<String>, timeout: Duration) -> Self {
        Self { token, timeout }
    }
}

#[async_trait]
impl InstanceTransport for RmcpTransport {
    async fn call(&self, endpoint: &str, tool: &str, args: Value) -> Result<Value, DataError> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint);
        if let Some(token) = &self.token {
            config = config.auth_header(format!("Bearer {token}"));
        }
        let transport =
            StreamableHttpClientTransport::with_client(reqwest::Client::new(), config);
        let client_config = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("doggy-dns-console", env!("CARGO_PKG_VERSION")),
        );
        let connect = client_config.serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Auto {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                legacy_version: Some(ProtocolVersion::V_2025_11_25),
            },
        );
        let client = tokio::time::timeout(self.timeout, connect)
            .await
            .map_err(|_| DataError::Upstream(format!("mcp connect timeout: {endpoint}")))?
            .map_err(|error| DataError::Upstream(format!("mcp connect {endpoint}: {error}")))?;

        let arguments = args.as_object().cloned().unwrap_or_default();
        let result = tokio::time::timeout(
            self.timeout,
            client.call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments)),
        )
        .await
        .map_err(|_| DataError::Upstream(format!("mcp call timeout: {endpoint}/{tool}")))?
        .map_err(|error| DataError::Upstream(format!("mcp call {endpoint}/{tool}: {error}")))?;
        let _ = client.cancel().await;

        if result.is_error == Some(true) {
            let message = result
                .content
                .iter()
                .find_map(|content| content.as_text())
                .map(|text| text.text.clone())
                .unwrap_or_else(|| "unknown mcp tool error".to_string());
            return Err(DataError::Upstream(message));
        }
        if let Some(structured) = result.structured_content {
            return Ok(structured);
        }
        let text = result
            .content
            .iter()
            .find_map(|content| content.as_text())
            .map(|text| text.text.clone())
            .ok_or_else(|| DataError::Upstream("mcp returned no text content".to_string()))?;
        match serde_json::from_str(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(Value::String(text)),
        }
    }
}

pub struct McpInstanceSource {
    endpoints: Vec<String>,
    transport: Arc<dyn InstanceTransport>,
}

impl McpInstanceSource {
    pub fn new(endpoints: Vec<String>, transport: Arc<dyn InstanceTransport>) -> Self {
        Self {
            endpoints,
            transport,
        }
    }

    async fn status_at(&self, endpoint: &str) -> Result<Value, DataError> {
        self.transport.call(endpoint, "get_status", Value::Object(Default::default())).await
    }
}

#[async_trait]
impl InstanceSource for McpInstanceSource {
    async fn list(&self) -> Result<Vec<Instance>, DataError> {
        let futures = self
            .endpoints
            .iter()
            .map(|endpoint| self.status_at(endpoint));
        let results = futures::future::join_all(futures).await;
        let mut instances = Vec::new();
        for (endpoint, result) in self.endpoints.iter().zip(results) {
            match result {
                Ok(value) => match instance_from_status(&value) {
                    Some(instance) => instances.push(instance),
                    None => tracing::warn!(endpoint = %endpoint, "mcp get_status missing instance identity"),
                },
                Err(error) => {
                    tracing::warn!(endpoint = %endpoint, error = %error, "mcp instance unavailable");
                }
            }
        }
        Ok(instances)
    }

    async fn detail(&self, id: &str) -> Result<InstanceDetail, DataError> {
        for endpoint in &self.endpoints {
            let status = match self.status_at(endpoint).await {
                Ok(status) => status,
                Err(error) => {
                    tracing::warn!(endpoint = %endpoint, error = %error, "mcp detail unavailable");
                    continue;
                }
            };
            let Some(mut instance) = instance_from_status(&status) else {
                continue;
            };
            if instance.id != id && instance.host != id {
                continue;
            }
            instance.mcp_reachable = true;
            instance.online = true;

            let plugins = self
                .transport
                .call(endpoint, "list_plugins", Value::Object(Default::default()))
                .await
                .ok()
                .map(|value| plugins_from_value(&value))
                .unwrap_or_default();
            let config_toml = self
                .transport
                .call(endpoint, "get_config", Value::Object(Default::default()))
                .await
                .ok()
                .and_then(|value| value.get("content").and_then(Value::as_str).map(ToString::to_string));

            return Ok(InstanceDetail {
                instance,
                uptime_seconds: status.get("uptimeSeconds").and_then(Value::as_u64),
                started_at: status
                    .get("startedAt")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                listen_addr: status
                    .get("listenAddr")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                protocols: status
                    .get("protocols")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                plugins,
                config_toml,
                recent_failures: 0,
            });
        }
        Err(DataError::NotFound(format!("instance {id}")))
    }
}

fn instance_from_status(value: &Value) -> Option<Instance> {
    let id = value
        .get("instanceId")
        .or_else(|| value.get("instance_id"))
        .and_then(Value::as_str)?;
    let host = value
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or(id);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    Some(Instance {
        id: id.to_string(),
        host: host.to_string(),
        version: value
            .get("version")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        online: true,
        last_seen: now,
        mcp_reachable: true,
    })
}

fn plugins_from_value(value: &Value) -> Vec<PluginInfo> {
    value
        .get("plugins")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(PluginInfo {
                        order: item.get("order").and_then(Value::as_u64).unwrap_or(0) as usize,
                        kind: item.get("kind").and_then(Value::as_str)?.to_string(),
                        enabled: item
                            .get("enabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        details: item
                            .get("details")
                            .cloned()
                            .unwrap_or(Value::Object(Default::default())),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}