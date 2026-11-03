use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::{FailureDetail, FailureFilter, FailureGroup, Range};
use crate::error::DataError;

#[derive(Debug, Deserialize)]
pub struct FailureQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
    /// Absolute bracket start in epoch milliseconds. Wins over `windowMinutes`
    /// when `to` is present too.
    pub from: Option<i64>,
    /// Absolute bracket end in epoch milliseconds; also anchors `windowMinutes`
    /// when `from` is absent. Defaults to the server clock.
    pub to: Option<i64>,
    pub domain: Option<String>,
    #[serde(rename = "responseCode")]
    pub response_code: Option<String>,
    pub instance: Option<String>,
    pub limit: Option<usize>,
}

pub async fn failures(
    State(state): State<ApiState>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<Vec<FailureGroup>>, ApiError> {
    let filter = investigation_filter(query, 50, state.service.now_ms())?;
    Ok(Json(state.service.failures(filter).await?))
}

pub async fn failure_detail(
    State(state): State<ApiState>,
    Path(trace_id): Path<String>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<FailureDetail>, ApiError> {
    // The detail must aggregate over the same bracket and filters as the list
    // entry it was opened from; the limit is irrelevant to a single trace.
    let filter = investigation_filter(query, 1, state.service.now_ms())?;
    Ok(Json(state.service.failure_detail(&trace_id, filter).await?))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UncoveredEvidence {
    pub unlabeled: u64,
}

/// Counts failures in the bracket that have no host label, i.e. the rows the
/// `instance` filter on `GET /api/failures` structurally cannot show.
///
/// Registered before `/api/failures/{trace_id}` so the literal path is not
/// swallowed by the trace-id segment; `uncovered_is_not_matched_by_the_trace_route`
/// pins that.
pub async fn uncovered(
    State(state): State<ApiState>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<UncoveredEvidence>, ApiError> {
    let filter = investigation_filter(query, 1, state.service.now_ms())?;
    Ok(Json(UncoveredEvidence {
        unlabeled: state.service.unlabeled_failures(filter).await?,
    }))
}

/// Turn a query string into the filter the investigation chain runs with.
///
/// The rejection list is exactly four rules, and the caller's bracket is never
/// clamped: an out-of-range request is a client bug, not something to guess at.
pub(crate) fn investigation_filter(
    query: FailureQuery,
    default_limit: usize,
    now_ms: i64,
) -> Result<FailureFilter, DataError> {
    let range = Range::resolve(query.from, query.to, query.window_minutes, now_ms)
        .map_err(DataError::Invalid)?;
    Ok(FailureFilter {
        range,
        domain: query.domain,
        response_code: query.response_code,
        instance: query.instance,
        limit: query.limit.unwrap_or(default_limit).clamp(1, 200),
    })
}
