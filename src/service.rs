use std::sync::Arc;

use crate::datasource::{InstanceSource, SystemClock, TelemetrySource};
use crate::domain::{
    FailureDetail, FailureFilter, FailureGroup, Instance, InstanceDetail, Overview, Window,
};
use crate::error::DataError;
use crate::evidence::assemble::assemble;
use crate::evidence::model::EvidenceBundle;

#[derive(Clone)]
pub struct ConsoleService {
    telemetry: Arc<dyn TelemetrySource>,
    instances: Arc<dyn InstanceSource>,
}

impl ConsoleService {
    pub fn new(telemetry: Arc<dyn TelemetrySource>, instances: Arc<dyn InstanceSource>) -> Self {
        Self {
            telemetry,
            instances,
        }
    }

    pub async fn overview(&self, window: Window) -> Result<Overview, DataError> {
        let mut overview = self.telemetry.overview(window).await?;
        let instances = self.instances.list().await?;
        overview.instances_online =
            instances.iter().filter(|instance| instance.online).count() as u32;
        overview.instances_total = instances.len() as u32;
        Ok(overview)
    }

    pub async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        self.telemetry.failures(filter).await
    }

    pub async fn failure_detail(
        &self,
        trace_id: &str,
        filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
        self.telemetry.failure_detail(trace_id, filter).await
    }

    /// Builds the deterministic evidence bundle for one failure trace.
    ///
    /// The caller supplies only the trace and the list/detail filter; the
    /// instance identity is derived from the trace's own telemetry so the
    /// evidence cannot be pointed at a different process.
    pub async fn evidence(
        &self,
        trace_id: &str,
        filter: &FailureFilter,
    ) -> Result<EvidenceBundle, DataError> {
        assemble(
            &*self.telemetry,
            &*self.instances,
            trace_id,
            filter,
            &SystemClock,
        )
        .await
    }

    pub async fn instances(&self) -> Result<Vec<Instance>, DataError> {
        self.instances.list().await
    }

    pub async fn instance_detail(&self, id: &str) -> Result<InstanceDetail, DataError> {
        let mut detail = self.instances.detail(id).await?;
        let filter = FailureFilter {
            window: Window::new(60),
            ..Default::default()
        };
        if let Ok(groups) = self.telemetry.failures(filter).await {
            detail.recent_failures = groups
                .iter()
                .filter(|group| {
                    group.instance.as_deref() == Some(detail.instance.id.as_str())
                        || group.instance.as_deref() == Some(detail.instance.host.as_str())
                })
                .map(|group| group.count)
                .sum();
        }
        Ok(detail)
    }
}
