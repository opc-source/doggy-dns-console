use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenObserveAuth {
    None,
    Basic { username: String, password: String },
    Bearer(String),
}

#[derive(Debug, Clone)]
pub struct OpenObserveConfig {
    pub base_url: String,
    pub org: String,
    pub auth: OpenObserveAuth,
    pub log_stream: String,
    pub trace_stream: String,
    pub metric_queries: String,
    pub metric_duration: String,
}

#[derive(Debug, Clone)]
pub struct OpenObserveMcpConfig {
    pub url: String,
    pub org: String,
    pub auth: OpenObserveAuth,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Static endpoints; probed first as an explicit override, then endpoints
    /// discovered from telemetry are appended as an escape hatch.
    pub endpoints: Vec<String>,
    /// Port used to derive `http://{host}:{port}/mcp` when a node publishes no
    /// `doggy_dns.ip_addr`/`doggy_dns.control.port`.
    pub control_port: u16,
    pub token: Option<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub base_url: Option<String>,
    pub name: String,
    pub api_key: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub addr: SocketAddr,
    pub db_url: String,
    pub app_name: String,
    pub user_id: String,
    pub demo: bool,
    pub ui_dist: Option<PathBuf>,
    pub openobserve: OpenObserveConfig,
    pub openobserve_mcp: OpenObserveMcpConfig,
    pub mcp: McpConfig,
    pub model: ModelConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is required (set it in .env or the environment)")]
    Missing(&'static str),
    #[error("invalid {name}: {value}")]
    Invalid { name: &'static str, value: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_with(|key| std::env::var(key).ok())
    }

    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let text = |key: &str| {
            get(key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let demo = match text("CONSOLE_DEMO").as_deref() {
            None => false,
            Some(value) => parse_bool(value).ok_or_else(|| ConfigError::Invalid {
                name: "CONSOLE_DEMO",
                value: value.to_string(),
            })?,
        };

        let addr_raw = text("CONSOLE_ADDR").unwrap_or_else(|| "127.0.0.1:8787".to_string());
        let addr = addr_raw
            .parse::<SocketAddr>()
            .map_err(|_| ConfigError::Invalid {
                name: "CONSOLE_ADDR",
                value: addr_raw.clone(),
            })?;

        let openobserve_base_url = text("OPENOBSERVE_BASE_URL");
        if !demo && openobserve_base_url.is_none() {
            return Err(ConfigError::Missing("OPENOBSERVE_BASE_URL"));
        }
        let openobserve_base_url = openobserve_base_url.unwrap_or_default();
        let openobserve_org = text("OPENOBSERVE_ORG").unwrap_or_else(|| "default".to_string());

        let openobserve_auth = match (
            text("OPENOBSERVE_TOKEN"),
            text("OPENOBSERVE_USER"),
            text("OPENOBSERVE_PASSWORD"),
        ) {
            (Some(token), _, _) => OpenObserveAuth::Bearer(token),
            (None, Some(username), Some(password)) => OpenObserveAuth::Basic { username, password },
            _ => OpenObserveAuth::None,
        };

        let model_api_key = text("MODEL_API_KEY")
            .or_else(|| text("QWEN_API_KEY"))
            .or_else(|| text("OPENAI_API_KEY"));
        if !demo && model_api_key.is_none() {
            return Err(ConfigError::Missing(
                "MODEL_API_KEY, QWEN_API_KEY, or OPENAI_API_KEY",
            ));
        }

        let mcp_endpoints = text("MCP_ENDPOINTS")
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let mcp_control_port = match text("MCP_CONTROL_PORT") {
            None => 19090,
            Some(raw) => raw
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| ConfigError::Invalid {
                    name: "MCP_CONTROL_PORT",
                    value: raw.clone(),
                })?,
        };

        let openobserve_mcp_url = text("OPENOBSERVE_MCP_URL").unwrap_or_else(|| {
            if openobserve_base_url.is_empty() {
                String::new()
            } else {
                format!(
                    "{}/api/{}/mcp",
                    openobserve_base_url.trim_end_matches('/'),
                    openobserve_org
                )
            }
        });
        let openobserve_mcp_auth = match text("OPENOBSERVE_MCP_TOKEN") {
            Some(token) => OpenObserveAuth::Bearer(token),
            None => openobserve_auth.clone(),
        };

        Ok(Self {
            addr,
            db_url: text("CONSOLE_DB").unwrap_or_else(|| "sqlite:data/console.db".to_string()),
            app_name: text("CONSOLE_APP_NAME").unwrap_or_else(|| "doggy-dns-console".to_string()),
            user_id: text("CONSOLE_USER_ID").unwrap_or_else(|| "local".to_string()),
            demo,
            ui_dist: Some(PathBuf::from(
                text("DOGGY_DNS_UI_DIST").unwrap_or_else(|| "doggy-dns-ui/dist".to_string()),
            )),
            openobserve: OpenObserveConfig {
                base_url: openobserve_base_url,
                org: openobserve_org.clone(),
                auth: openobserve_auth,
                log_stream: text("OPENOBSERVE_LOG_STREAM")
                    .unwrap_or_else(|| "doggy_dns".to_string()),
                trace_stream: text("OPENOBSERVE_TRACE_STREAM")
                    .unwrap_or_else(|| "doggy_dns".to_string()),
                metric_queries: text("OPENOBSERVE_METRIC_QUERIES")
                    .unwrap_or_else(|| "dns_queries".to_string()),
                metric_duration: text("OPENOBSERVE_METRIC_DURATION")
                    .unwrap_or_else(|| "dns_query_duration_bucket".to_string()),
            },
            openobserve_mcp: OpenObserveMcpConfig {
                url: openobserve_mcp_url.clone(),
                org: openobserve_org,
                auth: openobserve_mcp_auth,
                enabled: !demo && !openobserve_mcp_url.is_empty(),
            },
            mcp: McpConfig {
                endpoints: mcp_endpoints,
                control_port: mcp_control_port,
                token: text("MCP_TOKEN"),
                timeout: Duration::from_secs(2),
            },
            model: ModelConfig {
                base_url: text("MODEL_BASE_URL").or_else(|| text("OPENAI_BASE_URL")),
                name: text("MODEL_NAME").unwrap_or_else(|| "qwen3.8-flash".to_string()),
                api_key: model_api_key.unwrap_or_default(),
            },
        })
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}
