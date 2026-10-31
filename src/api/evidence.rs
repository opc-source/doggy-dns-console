use axum::Json;
use axum::extract::{Path, Query, State};

use crate::api::error::ApiError;
use crate::api::failures::{FailureQuery, filter_from};
use crate::api::state::ApiState;
use crate::evidence::model::EvidenceBundle;

/// Builds the evidence bundle for one failure trace.
///
/// The bundle is assembled over the same window/domain/responseCode filter as
/// the list and detail views it is opened from, so it cannot drift from the
/// entry the operator clicked. The process identity is derived server-side
/// from the trace; no identity is accepted from the caller.
pub async fn evidence(
    State(state): State<ApiState>,
    Path(trace_id): Path<String>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<EvidenceBundle>, ApiError> {
    let filter = filter_from(query, 1);
    Ok(Json(state.service.evidence(&trace_id, &filter).await?))
}
