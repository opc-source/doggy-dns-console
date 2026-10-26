use std::sync::Arc;

use anyhow::Result;
use doggy_dns_console::agent::oo_mcp::OoMcpClient;
use doggy_dns_console::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env()?;
    println!("OpenObserve MCP: {}", config.oo_mcp.url);
    let client = Arc::new(OoMcpClient::new(config.oo_mcp)?);
    let tools = client.allowed_tools().await?;
    println!("allowlisted tools ({}):", tools.len());
    for tool in tools {
        println!(
            "\n- {} ({})",
            tool.name(),
            tool.description()
        );
        if let Some(schema) = tool.parameters_schema() {
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
    }
    Ok(())
}
