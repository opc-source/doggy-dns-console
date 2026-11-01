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
use crate::analysis::prompt::{PromptFacts, system_prompt};
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

/// Attempts that produced no evidence — a tool error, an unknown tool name or a
/// duplicate call — get their own allowance instead of spending the drill-down
/// budget. In the 2026-11-01 golden run 2 to 4 of the 6 allowed calls died on
/// argument shape and guessed stream names, so the model could exhaust the
/// evidence budget without ever collecting evidence. The allowance still bounds a
/// model that retries with slightly different arguments forever.
pub const MAX_UNPRODUCTIVE_ATTEMPTS: usize = 4;

/// A completed attempt: the contract plus the audit trail persisted beside it.
/// Audits stay out of `AnalysisResult` because spec §6.2 has no field for them.
#[derive(Debug, Clone)]
pub struct ForensicOutcome {
    pub result: AnalysisResult,
    pub tool_calls: Vec<ToolCallAudit>,
    /// Truncated raw submission retained for triage when the model never
    /// produced a valid result (spec §6.3.2; Task 9 persists it in
    /// `failure_investigations.error`).
    pub diagnostic: Option<String>,
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

    let streams = drilldown.streams();
    let facts = PromptFacts {
        max_tool_calls,
        max_unproductive_attempts: MAX_UNPRODUCTIVE_ATTEMPTS,
        timeout,
        max_result_bytes: MAX_TOOL_RESULT_BYTES,
        log_stream: streams.log_stream,
        trace_stream: streams.trace_stream,
        tool_names: declared_tool_names(drilldown),
    };
    let mut messages = vec![
        Content::new("system").with_text(system_prompt(&facts)),
        Content::new("user").with_text(format!(
            "Investigate this failure evidence bundle and submit the structured analysis.\n\n{evidence_json}"
        )),
    ];

    let mut tools = tool_map(drilldown);
    tools.insert(SUBMIT_TOOL.to_string(), submit_declaration());

    let mut index = EvidenceIndex::from_bundle(bundle);
    let mut audits = Vec::new();
    let mut seen_calls = HashSet::new();
    let mut repair_used = false;
    // Counts rejected calls across turns so a submission that follows a
    // rejection can disclose the gap (spec §6.3.3).
    let mut budget_rejections = 0usize;
    let mut unproductive_rejections = 0usize;
    // Calls that returned evidence are the ones the budget pays for; attempts
    // that failed get MAX_UNPRODUCTIVE_ATTEMPTS of their own.
    let mut productive_calls = 0usize;
    let mut unproductive_attempts = 0usize;

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
            let diagnostic = Some(truncate(&text, 512));
            messages.push(assistant);
            if retry_after_invalid(&mut messages, &reason, &mut repair_used) {
                continue;
            }
            return Ok(ForensicOutcome {
                result: invalid_result(reason),
                tool_calls: audits,
                diagnostic,
            });
        }

        messages.push(assistant);

        let mut submitted = None;
        let mut rejection: Option<String> = None;
        let mut diagnostic: Option<String> = None;
        let mut submission_decided = false;

        for call in calls {
            if submission_decided {
                // A submission attempt ended this turn. Answer every remaining
                // call so the transcript has one response per function call.
                push_response(
                    &mut messages,
                    &call,
                    json!({"error": "not executed: submission ended the turn"}),
                );
                continue;
            }

            if call.name == SUBMIT_TOOL {
                match parse_submission(&call.args, &index) {
                    Ok(result) => {
                        submitted = Some(result);
                        break;
                    }
                    Err(reason) => {
                        push_response(&mut messages, &call, json!({"error": reason}));
                        diagnostic = Some(truncate(
                            &serde_json::to_string(&call.args).unwrap_or_default(),
                            512,
                        ));
                        rejection = Some(reason);
                        submission_decided = true;
                        continue;
                    }
                }
            }

            if unproductive_attempts >= MAX_UNPRODUCTIVE_ATTEMPTS {
                unproductive_rejections += 1;
                push_response(
                    &mut messages,
                    &call,
                    json!({"error": format!(
                        "too many failed drill-down attempts ({MAX_UNPRODUCTIVE_ATTEMPTS} produced no evidence); submit with the evidence you have"
                    )}),
                );
                continue;
            }

            if !tools.contains_key(&call.name) {
                unproductive_attempts += 1;
                push_response(
                    &mut messages,
                    &call,
                    json!({"error": format!("unknown tool: {}", call.name)}),
                );
                continue;
            }

            if productive_calls >= max_tool_calls {
                budget_rejections += 1;
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
                unproductive_attempts += 1;
                push_response(
                    &mut messages,
                    &call,
                    json!({"error": "duplicate drill-down call rejected: same tool and canonical arguments"}),
                );
                continue;
            }

            let call_number = audits.len() + 1;
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

            let (payload, is_error) = match outcome {
                Ok(Ok(value)) => (value, false),
                Ok(Err(error)) => (json!({"error": error}), true),
                Err(_) => {
                    let payload = json!({"error": "drill-down call timed out"});
                    let result_hash = hash_json(&payload);
                    audits.push(ToolCallAudit {
                        name: call.name.clone(),
                        canonical_args: canonical,
                        duration_ms,
                        result_hash: result_hash.clone(),
                        summary: summary_of(&payload, &result_hash, true),
                    });
                    return Ok(unavailable(
                        format!("drill-down call {call_number} timed out"),
                        audits,
                    ));
                }
            };

            let canonical_result = canonicalize(&payload);
            if is_error {
                unproductive_attempts += 1;
            } else {
                productive_calls += 1;
            }
            let result_hash = hash_json(&canonical_result);
            let capped = cap_result(&canonical_result, &result_hash);
            let summary = summary_of(&canonical_result, &result_hash, is_error);

            audits.push(ToolCallAudit {
                name: call.name.clone(),
                canonical_args: canonical,
                duration_ms,
                result_hash,
                summary,
            });

            if is_error {
                // A call that returned only an error produced no evidence, so it
                // must not mint a citable ID either: the loop would otherwise
                // define the call as unproductive in its budget and still let a
                // verdict stand on it.
                push_response(&mut messages, &call, json!({"result": capped}));
            } else {
                let evidence_id = format!("tool:{call_number}:result");
                index.insert(evidence_id.clone());
                push_response(
                    &mut messages,
                    &call,
                    json!({"evidenceId": evidence_id, "result": capped}),
                );
            }
        }

        if let Some(mut result) = submitted {
            if budget_rejections > 0 {
                push_gap(
                    &mut result,
                    &format!(
                        "drill-down budget exhausted after {max_tool_calls} calls; the verdict uses only the evidence collected before the limit"
                    ),
                );
            }
            if unproductive_rejections > 0 {
                push_gap(
                    &mut result,
                    &format!(
                        "drill-down stopped after {unproductive_attempts} failed drill-down attempts; the verdict uses only the evidence collected before that"
                    ),
                );
            }
            return Ok(ForensicOutcome {
                result,
                tool_calls: audits,
                diagnostic: None,
            });
        }

        if let Some(reason) = rejection {
            if retry_after_invalid(&mut messages, &reason, &mut repair_used) {
                continue;
            }
            return Ok(ForensicOutcome {
                result: invalid_result(reason),
                tool_calls: audits,
                diagnostic,
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

/// The names the drill-down actually declared. The prompt lists these and nothing
/// else: a model told about a tool that is not there spends a refusal discovering
/// it, and upstream descriptions stay out of the system prompt because they arrive
/// over the wire with the tool list rather than from this repo.
fn declared_tool_names(drilldown: &dyn Drilldown) -> Vec<String> {
    drilldown
        .tool_declarations()
        .iter()
        .filter_map(|declaration| {
            declaration
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
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

fn retry_after_invalid(messages: &mut Vec<Content>, reason: &str, repair_used: &mut bool) -> bool {
    if *repair_used {
        return false;
    }
    *repair_used = true;
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
        diagnostic: None,
    }
}

fn push_gap(result: &mut AnalysisResult, gap: &str) {
    if !result
        .not_determinable
        .iter()
        .any(|existing| existing == gap)
    {
        result.not_determinable.push(gap.to_string());
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

/// Structural digest persisted in the audit. It must never contain raw
/// telemetry rows (spec §8): only size, shape and the result hash survive.
fn summary_of(result: &Value, result_hash: &str, is_error: bool) -> String {
    let bytes = serde_json::to_string(result)
        .map(|text| text.len())
        .unwrap_or(0);
    let mut digest = json!({
        "bytes": bytes,
        "truncated": bytes > MAX_TOOL_RESULT_BYTES,
        "resultHash": result_hash,
    });
    if is_error {
        if let Some(error) = result.get("error").and_then(Value::as_str) {
            digest["error"] = Value::String(truncate(error, 256));
        }
    } else {
        match result {
            Value::Object(object) => {
                let mut keys: Vec<&String> = object.keys().collect();
                keys.sort_unstable();
                digest["keys"] = json!(keys.into_iter().take(32).cloned().collect::<Vec<String>>());
            }
            Value::Array(items) => digest["items"] = json!(items.len()),
            _ => {}
        }
    }
    truncate(
        &serde_json::to_string(&digest).unwrap_or_default(),
        MAX_TOOL_RESULT_BYTES,
    )
}

/// Truncates on a UTF-8 boundary so the result never exceeds `max_bytes`.
fn truncate(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    const MARKER: &str = "...";
    if max_bytes <= MARKER.len() {
        return MARKER[..max_bytes].to_string();
    }
    let mut end = max_bytes - MARKER.len();
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
