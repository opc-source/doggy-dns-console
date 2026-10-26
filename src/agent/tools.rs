use std::sync::Arc;

use adk_rust::prelude::{AdkError, Tool, ToolContext};
use adk_rust::tool::StatefulTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{FailureFilter, Window};
use crate::error::DataError;
use crate::service::ConsoleService;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverviewArgs {
    pub window_minutes: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FailureArgs {
    pub window_minutes: Option<u32>,
    pub domain: Option<String>,
    pub response_code: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceArgs {
    pub trace_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmptyArgs {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstanceArgs {
    pub instance_id: String,
}

pub struct ConsoleTools {
    pub service: Arc<ConsoleService>,
}

pub async fn get_overview(
    service: &ConsoleService,
    args: OverviewArgs,
) -> Result<Value, DataError> {
    let overview = service
        .overview(Window::new(args.window_minutes.unwrap_or(60)))
        .await?;
    serde_json::to_value(overview).map_err(|error| DataError::Invalid(error.to_string()))
}

pub async fn list_failures(
    service: &ConsoleService,
    args: FailureArgs,
) -> Result<Value, DataError> {
    let filter = FailureFilter {
        window: Window::new(args.window_minutes.unwrap_or(60)),
        domain: args.domain,
        response_code: args.response_code,
        limit: args.limit.unwrap_or(50).clamp(1, 200),
    };
    let groups = service.failures(filter).await?;
    serde_json::to_value(groups).map_err(|error| DataError::Invalid(error.to_string()))
}

pub async fn get_failure_detail(
    service: &ConsoleService,
    args: TraceArgs,
) -> Result<Value, DataError> {
    let detail = service.failure_detail(&args.trace_id).await?;
    serde_json::to_value(detail).map_err(|error| DataError::Invalid(error.to_string()))
}

pub async fn list_instances(service: &ConsoleService) -> Result<Value, DataError> {
    let instances = service.instances().await?;
    serde_json::to_value(instances).map_err(|error| DataError::Invalid(error.to_string()))
}

pub async fn get_instance_detail(
    service: &ConsoleService,
    args: InstanceArgs,
) -> Result<Value, DataError> {
    let detail = service.instance_detail(&args.instance_id).await?;
    serde_json::to_value(detail).map_err(|error| DataError::Invalid(error.to_string()))
}

pub fn build_tools(service: Arc<ConsoleService>) -> Vec<Arc<dyn Tool>> {
    let state = Arc::new(ConsoleTools { service });
    vec![
        Arc::new(
            StatefulTool::new(
                "get_overview",
                "Return DNS overview: instances online/total, query volume, failure count, success rate, p95 latency and the series. Use this before quoting any KPI.",
                state.clone(),
                tool_get_overview,
            )
            .with_parameters_schema::<OverviewArgs>()
            .with_read_only(true)
            .with_concurrency_safe(true),
        ),
        Arc::new(
            StatefulTool::new(
                "list_failures",
                "List DNS failures (SERVFAIL / REFUSED / FORMERR / timeouts) grouped by trace_id. NXDOMAIN is excluded unless responseCode=NXDOMAIN is explicitly requested.",
                state.clone(),
                tool_list_failures,
            )
            .with_parameters_schema::<FailureArgs>()
            .with_read_only(true)
            .with_concurrency_safe(true),
        ),
        Arc::new(
            StatefulTool::new(
                "get_failure_detail",
                "Return the logs and spans for one trace_id so the analysis can cite real evidence.",
                state.clone(),
                tool_failure_detail,
            )
            .with_parameters_schema::<TraceArgs>()
            .with_read_only(true)
            .with_concurrency_safe(true),
        ),
        Arc::new(
            StatefulTool::new(
                "list_instances",
                "List doggy-dns instances with online state, version, last seen and whether their MCP endpoint is reachable.",
                state.clone(),
                tool_list_instances,
            )
            .with_parameters_schema::<EmptyArgs>()
            .with_read_only(true)
            .with_concurrency_safe(true),
        ),
        Arc::new(
            StatefulTool::new(
                "get_instance_detail",
                "Return one instance's uptime, protocols, plugins and redacted TOML config.",
                state.clone(),
                tool_instance_detail,
            )
            .with_parameters_schema::<InstanceArgs>()
            .with_read_only(true)
            .with_concurrency_safe(true),
        ),
    ]
}

async fn tool_get_overview(
    state: Arc<ConsoleTools>,
    _ctx: Arc<dyn ToolContext>,
    args: Value,
) -> Result<Value, AdkError> {
    let args = parse(args)?;
    get_overview(&state.service, args).await.map_err(to_adk)
}

async fn tool_list_failures(
    state: Arc<ConsoleTools>,
    _ctx: Arc<dyn ToolContext>,
    args: Value,
) -> Result<Value, AdkError> {
    let args = parse(args)?;
    list_failures(&state.service, args).await.map_err(to_adk)
}

async fn tool_failure_detail(
    state: Arc<ConsoleTools>,
    _ctx: Arc<dyn ToolContext>,
    args: Value,
) -> Result<Value, AdkError> {
    let args = parse(args)?;
    get_failure_detail(&state.service, args)
        .await
        .map_err(to_adk)
}

async fn tool_list_instances(
    state: Arc<ConsoleTools>,
    _ctx: Arc<dyn ToolContext>,
    _args: Value,
) -> Result<Value, AdkError> {
    list_instances(&state.service).await.map_err(to_adk)
}

async fn tool_instance_detail(
    state: Arc<ConsoleTools>,
    _ctx: Arc<dyn ToolContext>,
    args: Value,
) -> Result<Value, AdkError> {
    let args = parse(args)?;
    get_instance_detail(&state.service, args)
        .await
        .map_err(to_adk)
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, AdkError> {
    serde_json::from_value(args)
        .map_err(|error| AdkError::tool(format!("invalid arguments: {error}")))
}

fn to_adk(error: DataError) -> AdkError {
    AdkError::tool(error.to_string())
}
