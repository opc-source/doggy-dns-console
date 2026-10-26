use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::{Overview, Window};

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
}

pub async fn overview(
    State(state): State<ApiState>,
    Query(query): Query<WindowQuery>,
) -> Result<Json<Overview>, ApiError> {
    Ok(Json(
        state
            .service
            .overview(Window::new(query.window_minutes.unwrap_or(60)))
            .await?,
    ))
}
