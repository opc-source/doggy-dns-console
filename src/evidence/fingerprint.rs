//! Deterministic evidence fingerprints.
//!
//! A fingerprint is `sha256:<64 lowercase hex>` over the canonical JSON of an
//! [`EvidenceBundle`]. Canonicalization:
//!
//! - zeroes `generatedAt`, clears `fingerprint` and drops the instance fields
//!   sampled at call time (`uptimeSeconds`, `runtime`);
//! - sorts and deduplicates the `missing` sets on the bundle and instance;
//! - sorts every object key recursively;
//! - removes all insignificant whitespace;
//! - sorts arrays whose elements are all objects with a string `id` by `id`;
//! - preserves the order of every other array.
//!
//! Arrays are sorted by `id` because two telemetry rows can share a timestamp
//! while differing in their remaining fields; the ID covers those fields, so
//! sorting by ID makes the fingerprint independent of fetch order without
//! dropping any evidence.
//!
//! Every evidence field participates in the fingerprint: trace ID, filter,
//! group, logs and spans including totals/truncation, instance identity,
//! plugins and config, metrics, and `missing`. The two instance fields the
//! control plane samples at call time — `uptimeSeconds` and the `runtime`
//! snapshot — are normalized away like `generatedAt`, because they move on
//! every read of unchanged evidence and would otherwise defeat the analysis
//! cache. Model, prompt and tool-call metadata are absent from
//! [`EvidenceBundle`] by construction and can therefore never leak into the
//! hash.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::evidence::hash::hex_lower;
use crate::evidence::model::EvidenceBundle;

/// Computes the stable fingerprint of an evidence bundle.
pub fn compute_fingerprint(bundle: &EvidenceBundle) -> String {
    let mut normalized = bundle.clone();
    normalized.generated_at = 0;
    normalized.fingerprint.clear();
    normalize_missing(&mut normalized);
    normalize_sampled_instance_fields(&mut normalized);

    // `EvidenceBundle` only contains strings, integers, booleans, options and
    // JSON values, so serialization cannot fail for a normal bundle.
    let value = serde_json::to_value(&normalized)
        .expect("evidence bundle contains only JSON-serializable values");
    let canonical = canonicalize(&value);

    let digest = Sha256::digest(canonical.as_bytes());
    format!("sha256:{}", hex_lower(&digest))
}

/// `missing` is a set: sort and deduplicate both the top-level and the
/// instance-level lists so their input order and repetition cannot affect the
/// fingerprint.
fn normalize_missing(bundle: &mut EvidenceBundle) {
    bundle.missing.sort_unstable();
    bundle.missing.dedup();
    if let Some(instance) = bundle.instance.as_mut() {
        instance.missing.sort_unstable();
        instance.missing.dedup();
    }
}

/// Drops the instance fields the control plane samples at call time.
///
/// `uptimeSeconds` is derived from the process start time and `runtime` is the
/// `get_runtime_status` snapshot, so both move on every read while the instance
/// is up, without saying anything about the failure under investigation.
/// Hashing them made the cache key churn once per second, which re-billed the
/// model on every repeated request; they stay in the bundle where the reader
/// can see them, and only leave the hash.
fn normalize_sampled_instance_fields(bundle: &mut EvidenceBundle) {
    if let Some(instance) = bundle.instance.as_mut() {
        instance.uptime_seconds = None;
        instance.runtime = None;
    }
}

/// Canonical JSON text: sorted object keys, no insignificant whitespace.
fn canonicalize(value: &Value) -> String {
    let mut output = String::new();
    write_canonical(value, &mut output);
    output
}

fn write_canonical(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(true) => output.push_str("true"),
        Value::Bool(false) => output.push_str("false"),
        Value::Number(number) => output.push_str(&number.to_string()),
        Value::String(text) => write_string(text, output),
        Value::Array(items) => {
            output.push('[');
            let ordered =
                sorted_by_id(items).unwrap_or_else(|| items.iter().collect::<Vec<&Value>>());
            write_array(&ordered, output);
            output.push(']');
        }
        Value::Object(object) => {
            output.push('{');
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_string(key, output);
                output.push(':');
                if let Some(inner) = object.get(key.as_str()) {
                    write_canonical(inner, output);
                }
            }
            output.push('}');
        }
    }
}

fn write_array(items: &[&Value], output: &mut String) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write_canonical(item, output);
    }
}

/// Returns `items` sorted by their string `id` when every element is an object
/// with one. Returns `None` for arrays without a uniform string ID so their
/// order is preserved.
fn sorted_by_id(items: &[Value]) -> Option<Vec<&Value>> {
    if items.is_empty() {
        return None;
    }

    let mut identified: Vec<(&str, &Value)> = Vec::with_capacity(items.len());
    for item in items {
        identified.push((item.get("id")?.as_str()?, item));
    }
    identified.sort_by(|left, right| left.0.cmp(right.0));
    Some(identified.into_iter().map(|(_, item)| item).collect())
}

fn write_string(text: &str, output: &mut String) {
    output.push_str(&serde_json::to_string(text).expect("serializing a JSON string cannot fail"));
}
