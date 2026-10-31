use std::sync::Arc;
use std::time::Duration;

use adk_rust::SingleAgentLoader;
use adk_rust::prelude::{Agent, Llm, Tool};
use adk_rust::server::{SecurityConfig, ServerBuilder, ServerConfig};
use adk_rust::session::{SessionService, SqliteSessionService};
use anyhow::Context;
use async_trait::async_trait;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

use crate::agent::{build_agent, build_model};
use crate::analysis::drilldown::{Drilldown, OpenObserveDrilldown};
use crate::analysis::service::AnalysisService;
use crate::analysis::store::SqliteInvestigationStore;
use crate::api::state::{ApiState, Probe};
use crate::config::Config;
use crate::datasource::fixture::FixtureSource;
use crate::datasource::instances::MergedInstanceSource;
use crate::datasource::mcp::{McpInstanceSource, RmcpTransport};
use crate::datasource::openobserve::{OpenObserveClient, OpenObserveTelemetry};
use crate::datasource::{Clock, InstanceSource, SystemClock, TelemetrySource};
use crate::error::DataError;
use crate::service::ConsoleService;

pub struct AppDeps {
    pub service: Arc<ConsoleService>,
    pub instances: Arc<dyn InstanceSource>,
    pub model: Arc<dyn Llm>,
    pub sessions: Arc<dyn SessionService>,
    pub extra_tools: Vec<Arc<dyn Tool>>,
    pub openobserve_health: Arc<dyn Probe>,
    pub openobserve_mcp_health: Option<Arc<dyn Probe>>,
    pub model_configured: bool,
    pub analysis: Option<Arc<AnalysisService>>,
}

pub async fn build_app(config: Config, deps: AppDeps) -> anyhow::Result<Router> {
    let agent: Arc<dyn Agent> = build_agent(deps.model, deps.service.clone(), deps.extra_tools)?;
    let loader = Arc::new(SingleAgentLoader::new(agent));
    let server_config =
        ServerConfig::new(loader, deps.sessions).with_security(SecurityConfig::development());
    let state = ApiState {
        service: deps.service,
        instances: deps.instances,
        openobserve_health: deps.openobserve_health,
        openobserve_mcp_health: deps.openobserve_mcp_health,
        model_configured: deps.model_configured,
        analysis: deps.analysis,
    };
    let mut builder =
        ServerBuilder::new(server_config).add_root_routes(crate::api::business_router(state));
    if let Some(dist) = &config.ui_dist
        && dist.is_dir()
    {
        let index = dist.join("index.html");
        builder = builder.add_root_routes(
            Router::new().fallback_service(ServeDir::new(dist).fallback(ServeFile::new(index))),
        );
    }
    Ok(builder.build())
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    // Both the session database and the investigation store share this path,
    // so it is prepared before either backend opens it.
    ensure_sqlite_parent(&config.db_url)?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let (
        service,
        instances,
        openobserve_health,
        openobserve_mcp_health,
        model,
        extra_tools,
        mcp_client,
    ) = if config.demo {
        let fixture = Arc::new(FixtureSource::new(clock.clone()));
        let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
        let instances = fixture as Arc<dyn InstanceSource>;
        let service = Arc::new(ConsoleService::new(telemetry, instances.clone()));
        (
            service,
            instances,
            Arc::new(AlwaysHealthy) as Arc<dyn Probe>,
            None,
            demo_model(),
            Vec::new(),
            None,
        )
    } else {
        let openobserve = Arc::new(OpenObserveClient::new(config.openobserve.clone())?);
        let telemetry: Arc<dyn TelemetrySource> = Arc::new(OpenObserveTelemetry::new(
            openobserve.clone(),
            clock.clone(),
        ));
        let mcp = Arc::new(McpInstanceSource::new(
            config.mcp.endpoints.clone(),
            Arc::new(RmcpTransport::new(
                config.mcp.token.clone(),
                config.mcp.timeout,
            )),
        ));
        let instances: Arc<dyn InstanceSource> = Arc::new(MergedInstanceSource::new(
            mcp,
            telemetry.clone(),
            config.mcp.control_port,
            clock.clone(),
        ));
        let service = Arc::new(ConsoleService::new(telemetry, instances.clone()));

        let (openobserve_mcp_health, extra_tools, mcp_client) = if config.openobserve_mcp.enabled {
            let client = Arc::new(crate::agent::openobserve_mcp::OpenObserveMcpClient::new(
                config.openobserve_mcp.clone(),
            )?);
            let tools = match client.allowed_tools().await {
                Ok(tools) => tools,
                Err(error) => {
                    tracing::warn!(error = %error, "openobserve mcp tools unavailable");
                    Vec::new()
                }
            };
            (Some(client.clone() as Arc<dyn Probe>), tools, Some(client))
        } else {
            (None, Vec::new(), None)
        };

        (
            service,
            instances,
            openobserve as Arc<dyn Probe>,
            openobserve_mcp_health,
            build_model(&config.model)?,
            extra_tools,
            mcp_client,
        )
    };

    let analysis = build_analysis(&config, model.clone(), mcp_client).await;

    let sqlite = SqliteSessionService::new(&config.db_url).await?;
    sqlite.migrate().await?;
    let sessions: Arc<dyn SessionService> = Arc::new(sqlite);
    let model_configured = config.demo || !config.model.api_key.is_empty();
    let deps = AppDeps {
        service,
        instances,
        model,
        sessions,
        extra_tools,
        openobserve_health,
        openobserve_mcp_health,
        model_configured,
        analysis,
    };
    let app = build_app(config.clone(), deps).await?;
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    tracing::info!(addr = %config.addr, "console listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Builds the optional analysis service.
///
/// Demo mode has no analysis service: the scripted chat model cannot produce a
/// valid forensic submission, and persisting guaranteed-invalid attempts would
/// be misleading. Outside demo mode the service needs both the analysis
/// feature and a model API key. A store-open failure disables analysis (the
/// console keeps serving and the endpoints answer 503) because the session
/// database has already surfaced a fatal error if the file itself is unusable.
async fn build_analysis(
    config: &Config,
    model: Arc<dyn Llm>,
    mcp_client: Option<Arc<crate::agent::openobserve_mcp::OpenObserveMcpClient>>,
) -> Option<Arc<AnalysisService>> {
    if config.demo || !config.analysis.enabled || config.model.api_key.is_empty() {
        return None;
    }

    let drilldown: Option<Arc<dyn Drilldown>> = match mcp_client {
        Some(client) => Some(Arc::new(OpenObserveDrilldown::new(client).await)),
        None => None,
    };

    let store = match SqliteInvestigationStore::new(&config.db_url).await {
        Ok(store) => store,
        Err(error) => {
            tracing::error!(%error, "investigation store unavailable; analysis disabled");
            return None;
        }
    };
    let service = Arc::new(AnalysisService::new(
        Arc::new(store),
        config.analysis.clone(),
        Some(model),
        drilldown,
    ));

    match service.prune().await {
        Ok(deleted) => tracing::info!(deleted, "retained failure investigations pruned"),
        Err(error) => tracing::warn!(%error, "failure investigation startup prune failed"),
    }
    spawn_retention_loop(service.clone());
    Some(service)
}

/// Prunes retained investigations once a day (spec §8). The startup prune has
/// already run; the loop skips the interval's immediate first tick.
fn spawn_retention_loop(analysis: Arc<AnalysisService>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            match analysis.prune().await {
                Ok(deleted) => tracing::info!(deleted, "retained failure investigations pruned"),
                Err(error) => tracing::warn!(%error, "failure investigation retention failed"),
            }
        }
    });
}

struct AlwaysHealthy;

#[async_trait]
impl Probe for AlwaysHealthy {
    async fn probe(&self) -> Result<(), DataError> {
        Ok(())
    }
}

fn demo_model() -> Arc<dyn Llm> {
    use adk_rust::model::{ScriptedLlm, ScriptedResponse};
    let mut script = Vec::new();
    for _ in 0..20 {
        script.push(ScriptedResponse::function_call(
            "get_overview",
            serde_json::json!({"windowMinutes": 60}),
        ));
        script.push(ScriptedResponse::text(
            "最近 1 小时整体成功率 99.7%，失败主要集中在 user-service 的 SERVFAIL。可以点开失败项看具体 trace。",
        ));
    }
    Arc::new(ScriptedLlm::new("demo", script))
}

fn ensure_sqlite_parent(db_url: &str) -> anyhow::Result<()> {
    let path = db_url
        .strip_prefix("sqlite://")
        .or_else(|| db_url.strip_prefix("sqlite:"))
        .unwrap_or(db_url);
    if path.starts_with(":memory:") {
        return Ok(());
    }
    let path = path.split('?').next().unwrap_or(path);
    let file = std::path::Path::new(path);
    if let Some(parent) = file.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create sqlite directory {}", parent.display()))?;
    }
    // SQLx's SQLite URL parser does not create the file by default.
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .with_context(|| format!("create sqlite file {}", file.display()))?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
