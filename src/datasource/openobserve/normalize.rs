use serde_json::Value;

use crate::domain::{LogRecord, SpanRecord};

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

    let trace_id = field(&["trace_id", "traceId"]).or_else(|| {
        object
            .get("trace_id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    })?;
    let ts = object
        .get("_timestamp")
        .or_else(|| object.get("timestamp"))
        .and_then(Value::as_i64)
        .map(to_millis)
        .unwrap_or(0);
    let severity = field(&["severity_text", "level", "severity"]).unwrap_or_else(|| "ERROR".into());
    let message = field(&["body", "message"]).unwrap_or_default();
    let span_id = field(&["span_id", "spanId"]);

    Some(NormalizedLog {
        record: LogRecord {
            ts,
            severity,
            message,
            trace_id: Some(trace_id),
            span_id,
            fields: Value::Object(attributes.clone()),
        },
        query: field(&["query", "dns.question.name"]),
        qtype: field(&["qtype", "dns.question.type"]),
        instance: field(&["host_name", "host.name", "service.instance.id"]),
        authority: field(&["authority", "dns.authority"]),
        response_code: field(&["response_code", "dns.response.code"]),
        error: field(&["error", "error.message"]),
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