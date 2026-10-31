use std::sync::Arc;

use async_trait::async_trait;

use crate::analysis::service::AnalysisService;
use crate::datasource::{InstanceSource, openobserve::OpenObserveClient};
use crate::error::DataError;
use crate::service::ConsoleService;

#[async_trait]
pub trait Probe: Send + Sync {
    async fn probe(&self) -> Result<(), DataError>;
}

#[async_trait]
impl Probe for OpenObserveClient {
    async fn probe(&self) -> Result<(), DataError> {
        self.list_streams().await.map(|_| ())
    }
}

#[derive(Clone)]
pub struct ApiState {
    pub service: Arc<ConsoleService>,
    pub instances: Arc<dyn InstanceSource>,
    pub openobserve_health: Arc<dyn Probe>,
    pub openobserve_mcp_health: Option<Arc<dyn Probe>>,
    pub model_configured: bool,
    /// `None` in demo mode or when analysis is disabled/misconfigured; the
    /// analysis endpoints then answer 503.
    pub analysis: Option<Arc<AnalysisService>>,
}
