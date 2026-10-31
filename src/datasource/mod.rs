pub mod fixture;
pub mod instances;
pub mod mcp;
pub mod openobserve;

use crate::domain::{
    FailureDetail, FailureFilter, FailureGroup, Instance, InstanceDetail, Overview, SeenInstance,
    Window,
};
use crate::error::DataError;
use async_trait::async_trait;

/// Bracket an IPv6 literal so it can be embedded in a URL authority.
pub(crate) fn bracketed(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

#[async_trait]
pub trait TelemetrySource: Send + Sync {
    async fn overview(&self, window: Window) -> Result<Overview, DataError>;
    async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError>;
    async fn failure_detail(
        &self,
        trace_id: &str,
        filter: FailureFilter,
    ) -> Result<FailureDetail, DataError>;
    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError>;
}

#[async_trait]
pub trait InstanceSource: Send + Sync {
    async fn list(&self) -> Result<Vec<Instance>, DataError>;
    async fn detail(&self, id: &str) -> Result<InstanceDetail, DataError>;
}
