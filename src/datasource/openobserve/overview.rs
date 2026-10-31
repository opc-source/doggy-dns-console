use crate::domain::Window;

/// Failure selector shared by every failure-rate query. It is the response
/// code set the console has always treated as failures.
const FAILURE_SELECTOR: &str = r#"dns_response_code=~"SERVFAIL|REFUSED|FORMERR""#;

pub fn total_query(metric: &str, window: Window) -> String {
    scoped_total_query(metric, window.minutes, None)
}

pub fn failure_query(metric: &str, window: Window) -> String {
    scoped_failure_query(metric, window.minutes, None)
}

/// Query-rate query, optionally scoped to the `host_name` label exported by
/// the doggy-dns metrics.
pub fn scoped_total_query(metric: &str, minutes: u32, host: Option<&str>) -> String {
    match host {
        Some(host) => format!(
            "sum(increase({metric}{{host_name=\"{}\"}}[{minutes}m]))",
            promql_label_value(host)
        ),
        None => format!("sum(increase({metric}[{minutes}m]))"),
    }
}

/// Failure-rate query, optionally scoped to `host_name`.
pub fn scoped_failure_query(metric: &str, minutes: u32, host: Option<&str>) -> String {
    match host {
        Some(host) => format!(
            "sum(increase({metric}{{host_name=\"{}\",{FAILURE_SELECTOR}}}[{minutes}m]))",
            promql_label_value(host)
        ),
        None => format!("sum(increase({metric}{{{FAILURE_SELECTOR}}}[{minutes}m]))"),
    }
}

/// Escapes a value embedded in a double-quoted PromQL label matcher.
fn promql_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub fn p95_query(metric: &str, window: Window) -> String {
    format!(
        "histogram_quantile(0.95, sum by (le) (rate({metric}[{}m])))",
        window.minutes
    )
}

pub fn series_query(metric: &str, failures: bool) -> String {
    if failures {
        failure_series_query(metric)
    } else {
        format!("sum(rate({metric}[5m])) * 60")
    }
}

pub fn failure_series_query(metric: &str) -> String {
    format!("sum(rate({metric}{{dns_response_code=~\"SERVFAIL|REFUSED|FORMERR\"}}[5m])) * 60")
}

pub fn seen_query(metric: &str) -> String {
    format!(
        "sum by (host_name, doggy_dns_ip_addr, doggy_dns_control_port) (increase({metric}[5m]))"
    )
}
