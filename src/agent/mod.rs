pub mod openobserve_mcp;
pub mod prompt;
pub mod tools;

use std::sync::Arc;

use adk_rust::TOOL_ERROR_MESSAGE_LIMIT;
use adk_rust::agent::LlmAgentBuilder;
use adk_rust::model::openai_compatible::{OpenAICompatible, OpenAICompatibleConfig};
use adk_rust::prelude::{AdkError, Agent, Llm, Tool};

use crate::config::ModelConfig;
use crate::service::ConsoleService;

pub fn build_model(cfg: &ModelConfig) -> anyhow::Result<Arc<dyn Llm>> {
    let mut config = OpenAICompatibleConfig::new(cfg.api_key.clone(), cfg.name.clone());
    if let Some(base_url) = &cfg.base_url {
        config = config.with_base_url(base_url.clone());
    }
    Ok(Arc::new(OpenAICompatible::new(config)?))
}

/// Matches ADK's own cap on a rejection's message.
fn bounded(message: String) -> String {
    if message.chars().count() <= TOOL_ERROR_MESSAGE_LIMIT {
        return message;
    }
    let head: String = message.chars().take(TOOL_ERROR_MESSAGE_LIMIT).collect();
    format!("{head}… (truncated)")
}

pub fn build_agent(
    model: Arc<dyn Llm>,
    service: Arc<ConsoleService>,
    extra_tools: Vec<Arc<dyn Tool>>,
) -> anyhow::Result<Arc<dyn Agent>> {
    let mut builder = LlmAgentBuilder::new("doggy-dns-console")
        .description(
            "DNS observability copilot for doggy-dns; read-only except runtime log-level control",
        )
        .instruction(prompt::SYSTEM_PROMPT)
        .model(model)
        // ADK withholds the message of an Unavailable tool error by default.
        // The control tool's answer is bounded and endpoint-free (transport
        // detail stays in the console log); raw OpenObserve tools keep the
        // default policy. A timeout or panic may have taken effect, so it keeps
        // ADK's outcome-unknown answer and the model checks before retrying.
        .on_tool_error(Box::new(
            |ctx: Arc<dyn adk_rust::CallbackContext>,
             tool: Arc<dyn Tool>,
             _args: serde_json::Value,
             message: String| {
                let mid_flight = ctx
                    .tool_outcome()
                    .and_then(|outcome| outcome.error)
                    .is_some_and(|error| matches!(error.code, "tool.timeout" | "tool.panicked"));
                let expose = tool.name() == "set_instance_log_level" && !mid_flight;
                Box::pin(async move {
                    Ok::<Option<serde_json::Value>, AdkError>(
                        expose.then(|| serde_json::json!({ "error": bounded(message) })),
                    )
                })
            },
        ));
    for tool in tools::build_tools(service) {
        builder = builder.tool(tool);
    }
    for tool in extra_tools {
        builder = builder.tool(tool);
    }
    Ok(Arc::new(builder.build()?))
}
