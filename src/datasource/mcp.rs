use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, ErrorCode, Implementation,
    ProtocolVersion,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt, RoleClient};
use serde_json::{Value, json};

use crate::datasource::InstanceSource;
use crate::domain::{Instance, InstanceDetail, LogLevelChange, LogLevelState, PluginInfo};
use crate::error::DataError;

type McpClient = RunningService<RoleClient, ClientConfig>;

/// One endpoint to probe. `host` is the telemetry identity that published the
/// endpoint, so a failed probe still merges with its telemetry row instead of
/// appearing as a separate instance named after the address.
#[derive(Debug, Clone)]
pub struct Probe {
    pub endpoint: String,
    pub host: Option<String>,
}

impl Probe {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            host: None,
        }
    }

    pub fn discovered(endpoint: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            host: Some(host.into()),
        }
    }
}

#[async_trait]
pub trait InstanceTransport: Send + Sync {
    async fn call(&self, endpoint: &str, tool: &str, args: Value) -> Result<Value, DataError>;

    /// Call a tool that changes runtime state. The default keeps the read
    /// path's behavior; real transports classify upstream validation errors.
    async fn call_control(
        &self,
        endpoint: &str,
        tool: &str,
        args: Value,
    ) -> Result<Value, DataError> {
        self.call(endpoint, tool, args).await
    }

    /// Call several no-argument tools on one endpoint. The default keeps the
    /// per-call behavior; real transports reuse a single MCP session so a
    /// detail view costs one handshake instead of one per tool.
    async fn call_tools(
        &self,
        endpoint: &str,
        tools: &[&'static str],
    ) -> Vec<Result<Value, DataError>> {
        let mut results = Vec::with_capacity(tools.len());
        for tool in tools {
            results.push(
                self.call(endpoint, tool, Value::Object(Default::default()))
                    .await,
            );
        }
        results
    }
}

#[derive(Clone, Copy)]
enum CallKind {
    Read,
    Control,
}

impl CallKind {
    fn wrap(self, message: String) -> DataError {
        match self {
            Self::Read => DataError::Upstream(message),
            // Control failures reach the model, so keep the answer
            // endpoint-free and generic; the transport detail stays in the log.
            Self::Control => {
                tracing::warn!(error = %message, "mcp control call failed");
                DataError::Control("mcp control call failed; see the console log".to_string())
            }
        }
    }

    fn wrap_service(self, error: ServiceError, endpoint: &str, tool: &str) -> DataError {
        match self {
            Self::Read => DataError::Upstream(format!("mcp call {endpoint}/{tool}: {error}")),
            Self::Control => match error {
                ServiceError::McpError(mcp) if mcp.code == ErrorCode::INVALID_PARAMS => {
                    DataError::Invalid(mcp.message.into_owned())
                }
                other => {
                    tracing::warn!(endpoint, tool, error = %other, "mcp control call failed");
                    DataError::Control("mcp control call failed; see the console log".to_string())
                }
            },
        }
    }
}

pub struct RmcpTransport {
    token: Option<String>,
    timeout: Duration,
    // Reused across probes so an instance sweep keeps connections warm.
    http: reqwest::Client,
}

impl RmcpTransport {
    pub fn new(token: Option<String>, timeout: Duration) -> Self {
        Self {
            token,
            timeout,
            http: reqwest::Client::new(),
        }
    }

    async fn connect(&self, endpoint: &str) -> Result<McpClient, String> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint);
        if let Some(token) = &self.token {
            // rmcp adds the `Bearer ` prefix itself (reqwest `bearer_auth`).
            config = config.auth_header(token.clone());
        }
        let transport = StreamableHttpClientTransport::with_client(self.http.clone(), config);
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
        tokio::time::timeout(self.timeout, connect)
            .await
            .map_err(|_| format!("mcp connect timeout: {endpoint}"))?
            .map_err(|error| format!("mcp connect {endpoint}: {error}"))
    }

    async fn call_on(
        &self,
        client: &McpClient,
        endpoint: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value, DataError> {
        self.call_on_kind(client, endpoint, tool, args, CallKind::Read)
            .await
    }

    async fn call_on_kind(
        &self,
        client: &McpClient,
        endpoint: &str,
        tool: &str,
        args: &Value,
        kind: CallKind,
    ) -> Result<Value, DataError> {
        let arguments = args.as_object().cloned().unwrap_or_default();
        let result = tokio::time::timeout(
            self.timeout,
            client
                .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments)),
        )
        .await
        .map_err(|_| kind.wrap(format!("mcp call timeout: {endpoint}/{tool}")))?
        .map_err(|error| kind.wrap_service(error, endpoint, tool))?;

        if result.is_error == Some(true) {
            let message = result
                .content
                .iter()
                .find_map(|content| content.as_text())
                .map(|text| text.text.clone())
                .unwrap_or_else(|| "unknown mcp tool error".to_string());
            return Err(kind.wrap(message));
        }
        if let Some(structured) = result.structured_content {
            return Ok(structured);
        }
        let text = result
            .content
            .iter()
            .find_map(|content| content.as_text())
            .map(|text| text.text.clone())
            .ok_or_else(|| kind.wrap("mcp returned no text content".to_string()))?;
        match serde_json::from_str(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(Value::String(text)),
        }
    }
}

#[async_trait]
impl InstanceTransport for RmcpTransport {
    async fn call(&self, endpoint: &str, tool: &str, args: Value) -> Result<Value, DataError> {
        let client = self.connect(endpoint).await.map_err(DataError::Upstream)?;
        let result = self.call_on(&client, endpoint, tool, &args).await;
        let _ = client.cancel().await;
        result
    }

    async fn call_control(
        &self,
        endpoint: &str,
        tool: &str,
        args: Value,
    ) -> Result<Value, DataError> {
        let client = self.connect(endpoint).await.map_err(|error| {
            tracing::warn!(endpoint, error = %error, "mcp control connect failed");
            DataError::Control("mcp control endpoint unreachable".to_string())
        })?;
        let result = self
            .call_on_kind(&client, endpoint, tool, &args, CallKind::Control)
            .await;
        let _ = client.cancel().await;
        result
    }

    async fn call_tools(
        &self,
        endpoint: &str,
        tools: &[&'static str],
    ) -> Vec<Result<Value, DataError>> {
        let client = match self.connect(endpoint).await {
            Ok(client) => client,
            Err(reason) => {
                return tools
                    .iter()
                    .map(|_| Err(DataError::Upstream(reason.clone())))
                    .collect();
            }
        };
        let empty = Value::Object(Default::default());
        let mut results = Vec::with_capacity(tools.len());
        for tool in tools {
            results.push(self.call_on(&client, endpoint, tool, &empty).await);
        }
        let _ = client.cancel().await;
        results
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

    /// Statically configured endpoints (manual override / escape hatch).
    pub fn endpoints(&self) -> &[String] {
        &self.endpoints
    }

    async fn status_at(&self, endpoint: &str) -> Result<Value, DataError> {
        self.transport
            .call(endpoint, "get_status", Value::Object(Default::default()))
            .await
    }

    /// Query each endpoint's `get_status`. Unreachable endpoints degrade to an
    /// offline instance instead of failing the whole list.
    pub async fn list_endpoints(&self, probes: &[Probe]) -> Vec<Instance> {
        let futures = probes.iter().map(|probe| self.status_at(&probe.endpoint));
        let results = futures::future::join_all(futures).await;
        let mut instances = Vec::new();
        for (probe, result) in probes.iter().zip(results) {
            match result {
                Ok(value) => match instance_from_status(&value) {
                    Some(instance) => instances.push(instance),
                    None => {
                        tracing::warn!(endpoint = %probe.endpoint, "mcp get_status missing instance identity")
                    }
                },
                Err(error) => {
                    tracing::warn!(endpoint = %probe.endpoint, error = %error, "mcp instance unavailable");
                    instances.push(unreachable_instance(&probe.endpoint, probe.host.as_deref()));
                }
            }
        }
        instances
    }

    /// The first probe whose `get_status` identifies `id` (or its host), with
    /// the identity and the raw status payload. Unreachable or foreign probes
    /// are logged and skipped, so a later endpoint can still answer.
    async fn matched_status<'a>(
        &self,
        probes: &'a [Probe],
        id: &str,
        context: &'static str,
        exact: bool,
    ) -> Option<(&'a Probe, Instance, Value)> {
        for probe in probes {
            let status = match self.status_at(&probe.endpoint).await {
                Ok(status) => status,
                Err(error) => {
                    tracing::warn!(endpoint = %probe.endpoint, error = %error, "mcp {} unavailable", context);
                    continue;
                }
            };
            let Some(instance) = instance_from_status(&status) else {
                tracing::warn!(endpoint = %probe.endpoint, "mcp get_status missing instance identity");
                continue;
            };
            let matches = if exact {
                instance.id == id
            } else {
                instance.id == id || instance.host == id
            };
            if !matches {
                continue;
            }
            return Some((probe, instance, status));
        }
        None
    }

    pub async fn detail_at(&self, probes: &[Probe], id: &str) -> Result<InstanceDetail, DataError> {
        let Some((probe, mut instance, status)) =
            self.matched_status(probes, id, "detail", false).await
        else {
            return Err(DataError::NotFound(format!("instance {id}")));
        };
        instance.mcp_reachable = true;
        instance.online = true;

        let mut auxiliary = self
            .transport
            .call_tools(
                &probe.endpoint,
                &["list_plugins", "get_config", "get_runtime_status"],
            )
            .await
            .into_iter();
        let plugins = auxiliary
            .next()
            .and_then(Result::ok)
            .map(|value| plugins_from_value(&value))
            .unwrap_or_default();
        let config = auxiliary.next().and_then(Result::ok);
        let config_toml = config
            .as_ref()
            .and_then(|value| value.get("content").and_then(Value::as_str))
            .map(ToString::to_string);
        let redacted_keys = config
            .as_ref()
            .map(|value| string_array(value, "redactedKeys"))
            .unwrap_or_default();
        let runtime = auxiliary.next().and_then(Result::ok);

        Ok(InstanceDetail {
            instance,
            pid: status
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|pid| u32::try_from(pid).ok()),
            status: status
                .get("status")
                .and_then(Value::as_str)
                .map(ToString::to_string),
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
            redacted_keys,
            runtime,
            failure_count: None,
        })
    }

    pub async fn set_log_level_at(
        &self,
        probes: &[Probe],
        id: &str,
        change: LogLevelChange,
    ) -> Result<LogLevelState, DataError> {
        let Some((probe, _, _)) = self.matched_status(probes, id, "control", true).await else {
            return Err(DataError::NotFound(format!("instance {id}")));
        };
        let args = match &change {
            LogLevelChange::Set(directives) => json!({ "directives": directives }),
            LogLevelChange::Reset => json!({ "reset": true }),
        };
        let state = self
            .transport
            .call_control(&probe.endpoint, "set_log_level", args)
            .await?;
        log_level_from_value(&state).ok_or_else(|| {
            DataError::Control("mcp set_log_level returned an unexpected body".to_string())
        })
    }
}

#[async_trait]
impl InstanceSource for McpInstanceSource {
    async fn list(&self) -> Result<Vec<Instance>, DataError> {
        Ok(self.list_endpoints(&self.static_probes()).await)
    }

    async fn detail(&self, id: &str) -> Result<InstanceDetail, DataError> {
        self.detail_at(&self.static_probes(), id).await
    }
}

impl McpInstanceSource {
    fn static_probes(&self) -> Vec<Probe> {
        self.endpoints.iter().cloned().map(Probe::new).collect()
    }
}

fn unreachable_instance(endpoint: &str, host: Option<&str>) -> Instance {
    let host = host
        .map(str::to_string)
        .or_else(|| {
            url::Url::parse(endpoint)
                .ok()
                .and_then(|url| url.host_str().map(ToString::to_string))
        })
        .unwrap_or_else(|| endpoint.to_string());
    Instance {
        id: host.clone(),
        host,
        version: None,
        online: false,
        last_seen: 0,
        mcp_reachable: false,
    }
}

fn instance_from_status(value: &Value) -> Option<Instance> {
    let id = value
        .get("instanceId")
        .or_else(|| value.get("instance_id"))
        .and_then(Value::as_str)?;
    let host = value.get("host").and_then(Value::as_str).unwrap_or(id);
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

fn log_level_from_value(value: &Value) -> Option<LogLevelState> {
    let object = value.as_object()?;
    if !object.contains_key("stdout") || !object.contains_key("otlp") {
        return None;
    }
    Some(LogLevelState {
        stdout: value
            .get("stdout")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        otlp: value
            .get("otlp")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
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
                        enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(true),
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

#[cfg(test)]
mod tests {
    use rmcp::model::ErrorData;
    use rmcp::service::ServiceError;

    use super::*;

    #[test]
    fn invalid_params_from_a_control_call_is_the_callers_error() {
        let mapped = CallKind::Control.wrap_service(
            ServiceError::McpError(ErrorData::invalid_params("bad directives", None)),
            "http://node/mcp",
            "set_log_level",
        );
        assert!(matches!(mapped, DataError::Invalid(message) if message == "bad directives"));
    }

    #[test]
    fn other_control_failures_are_control_errors() {
        let mapped = CallKind::Control.wrap_service(
            ServiceError::TransportClosed,
            "http://node/mcp",
            "set_log_level",
        );
        assert!(matches!(
            mapped,
            DataError::Control(message) if message == "mcp control call failed; see the console log"
        ));
    }

    #[test]
    fn read_calls_keep_their_upstream_error() {
        let mapped = CallKind::Read.wrap_service(
            ServiceError::McpError(ErrorData::internal_error("boom", None)),
            "http://node/mcp",
            "get_status",
        );
        assert!(matches!(mapped, DataError::Upstream(_)));
    }
}
