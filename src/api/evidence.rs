use axum::Json;
use axum::extract::{Path, Query, State};

use crate::api::error::ApiError;
use crate::api::failures::{FailureQuery, investigation_filter};
use crate::api::state::ApiState;
use crate::evidence::model::EvidenceBundle;

/// Builds the evidence bundle for one failure trace.
///
/// The bundle is assembled over the same bracket/domain/responseCode/instance
/// filter as the list and detail views it is opened from, so it cannot drift
/// from the entry the operator clicked. The bracket does not enter the
/// fingerprint: logs and spans are matched by trace id and the metric window is
/// centred on the trace, so one analysis is intentionally reusable across
/// overlapping brackets. The process identity is derived server-side from the
/// trace; no identity is accepted from the caller.
pub async fn evidence(
    State(state): State<ApiState>,
    Path(trace_id): Path<String>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<EvidenceBundle>, ApiError> {
    let filter = investigation_filter(query, 1, state.service.now_ms())?;
    Ok(Json(state.service.evidence(&trace_id, &filter).await?))
}
