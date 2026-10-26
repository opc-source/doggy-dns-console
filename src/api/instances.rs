use axum::Json;
use axum::extract::{Path, State};

use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::{Instance, InstanceDetail};

pub async fn instances(State(state): State<ApiState>) -> Result<Json<Vec<Instance>>, ApiError> {
    Ok(Json(state.service.instances().await?))
}

pub async fn instance_detail(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<InstanceDetail>, ApiError> {
    Ok(Json(state.service.instance_detail(&id).await?))
}
