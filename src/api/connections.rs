use axum::extract::State;
use axum::Json;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::api::state::ApiState;

pub async fn connections(State(state): State<ApiState>) -> Json<Value> {
    let oo_start = Instant::now();
    let oo_result = state.openobserve_health.probe().await;
    let oo_ms = oo_start.elapsed().as_millis();

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

    let oo_mcp = match &state.oo_mcp_health {
        Some(probe) => {
            let started = Instant::now();
            let result = probe.probe().await;
            json!({ "ok": result.is_ok(), "latencyMs": started.elapsed().as_millis() })
        }
        None => json!({ "ok": false, "latencyMs": 0 }),
    };

    Json(json!({
        "openobserve": { "ok": oo_result.is_ok(), "latencyMs": oo_ms },
        "openobserveMcp": oo_mcp,
        "mcp": { "reachable": mcp_reachable, "total": mcp_total },
        "model": { "configured": state.model_configured },
    }))
}
