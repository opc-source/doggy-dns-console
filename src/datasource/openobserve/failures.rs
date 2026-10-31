use crate::domain::FailureFilter;

use super::normalize::sql_escape;

/// Predicate shared by the failure list and the failure detail. Keeping it in
/// one place is what guarantees a trace's detail group matches its list entry.
const FAILURE_PREDICATE: &str = "severity IN ('WARN','ERROR') \
     AND response_code IS NOT NULL \
     AND upper(response_code) != 'NXDOMAIN' \
     AND trace_id IS NOT NULL \
     AND trace_id != ''";

/// Aggregate columns that build one [`FailureGroup`] per trace. `first_value`
/// carries the newest row's metadata, while `array_agg` lets the mapper apply
/// the query/error fallback rules without a second row scan.
const GROUP_COLUMNS: &str = "trace_id, \
     count(*) AS count, \
     min(_timestamp) AS first_seen, \
     max(_timestamp) AS last_seen, \
     array_agg(coalesce(query, query_name) ORDER BY _timestamp DESC) AS queries, \
     first_value(qtype ORDER BY _timestamp DESC) AS qtype, \
     first_value(host_name ORDER BY _timestamp DESC) AS host_name, \
     first_value(authority ORDER BY _timestamp DESC) AS authority, \
     first_value(response_code ORDER BY _timestamp DESC) AS response_code, \
     array_agg(error ORDER BY _timestamp DESC) AS errors";

pub fn failure_groups_query(
    stream: &str,
    filter: &FailureFilter,
    start_ms: i64,
    end_ms: i64,
    limit: usize,
) -> String {
    format!(
        "SELECT {GROUP_COLUMNS} FROM \"{stream}\" WHERE {FAILURE_PREDICATE}{} \
         AND _timestamp >= {} AND _timestamp <= {} \
         GROUP BY trace_id ORDER BY count DESC, last_seen DESC LIMIT {limit}",
        extra_filters(filter),
        start_ms.saturating_mul(1000),
        end_ms.saturating_mul(1000),
    )
}

pub fn failure_group_query(
    stream: &str,
    trace_id: &str,
    filter: &FailureFilter,
    start_ms: i64,
    end_ms: i64,
) -> String {
    format!(
        "SELECT {GROUP_COLUMNS} FROM \"{stream}\" WHERE trace_id = '{}' AND {FAILURE_PREDICATE}{} \
         AND _timestamp >= {} AND _timestamp <= {} GROUP BY trace_id LIMIT 1",
        sql_escape(trace_id),
        extra_filters(filter),
        start_ms.saturating_mul(1000),
        end_ms.saturating_mul(1000),
    )
}

/// Trace logs for the drawer. Newest first so a busy trace cannot hide the
/// failure row behind an old-log cap; the caller restores chronological order.
pub fn failure_logs_query(
    stream: &str,
    trace_id: &str,
    start_ms: i64,
    end_ms: i64,
    limit: usize,
) -> String {
    format!(
        "SELECT * FROM \"{stream}\" WHERE trace_id = '{}' \
         AND _timestamp >= {} AND _timestamp <= {} \
         ORDER BY _timestamp DESC LIMIT {limit}",
        sql_escape(trace_id),
        start_ms.saturating_mul(1000),
        end_ms.saturating_mul(1000),
    )
}

pub fn trace_spans_query(stream: &str, trace_id: &str, limit: usize) -> String {
    format!(
        "SELECT * FROM \"{stream}\" WHERE trace_id = '{}' ORDER BY start_time ASC LIMIT {limit}",
        sql_escape(trace_id),
    )
}

fn extra_filters(filter: &FailureFilter) -> String {
    let mut clauses = String::new();
    if let Some(code) = &filter.response_code {
        clauses.push_str(&format!(
            " AND lower(response_code) = lower('{}')",
            sql_escape(code)
        ));
    }
    if let Some(domain) = &filter.domain {
        clauses.push_str(&format!(
            " AND position(lower('{}') in lower(coalesce(query, query_name))) > 0",
            sql_escape(domain)
        ));
    }
    clauses
}
