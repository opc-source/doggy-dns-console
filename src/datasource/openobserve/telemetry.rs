use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::datasource::openobserve::client::{OpenObserveClient, PromSeries};
use crate::datasource::openobserve::failures::{
    failure_group_query, failure_groups_query, failure_logs_query, trace_spans_query,
};
use crate::datasource::openobserve::normalize::{
    normalize_failure_group_row, normalize_log_row, normalize_span_row,
};
use crate::datasource::openobserve::overview::{
    failure_query, failure_series_query, p95_query, scoped_failure_query, scoped_total_query,
    seen_query, series_query, total_query,
};
use crate::datasource::{Clock, TelemetrySource};
use crate::domain::{
    FailureDetail, FailureFilter, FailureGroup, MetricCounts, Overview, SeenInstance, SeriesPoint,
    Window, WindowMetrics,
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
        let sql =
            failure_groups_query(&self.client.config().log_stream, &filter, start, end, limit);
        let rows = self.client.search_logs(&sql, start, end, limit).await?;
        Ok(rows
            .iter()
            .filter_map(normalize_failure_group_row)
            .take(limit)
            .collect())
    }

    async fn failure_detail(
        &self,
        trace_id: &str,
        filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
        let (start, end) = self.range(filter.window);
        let log_stream = &self.client.config().log_stream;
        let trace_stream = &self.client.config().trace_stream;

        let group_sql = failure_group_query(log_stream, trace_id, &filter, start, end);
        let group_rows = self.client.search_logs(&group_sql, start, end, 1).await?;
        let Some(group) = group_rows
            .iter()
            .filter_map(normalize_failure_group_row)
            .find(|group| group.trace_id.as_deref() == Some(trace_id))
        else {
            return Err(DataError::NotFound(format!("failure trace {trace_id}")));
        };

        // Newest logs first so a chatty trace cannot hide the failure evidence
        // behind an old-log cap; restore chronological order for the drawer.
        let logs_sql = failure_logs_query(log_stream, trace_id, start, end, 500);
        let mut logs: Vec<_> = self
            .client
            .search_logs(&logs_sql, start, end, 500)
            .await?
            .iter()
            .filter_map(normalize_log_row)
            .filter(|log| log.record.trace_id.as_deref() == Some(trace_id))
            .map(|log| log.record)
            .collect();
        logs.sort_by_key(|log| log.ts);

        let span_sql = trace_spans_query(trace_stream, trace_id, 200);
        let (spans, spans_unavailable) =
            match self.client.search_traces(&span_sql, start, end, 200).await {
                Ok(rows) => (rows.iter().filter_map(normalize_span_row).collect(), false),
                Err(error) => {
                    tracing::warn!(
                        trace_id = trace_id,
                        error = %error,
                        "trace span lookup failed"
                    );
                    (Vec::new(), true)
                }
            };
        Ok(FailureDetail {
            group,
            logs,
            spans,
            spans_unavailable,
        })
    }

    async fn failure_metrics(
        &self,
        host: Option<&str>,
        start_ms: u64,
        end_ms: u64,
    ) -> Result<WindowMetrics, DataError> {
        if end_ms <= start_ms {
            return Err(DataError::Invalid(
                "metric window must have a positive duration".to_string(),
            ));
        }
        let start_ms = start_ms as i64;
        let end_ms = end_ms as i64;
        let minutes = ((end_ms - start_ms + 59_999) / 60_000).max(1) as u32;
        let step = u64::from(minutes) * 60;
        let metric = &self.client.config().metric_queries;

        let global_queries_query = scoped_total_query(metric, minutes, None);
        let global_failures_query = scoped_failure_query(metric, minutes, None);
        let (global_queries, global_failures) = tokio::try_join!(
            self.client
                .query_promql(&global_queries_query, start_ms, end_ms, step),
            self.client
                .query_promql(&global_failures_query, start_ms, end_ms, step),
        )?;

        let host_counts = match host {
            Some(host) => {
                let host_queries_query = scoped_total_query(metric, minutes, Some(host));
                let host_failures_query = scoped_failure_query(metric, minutes, Some(host));
                let (host_queries, host_failures) = tokio::try_join!(
                    self.client
                        .query_promql(&host_queries_query, start_ms, end_ms, step),
                    self.client
                        .query_promql(&host_failures_query, start_ms, end_ms, step),
                )?;
                if host_queries.is_empty() && host_failures.is_empty() {
                    // No series for this host: the evidence contract keeps
                    // host metrics absent rather than inventing a zero.
                    None
                } else {
                    Some(MetricCounts {
                        queries: sum_last(&host_queries),
                        failures: sum_last(&host_failures),
                    })
                }
            }
            None => None,
        };

        Ok(WindowMetrics {
            host: host_counts,
            global: MetricCounts {
                queries: sum_last(&global_queries),
                failures: sum_last(&global_failures),
            },
        })
    }

    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError> {
        let (start, end) = self.range(window);
        let seen = seen_query(&self.client_config().metric_queries);
        let series = self.client.query_promql(&seen, start, end, 60).await?;

        let mut last_seen_by_host: BTreeMap<String, i64> = BTreeMap::new();
        let mut endpoint_by_host: BTreeMap<String, (i64, String)> = BTreeMap::new();
        for item in series {
            let Some(host) = item
                .labels
                .get("host_name")
                .map(|host| host.trim())
                .filter(|host| !host.is_empty())
            else {
                continue;
            };
            // A live node keeps exporting samples even when it handled no
            // queries in the window, so `increase` can be 0 at the newest
            // sample; recency comes from the sample time, not its value.
            let Some(latest) = item.points.iter().map(|(ts, _)| *ts).max() else {
                continue;
            };
            last_seen_by_host
                .entry(host.to_string())
                .and_modify(|previous| *previous = (*previous).max(latest))
                .or_insert(latest);
            // A node that was restarted may appear as several series with
            // different attributes; keep the newest advertised endpoint.
            if let Some(endpoint) = item
                .labels
                .get("doggy_dns_ip_addr")
                .zip(
                    item.labels
                        .get("doggy_dns_control_port")
                        .and_then(|port| port.trim().parse::<u16>().ok())
                        .filter(|port| *port > 0),
                )
                .and_then(|(ip, port)| control_endpoint(ip, port))
            {
                endpoint_by_host
                    .entry(host.to_string())
                    .and_modify(|entry| {
                        if latest > entry.0 {
                            *entry = (latest, endpoint.clone());
                        }
                    })
                    .or_insert_with(|| (latest, endpoint));
            }
        }

        Ok(last_seen_by_host
            .into_iter()
            .map(|(host, last_seen)| SeenInstance {
                endpoint: endpoint_by_host
                    .get(&host)
                    .map(|(_, endpoint)| endpoint.clone()),
                id: host.clone(),
                host,
                last_seen,
            })
            .collect())
    }
}

impl OpenObserveTelemetry {
    fn client_config(&self) -> &crate::config::OpenObserveConfig {
        self.client.config()
    }
}

/// Join the node's published address with its control port into an MCP
/// endpoint. A wildcard address is not dialable and is dropped.
fn control_endpoint(ip: &str, port: u16) -> Option<String> {
    let ip: std::net::IpAddr = ip.trim().parse().ok()?;
    if ip.is_unspecified() {
        return None;
    }
    Some(format!(
        "http://{}:{port}/mcp",
        crate::datasource::bracketed(&ip.to_string())
    ))
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

/// Sums the newest sample of every series.
///
/// The metric queries use a step equal to the whole window, so only the last
/// sample describes the requested range; summing every sample would count the
/// same trailing `increase` window repeatedly.
fn sum_last(series: &[PromSeries]) -> u64 {
    series
        .iter()
        .filter_map(|item| item.points.last())
        .map(|(_, value)| value.max(0.0).round() as u64)
        .sum()
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
