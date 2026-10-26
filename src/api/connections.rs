use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::api::state::ApiState;

pub async fn connections(State(state): State<ApiState>) -> Json<Value> {
    let openobserve_start = Instant::now();
    let openobserve_result = state.openobserve_health.probe().await;
    let openobserve_ms = openobserve_start.elapsed().as_millis();

    let mcp_result = state.instances.list().await;
    let (mcp_reachable, mcp_total) = match mcp_result {
        Ok(instances) => (
            instances.iter().filter(|item| item.mcp_reachable).count(),
            instances.len(),
        ),
        Err(error) => {
            tracing::warn!(error = %error, "mcp health probe failed");
            (0, 0)
        }
    };

    let openobserve_mcp = match &state.openobserve_mcp_health {
        Some(probe) => {
            let started = Instant::now();
            let result = probe.probe().await;
            json!({ "ok": result.is_ok(), "latencyMs": started.elapsed().as_millis() })
        }
        None => json!({ "ok": false, "latencyMs": 0 }),
    };

    Json(json!({
        "openobserve": { "ok": openobserve_result.is_ok(), "latencyMs": openobserve_ms },
        "openobserveMcp": openobserve_mcp,
        "mcp": { "reachable": mcp_reachable, "total": mcp_total },
        "model": { "configured": state.model_configured },
    }))
}
