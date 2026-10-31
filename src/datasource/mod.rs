pub mod fixture;
pub mod instances;
pub mod mcp;
pub mod openobserve;

use crate::domain::{
    FailureDetail, FailureFilter, FailureGroup, Instance, InstanceDetail, InstanceIdentity,
    MatchQuality, MatchedInstance, Overview, SeenInstance, Window, WindowMetrics,
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

    /// Aggregate query/failure counts for a window. Sources without metric
    /// support fail explicitly instead of returning zero counts, which would
    /// read as "no failures".
    async fn failure_metrics(
        &self,
        _host: Option<&str>,
        _start_ms: u64,
        _end_ms: u64,
    ) -> Result<WindowMetrics, DataError> {
        Err(DataError::NotConfigured(
            "failure_metrics not implemented".to_string(),
        ))
    }
}

#[async_trait]
pub trait InstanceSource: Send + Sync {
    async fn list(&self) -> Result<Vec<Instance>, DataError>;
    async fn detail(&self, id: &str) -> Result<InstanceDetail, DataError>;

    /// Resolve an exact `service_instance_id` through `detail`. Sources that
    /// can discover instances by host/ip override this.
    ///
    /// The detail is only returned as `Exact` when it identifies the requested
    /// process; a detail resolved by host alone is not an exact match.
    async fn resolve(
        &self,
        wanted: &InstanceIdentity,
    ) -> Result<Option<MatchedInstance>, DataError> {
        let Some(sid) = wanted
            .service_instance_id
            .as_deref()
            .map(str::trim)
            .filter(|sid| !sid.is_empty())
        else {
            return Ok(None);
        };
        match self.detail(sid).await {
            Ok(detail) if detail.instance.id.eq_ignore_ascii_case(sid) => {
                Ok(Some(MatchedInstance {
                    quality: MatchQuality::Exact,
                    reachable: detail.instance.mcp_reachable,
                    detail,
                }))
            }
            Ok(_) => Ok(None),
            Err(DataError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}
