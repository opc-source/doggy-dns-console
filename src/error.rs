#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("openobserve unavailable: {0}")]
    Upstream(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("not configured: {0}")]
    NotConfigured(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("internal: {0}")]
    Internal(String),
}
