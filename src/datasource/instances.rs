use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::datasource::mcp::McpInstanceSource;
use crate::datasource::{Clock, InstanceSource, TelemetrySource};
use crate::domain::{Instance, InstanceDetail, Window};
use crate::error::DataError;

pub struct MergedInstanceSource {
    mcp: Arc<McpInstanceSource>,
    telemetry: Arc<dyn TelemetrySource>,
    clock: Arc<dyn Clock>,
}

impl MergedInstanceSource {
    pub fn new(
        mcp: Arc<McpInstanceSource>,
        telemetry: Arc<dyn TelemetrySource>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            mcp,
            telemetry,
            clock,
        }
    }
}

#[async_trait]
impl InstanceSource for MergedInstanceSource {
    async fn list(&self) -> Result<Vec<Instance>, DataError> {
        let (mcp, seen) = tokio::join!(
            self.mcp.list(),
            self.telemetry.seen_instances(Window::new(1440))
        );
        let mcp = mcp.unwrap_or_else(|error| {
            tracing::warn!(error = %error, "mcp instance list failed");
            Vec::new()
        });
        let seen = seen.unwrap_or_else(|error| {
            tracing::warn!(error = %error, "telemetry instance list failed");
            Vec::new()
        });

        let now = self.clock.now_ms();
        let mut merged: BTreeMap<String, Instance> = BTreeMap::new();
        for instance in mcp {
            merged.insert(instance.host.clone(), instance);
        }
        for instance in seen {
            merged
                .entry(instance.host.clone())
                .or_insert_with(|| Instance {
                    id: instance.id,
                    host: instance.host,
                    version: None,
                    online: instance.last_seen >= now - 120_000,
                    last_seen: instance.last_seen,
                    mcp_reachable: false,
                });
        }
        let mut instances: Vec<_> = merged.into_values().collect();
        instances.sort_by(|left, right| {
            right
                .online
                .cmp(&left.online)
                .then_with(|| left.host.cmp(&right.host))
        });
        Ok(instances)
    }

    async fn detail(&self, id: &str) -> Result<InstanceDetail, DataError> {
        self.mcp.detail(id).await
    }
}
