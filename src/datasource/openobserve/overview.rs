use crate::domain::Window;

pub fn total_query(metric: &str, window: Window) -> String {
    format!("sum(increase({metric}[{}m]))", window.minutes)
}

pub fn failure_query(metric: &str, window: Window) -> String {
    format!(
        "sum(increase({metric}{{dns_response_code=~\"SERVFAIL|REFUSED|FORMERR\"}}[{}m]))",
        window.minutes
    )
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
    format!(
        "sum(rate({metric}{{dns_response_code=~\"SERVFAIL|REFUSED|FORMERR\"}}[5m])) * 60"
    )
}

pub fn seen_query(metric: &str) -> String {
    format!("sum by (host_name) (increase({metric}[5m]))")
}