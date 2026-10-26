use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Client;
use serde_json::Value;

use crate::config::{OoAuth, OpenObserveConfig};
use crate::error::DataError;

#[derive(Debug, Clone, PartialEq)]
pub struct PromSeries {
    pub labels: BTreeMap<String, String>,
    pub points: Vec<(i64, f64)>,
}

pub struct OpenObserveClient {
    http: Client,
    cfg: OpenObserveConfig,
}

impl OpenObserveClient {
    pub fn new(cfg: OpenObserveConfig) -> Result<Self, DataError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| DataError::Upstream(error.to_string()))?;
        Ok(Self { http, cfg })
    }

    pub async fn search_logs(
        &self,
        sql: &str,
        start_ms: i64,
        end_ms: i64,
        size: usize,
    ) -> Result<Vec<Value>, DataError> {
        self.search("logs", sql, start_ms, end_ms, size).await
    }

    pub async fn search_traces(
        &self,
        sql: &str,
        start_ms: i64,
        end_ms: i64,
        size: usize,
    ) -> Result<Vec<Value>, DataError> {
        self.search("traces", sql, start_ms, end_ms, size).await
    }

    async fn search(
        &self,
        kind: &str,
        sql: &str,
        start_ms: i64,
        end_ms: i64,
        size: usize,
    ) -> Result<Vec<Value>, DataError> {
        let url = format!(
            "{}/api/{}/_search",
            self.cfg.base_url.trim_end_matches('/'),
            self.cfg.org
        );
        let body = serde_json::json!({
            "query": {
                "sql": sql,
                "start_time": start_ms.saturating_mul(1000),
                "end_time": end_ms.saturating_mul(1000),
                "from": 0,
                "size": size,
            },
            "search_type": "ui",
        });
        let request = self
            .authorize(self.http.post(url).query(&[("type", kind)]))
            .json(&body);
        let response = request
            .send()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve search: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DataError::Upstream(format!(
                "openobserve search returned {status}: {}",
                truncate(&body, 200)
            )));
        }

        let value: Value = response
            .json()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve search json: {error}")))?;
        Ok(value
            .get("hits")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn query_promql(
        &self,
        query: &str,
        start_ms: i64,
        end_ms: i64,
        step_seconds: u64,
    ) -> Result<Vec<PromSeries>, DataError> {
        let url = format!(
            "{}/api/{}/prometheus/api/v1/query_range",
            self.cfg.base_url.trim_end_matches('/'),
            self.cfg.org
        );
        let request = self.authorize(self.http.get(url).query(&[
            ("query", query.to_string()),
            ("start", (start_ms / 1000).to_string()),
            ("end", (end_ms / 1000).to_string()),
            ("step", step_seconds.to_string()),
        ]));
        let response = request
            .send()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve promql: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DataError::Upstream(format!(
                "openobserve promql returned {status}: {}",
                truncate(&body, 200)
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve promql json: {error}")))?;

        let mut series = Vec::new();
        for item in value
            .pointer("/data/result")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let labels = item
                .get("metric")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let points = item
                .get("values")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|pair| {
                            let parts = pair.as_array()?;
                            let ts = parts.first()?.as_f64()? as i64 * 1000;
                            let raw = parts.get(1)?;
                            let value = raw
                                .as_f64()
                                .or_else(|| raw.as_str().and_then(|value| value.parse().ok()))?;
                            Some((ts, value))
                        })
                        .collect()
                })
                .unwrap_or_default();
            series.push(PromSeries { labels, points });
        }
        Ok(series)
    }

    pub async fn list_streams(&self) -> Result<Vec<String>, DataError> {
        let url = format!(
            "{}/api/{}/streams",
            self.cfg.base_url.trim_end_matches('/'),
            self.cfg.org
        );
        let response = self
            .authorize(self.http.get(url))
            .send()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve streams: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DataError::Upstream(format!(
                "openobserve streams returned {status}: {}",
                truncate(&body, 200)
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|error| DataError::Upstream(format!("openobserve streams json: {error}")))?;
        let list = value
            .get("list")
            .or_else(|| value.get("streams"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(list
            .into_iter()
            .filter_map(|item| match item {
                Value::String(name) => Some(name),
                Value::Object(map) => map
                    .get("name")
                    .or_else(|| map.get("stream_name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                _ => None,
            })
            .collect())
    }

    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.cfg.auth {
            OoAuth::None => request,
            OoAuth::Basic { username, password } => {
                request.basic_auth(username, Some(password))
            }
            OoAuth::Bearer(token) => request.bearer_auth(token),
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let preview: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}