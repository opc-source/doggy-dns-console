//! Drill-down access for the forensic agent loop.
//!
//! The model sees exposed tool names (`openobserve_search_sql`), while the MCP
//! client accepts upstream names (`SearchSQL`). This module owns that mapping
//! and keeps the loop independent from the transport.

use std::sync::Arc;

use adk_rust::prelude::Tool;
use async_trait::async_trait;
use serde_json::Value;

use crate::agent::openobserve_mcp::{ALLOWED_OPENOBSERVE_TOOLS, OpenObserveMcpClient};

/// Executes the read-only tools an analysis attempt may use.
#[async_trait]
pub trait Drilldown: Send + Sync {
    /// Tool declarations exactly as the model should see them.
    fn tool_declarations(&self) -> Vec<Value>;

    /// Runs one exposed tool name. Errors are returned to the model, never
    /// escalated as analysis failures.
    async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value, String>;
}

/// Maps an exposed tool name back to the upstream MCP name.
pub fn upstream_tool_name(exposed: &str) -> Option<&'static str> {
    ALLOWED_OPENOBSERVE_TOOLS
        .iter()
        .find(|(_, name)| *name == exposed)
        .map(|(upstream, _)| *upstream)
}

/// OpenObserve-backed drill-down. Tool declarations are discovered once.
pub struct OpenObserveDrilldown {
    client: Arc<OpenObserveMcpClient>,
    declarations: Vec<Value>,
}

impl OpenObserveDrilldown {
    /// Discovers the exposed declarations. Discovery failure leaves the agent
    /// with no drill-down tools; it can still submit from the bundle alone.
    pub async fn new(client: Arc<OpenObserveMcpClient>) -> Self {
        let declarations = match client.allowed_tools().await {
            Ok(tools) => tools
                .iter()
                .map(|tool: &Arc<dyn Tool>| tool.declaration())
                .collect(),
            Err(error) => {
                tracing::warn!(%error, "openobserve drilldown tool discovery failed");
                Vec::new()
            }
        };
        Self {
            client,
            declarations,
        }
    }
}

#[async_trait]
impl Drilldown for OpenObserveDrilldown {
    fn tool_declarations(&self) -> Vec<Value> {
        self.declarations.clone()
    }

    async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value, String> {
        let upstream = upstream_tool_name(tool_name)
            .ok_or_else(|| format!("tool not allowed: {tool_name}"))?;
        self.client
            .call_tool(upstream, args)
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::upstream_tool_name;

    #[test]
    fn exposed_names_map_to_upstream_names() {
        assert_eq!(
            upstream_tool_name("openobserve_search_sql"),
            Some("SearchSQL")
        );
        assert_eq!(
            upstream_tool_name("openobserve_trace_dag"),
            Some("GetTraceDAG")
        );
        assert_eq!(upstream_tool_name("SearchSQL"), None);
        assert_eq!(upstream_tool_name("unknown"), None);
    }
}
