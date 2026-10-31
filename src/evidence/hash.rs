//! Stable short hashes for evidence content.
//!
//! The hash intentionally covers only fields that describe the log's content.
//! Timestamp, trace ID and span ID are excluded: timestamps and span IDs are
//! already part of the log ID, and trace-wide metadata must not make identical
//! content hash differently.

use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::LogRecord;

/// First four SHA-256 digest bytes as 8 lowercase hex characters.
pub fn short_hash(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

/// Normalized content used as the `short_hash` input for a log.
///
/// Only stable, content-bearing fields are included. The map-like result is
/// serialized with sorted keys so field insertion order cannot change the hash.
pub fn log_hash_input(log: &LogRecord) -> String {
    let mut normalized: BTreeMap<&'static str, String> = BTreeMap::new();
    normalized.insert("severity", log.severity.trim().to_ascii_uppercase());

    let message = log.message.trim();
    if !message.is_empty() {
        normalized.insert("message", message.to_string());
    }
    if let Some(query) = field_text(
        &log.fields,
        &["query", "query_name", "qname", "dns.question.name"],
    ) {
        normalized.insert("query", query);
    }
    if let Some(qtype) = field_text(&log.fields, &["qtype", "query_type", "dns.question.type"]) {
        normalized.insert("qtype", qtype);
    }
    if let Some(response_code) = field_text(
        &log.fields,
        &["response_code", "responseCode", "dns.response.code"],
    ) {
        normalized.insert("response_code", response_code);
    }
    if let Some(error) = field_text(&log.fields, &["error", "error.message"]) {
        normalized.insert("error", error);
    }

    serde_json::to_string(&normalized).unwrap_or_default()
}

/// 8-character content hash for a log, used in its evidence ID.
pub fn log_hash(log: &LogRecord) -> String {
    short_hash(&log_hash_input(log))
}

/// First present, non-empty alias from `fields`, normalized as text.
fn field_text(fields: &Value, keys: &[&str]) -> Option<String> {
    let object = fields.as_object()?;
    for key in keys {
        if let Some(value) = object.get(*key)
            && let Some(text) = scalar_text(value)
        {
            return Some(text);
        }
    }
    None
}

fn scalar_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}
