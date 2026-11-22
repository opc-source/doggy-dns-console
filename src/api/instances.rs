use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use crate::api::RangeQuery;
use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::{Instance, InstanceDetail, LogLevelChange, LogLevelState};
use crate::error::DataError;

/// The detail payload plus the process-wide capability bit the drawer needs to
/// decide whether to render the log-level control.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceDetailResponse {
    #[serde(flatten)]
    detail: InstanceDetail,
    control_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLevelRequest {
    pub directives: Option<String>,
    pub reset: Option<bool>,
}

impl LogLevelRequest {
    fn into_change(self) -> Result<LogLevelChange, DataError> {
        LogLevelChange::from_parts(self.directives, self.reset).map_err(DataError::Invalid)
    }
}

/// The list takes no bracket: it is the fleet inventory, not a time series
/// (spec D17). Only the detail resolves one.
pub async fn instances(State(state): State<ApiState>) -> Result<Json<Vec<Instance>>, ApiError> {
    Ok(Json(state.service.instances().await?))
}

pub async fn instance_detail(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<InstanceDetailResponse>, ApiError> {
    let range = query.range(state.service.now_ms())?;
    let detail = state.service.instance_detail(&id, range).await?;
    Ok(Json(InstanceDetailResponse {
        detail,
        control_enabled: state.service.control_enabled(),
    }))
}

pub async fn set_log_level(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    payload: Result<Json<LogLevelRequest>, JsonRejection>,
) -> Result<Json<LogLevelState>, ApiError> {
    let Json(request) = payload.map_err(|rejection| DataError::Invalid(rejection.body_text()))?;
    let change = request.into_change()?;
    Ok(Json(state.service.set_log_level(&id, change).await?))
}
