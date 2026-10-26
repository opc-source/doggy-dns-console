use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

pub fn routes() -> Router {
    Router::new().route("/api/health", get(health))
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn console_health() -> Json<Value> {
    health().await
}
