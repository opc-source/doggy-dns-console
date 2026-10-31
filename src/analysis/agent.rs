//! The bounded forensic agent loop.
//!
//! One call drives one analysis attempt: the model reads the evidence bundle,
//! may use up to the drill-down budget, and terminates by calling
//! [`SUBMIT_TOOL`]. Budget, wall-clock time, duplicate calls and result size are
//! enforced here rather than trusted to the prompt (spec §6.1, §6.3).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use adk_rust::FunctionResponseData;
use adk_rust::prelude::{Content, Llm, LlmRequest, Part};
use futures::StreamExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::analysis::drilldown::Drilldown;
use crate::analysis::prompt::system_prompt;
use crate::analysis::validate::{EvidenceIndex, validate_result};
use crate::analysis::{
    AnalysisResult, AnalysisStatus, AnalysisVerdict, Confidence, ToolCallAudit, VerdictCategory,
};
use crate::evidence::hash::hex_lower;
use crate::evidence::model::EvidenceBundle;

/// The terminal tool the model must call to finish an attempt.
pub const SUBMIT_TOOL: &str = "submit_failure_analysis";

/// Maximum bytes kept for one tool result summary (spec §6.1).
pub const MAX_TOOL_RESULT_BYTES: usize = 8 * 1024;

/// A completed attempt: the contract plus the audit trail persisted beside it.
/// Audits stay out of `AnalysisResult` because spec §6.2 has no field for them.
#[derive(Debug, Clone)]
pub struct ForensicOutcome {
    pub result: AnalysisResult,
    pub tool_calls: Vec<ToolCallAudit>,
}

struct Call {
    name: String,
    args: Value,
    id: Option<String>,
}

/// Runs one analysis attempt to a terminal submission or a degraded outcome.
///
/// `Err` is reserved for failures to serialize the bundle or request. Model
/// errors, timeouts and exhausted budgets are normal degraded outcomes and are
/// returned as `Ok` with `AnalysisStatus::Unavailable`.
pub async fn run_forensic_loop(
    llm: &dyn Llm,
    bundle: &EvidenceBundle,
    drilldown: &dyn Drilldown,
    max_tool_calls: usize,
    timeout: Duration,
) -> Result<ForensicOutcome, String> {
    let deadline = Instant::now() + timeout;
    let evidence_json = serde_json::to_string_pretty(bundle).map_err(|error| error.to_string())?;

    let mut messages = vec![
        Content::new("system").with_text(system_prompt()),
        Content::new("user").with_text(format!(
            "Investigate this failure evidence bundle and submit the structured analysis.\n\n{evidence_json}"
        )),
    ];

    let mut tools = tool_map(drilldown);
    tools.insert(SUBMIT_TOOL.to_string(), submit_declaration());

    let mut index = EvidenceIndex::from_bundle(bundle);
    let mut audits = Vec::new();
    let mut seen_calls = HashSet::new();
    let mut repairs_used = 0u32;

    loop {
        if remaining(deadline).is_none() {
            return Ok(unavailable(
                "analysis timed out before a valid submission",
                audits,
            ));
        }

        let mut request = LlmRequest::new(llm.name(), messages.clone());
        request.tools = tools.clone();

        let assistant = match call_model(llm, request, deadline).await {
            Ok(content) => content,
            Err(error) => return Ok(unavailable(format!("model call failed: {error}"), audits)),
        };

        let calls: Vec<Call> = assistant
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::FunctionCall { name, args, id, .. } => Some(Call {
                    name: name.clone(),
                    args: args.clone(),
                    id: id.clone(),
                }),
                _ => None,
            })
            .collect();

        if calls.is_empty() {
            let text = text_of(&assistant);
            let reason = if text.trim().is_empty() {
                "the model replied without calling submit_failure_analysis".to_string()
            } else {
                format!(
                    "the model replied without calling submit_failure_analysis: {}",
                    truncate(&text, 512)
                )
            };
            messages.push(assistant);
            if retry_after_invalid(&mut messages, &reason, &mut repairs_used) {
                continue;
            }
            return Ok(ForensicOutcome {
                result: invalid_result(reason),
                tool_calls: audits,
            });
        }

        messages.push(assistant);

        let mut submitted = None;
        let mut repair_reason = None;

        for call in calls {
            if call.name == SUBMIT_TOOL {
                match parse_submission(&call.args, &index) {
                    Ok(result) => {
                        submitted = Some(result);
                        break;
                    }
                    Err(reason) => {
                        repair_reason = Some(reason);
                        break;
                    }
                }
            }

            if !tools.contains_key(&call.name) {
                push_response(
                    &mut messages,
                    &call,
                    json!({"error": format!("unknown tool: {}", call.name)}),
                );
                continue;
            }

            if audits.len() >= max_tool_calls {
                push_response(
                    &mut messages,
                    &call,
                    json!({"error": format!(
                        "drill-down budget exhausted ({max_tool_calls} calls); submit with the evidence you have"
                    )}),
                );
                continue;
            }

            let canonical = canonicalize(&call.args);
            let fingerprint = format!(
                "{}\u{0}{}",
                call.name,
                serde_json::to_string(&canonical).unwrap_or_default()
            );
            if !seen_calls.insert(fingerprint) {
                push_response(
                    &mut messages,
                    &call,
                    json!({"error": "duplicate drill-down call rejected: same tool and canonical arguments"}),
                );
                continue;
            }

            let call_number = audits.len() + 1;
            let evidence_id = format!("tool:{call_number}:result");
            let left = match remaining(deadline) {
                Some(left) => left,
                None => {
                    return Ok(unavailable(
                        format!("analysis timed out before drill-down call {call_number}"),
                        audits,
                    ));
                }
            };

            let started = Instant::now();
            let outcome =
                tokio::time::timeout(left, drilldown.call_tool(&call.name, call.args.clone()))
                    .await;
            let duration_ms = started.elapsed().as_millis() as u64;

            let payload = match outcome {
                Ok(Ok(value)) => value,
                Ok(Err(error)) => json!({"error": error}),
                Err(_) => {
                    return Ok(unavailable(
                        format!("drill-down call {call_number} timed out"),
                        audits,
                    ));
                }
            };

            let canonical_result = canonicalize(&payload);
            let result_hash = hash_json(&canonical_result);
            let capped = cap_result(&canonical_result, &result_hash);
            let summary = truncate(
                &serde_json::to_string(&capped).unwrap_or_default(),
                MAX_TOOL_RESULT_BYTES,
            );

            audits.push(ToolCallAudit {
                name: call.name.clone(),
                canonical_args: canonical,
                duration_ms,
                result_hash,
                summary,
            });
            index.insert(evidence_id.clone());

            push_response(
                &mut messages,
                &call,
                json!({"evidenceId": evidence_id, "result": capped}),
            );
        }

        if let Some(result) = submitted {
            return Ok(ForensicOutcome {
                result,
                tool_calls: audits,
            });
        }

        if let Some(reason) = repair_reason {
            if retry_after_invalid(&mut messages, &reason, &mut repairs_used) {
                continue;
            }
            return Ok(ForensicOutcome {
                result: invalid_result(reason),
                tool_calls: audits,
            });
        }
    }
}

async fn call_model(
    llm: &dyn Llm,
    request: LlmRequest,
    deadline: Instant,
) -> Result<Content, String> {
    let left = remaining(deadline).ok_or_else(|| "model call timed out".to_string())?;
    let mut stream = tokio::time::timeout(left, llm.generate_content(request, false))
        .await
        .map_err(|_| "model call timed out".to_string())?
        .map_err(|error| error.to_string())?;

    let mut content = Content::new("assistant");
    loop {
        let left = remaining(deadline).ok_or_else(|| "model stream timed out".to_string())?;
        let next = tokio::time::timeout(left, stream.next())
            .await
            .map_err(|_| "model stream timed out".to_string())?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| error.to_string())?;
        if let Some(chunk_content) = chunk.content {
            content.parts.extend(chunk_content.parts);
        }
    }
    Ok(content)
}

fn remaining(deadline: Instant) -> Option<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    (!left.is_zero()).then_some(left)
}

fn tool_map(drilldown: &dyn Drilldown) -> HashMap<String, Value> {
    let mut tools = HashMap::new();
    for declaration in drilldown.tool_declarations() {
        if let Some(name) = declaration.get("name").and_then(Value::as_str) {
            tools.insert(name.to_string(), declaration);
        }
    }
    tools
}

fn push_response(messages: &mut Vec<Content>, call: &Call, response: Value) {
    messages.push(Content {
        role: "function".to_string(),
        parts: vec![Part::FunctionResponse {
            function_response: FunctionResponseData::new(call.name.clone(), response),
            id: call.id.clone(),
            annotations: None,
        }],
    });
}

fn parse_submission(args: &Value, index: &EvidenceIndex) -> Result<AnalysisResult, String> {
    let result: AnalysisResult = serde_json::from_value(args.clone())
        .map_err(|error| format!("submission does not match the analysis contract: {error}"))?;
    validate_result(&result, index).map_err(|error| error.to_string())?;
    Ok(result)
}

fn retry_after_invalid(messages: &mut Vec<Content>, reason: &str, repairs_used: &mut u32) -> bool {
    if *repairs_used > 0 {
        return false;
    }
    *repairs_used = 1;
    messages.push(Content::new("user").with_text(format!(
        "Your submission was rejected: {reason}. Call submit_failure_analysis again with a corrected JSON object that satisfies the contract. Do not add prose."
    )));
    true
}

fn invalid_result(reason: String) -> AnalysisResult {
    AnalysisResult {
        status: AnalysisStatus::Invalid,
        verdict: AnalysisVerdict {
            category: VerdictCategory::Unknown,
            summary: "the model did not submit a valid analysis".to_string(),
            confidence: Confidence::Low,
        },
        findings: Vec::new(),
        next_steps: Vec::new(),
        not_determinable: vec![reason],
    }
}

fn unavailable(gap: impl Into<String>, tool_calls: Vec<ToolCallAudit>) -> ForensicOutcome {
    ForensicOutcome {
        result: AnalysisResult {
            status: AnalysisStatus::Unavailable,
            verdict: AnalysisVerdict {
                category: VerdictCategory::Unknown,
                summary: "the analysis did not complete".to_string(),
                confidence: Confidence::Low,
            },
            findings: Vec::new(),
            next_steps: Vec::new(),
            not_determinable: vec![gap.into()],
        },
        tool_calls,
    }
}

fn text_of(content: &Content) -> String {
    content
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sorted object keys, array order preserved.
fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort_unstable();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonicalize(&object[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

fn hash_json(value: &Value) -> String {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let digest = Sha256::digest(serialized.as_bytes());
    format!("sha256:{}", hex_lower(&digest))
}

/// Keeps the part of a tool result that is sent back to the model within the
/// 8 KB budget, marking truncation explicitly.
fn cap_result(result: &Value, result_hash: &str) -> Value {
    let serialized = serde_json::to_string(result).unwrap_or_default();
    if serialized.len() <= MAX_TOOL_RESULT_BYTES {
        return result.clone();
    }
    let preview = truncate(&serialized, MAX_TOOL_RESULT_BYTES.saturating_sub(256));
    json!({
        "truncated": true,
        "resultHash": result_hash,
        "preview": preview,
    })
}

/// Truncates on a UTF-8 boundary so the result never exceeds `max_bytes`.
fn truncate(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    const MARKER: &str = "...";
    let mut end = max_bytes.saturating_sub(MARKER.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &text[..end], MARKER)
}

fn submit_declaration() -> Value {
    json!({
        "name": SUBMIT_TOOL,
        "description": "Submit the final structured failure analysis. This terminates the investigation.",
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "required": ["status", "verdict", "findings", "nextSteps", "notDeterminable"],
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["ok", "insufficient_evidence", "unavailable", "invalid"]
                },
                "verdict": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["category", "summary", "confidence"],
                    "properties": {
                        "category": {
                            "type": "string",
                            "enum": [
                                "upstream_unreachable",
                                "upstream_timeout",
                                "upstream_refused",
                                "policy_refused",
                                "config_invalid",
                                "nacos_stale",
                                "xdp_path",
                                "unknown"
                            ]
                        },
                        "summary": {"type": "string"},
                        "confidence": {
                            "type": "string",
                            "enum": ["low", "medium", "high"]
                        }
                    }
                },
                "findings": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["claim", "evidenceIds", "kind"],
                        "properties": {
                            "claim": {"type": "string"},
                            "evidenceIds": {
                                "type": "array",
                                "items": {"type": "string"}
                            },
                            "kind": {"type": "string", "enum": ["observed", "inferred"]}
                        }
                    }
                },
                "nextSteps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["action", "reason", "risk"],
                        "properties": {
                            "action": {"type": "string"},
                            "reason": {"type": "string"},
                            "risk": {"type": "string", "enum": ["low", "medium", "high"]}
                        }
                    }
                },
                "notDeterminable": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            }
        }
    })
}
