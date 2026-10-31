//! Failure analysis endpoints (spec §7).
//!
//! `POST` assembles the evidence server-side, then asks the [`AnalysisService`]
//! for a cache hit, an in-flight attempt or a new durable job. `GET` reports
//! the latest (or requested) attempt as a snapshot. The batch markers endpoint
//! gives the failure list a lightweight status projection without result
//! bodies.
//!
//! The caller only supplies the trace id and the shared failure filter. The
//! instance identity is always derived from the trace's own evidence, never
//! from query parameters.

use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::analysis::service::{AnalysisService, AnalysisStart};
use crate::analysis::store::{InvestigationMarker, InvestigationRecord, InvestigationStatus};
use crate::analysis::{AnalysisResult, Confidence, ToolCallAudit, VerdictCategory};
use crate::api::error::ApiError;
use crate::api::failures::{FailureQuery, filter_from};
use crate::api::state::ApiState;
use crate::domain::FailureFilter;
use crate::error::DataError;
use crate::evidence::model::EvidenceFilter;

/// Upper bound on unique trace ids accepted by the markers endpoint.
pub const MAX_MARKER_IDS: usize = 200;

/// Query accepted by `POST /api/failures/{trace_id}/analysis`.
#[derive(Debug, Deserialize)]
pub struct StartQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
    pub domain: Option<String>,
    #[serde(rename = "responseCode")]
    pub response_code: Option<String>,
    pub force: Option<bool>,
}

/// Query accepted by `GET /api/failures/{trace_id}/analysis`. Unrelated
/// parameters are ignored.
#[derive(Debug, Deserialize)]
pub struct AnalysisQuery {
    pub analysis: Option<String>,
}

/// Query accepted by `GET /api/analysis/markers`.
#[derive(Debug, Deserialize)]
pub struct MarkerQuery {
    #[serde(rename = "traceIds")]
    pub trace_ids: Option<String>,
}

/// One persisted analysis attempt as the API presents it.
///
/// `startedAt`/`finishedAt` are epoch **milliseconds**, the same wire unit as
/// evidence timestamps; the store keeps seconds and the conversion happens
/// only here.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSnapshot {
    pub analysis_id: String,
    pub trace_id: String,
    pub status: InvestigationStatus,
    pub attempt: u32,
    pub tool_call_count: usize,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// Serialized as the evidence filter (`{windowMinutes, domain,
    /// responseCode}`): the list-only `limit` is not part of an analysis.
    #[serde(serialize_with = "serialize_filter")]
    pub filter: FailureFilter,
    pub evidence_fingerprint: String,
    pub model: String,
    pub prompt_version: String,
    pub verdict_category: Option<VerdictCategory>,
    pub confidence: Option<Confidence>,
    pub result: Option<AnalysisResult>,
    pub tool_calls: Vec<ToolCallAudit>,
    pub error: Option<String>,
}

impl From<&InvestigationRecord> for AnalysisSnapshot {
    fn from(record: &InvestigationRecord) -> Self {
        Self {
            analysis_id: record.id.clone(),
            trace_id: record.trace_id.clone(),
            status: record.status,
            attempt: record.attempt,
            tool_call_count: record.tool_calls.len(),
            started_at: to_millis(record.created_at),
            finished_at: record.finished_at.map(to_millis),
            filter: record.filter.clone(),
            evidence_fingerprint: record.evidence_fingerprint.clone(),
            model: record.model.clone(),
            prompt_version: record.prompt_version.clone(),
            verdict_category: record.verdict_category,
            confidence: record.confidence,
            result: record.result.clone(),
            tool_calls: record.tool_calls.clone(),
            error: record.error.clone(),
        }
    }
}

/// One lightweight investigation marker as the API presents it. Timestamps use
/// the same epoch-millisecond unit as [`AnalysisSnapshot`].
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisMarker {
    pub id: String,
    pub trace_id: String,
    pub status: InvestigationStatus,
    pub verdict_category: Option<VerdictCategory>,
    pub confidence: Option<Confidence>,
    pub attempt: u32,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

impl From<InvestigationMarker> for AnalysisMarker {
    fn from(marker: InvestigationMarker) -> Self {
        Self {
            id: marker.id,
            trace_id: marker.trace_id,
            status: marker.status,
            verdict_category: marker.verdict_category,
            confidence: marker.confidence,
            attempt: marker.attempt,
            created_at: to_millis(marker.created_at),
            finished_at: marker.finished_at.map(to_millis),
        }
    }
}

/// Converts the store's epoch seconds to the API's epoch milliseconds.
fn to_millis(seconds: i64) -> i64 {
    seconds.saturating_mul(1000)
}

fn serialize_filter<S>(filter: &FailureFilter, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    EvidenceFilter::from(filter).serialize(serializer)
}

/// Starts an analysis job or returns the cached/in-flight attempt for the same
/// evidence, model and prompt version.
///
/// Configuration is checked before the evidence lookup, so a console without
/// an analysis service answers 503 without touching telemetry.
pub async fn start(
    State(state): State<ApiState>,
    Path(trace_id): Path<String>,
    Query(query): Query<StartQuery>,
) -> Result<Response, ApiError> {
    let analysis = analysis_service(&state)?;
    let filter = filter_from(
        FailureQuery {
            window_minutes: query.window_minutes,
            domain: query.domain,
            response_code: query.response_code,
            limit: None,
        },
        1,
    );
    let bundle = state.service.evidence(&trace_id, &filter).await?;
    match analysis
        .start(&bundle, &filter, query.force.unwrap_or(false))
        .await?
    {
        AnalysisStart::Cached(record) => Ok(Json(AnalysisSnapshot::from(&record)).into_response()),
        AnalysisStart::Started(record) | AnalysisStart::InFlight(record) => Ok((
            StatusCode::ACCEPTED,
            Json(json!({ "analysisId": record.id })),
        )
            .into_response()),
    }
}

/// Reports the latest attempt for the trace, or the attempt named by
/// `?analysis=<id>` when it belongs to that trace.
pub async fn status(
    State(state): State<ApiState>,
    Path(trace_id): Path<String>,
    Query(query): Query<AnalysisQuery>,
) -> Result<Json<AnalysisSnapshot>, ApiError> {
    let analysis = analysis_service(&state)?;
    let record = analysis
        .attempt(&trace_id, query.analysis.as_deref())
        .await?;
    Ok(Json(AnalysisSnapshot::from(&record)))
}

/// Latest investigation marker per requested trace. Unknown traces are simply
/// absent from the result; no result bodies are included.
pub async fn markers(
    State(state): State<ApiState>,
    Query(query): Query<MarkerQuery>,
) -> Result<Json<Vec<AnalysisMarker>>, ApiError> {
    let trace_ids = parse_trace_ids(query.trace_ids.as_deref())?;
    if trace_ids.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let analysis = analysis_service(&state)?;
    Ok(Json(
        analysis
            .markers(&trace_ids)
            .await?
            .into_iter()
            .map(AnalysisMarker::from)
            .collect(),
    ))
}

fn analysis_service(state: &ApiState) -> Result<&Arc<AnalysisService>, ApiError> {
    state.analysis.as_ref().ok_or_else(|| {
        ApiError(DataError::NotConfigured(
            "analysis is not configured".to_string(),
        ))
    })
}

/// Trims, drops empties and deduplicates ids while preserving order, then
/// enforces the batch limit.
fn parse_trace_ids(raw: Option<&str>) -> Result<Vec<String>, ApiError> {
    let mut seen = HashSet::new();
    let trace_ids: Vec<String> = raw
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .filter(|id| seen.insert((*id).to_string()))
        .map(ToString::to_string)
        .collect();
    if trace_ids.len() > MAX_MARKER_IDS {
        return Err(DataError::Invalid(format!(
            "traceIds accepts at most {MAX_MARKER_IDS} unique ids"
        ))
        .into());
    }
    Ok(trace_ids)
}
