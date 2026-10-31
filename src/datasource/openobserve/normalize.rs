use serde_json::Value;

use crate::domain::{FailureGroup, LogRecord, SpanRecord};

#[derive(Debug, Clone)]
pub struct NormalizedLog {
    pub record: LogRecord,
    pub query: Option<String>,
    pub qtype: Option<String>,
    pub instance: Option<String>,
    pub authority: Option<String>,
    pub response_code: Option<String>,
    pub error: Option<String>,
}

pub fn normalize_log_row(row: &Value) -> Option<NormalizedLog> {
    let object = row.as_object()?;
    let attributes = object
        .get("attributes")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let field = |names: &[&str]| -> Option<String> {
        for name in names {
            if let Some(value) = object.get(*name).and_then(Value::as_str) {
                return Some(value.to_string());
            }
            if let Some(value) = attributes.get(*name).and_then(Value::as_str) {
                return Some(value.to_string());
            }
        }
        None
    };

    let trace_id = field(&["trace_id", "traceId"])?;
    let ts = object
        .get("_timestamp")
        .or_else(|| object.get("timestamp"))
        .and_then(Value::as_i64)
        .map(to_millis)
        .unwrap_or(0);
    let severity = field(&["severity"])?;
    let message = field(&["body", "message"]).unwrap_or_default();
    let span_id = field(&["span_id", "spanId"]);

    let mut fields: serde_json::Map<String, Value> = object
        .iter()
        .filter(|(key, _)| !is_core_log_field(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    // Flat attributes are the norm in OpenObserve, but nested ones can appear
    // when a row was ingested with an `attributes` object. Keep top-level
    // values authoritative and never let either shape re-introduce a core key.
    for (key, value) in &attributes {
        if !is_core_log_field(key) {
            fields.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    Some(NormalizedLog {
        record: LogRecord {
            ts,
            severity,
            message,
            trace_id: Some(trace_id),
            span_id,
            fields: Value::Object(fields),
        },
        query: field(&["query", "query_name", "dns.question.name"]),
        qtype: field(&["qtype", "query_type", "dns.question.type"]),
        instance: field(&["host_name", "host.name", "service.instance.id"]),
        authority: field(&["authority", "dns.authority"]),
        response_code: field(&["response_code", "dns.response.code"]),
        error: field(&["error", "error.message"]),
    })
}

/// Map one aggregate row from [`super::failures`] into a [`FailureGroup`].
/// The list and the detail use the same query shape, so this mapping is the
/// single place their group metadata is built.
pub fn normalize_failure_group_row(row: &Value) -> Option<FailureGroup> {
    let object = row.as_object()?;
    let trace_id = object.get("trace_id").and_then(Value::as_str)?.to_string();
    if trace_id.is_empty() {
        return None;
    }
    let count = object
        .get("count")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|count| count.max(0.0) as u64))
        })
        .unwrap_or(0);
    let timestamp = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_i64)
            .map(to_millis)
            .unwrap_or(0)
    };
    let first_seen = timestamp("first_seen");
    let last_seen = object
        .get("last_seen")
        .and_then(Value::as_i64)
        .map(to_millis)
        .unwrap_or(first_seen);

    Some(FailureGroup {
        key: format!("trace:{trace_id}"),
        trace_id: Some(trace_id),
        query: first_text(object.get("queries")).unwrap_or_else(|| "unknown".to_string()),
        qtype: first_text(object.get("qtype")).unwrap_or_else(|| "A".to_string()),
        instance: first_text(object.get("host_name")),
        authority: first_text(object.get("authority")),
        response_code: first_text(object.get("response_code"))
            .unwrap_or_else(|| "UNKNOWN".to_string()),
        error: first_text(object.get("errors")),
        count,
        first_seen,
        last_seen,
        duration_ms: None,
    })
}

pub fn normalize_span_row(row: &Value) -> Option<SpanRecord> {
    let object = row.as_object()?;
    let span_id = object
        .get("span_id")
        .or_else(|| object.get("spanId"))
        .and_then(Value::as_str)?
        .to_string();
    let name = object
        .get("operation_name")
        .or_else(|| object.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("span")
        .to_string();
    let duration_ms = object
        .get("duration_ms")
        .and_then(Value::as_f64)
        .or_else(|| {
            object
                .get("duration")
                .and_then(Value::as_f64)
                .map(|micros| micros / 1000.0)
        });
    let status = object
        .get("status_code")
        .or_else(|| object.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("UNSET")
        .to_string();
    let attributes = object
        .get("attributes")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    Some(SpanRecord {
        span_id,
        name,
        duration_ms,
        status,
        attributes,
    })
}

/// Keys already surfaced as dedicated log fields (or consumed as containers)
/// and therefore not repeated in the drawer's raw field map.
fn is_core_log_field(key: &str) -> bool {
    matches!(
        key,
        "_timestamp"
            | "timestamp"
            | "body"
            | "message"
            | "severity"
            | "trace_id"
            | "traceId"
            | "span_id"
            | "spanId"
            | "attributes"
    )
}

/// Accept both the `array_agg` arrays and plain strings, taking the newest
/// non-empty value; the aggregate orders arrays newest-first.
fn first_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .find(|item| !item.is_empty())
            .map(ToString::to_string),
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn to_millis(value: i64) -> i64 {
    if value > 10_000_000_000_000 {
        value / 1000
    } else {
        value
    }
}

pub fn sql_escape(value: &str) -> String {
    value.replace('\'', "''")
}
