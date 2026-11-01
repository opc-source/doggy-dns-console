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

/// The streams a drill-down can read, so the prompt can name them instead of
/// letting the model guess and burn calls on "stream not found".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrilldownStreams {
    pub log_stream: Option<String>,
    pub trace_stream: Option<String>,
}

impl DrilldownStreams {
    /// Blank and whitespace-only names are treated as absent: the prompt states
    /// "no stream is configured" rather than offering an empty stream to query.
    pub fn new(log_stream: impl Into<String>, trace_stream: impl Into<String>) -> Self {
        fn named(value: String) -> Option<String> {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Self {
            log_stream: named(log_stream.into()),
            trace_stream: named(trace_stream.into()),
        }
    }
}

/// Executes the read-only tools an analysis attempt may use.
#[async_trait]
pub trait Drilldown: Send + Sync {
    /// Tool declarations exactly as the model should see them.
    fn tool_declarations(&self) -> Vec<Value>;

    /// Runs one exposed tool name. Errors are returned to the model, never
    /// escalated as analysis failures.
    async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value, String>;

    /// Streams the tools above query. The default (no streams) tells the prompt
    /// there is nothing to query, which is what a source-less drill-down is.
    fn streams(&self) -> DrilldownStreams {
        DrilldownStreams::default()
    }
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
    streams: DrilldownStreams,
}

impl OpenObserveDrilldown {
    /// Discovers the exposed declarations. Discovery failure leaves the agent
    /// with no drill-down tools; it can still submit from the bundle alone.
    /// `streams` are the configured query streams, reported to the model so its
    /// SQL names a stream that exists.
    pub async fn new(client: Arc<OpenObserveMcpClient>, streams: DrilldownStreams) -> Self {
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
            streams,
        }
    }
}

#[async_trait]
impl Drilldown for OpenObserveDrilldown {
    fn tool_declarations(&self) -> Vec<Value> {
        self.declarations.clone()
    }

    fn streams(&self) -> DrilldownStreams {
        self.streams.clone()
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
    use super::{DrilldownStreams, upstream_tool_name};

    #[test]
    fn blank_stream_names_are_treated_as_absent() {
        let streams = DrilldownStreams::new("  ", "doggy_dns");
        assert_eq!(
            streams.log_stream, None,
            "a blank name must not reach the model as a stream"
        );
        assert_eq!(streams.trace_stream.as_deref(), Some("doggy_dns"));
    }

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
