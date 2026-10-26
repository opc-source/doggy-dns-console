use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OoAuth {
    None,
    Basic { username: String, password: String },
    Bearer(String),
}

#[derive(Debug, Clone)]
pub struct OpenObserveConfig {
    pub base_url: String,
    pub org: String,
    pub auth: OoAuth,
    pub log_stream: String,
    pub trace_stream: String,
    pub metric_queries: String,
    pub metric_duration: String,
}

#[derive(Debug, Clone)]
pub struct OoMcpConfig {
    pub url: String,
    pub org: String,
    pub auth: OoAuth,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct McpConfig {
    pub endpoints: Vec<String>,
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
    pub openobserve: OpenObserveConfig,
    pub oo_mcp: OoMcpConfig,
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

        let oo_base_url = text("OO_BASE_URL");
        if !demo && oo_base_url.is_none() {
            return Err(ConfigError::Missing("OO_BASE_URL"));
        }
        let oo_base_url = oo_base_url.unwrap_or_default();
        let oo_org = text("OO_ORG").unwrap_or_else(|| "default".to_string());

        let oo_auth = match (text("OO_TOKEN"), text("OO_USER"), text("OO_PASSWORD")) {
            (Some(token), _, _) => OoAuth::Bearer(token),
            (None, Some(username), Some(password)) => OoAuth::Basic { username, password },
            _ => OoAuth::None,
        };

        let model_api_key = text("MODEL_API_KEY");
        if !demo && model_api_key.is_none() {
            return Err(ConfigError::Missing("MODEL_API_KEY"));
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

        let oo_mcp_url = text("OO_MCP_URL").unwrap_or_else(|| {
            if oo_base_url.is_empty() {
                String::new()
            } else {
                format!("{}/api/{}/mcp", oo_base_url.trim_end_matches('/'), oo_org)
            }
        });
        let oo_mcp_auth = match text("OO_MCP_TOKEN") {
            Some(token) => OoAuth::Bearer(token),
            None => oo_auth.clone(),
        };

        Ok(Self {
            addr,
            db_url: text("CONSOLE_DB").unwrap_or_else(|| "sqlite:data/console.db".to_string()),
            app_name: text("CONSOLE_APP_NAME").unwrap_or_else(|| "doggy-dns-console".to_string()),
            user_id: text("CONSOLE_USER_ID").unwrap_or_else(|| "local".to_string()),
            demo,
            openobserve: OpenObserveConfig {
                base_url: oo_base_url,
                org: oo_org.clone(),
                auth: oo_auth,
                log_stream: text("OO_LOG_STREAM").unwrap_or_else(|| "doggy-dns".to_string()),
                trace_stream: text("OO_TRACE_STREAM").unwrap_or_else(|| "doggy-dns".to_string()),
                metric_queries: text("OO_METRIC_QUERIES")
                    .unwrap_or_else(|| "dns_queries_total".to_string()),
                metric_duration: text("OO_METRIC_DURATION")
                    .unwrap_or_else(|| "dns_query_duration_milliseconds_bucket".to_string()),
            },
            oo_mcp: OoMcpConfig {
                url: oo_mcp_url.clone(),
                org: oo_org,
                auth: oo_mcp_auth,
                enabled: !demo && !oo_mcp_url.is_empty(),
            },
            mcp: McpConfig {
                endpoints: mcp_endpoints,
                token: text("MCP_TOKEN"),
                timeout: Duration::from_secs(2),
            },
            model: ModelConfig {
                base_url: text("MODEL_BASE_URL"),
                name: text("MODEL_NAME").unwrap_or_else(|| "deepseek-chat".to_string()),
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
