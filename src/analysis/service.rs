//! Asynchronous orchestration of failure investigations.
//!
//! The service owns the spec §6.4 cache key (trace, evidence fingerprint, model
//! and prompt version), the in-process rate limit and the one-job-per-key
//! guarantee. It receives an already assembled [`EvidenceBundle`] and the
//! [`FailureFilter`] it was built from, so the exact filter can be persisted
//! without a lossy round-trip. HTTP wiring lives in the API layer; this module
//! only depends on the store, the model client and the drill-down trait.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use adk_rust::prelude::Llm;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::analysis::AnalysisStatus;
use crate::analysis::agent::run_forensic_loop;
use crate::analysis::drilldown::Drilldown;
use crate::analysis::prompt::PROMPT_VERSION;
use crate::analysis::store::{
    CacheKey, InvestigationMarker, InvestigationOutcome, InvestigationRecord, InvestigationStatus,
    InvestigationStore, NewInvestigation,
};
use crate::config::AnalysisConfig;
use crate::domain::FailureFilter;
use crate::error::DataError;
use crate::evidence::model::EvidenceBundle;

/// Outcome of a `start` request.
#[derive(Debug)]
pub enum AnalysisStart {
    /// A succeeded attempt for the same cache key; the model was not called.
    Cached(InvestigationRecord),
    /// A newly queued attempt whose job runs in the background.
    Started(InvestigationRecord),
    /// An attempt with the same cache key is already queued or running.
    InFlight(InvestigationRecord),
}

impl AnalysisStart {
    pub fn record(&self) -> &InvestigationRecord {
        match self {
            Self::Cached(record) | Self::Started(record) | Self::InFlight(record) => record,
        }
    }

    pub fn analysis_id(&self) -> &str {
        &self.record().id
    }
}

/// Orchestrates investigation attempts over the durable store.
pub struct AnalysisService {
    store: Arc<dyn InvestigationStore>,
    config: AnalysisConfig,
    llm: Option<Arc<dyn Llm>>,
    drilldown: Option<Arc<dyn Drilldown>>,
    /// Short-lived map of per-key async mutexes. The std mutex is only held long
    /// enough to clone the `Arc`; the async lock serializes `start` calls for
    /// one cache key so two requests cannot both create an attempt.
    key_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    rate_limit: AsyncMutex<TokenBucket>,
}

impl AnalysisService {
    pub fn new(
        store: Arc<dyn InvestigationStore>,
        config: AnalysisConfig,
        llm: Option<Arc<dyn Llm>>,
        drilldown: Option<Arc<dyn Drilldown>>,
    ) -> Self {
        let bucket = TokenBucket::new(config.rate_per_minute);
        Self {
            store,
            config,
            llm,
            drilldown,
            key_locks: Mutex::new(HashMap::new()),
            rate_limit: AsyncMutex::new(bucket),
        }
    }

    /// Returns a cache hit, the existing in-flight attempt, or a newly queued
    /// job. `force` skips the cache but never starts a second concurrent job
    /// for the same key.
    pub async fn start(
        self: &Arc<Self>,
        bundle: &EvidenceBundle,
        filter: &FailureFilter,
        force: bool,
    ) -> Result<AnalysisStart, DataError> {
        if !self.config.enabled {
            return Err(DataError::NotConfigured("analysis is disabled".to_string()));
        }
        let llm = self.llm.clone().ok_or_else(|| {
            DataError::NotConfigured("analysis model is not configured".to_string())
        })?;

        let model = llm.name();
        let key = CacheKey {
            trace_id: &bundle.trace_id,
            evidence_fingerprint: &bundle.fingerprint,
            model,
            prompt_version: PROMPT_VERSION,
        };
        let key_text = format!(
            "{}\u{0}{}\u{0}{}\u{0}{}",
            bundle.trace_id, bundle.fingerprint, model, PROMPT_VERSION
        );

        // The guard serializes the cache check and the attempt creation for this
        // key, so concurrent starts cannot both miss and both insert.
        let lock = self.key_lock(&key_text);
        let _guard = lock.lock().await;

        if !force
            && let Some(record) = self
                .store
                .cache_hit(&key)
                .await
                .map_err(DataError::Internal)?
        {
            return Ok(AnalysisStart::Cached(record));
        }

        if let Some(record) = self
            .store
            .running(&key)
            .await
            .map_err(DataError::Internal)?
        {
            if !self.is_stale(&record) {
                return Ok(AnalysisStart::InFlight(record));
            }
            let recovered = InvestigationOutcome {
                status: InvestigationStatus::Failed,
                result: None,
                tool_calls: Vec::new(),
                error: Some("stale running attempt recovered".to_string()),
            };
            self.store
                .finish(&record.id, &recovered)
                .await
                .map_err(DataError::Internal)?;
            tracing::warn!(attempt = %record.id, "recovered a stale investigation attempt");
        }

        {
            let mut bucket = self.rate_limit.lock().await;
            if !bucket.try_acquire() {
                return Err(DataError::RateLimited(
                    "analysis rate limit exceeded".to_string(),
                ));
            }
        }

        let record = self
            .store
            .create_attempt(&NewInvestigation {
                trace_id: &bundle.trace_id,
                filter,
                evidence_fingerprint: &bundle.fingerprint,
                model,
                prompt_version: PROMPT_VERSION,
            })
            .await
            .map_err(DataError::Internal)?;

        let service = Arc::clone(self);
        let id = record.id.clone();
        let bundle = bundle.clone();
        tokio::spawn(async move {
            service.run_job(id, bundle).await;
        });
        Ok(AnalysisStart::Started(record))
    }

    /// Latest attempt for the trace, or a specific attempt that must belong to
    /// it.
    pub async fn attempt(
        &self,
        trace_id: &str,
        analysis_id: Option<&str>,
    ) -> Result<InvestigationRecord, DataError> {
        match analysis_id {
            Some(id) => match self.store.get(id).await.map_err(DataError::Internal)? {
                Some(record) if record.trace_id == trace_id => Ok(record),
                _ => Err(DataError::NotFound(format!(
                    "analysis {id} not found for trace {trace_id}"
                ))),
            },
            None => match self
                .store
                .latest(trace_id)
                .await
                .map_err(DataError::Internal)?
            {
                Some(record) => Ok(record),
                None => Err(DataError::NotFound(format!(
                    "no analysis for trace {trace_id}"
                ))),
            },
        }
    }

    /// Latest marker per trace. An empty slice yields an empty vector.
    pub async fn markers(
        &self,
        trace_ids: &[String],
    ) -> Result<Vec<InvestigationMarker>, DataError> {
        self.store
            .markers(trace_ids)
            .await
            .map_err(DataError::Internal)
    }

    /// Deletes investigations older than the configured retention and returns
    /// the number of removed rows.
    pub async fn prune(&self) -> Result<u64, DataError> {
        let retention_days = u32::try_from(self.config.retention_days).unwrap_or(u32::MAX);
        self.store
            .prune_old(retention_days)
            .await
            .map_err(DataError::Internal)
    }

    fn key_lock(&self, key: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .key_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    fn is_stale(&self, record: &InvestigationRecord) -> bool {
        let threshold = i64::try_from(
            self.config
                .timeout
                .as_secs()
                .saturating_mul(2)
                .saturating_add(60),
        )
        .unwrap_or(i64::MAX);
        now_secs().saturating_sub(record.created_at) > threshold
    }

    /// Runs one attempt to a terminal store record. There is no caller left to
    /// report failures to, so store errors are logged and dropped.
    async fn run_job(self: Arc<Self>, id: String, bundle: EvidenceBundle) {
        if let Err(error) = self.store.mark_running(&id).await {
            tracing::warn!(%error, %id, "analysis attempt could not start");
            return;
        }

        let Some(llm) = self.llm.clone() else {
            tracing::warn!(%id, "analysis model vanished before the job started");
            let outcome = InvestigationOutcome {
                status: InvestigationStatus::Failed,
                result: None,
                tool_calls: Vec::new(),
                error: Some("analysis model is not configured".to_string()),
            };
            if let Err(error) = self.store.finish(&id, &outcome).await {
                tracing::warn!(%error, %id, "analysis attempt could not be finished");
            }
            return;
        };
        let drilldown: Arc<dyn Drilldown> = self
            .drilldown
            .clone()
            .unwrap_or_else(|| Arc::new(NoDrilldown));
        let max_tool_calls = usize::try_from(self.config.max_tool_calls).unwrap_or(usize::MAX);

        let outcome =
            match run_forensic_loop(
                llm.as_ref(),
                &bundle,
                drilldown.as_ref(),
                max_tool_calls,
                self.config.timeout,
            )
            .await
            {
                Err(error) => InvestigationOutcome {
                    status: InvestigationStatus::Failed,
                    result: None,
                    tool_calls: Vec::new(),
                    error: Some(error),
                },
                Ok(outcome) if outcome.result.status == AnalysisStatus::Invalid => {
                    InvestigationOutcome {
                        status: InvestigationStatus::Invalid,
                        result: None,
                        tool_calls: outcome.tool_calls,
                        error: Some(outcome.diagnostic.unwrap_or_else(|| {
                            "the model produced an invalid analysis".to_string()
                        })),
                    }
                }
                Ok(outcome) => InvestigationOutcome {
                    status: InvestigationStatus::Succeeded,
                    result: Some(outcome.result),
                    tool_calls: outcome.tool_calls,
                    error: None,
                },
            };

        if let Err(error) = self.store.finish(&id, &outcome).await {
            tracing::warn!(%error, %id, "analysis attempt could not be finished");
        }
    }
}

/// Evidence-only analysis: the model still submits from the bundle, but no
/// drill-down tools are advertised.
struct NoDrilldown;

#[async_trait]
impl Drilldown for NoDrilldown {
    fn tool_declarations(&self) -> Vec<Value> {
        Vec::new()
    }

    async fn call_tool(&self, _tool_name: &str, _args: Value) -> Result<Value, String> {
        Err("drill-down is not configured".to_string())
    }
}

/// Continuous-refill token bucket (spec §7).
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    per_second: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(per_minute: u64) -> Self {
        let capacity = per_minute.max(1) as f64;
        Self {
            capacity,
            tokens: capacity,
            per_second: capacity / 60.0,
            last: Instant::now(),
        }
    }

    fn try_acquire(&mut self) -> bool {
        self.try_acquire_at(Instant::now())
    }

    fn try_acquire_at(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.per_second).min(self.capacity);
            self.last = now;
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::TokenBucket;
    use std::time::{Duration, Instant};

    #[test]
    fn token_bucket_refills_continuously() {
        // The bucket is created first so `start` is at or after its epoch; the
        // first acquisition then anchors the refill clock on `start`.
        let mut bucket = TokenBucket::new(2);
        let start = Instant::now();

        // Capacity 2: two acquisitions drain it, the third fails.
        assert!(bucket.try_acquire_at(start));
        assert!(bucket.try_acquire_at(start));
        assert!(!bucket.try_acquire_at(start));

        // 2 tokens per minute is one token every 30 seconds.
        assert!(!bucket.try_acquire_at(start + Duration::from_secs(29)));
        assert!(bucket.try_acquire_at(start + Duration::from_secs(30)));

        // Refill never exceeds the capacity, even after a long idle period.
        let later = start + Duration::from_secs(600);
        assert!(bucket.try_acquire_at(later));
        assert!(bucket.try_acquire_at(later));
        assert!(!bucket.try_acquire_at(later));
        assert!(bucket.tokens <= bucket.capacity);
    }
}
