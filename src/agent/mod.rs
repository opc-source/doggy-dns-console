pub mod prompt;
pub mod tools;

use std::sync::Arc;

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::model::openai_compatible::{OpenAICompatible, OpenAICompatibleConfig};
use adk_rust::prelude::{Agent, Llm, Tool};

use crate::config::ModelConfig;
use crate::service::ConsoleService;

pub fn build_model(cfg: &ModelConfig) -> anyhow::Result<Arc<dyn Llm>> {
    let mut config = OpenAICompatibleConfig::new(cfg.api_key.clone(), cfg.name.clone());
    if let Some(base_url) = &cfg.base_url {
        config = config.with_base_url(base_url.clone());
    }
    Ok(Arc::new(OpenAICompatible::new(config)?))
}

pub fn build_agent(
    model: Arc<dyn Llm>,
    service: Arc<ConsoleService>,
    extra_tools: Vec<Arc<dyn Tool>>,
) -> anyhow::Result<Arc<dyn Agent>> {
    let mut builder = LlmAgentBuilder::new("doggy-dns-copilot")
        .description("Read-only DNS observability copilot for doggy-dns")
        .instruction(prompt::SYSTEM_PROMPT)
        .model(model);
    for tool in tools::build_tools(service) {
        builder = builder.tool(tool);
    }
    for tool in extra_tools {
        builder = builder.tool(tool);
    }
    Ok(Arc::new(builder.build()?))
}
