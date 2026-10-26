use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::error::DataError;

pub struct ApiError(pub DataError);

impl From<DataError> for ApiError {
    fn from(error: DataError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            DataError::Upstream(_) => (StatusCode::BAD_GATEWAY, "openobserve_unavailable"),
            DataError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            DataError::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
            DataError::NotConfigured(_) => (StatusCode::SERVICE_UNAVAILABLE, "not_configured"),
        };
        (
            status,
            Json(json!({
                "error": {
                    "code": code,
                    "message": self.0.to_string(),
                }
            })),
        )
            .into_response()
    }
}
