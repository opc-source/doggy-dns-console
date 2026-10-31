use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::{FailureDetail, FailureFilter, FailureGroup, Window};

#[derive(Debug, Deserialize)]
pub struct FailureQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
    pub domain: Option<String>,
    #[serde(rename = "responseCode")]
    pub response_code: Option<String>,
    pub limit: Option<usize>,
}

pub async fn failures(
    State(state): State<ApiState>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<Vec<FailureGroup>>, ApiError> {
    let filter = filter_from(query, 50);
    Ok(Json(state.service.failures(filter).await?))
}

pub async fn failure_detail(
    State(state): State<ApiState>,
    Path(trace_id): Path<String>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<FailureDetail>, ApiError> {
    // The detail must aggregate over the same window and filters as the list
    // entry it was opened from; the limit is irrelevant to a single trace.
    let filter = filter_from(query, 1);
    Ok(Json(state.service.failure_detail(&trace_id, filter).await?))
}

pub(crate) fn filter_from(query: FailureQuery, default_limit: usize) -> FailureFilter {
    FailureFilter {
        window: Window::new(query.window_minutes.unwrap_or(60)),
        domain: query.domain,
        response_code: query.response_code,
        limit: query.limit.unwrap_or(default_limit).clamp(1, 200),
    }
}
