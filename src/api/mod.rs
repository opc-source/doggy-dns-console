pub mod connections;
pub mod error;
pub mod evidence;
pub mod failures;
pub mod health;
pub mod instances;
pub mod overview;
pub mod state;

use axum::Router;
use axum::routing::get;

use crate::api::state::ApiState;

pub fn router() -> Router {
    health::routes()
}

pub fn business_router(state: ApiState) -> Router {
    Router::new()
        .route("/api/console/health", get(health::console_health))
        .route("/api/overview", get(overview::overview))
        .route("/api/failures", get(failures::failures))
        .route("/api/failures/{trace_id}", get(failures::failure_detail))
        .route("/api/failures/{trace_id}/evidence", get(evidence::evidence))
        .route("/api/instances", get(instances::instances))
        .route("/api/instances/{id}", get(instances::instance_detail))
        .route("/api/connections", get(connections::connections))
        .with_state(state)
}
