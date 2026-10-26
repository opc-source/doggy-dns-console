use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::datasource::openobserve::client::{OpenObserveClient, PromSeries};
use crate::datasource::openobserve::normalize::{
    normalize_log_row, normalize_span_row, sql_escape,
};
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

    async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        let (start, end) = self.range(filter.window);
        let limit = filter.limit.clamp(1, 200);
        let sql = format!(
            "SELECT * FROM \"{}\" WHERE severity_text IN ('WARN','ERROR') AND _timestamp >= {} AND _timestamp <= {} ORDER BY _timestamp DESC LIMIT {}",
            self.client.config().log_stream,
            start.saturating_mul(1000),
            end.saturating_mul(1000),
            limit
        );
        let rows = self.client.search_logs(&sql, start, end, limit).await?;
        let mut groups: BTreeMap<String, FailureGroup> = BTreeMap::new();
        for row in rows {
            let Some(log) = normalize_log_row(&row) else {
                continue;
            };
            let code = log
                .response_code
                .clone()
                .unwrap_or_else(|| "SERVFAIL".into());
            if filter.response_code.is_none() && code == "NXDOMAIN" {
                continue;
            }
            if let Some(expected) = &filter.response_code
                && !code.eq_ignore_ascii_case(expected)
            {
                continue;
            }
            if let Some(domain) = &filter.domain
                && !log
                    .query
                    .as_deref()
                    .is_some_and(|query| query.contains(domain.as_str()))
            {
                continue;
            }
            let trace_id = log.record.trace_id.clone().unwrap_or_default();
            if trace_id.is_empty() {
                continue;
            }
            let entry = groups
                .entry(trace_id.clone())
                .or_insert_with(|| FailureGroup {
                    key: format!("trace:{trace_id}"),
                    trace_id: Some(trace_id.clone()),
                    query: log.query.clone().unwrap_or_else(|| "unknown".to_string()),
                    qtype: log.qtype.clone().unwrap_or_else(|| "A".to_string()),
                    instance: log.instance.clone(),
                    authority: log.authority.clone(),
                    response_code: code.clone(),
                    error: log.error.clone(),
                    count: 0,
                    first_seen: log.record.ts,
                    last_seen: log.record.ts,
                    duration_ms: None,
                });
            entry.count += 1;
            entry.first_seen = entry.first_seen.min(log.record.ts);
            entry.last_seen = entry.last_seen.max(log.record.ts);
            if entry.query == "unknown"
                && let Some(query) = log.query
            {
                entry.query = query;
            }
            if entry.error.is_none() {
                entry.error = log.error;
            }
        }
        let mut result: Vec<_> = groups.into_values().collect();
        result.sort_by_key(|left| std::cmp::Reverse(left.last_seen));
        result.truncate(limit);
        Ok(result)
    }

    async fn failure_detail(&self, trace_id: &str) -> Result<FailureDetail, DataError> {
        let end = self.clock.now_ms();
        let start = end - 24 * 60 * 60 * 1000;
        let escaped = sql_escape(trace_id);
        let sql = format!(
            "SELECT * FROM \"{}\" WHERE trace_id = '{}' AND _timestamp >= {} AND _timestamp <= {} ORDER BY _timestamp ASC LIMIT 200",
            self.client.config().log_stream,
            escaped,
            start.saturating_mul(1000),
            end.saturating_mul(1000)
        );
        let rows = self.client.search_logs(&sql, start, end, 200).await?;
        let logs: Vec<_> = rows.iter().filter_map(normalize_log_row).collect();
        if logs.is_empty() {
            return Err(DataError::NotFound(format!("trace {trace_id}")));
        }
        let first = &logs[0];
        let last_ts = logs
            .iter()
            .map(|log| log.record.ts)
            .max()
            .unwrap_or(first.record.ts);
        let group = FailureGroup {
            key: format!("trace:{trace_id}"),
            trace_id: Some(trace_id.to_string()),
            query: first.query.clone().unwrap_or_else(|| "unknown".to_string()),
            qtype: first.qtype.clone().unwrap_or_else(|| "A".to_string()),
            instance: first.instance.clone(),
            authority: first.authority.clone(),
            response_code: first
                .response_code
                .clone()
                .unwrap_or_else(|| "SERVFAIL".to_string()),
            error: first.error.clone(),
            count: logs.len() as u64,
            first_seen: first.record.ts,
            last_seen: last_ts,
            duration_ms: None,
        };
        let spans = self
            .client
            .search_traces(
                &format!(
                    "SELECT * FROM \"{}\" WHERE trace_id = '{}' ORDER BY start_time ASC LIMIT 200",
                    self.client.config().trace_stream,
                    escaped
                ),
                start,
                end,
                200,
            )
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(normalize_span_row)
            .collect();
        Ok(FailureDetail {
            group,
            logs: logs.into_iter().map(|log| log.record).collect(),
            spans,
        })
    }

    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError> {
        let (start, end) = self.range(window);
        let seen = seen_query(&self.client_config().metric_queries);
        let series = self.client.query_promql(&seen, start, end, 300).await?;
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
