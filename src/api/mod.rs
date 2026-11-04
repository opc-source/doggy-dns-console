pub mod analysis;
pub mod connections;
pub mod error;
pub mod evidence;
pub mod failures;
pub mod health;
pub mod instances;
pub mod overview;
pub mod state;

use axum::Router;
use axum::routing::{get, post};
use serde::Deserialize;

use crate::api::state::ApiState;
use crate::domain::Range;
use crate::error::DataError;

/// The bracket grammar every time-ranged endpoint reads: `from`/`to` in epoch
/// milliseconds win over `windowMinutes`, a lone bound still participates, and
/// an illegal span is a `400` — never a clamp (spec D2). `FailureQuery` and
/// `StartQuery` keep their own copies of these three fields because they also
/// carry domain/responseCode/instance; reusing them here would advertise
/// filters the overview does not have.
#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl RangeQuery {
    pub fn range(&self, now_ms: i64) -> Result<Range, DataError> {
        Range::resolve(self.from, self.to, self.window_minutes, now_ms).map_err(DataError::Invalid)
    }
}

pub fn router() -> Router {
    health::routes()
}

pub fn business_router(state: ApiState) -> Router {
    Router::new()
        .route("/api/console/health", get(health::console_health))
        .route("/api/overview", get(overview::overview))
        .route("/api/failures", get(failures::failures))
        .route("/api/failures/uncovered", get(failures::uncovered))
        .route("/api/failures/{trace_id}", get(failures::failure_detail))
        .route("/api/failures/{trace_id}/evidence", get(evidence::evidence))
        .route(
            "/api/failures/{trace_id}/analysis",
            post(analysis::start).get(analysis::status),
        )
        .route("/api/analysis/markers", get(analysis::markers))
        .route("/api/instances", get(instances::instances))
        .route("/api/instances/{id}", get(instances::instance_detail))
        .route("/api/connections", get(connections::connections))
        .with_state(state)
}
