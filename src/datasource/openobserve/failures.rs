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

/// Everything except the host equality: the bracket's failure scope.
fn scope_filters(filter: &FailureFilter) -> String {
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

/// The instance narrowing is an equality, not a substring: the console derives
/// instance identity from hostname-ip / host-pid heuristics, and a fuzzy host
/// match would silently group other nodes' rows into the answer.
fn extra_filters(filter: &FailureFilter) -> String {
    let mut clauses = scope_filters(filter);
    if let Some(instance) = &filter.instance {
        clauses.push_str(&format!(
            " AND lower(host_name) = lower('{}')",
            sql_escape(instance)
        ));
    }
    clauses
}

/// How many failures in the same scope carry no `host_name` at all, and are
/// therefore unreachable by the instance equality above. Reported next to the
/// filtered list so an empty result is never mistaken for a healthy node.
///
/// The count shares the list's scope exactly: it folds in `FAILURE_PREDICATE`
/// so successes and NXDOMAIN rows never inflate it, keeps the responseCode and
/// domain narrowing via [`scope_filters`], and drops only the host equality —
/// had `lower(host_name) = lower(?)` appeared here the count would be zero by
/// construction.
pub fn unlabeled_failures_query(
    stream: &str,
    filter: &FailureFilter,
    start_ms: i64,
    end_ms: i64,
) -> String {
    format!(
        "SELECT COUNT(*) AS count FROM \"{stream}\" WHERE {FAILURE_PREDICATE}{} \
         AND _timestamp >= {} AND _timestamp <= {} \
         AND (host_name IS NULL OR host_name = '')",
        scope_filters(filter),
        start_ms.saturating_mul(1000),
        end_ms.saturating_mul(1000),
    )
}

/// How many failure rows one host produced inside the bracket. The instance
/// detail reports this instead of summing list groups: a seven-day bracket
/// easily exceeds the 200-group cap, and a silently capped total would read as
/// "this node was healthy that week".
///
/// `filter.instance` is what scopes it — without one this is the whole bracket
/// across every host, which is the list's number, not a node's.
pub fn instance_failure_count_query(
    stream: &str,
    filter: &FailureFilter,
    start_ms: i64,
    end_ms: i64,
) -> String {
    format!(
        "SELECT COUNT(*) AS count FROM \"{stream}\" WHERE {FAILURE_PREDICATE}{} \
         AND _timestamp >= {} AND _timestamp <= {}",
        extra_filters(filter),
        start_ms.saturating_mul(1000),
        end_ms.saturating_mul(1000),
    )
}
