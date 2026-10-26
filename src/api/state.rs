use std::sync::Arc;

use async_trait::async_trait;

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
    pub oo_mcp_health: Option<Arc<dyn Probe>>,
    pub model_configured: bool,
}
