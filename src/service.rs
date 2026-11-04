use std::sync::Arc;

use crate::datasource::{Clock, InstanceSource, TelemetrySource};
use crate::domain::{
    FailureDetail, FailureFilter, FailureGroup, Instance, InstanceDetail, Overview, Range,
    host_filter,
};
use crate::error::DataError;
use crate::evidence::assemble::assemble;
use crate::evidence::model::EvidenceBundle;

#[derive(Clone)]
pub struct ConsoleService {
    telemetry: Arc<dyn TelemetrySource>,
    instances: Arc<dyn InstanceSource>,
    clock: Arc<dyn Clock>,
}

impl ConsoleService {
    pub fn new(
        telemetry: Arc<dyn TelemetrySource>,
        instances: Arc<dyn InstanceSource>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            telemetry,
            instances,
            clock,
        }
    }

    /// The single source of truth for "now" behind every resolved bracket, so
    /// the service and its datasource agree even in demo mode, where the fixture
    /// is anchored on the same clock.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    pub async fn overview(&self, range: Range) -> Result<Overview, DataError> {
        let mut overview = self.telemetry.overview(range).await?;
        let instances = self.instances.list().await?;
        overview.instances_online =
            instances.iter().filter(|instance| instance.online).count() as u32;
        overview.instances_total = instances.len() as u32;
        Ok(overview)
    }

    pub async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        self.telemetry.failures(filter).await
    }

    pub async fn unlabeled_failures(&self, filter: FailureFilter) -> Result<u64, DataError> {
        self.telemetry.unlabeled_failures(filter).await
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
            &*self.clock,
        )
        .await
    }

    pub async fn instances(&self) -> Result<Vec<Instance>, DataError> {
        self.instances.list().await
    }

    pub async fn instance_detail(
        &self,
        id: &str,
        range: Range,
    ) -> Result<InstanceDetail, DataError> {
        let mut detail = self.instances.detail(id).await?;
        // 这个数是一条 host 等值查询,所以只有"这台实例真有 host 标签"才有意义。
        // host 为空时 filter 会退化成"不加实例条件",把整支队的失败算到一台头上。
        let Some(host) = host_filter(Some(detail.instance.host.clone())) else {
            tracing::warn!(
                instance = %detail.instance.id,
                "instance has no host label; a host-equality count would be fleet-wide, reporting no number"
            );
            return Ok(detail);
        };
        let filter = FailureFilter {
            instance: Some(host),
            ..FailureFilter::new(range)
        };
        // A count the telemetry cannot produce stays `None`: the drawer renders
        // an em dash, because `0` would read as "this node was healthy".
        match self.telemetry.instance_failure_count(filter).await {
            Ok(count) => detail.failure_count = Some(count),
            Err(error) => tracing::warn!(
                instance = %detail.instance.id,
                error = %error,
                "instance failure count unavailable; reporting no number instead of 0"
            ),
        }
        Ok(detail)
    }
}
