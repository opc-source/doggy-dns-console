use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::domain::{Overview, Range, Window};

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
}

pub async fn overview(
    State(state): State<ApiState>,
    Query(query): Query<WindowQuery>,
) -> Result<Json<Overview>, ApiError> {
    // Temporary bridge until Task 3: the HTTP edge still only accepts
    // `windowMinutes`, so the bracket is expanded from the service clock here
    // and the 5-1440 clamp still lives in `Window::new`.
    let range = Range::from_window(
        Window::new(query.window_minutes.unwrap_or(60)),
        state.service.now_ms(),
    );
    Ok(Json(state.service.overview(range).await?))
}
