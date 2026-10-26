use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::datasource::openobserve::client::{OpenObserveClient, PromSeries};
use crate::datasource::openobserve::overview::{
    failure_query, failure_series_query, p95_query, seen_query, series_query, total_query,
};
use crate::datasource::{Clock, TelemetrySource};
use crate::domain::{
    FailureDetail, FailureFilter, FailureGroup, Overview, SeenInstance, SeriesPoint, Window,
};
use crate::error::DataError;

pub struct OpenObserveTelemetry {
    client: Arc<OpenObserveClient>,
    clock: Arc<dyn Clock>,
}

impl OpenObserveTelemetry {
    pub fn new(client: Arc<OpenObserveClient>, clock: Arc<dyn Clock>) -> Self {
        Self { client, clock }
    }

    fn range(&self, window: Window) -> (i64, i64) {
        let end = self.clock.now_ms();
        (end - i64::from(window.minutes) * 60_000, end)
    }
}

#[async_trait]
impl TelemetrySource for OpenObserveTelemetry {
    async fn overview(&self, window: Window) -> Result<Overview, DataError> {
        let (start, end) = self.range(window);
        let cfg = self.client.config();
        let total = total_query(&cfg.metric_queries, window);
        let failure = failure_query(&cfg.metric_queries, window);
        let p95 = p95_query(&cfg.metric_duration, window);
        let volume = series_query(&cfg.metric_queries, false);
        let failure_volume = failure_series_query(&cfg.metric_queries);

        let queries = self.client.query_promql(&total, start, end, 60);
        let failures = self.client.query_promql(&failure, start, end, 60);
        let p95 = self.client.query_promql(&p95, start, end, 60);
        let volume_series = self.client.query_promql(&volume, start, end, 60);
        let failure_series = self.client.query_promql(&failure_volume, start, end, 60);

        let (queries, failures, p95, volume_series, failure_series) =
            tokio::join!(queries, failures, p95, volume_series, failure_series);
        let queries = queries?;
        let failures = failures?;
        let p95 = p95?;
        let volume_series = volume_series?;
        let failure_series = failure_series?;

        let query_total = last_u64(&queries).unwrap_or(0);
        let failure_total = last_u64(&failures).unwrap_or(0);
        let p95_value = last_value(&p95).map(|value| (value * 100.0).round() / 100.0);
        let series = zip_series(&volume_series, &failure_series);

        Ok(Overview {
            window_minutes: window.minutes,
            instances_online: 0,
            instances_total: 0,
            queries: query_total,
            failures: failure_total,
            success_rate: Overview::success_rate(query_total, failure_total),
            p95_latency_ms: p95_value,
            series,
        })
    }

    async fn failures(&self, _filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        Err(DataError::NotConfigured(
            "failure queries are not implemented yet".to_string(),
        ))
    }

    async fn failure_detail(&self, _trace_id: &str) -> Result<FailureDetail, DataError> {
        Err(DataError::NotConfigured(
            "failure queries are not implemented yet".to_string(),
        ))
    }

    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError> {
        let (start, end) = self.range(window);
        let seen = seen_query(&self.client_config().metric_queries);
        let series = self
            .client
            .query_promql(&seen, start, end, 300)
            .await?;
        Ok(series
            .into_iter()
            .filter_map(|item| {
                let host = item.labels.get("host_name")?.clone();
                let last_seen = item
                    .points
                    .iter()
                    .rev()
                    .find(|(_, value)| *value > 0.0)
                    .map(|(ts, _)| *ts)?;
                Some(SeenInstance {
                    id: host.clone(),
                    host,
                    last_seen,
                })
            })
            .collect())
    }
}

impl OpenObserveTelemetry {
    fn client_config(&self) -> &crate::config::OpenObserveConfig {
        self.client.config()
    }
}

fn last_value(series: &[PromSeries]) -> Option<f64> {
    series
        .first()
        .and_then(|item| item.points.last())
        .map(|(_, value)| *value)
}

fn last_u64(series: &[PromSeries]) -> Option<u64> {
    last_value(series).map(|value| value.max(0.0).round() as u64)
}

fn zip_series(volume: &[PromSeries], failures: &[PromSeries]) -> Vec<SeriesPoint> {
    let Some(volume) = volume.first() else {
        return Vec::new();
    };
    let failures: BTreeMap<i64, f64> = failures
        .first()
        .map(|item| item.points.iter().copied().collect())
        .unwrap_or_default();
    volume
        .points
        .iter()
        .map(|(ts, queries)| SeriesPoint {
            ts: *ts,
            queries: queries.max(0.0).round() as u64,
            failures: failures.get(ts).copied().unwrap_or(0.0).max(0.0).round() as u64,
        })
        .collect()
}