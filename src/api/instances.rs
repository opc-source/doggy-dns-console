use axum::Json;
use axum::extract::{Path, Query, State};

use crate::api::RangeQuery;
use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::{Instance, InstanceDetail};

/// The list takes no bracket: it is the fleet inventory, not a time series
/// (spec D17). Only the detail resolves one.
pub async fn instances(State(state): State<ApiState>) -> Result<Json<Vec<Instance>>, ApiError> {
    Ok(Json(state.service.instances().await?))
}

pub async fn instance_detail(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<InstanceDetail>, ApiError> {
    let range = query.range(state.service.now_ms())?;
    Ok(Json(state.service.instance_detail(&id, range).await?))
}
