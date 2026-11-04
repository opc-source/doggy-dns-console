use axum::Json;
use axum::extract::{Query, State};

use crate::api::RangeQuery;
use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::Overview;

pub async fn overview(
    State(state): State<ApiState>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<Overview>, ApiError> {
    let range = query.range(state.service.now_ms())?;
    Ok(Json(state.service.overview(range).await?))
}
