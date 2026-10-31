use std::collections::HashSet;

use doggy_dns_console::domain::{
    FailureGroup, Instance, InstanceDetail, InstanceIdentity, LogRecord, MatchQuality,
    MetricCounts, PluginInfo, SpanRecord,
};
use doggy_dns_console::evidence::fingerprint::compute_fingerprint;
use doggy_dns_console::evidence::hash::{log_hash, log_hash_input, short_hash};
use doggy_dns_console::evidence::ids::IdAllocator;
use doggy_dns_console::evidence::model::{
    EvidenceBundle, EvidenceConfig, EvidenceFilter, EvidenceHostMetrics, EvidenceInstance,
    EvidenceLog, EvidenceLogs, EvidenceMetrics, EvidencePlugin, EvidenceRange, EvidenceSpan,
    EvidenceSpans, MissingEvidence,
};
use doggy_dns_console::evidence::select::{MAX_LOGS, select_logs};
use serde_json::json;

fn log(ts: i64, severity: &str, message: &str) -> LogRecord {
    LogRecord {
        ts,
        severity: severity.to_string(),
        message: message.to_string(),
        trace_id: None,
        span_id: None,
        fields: json!({}),
    }
}

fn info_logs(count: usize) -> Vec<LogRecord> {
    (0..count)
        .map(|i| log(1_000 + i as i64, "INFO", &format!("info {i}")))
        .collect()
}

fn messages(items: &[LogRecord]) -> Vec<&str> {
    items.iter().map(|log| log.message.as_str()).collect()
}

// ---------------------------------------------------------------- evidence IDs

#[test]
fn id_allocator_builds_documented_ids() {
    let mut ids = IdAllocator::new();

    assert_eq!(
        ids.log(1_761_436_800_000, Some("s1"), "deadbeef"),
        "log:1761436800000:s1:deadbeef"
    );
    assert_eq!(
        ids.log(1_761_436_800_001, None, "cafebabe"),
        "log:1761436800001:none:cafebabe"
    );
    assert_eq!(ids.span("s1"), "span:s1");
    assert_eq!(ids.plugin("inst-1", 0), "plugin:inst-1:0");
    assert_eq!(ids.config("inst-1"), "config:inst-1");
    assert_eq!(
        ids.metric("host-a", "queries", 10, 20),
        "metric:host-a:queries:10-20"
    );
    assert_eq!(
        ids.metric("global", "failures", 10, 20),
        "metric:global:failures:10-20"
    );
    assert_eq!(ids.tool_result("call-1"), "tool:call-1:result");
}

#[test]
fn id_allocator_suffixes_duplicates_in_first_seen_order() {
    let mut ids = IdAllocator::new();

    assert_eq!(ids.log(1000, Some("s"), "abc12345"), "log:1000:s:abc12345");
    assert_eq!(
        ids.log(1000, Some("s"), "abc12345"),
        "log:1000:s:abc12345#2"
    );
    assert_eq!(
        ids.log(1000, Some("s"), "abc12345"),
        "log:1000:s:abc12345#3"
    );
    // A different base keeps the unsuffixed form and has its own counter.
    assert_eq!(ids.log(1000, Some("s"), "def12345"), "log:1000:s:def12345");
    assert_eq!(ids.plugin("inst", 1), "plugin:inst:1");
    assert_eq!(ids.plugin("inst", 1), "plugin:inst:1#2");
}

#[test]
fn id_allocator_never_emits_duplicate_ids() {
    let mut ids = IdAllocator::new();
    let mut seen = HashSet::new();

    for _ in 0..10 {
        assert!(seen.insert(ids.plugin("inst", 0)));
        assert!(seen.insert(ids.span("span")));
        assert!(seen.insert(ids.config("inst")));
        assert!(seen.insert(ids.tool_result("call")));
    }

    assert_eq!(seen.len(), 40);
}

// --------------------------------------------------------------------- hashing

#[test]
fn short_hash_is_the_sha256_prefix_as_8_lowercase_hex_chars() {
    // Known SHA-256 vector, so the truncation/formatting can never drift.
    assert_eq!(short_hash("test input data"), "0c7a2839");
    assert_eq!(short_hash("another test"), short_hash("another test"));

    let hash = short_hash("x");
    assert_eq!(hash.len(), 8);
    assert!(
        hash.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected lowercase hex, got {hash}"
    );
}

#[test]
fn log_hash_input_uses_only_normalized_relevant_fields() {
    let record = LogRecord {
        ts: 1_761_436_800_000,
        severity: "error".to_string(),
        message: " upstream forward lookup failed ".to_string(),
        trace_id: Some("t1".to_string()),
        span_id: Some("s1".to_string()),
        fields: json!({
            "query": "a.example.",
            "qtype": "A",
            "response_code": "SERVFAIL",
            "error": "timeout",
            "host_name": "dns-node-1",
            "authority": "forward[8.8.8.8:53]",
            "duration_ms": 12
        }),
    };

    assert_eq!(
        log_hash_input(&record),
        r#"{"error":"timeout","message":"upstream forward lookup failed","qtype":"A","query":"a.example.","response_code":"SERVFAIL","severity":"ERROR"}"#
    );
}

#[test]
fn log_hash_ignores_timestamp_span_trace_and_unrelated_fields() {
    let mut record = log(1_761_436_800_000, "ERROR", "boom");
    record.fields = json!({ "query": "a.example.", "response_code": "SERVFAIL" });
    let original = log_hash(&record);

    let mut moved = record.clone();
    moved.ts = 1_761_436_899_999;
    moved.span_id = Some("another-span".to_string());
    moved.trace_id = Some("another-trace".to_string());
    moved.fields["authority"] = json!("forward[1.1.1.1:53]");
    moved.fields["host_name"] = json!("dns-node-2");
    moved.fields["duration_ms"] = json!(99);

    assert_eq!(original, log_hash(&moved));
}

#[test]
fn log_hash_changes_when_any_relevant_field_changes() {
    let record = LogRecord {
        ts: 1000,
        severity: "ERROR".to_string(),
        message: "boom".to_string(),
        trace_id: None,
        span_id: None,
        fields: json!({
            "query": "a.example.",
            "qtype": "A",
            "response_code": "SERVFAIL",
            "error": "timeout"
        }),
    };
    let original = log_hash(&record);

    let mut changed = record.clone();
    changed.severity = "WARN".to_string();
    assert_ne!(
        original,
        log_hash(&changed),
        "severity must affect the hash"
    );

    let mut changed = record.clone();
    changed.message = "different".to_string();
    assert_ne!(original, log_hash(&changed), "message must affect the hash");

    let mut changed = record.clone();
    changed.fields["query"] = json!("b.example.");
    assert_ne!(original, log_hash(&changed), "query must affect the hash");

    let mut changed = record.clone();
    changed.fields["qtype"] = json!("AAAA");
    assert_ne!(original, log_hash(&changed), "qtype must affect the hash");

    let mut changed = record.clone();
    changed.fields["response_code"] = json!("REFUSED");
    assert_ne!(
        original,
        log_hash(&changed),
        "response code must affect the hash"
    );

    let mut changed = record.clone();
    changed.fields["error"] = json!("refused");
    assert_ne!(original, log_hash(&changed), "error must affect the hash");
}

#[test]
fn log_hash_accepts_alternative_telemetry_field_names() {
    let mut aliases = log(1000, "INFO", "completed");
    aliases.fields = json!({
        "query_name": "a.example.",
        "query_type": "AAAA",
        "dns.response.code": "REFUSED"
    });

    let mut canonical = log(1000, "INFO", "completed");
    canonical.fields = json!({
        "query": "a.example.",
        "qtype": "AAAA",
        "response_code": "REFUSED"
    });

    assert_eq!(log_hash(&aliases), log_hash(&canonical));
}

#[test]
fn log_hash_input_orders_map_like_fields_deterministically() {
    let mut first = log(1000, "ERROR", "boom");
    first.fields = json!({
        "error": "timeout",
        "response_code": "SERVFAIL",
        "qtype": "A",
        "query": "a.example."
    });

    let mut second = log(1000, "ERROR", "boom");
    second.fields = json!({
        "query": "a.example.",
        "qtype": "A",
        "response_code": "SERVFAIL",
        "error": "timeout"
    });

    assert_eq!(log_hash_input(&first), log_hash_input(&second));
}

// ------------------------------------------------------------- log selection

#[test]
fn select_logs_keeps_everything_under_the_limit_in_chronological_order() {
    let selection = select_logs(vec![
        log(30, "INFO", "third"),
        log(10, "INFO", "first"),
        log(20, "INFO", "second"),
    ]);

    assert_eq!(selection.total, 3);
    assert_eq!(selection.omitted, 0);
    assert!(!selection.truncated);
    assert_eq!(
        selection.items.iter().map(|log| log.ts).collect::<Vec<_>>(),
        vec![10, 20, 30]
    );
}

#[test]
fn select_logs_treats_an_empty_input_as_empty() {
    let selection = select_logs(Vec::new());

    assert!(selection.items.is_empty());
    assert_eq!(selection.total, 0);
    assert_eq!(selection.omitted, 0);
    assert!(!selection.truncated);
}

#[test]
fn select_logs_at_the_limit_is_not_truncated() {
    let selection = select_logs(info_logs(MAX_LOGS));

    assert_eq!(MAX_LOGS, 200);
    assert_eq!(selection.items.len(), MAX_LOGS);
    assert_eq!(selection.total, MAX_LOGS);
    assert_eq!(selection.omitted, 0);
    assert!(!selection.truncated);
}

#[test]
fn select_logs_caps_info_only_at_200_keeping_first_and_last_info() {
    let selection = select_logs(info_logs(600));

    assert_eq!(selection.items.len(), MAX_LOGS);
    assert_eq!(selection.total, 600);
    assert_eq!(selection.omitted, 400);
    assert!(selection.truncated);
    assert!(
        selection
            .items
            .windows(2)
            .all(|window| window[0].ts < window[1].ts)
    );
    assert_eq!(selection.items.first().unwrap().ts, 1_000);
    assert_eq!(selection.items.last().unwrap().ts, 1_599);
}

#[test]
fn select_logs_keeps_failure_warn_and_error_rows() {
    let mut logs = info_logs(300);
    for index in [10usize, 100, 101, 250] {
        logs[index].severity = "ERROR".to_string();
        logs[index].message = format!("error at {index}");
    }
    logs[180].severity = "WARN".to_string();
    logs[180].message = "warn at 180".to_string();

    let selection = select_logs(logs);
    let selected = messages(&selection.items);

    for expected in [
        "error at 10",
        "error at 100",
        "error at 101",
        "error at 250",
        "warn at 180",
    ] {
        assert!(selected.contains(&expected), "{expected} was dropped");
    }
    assert_eq!(selection.total, 300);
    assert_eq!(selection.omitted, 100);
    assert!(selection.truncated);
}

#[test]
fn select_logs_keeps_immediate_neighbors_of_failure_rows() {
    let mut logs = info_logs(300);
    logs[150].severity = "ERROR".to_string();
    logs[150].message = "boom".to_string();

    let selection = select_logs(logs);
    let selected = messages(&selection.items);

    assert!(selected.contains(&"info 149"), "previous neighbor dropped");
    assert!(selected.contains(&"boom"), "failure row dropped");
    assert!(selected.contains(&"info 151"), "next neighbor dropped");
    // First INFO and last INFO bound the retained window.
    assert!(selected.contains(&"info 0"), "first INFO dropped");
    assert!(selected.contains(&"info 299"), "last INFO dropped");
}

#[test]
fn select_logs_keeps_the_earliest_200_priority_rows_when_priority_exceeds_the_cap() {
    let mut logs = Vec::new();
    for index in 0..250 {
        logs.push(log(index, "ERROR", &format!("error {index}")));
    }
    for index in 0..10 {
        logs.push(log(1_000 + index, "INFO", &format!("info {index}")));
    }
    // Newest first: the selection must not depend on input order.
    logs.reverse();

    let selection = select_logs(logs);

    assert_eq!(selection.items.len(), MAX_LOGS);
    assert_eq!(selection.total, 260);
    assert_eq!(selection.omitted, 60);
    assert!(selection.truncated);
    assert_eq!(
        selection.items.iter().map(|log| log.ts).collect::<Vec<_>>(),
        (0..200).collect::<Vec<_>>()
    );
    assert!(selection.items.iter().all(|log| log.severity == "ERROR"));
}

#[test]
fn select_logs_orders_equal_timestamps_deterministically() {
    let first = vec![
        log(5, "INFO", "b"),
        log(5, "INFO", "a"),
        log(5, "INFO", "c"),
    ];
    let second = vec![
        log(5, "INFO", "c"),
        log(5, "INFO", "a"),
        log(5, "INFO", "b"),
    ];

    let first_messages: Vec<String> = select_logs(first)
        .items
        .iter()
        .map(|log| log.message.clone())
        .collect();
    let second_messages: Vec<String> = select_logs(second)
        .items
        .iter()
        .map(|log| log.message.clone())
        .collect();

    assert_eq!(first_messages, vec!["a", "b", "c"]);
    assert_eq!(first_messages, second_messages);
}

#[test]
fn select_logs_reports_accurate_metadata() {
    let mut logs = info_logs(203);
    logs[1].severity = "ERROR".to_string();
    logs[5].severity = "WARN".to_string();

    let selection = select_logs(logs);

    assert_eq!(selection.total, 203);
    assert_eq!(selection.items.len(), MAX_LOGS);
    assert_eq!(selection.omitted, selection.total - selection.items.len());
    assert_eq!(selection.truncated, selection.omitted > 0);
    assert!(selection.truncated);
}

// ---------------------------------------------------------- evidence contract

const TRACE_ID: &str = "trace-1";
const GENERATED_AT: i64 = 1_793_443_200_000;

fn failure_group() -> FailureGroup {
    FailureGroup {
        key: "SERVFAIL|a.example.|A|dns-node-1".to_string(),
        trace_id: Some(TRACE_ID.to_string()),
        query: "a.example.".to_string(),
        qtype: "A".to_string(),
        instance: Some("dns-node-1".to_string()),
        authority: Some("forward[8.8.8.8:53]".to_string()),
        response_code: "SERVFAIL".to_string(),
        error: Some("upstream timeout".to_string()),
        count: 3,
        first_seen: 1_793_439_600_000,
        last_seen: 1_793_443_200_000,
        duration_ms: Some(12),
    }
}

fn instance_identity() -> InstanceIdentity {
    InstanceIdentity {
        service_instance_id: Some("dns-node-1-1326".to_string()),
        host: Some("dns-node-1".to_string()),
        ip: Some("127.0.0.1".to_string()),
    }
}

/// Built from the same [`InstanceDetail`] shape the datasource returns, so the
/// evidence contract is exercised against real domain data.
fn instance_detail() -> InstanceDetail {
    InstanceDetail {
        instance: Instance {
            id: "dns-node-1".to_string(),
            host: "dns-node-1".to_string(),
            version: Some("0.1.0".to_string()),
            online: true,
            last_seen: GENERATED_AT - 1_000,
            mcp_reachable: true,
        },
        pid: Some(1326),
        status: Some("running".to_string()),
        uptime_seconds: Some(1860),
        started_at: Some("2026-10-31T08:00:00Z".to_string()),
        listen_addr: Some("127.0.0.1:15353".to_string()),
        protocols: vec!["udp".to_string(), "tcp".to_string()],
        plugins: vec![
            PluginInfo {
                order: 0,
                kind: "nacos".to_string(),
                enabled: false,
                details: json!({ "dnsZone": "nacos.local", "namespace": "public" }),
            },
            PluginInfo {
                order: 1,
                kind: "forward".to_string(),
                enabled: true,
                details: json!({ "upstream": ["8.8.8.8:53"], "cacheSize": 1024 }),
            },
        ],
        config_toml: Some("[server]\nlisten_addr = \"127.0.0.1\"\n".to_string()),
        redacted_keys: vec!["observability.basic_auth.password".to_string()],
        runtime: Some(json!({ "signals": { "xdp": { "attached": true } } })),
        recent_failures: 0,
    }
}

fn evidence_instance() -> EvidenceInstance {
    let detail = instance_detail();
    let instance_id = detail.instance.id.clone();
    EvidenceInstance {
        match_quality: MatchQuality::Exact,
        identity: instance_identity(),
        reachable: true,
        status: detail.status,
        pid: detail.pid,
        uptime_seconds: detail.uptime_seconds,
        listen_addr: detail.listen_addr,
        plugins: detail
            .plugins
            .into_iter()
            .map(|plugin| EvidencePlugin {
                id: format!("plugin:{instance_id}:{}", plugin.order),
                order: plugin.order,
                kind: plugin.kind,
                enabled: plugin.enabled,
                details: plugin.details,
            })
            .collect(),
        config: detail.config_toml.map(|content| EvidenceConfig {
            id: format!("config:{instance_id}"),
            content,
            redacted_keys: detail.redacted_keys,
        }),
        runtime: detail.runtime,
        missing: Vec::new(),
    }
}

fn evidence_log(ts: i64, severity: &str, message: &str) -> EvidenceLog {
    let record = LogRecord {
        ts,
        severity: severity.to_string(),
        message: message.to_string(),
        trace_id: Some(TRACE_ID.to_string()),
        span_id: Some("span-1".to_string()),
        fields: json!({
            "query": "a.example.",
            "qtype": "A",
            "response_code": "SERVFAIL",
            "error": "timeout"
        }),
    };

    EvidenceLog {
        id: format!(
            "log:{}:{}:{}",
            record.ts,
            record.span_id.as_deref().unwrap_or("none"),
            log_hash(&record)
        ),
        ts: record.ts,
        severity: record.severity,
        message: record.message,
        span_id: record.span_id,
        query: Some("a.example.".to_string()),
        qtype: Some("A".to_string()),
        response_code: Some("SERVFAIL".to_string()),
        error: Some("timeout".to_string()),
    }
}

fn evidence_span(span_id: &str, name: &str, duration_ms: f64, status: &str) -> EvidenceSpan {
    let record = SpanRecord {
        span_id: span_id.to_string(),
        name: name.to_string(),
        duration_ms: Some(duration_ms),
        status: status.to_string(),
        attributes: json!({ "dns.qtype": "A" }),
    };

    EvidenceSpan {
        id: format!("span:{}", record.span_id),
        span_id: record.span_id,
        name: record.name,
        duration_ms: record.duration_ms,
        status: record.status,
    }
}

fn bundle() -> EvidenceBundle {
    EvidenceBundle {
        trace_id: TRACE_ID.to_string(),
        filter: EvidenceFilter {
            window_minutes: 60,
            domain: None,
            response_code: None,
        },
        generated_at: GENERATED_AT,
        fingerprint: String::new(),
        group: failure_group(),
        instance: Some(evidence_instance()),
        logs: EvidenceLogs {
            items: vec![
                evidence_log(1_793_443_140_000, "ERROR", "upstream timeout"),
                evidence_log(1_793_443_150_000, "INFO", "retrying"),
            ],
            total: 2,
            truncated: false,
            omitted: 0,
        },
        spans: EvidenceSpans {
            items: vec![
                evidence_span("span-1", "dns.query", 12.5, "error"),
                evidence_span("span-2", "forward.lookup", 8.0, "error"),
            ],
            total: 2,
            truncated: false,
        },
        metrics: Some(EvidenceMetrics {
            range: EvidenceRange {
                start: 1_793_439_600_000,
                end: 1_793_441_400_000,
            },
            host: Some(EvidenceHostMetrics {
                name: "dns-node-1".to_string(),
                counts: MetricCounts {
                    queries: 2,
                    failures: 2,
                },
            }),
            global: MetricCounts {
                queries: 18,
                failures: 0,
            },
        }),
        missing: Vec::new(),
    }
}

fn mutated(mutate: impl FnOnce(&mut EvidenceBundle)) -> String {
    let mut bundle = bundle();
    mutate(&mut bundle);
    compute_fingerprint(&bundle)
}

fn assert_fingerprint_changes(mutate: impl FnOnce(&mut EvidenceBundle), context: &str) {
    assert_ne!(compute_fingerprint(&bundle()), mutated(mutate), "{context}");
}

#[test]
fn evidence_bundle_serializes_camel_case_and_snake_case_missing() {
    let bundle = bundle();
    let value = serde_json::to_value(&bundle).expect("bundle serializes");

    for key in [
        "traceId",
        "filter",
        "generatedAt",
        "fingerprint",
        "group",
        "instance",
        "logs",
        "spans",
        "metrics",
        "missing",
    ] {
        assert!(value.get(key).is_some(), "missing top-level key {key}");
    }
    assert_eq!(value["filter"]["windowMinutes"], 60);
    assert!(value["filter"]["domain"].is_null());
    assert!(value["filter"]["responseCode"].is_null());
    assert_eq!(value["instance"]["matchQuality"], "exact");
    assert_eq!(
        value["instance"]["identity"]["serviceInstanceId"],
        "dns-node-1-1326"
    );
    assert_eq!(value["instance"]["uptimeSeconds"], 1860);
    assert_eq!(value["instance"]["listenAddr"], "127.0.0.1:15353");
    assert_eq!(
        value["instance"]["config"]["redactedKeys"][0],
        "observability.basic_auth.password"
    );
    assert_eq!(value["logs"]["items"][0]["spanId"], "span-1");
    assert_eq!(value["spans"]["items"][0]["durationMs"], 12.5);
    assert_eq!(value["spans"]["items"][0]["spanId"], "span-1");
    assert_eq!(
        value["metrics"]["range"]["start"].as_i64(),
        Some(1_793_439_600_000)
    );
    assert_eq!(value["metrics"]["global"]["queries"], 18);
    // Host metrics keep the domain `MetricCounts` shape flattened.
    assert_eq!(value["metrics"]["host"]["name"], "dns-node-1");
    assert_eq!(value["metrics"]["host"]["queries"], 2);
    assert!(value["metrics"]["host"].get("counts").is_none());

    let missing = serde_json::to_value([
        MissingEvidence::InstanceUnreachable,
        MissingEvidence::InstanceNotFound,
        MissingEvidence::SpansUnavailable,
        MissingEvidence::MetricsUnavailable,
        MissingEvidence::LogsTruncated,
    ])
    .expect("missing evidence serializes");
    assert_eq!(
        missing,
        json!([
            "instance_unreachable",
            "instance_not_found",
            "spans_unavailable",
            "metrics_unavailable",
            "logs_truncated"
        ])
    );

    let decoded: EvidenceBundle = serde_json::from_value(value).expect("bundle round-trips");
    assert_eq!(decoded, bundle);
}

#[test]
fn fingerprint_is_deterministic_for_the_same_bundle() {
    let first = bundle();
    let second = first.clone();

    assert_eq!(
        compute_fingerprint(&first),
        compute_fingerprint(&second),
        "identical bundles must fingerprint identically"
    );
    assert_eq!(
        compute_fingerprint(&first),
        compute_fingerprint(&first),
        "repeated computation must be stable"
    );
}

#[test]
fn fingerprint_is_prefixed_sha256_with_64_lowercase_hex_chars() {
    let fingerprint = compute_fingerprint(&bundle());

    assert!(fingerprint.starts_with("sha256:"));
    assert_eq!(fingerprint.len(), 71, "sha256: plus 64 hex chars");
    let hex = &fingerprint["sha256:".len()..];
    assert_eq!(hex.len(), 64);
    assert!(
        hex.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected lowercase hex, got {hex}"
    );
}

#[test]
fn fingerprint_ignores_generated_at() {
    let first = mutated(|bundle| bundle.generated_at = 1_000);
    let second = mutated(|bundle| bundle.generated_at = 2_000);

    assert_eq!(first, second);
    assert_eq!(first, compute_fingerprint(&bundle()));
}

#[test]
fn fingerprint_ignores_preexisting_fingerprint() {
    let with_old_fingerprint = mutated(|bundle| {
        bundle.fingerprint = "sha256:deadbeef".to_string();
    });

    assert_eq!(with_old_fingerprint, compute_fingerprint(&bundle()));
}

#[test]
fn fingerprint_changes_with_trace_id_and_filter() {
    assert_fingerprint_changes(|bundle| bundle.trace_id = "trace-2".to_string(), "traceId");
    assert_fingerprint_changes(|bundle| bundle.filter.window_minutes = 30, "windowMinutes");
    assert_fingerprint_changes(
        |bundle| bundle.filter.domain = Some("example.com".to_string()),
        "filter domain",
    );
    assert_fingerprint_changes(
        |bundle| bundle.filter.response_code = Some("SERVFAIL".to_string()),
        "filter responseCode",
    );
}

#[test]
fn fingerprint_changes_with_group() {
    assert_fingerprint_changes(|bundle| bundle.group.key = "other".to_string(), "group key");
    assert_fingerprint_changes(|bundle| bundle.group.count = 4, "group count");
    assert_fingerprint_changes(
        |bundle| bundle.group.trace_id = Some("other-trace".to_string()),
        "group traceId",
    );
    assert_fingerprint_changes(
        |bundle| bundle.group.duration_ms = Some(99),
        "group durationMs",
    );
    assert_fingerprint_changes(
        |bundle| bundle.group.error = Some("different".to_string()),
        "group error",
    );
}

#[test]
fn fingerprint_changes_with_log_content() {
    assert_fingerprint_changes(
        |bundle| bundle.logs.items[0].message = "different".to_string(),
        "log message",
    );
    assert_fingerprint_changes(
        |bundle| bundle.logs.items[0].severity = "WARN".to_string(),
        "log severity",
    );
    assert_fingerprint_changes(
        |bundle| bundle.logs.items[0].query = Some("b.example.".to_string()),
        "log query",
    );
    assert_fingerprint_changes(
        |bundle| bundle.logs.items[0].qtype = Some("AAAA".to_string()),
        "log qtype",
    );
    assert_fingerprint_changes(
        |bundle| bundle.logs.items[0].response_code = Some("REFUSED".to_string()),
        "log responseCode",
    );
    assert_fingerprint_changes(|bundle| bundle.logs.items[0].error = None, "log error");
    assert_fingerprint_changes(|bundle| bundle.logs.items[0].ts += 1, "log ts");
    assert_fingerprint_changes(|bundle| bundle.logs.items[0].span_id = None, "log spanId");
    assert_fingerprint_changes(
        |bundle| bundle.logs.items[0].id = "log:changed".to_string(),
        "log id",
    );
}

#[test]
fn fingerprint_changes_with_log_counts_truncation_and_missing() {
    assert_fingerprint_changes(|bundle| bundle.logs.total = 3, "logs total");
    assert_fingerprint_changes(|bundle| bundle.logs.omitted = 1, "logs omitted");
    assert_fingerprint_changes(|bundle| bundle.logs.truncated = true, "logs truncated");
    assert_fingerprint_changes(
        |bundle| bundle.missing.push(MissingEvidence::LogsTruncated),
        "logs_truncated missing entry",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle.logs.items.pop();
        },
        "removing a log item",
    );
}

#[test]
fn fingerprint_changes_with_spans() {
    assert_fingerprint_changes(
        |bundle| bundle.spans.items[0].name = "other.span".to_string(),
        "span name",
    );
    assert_fingerprint_changes(
        |bundle| bundle.spans.items[0].duration_ms = Some(20.0),
        "span durationMs",
    );
    assert_fingerprint_changes(
        |bundle| bundle.spans.items[0].status = "ok".to_string(),
        "span status",
    );
    assert_fingerprint_changes(
        |bundle| bundle.spans.items[0].id = "span:changed".to_string(),
        "span id",
    );
    assert_fingerprint_changes(|bundle| bundle.spans.total = 3, "spans total");
    assert_fingerprint_changes(|bundle| bundle.spans.truncated = true, "spans truncated");
    assert_fingerprint_changes(
        |bundle| bundle.missing.push(MissingEvidence::SpansUnavailable),
        "spans_unavailable missing entry",
    );
}

#[test]
fn fingerprint_changes_with_instance() {
    assert_fingerprint_changes(
        |bundle| bundle.instance.as_mut().unwrap().reachable = false,
        "instance reachable",
    );
    assert_fingerprint_changes(
        |bundle| bundle.instance.as_mut().unwrap().match_quality = MatchQuality::Host,
        "instance matchQuality",
    );
    assert_fingerprint_changes(
        |bundle| bundle.instance.as_mut().unwrap().identity.host = Some("other".to_string()),
        "instance identity host",
    );
    assert_fingerprint_changes(
        |bundle| bundle.instance.as_mut().unwrap().status = Some("stopped".to_string()),
        "instance status",
    );
    assert_fingerprint_changes(
        |bundle| bundle.instance.as_mut().unwrap().pid = Some(9),
        "instance pid",
    );
    assert_fingerprint_changes(
        |bundle| bundle.instance.as_mut().unwrap().uptime_seconds = Some(1),
        "instance uptimeSeconds",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle.instance.as_mut().unwrap().listen_addr = Some("0.0.0.0:53".to_string());
        },
        "instance listenAddr",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle.instance.as_mut().unwrap().plugins[0].kind = "native".to_string();
        },
        "instance plugin kind",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle.instance.as_mut().unwrap().plugins[0].details["dnsZone"] = json!("other.local");
        },
        "instance plugin details",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle.instance.as_mut().unwrap().plugins[0].enabled = true;
        },
        "instance plugin enabled",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle
                .instance
                .as_mut()
                .unwrap()
                .config
                .as_mut()
                .unwrap()
                .content = "[changed]".to_string();
        },
        "instance config content",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle
                .instance
                .as_mut()
                .unwrap()
                .config
                .as_mut()
                .unwrap()
                .redacted_keys
                .push("other".to_string());
        },
        "instance config redactedKeys",
    );
    assert_fingerprint_changes(
        |bundle| bundle.instance.as_mut().unwrap().runtime = Some(json!({ "signals": {} })),
        "instance runtime",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle
                .instance
                .as_mut()
                .unwrap()
                .missing
                .push(MissingEvidence::InstanceUnreachable);
        },
        "instance missing entry",
    );
    assert_fingerprint_changes(|bundle| bundle.instance = None, "instance absent");
    assert_fingerprint_changes(
        |bundle| bundle.missing.push(MissingEvidence::InstanceNotFound),
        "instance_not_found missing entry",
    );
}

#[test]
fn fingerprint_changes_with_metrics() {
    assert_fingerprint_changes(|bundle| bundle.metrics = None, "metrics absent");
    assert_fingerprint_changes(
        |bundle| bundle.metrics.as_mut().unwrap().range.start -= 1,
        "metrics range start",
    );
    assert_fingerprint_changes(
        |bundle| bundle.metrics.as_mut().unwrap().range.end += 1,
        "metrics range end",
    );
    assert_fingerprint_changes(
        |bundle| bundle.metrics.as_mut().unwrap().host = None,
        "host metrics absent",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle.metrics.as_mut().unwrap().host.as_mut().unwrap().name = "other-host".to_string();
        },
        "host metrics name",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle
                .metrics
                .as_mut()
                .unwrap()
                .host
                .as_mut()
                .unwrap()
                .counts
                .queries += 1;
        },
        "host metrics queries",
    );
    assert_fingerprint_changes(
        |bundle| {
            bundle
                .metrics
                .as_mut()
                .unwrap()
                .host
                .as_mut()
                .unwrap()
                .counts
                .failures += 1;
        },
        "host metrics failures",
    );
    assert_fingerprint_changes(
        |bundle| bundle.metrics.as_mut().unwrap().global.queries += 1,
        "global metrics queries",
    );
    assert_fingerprint_changes(
        |bundle| bundle.metrics.as_mut().unwrap().global.failures += 1,
        "global metrics failures",
    );
    assert_fingerprint_changes(
        |bundle| bundle.missing.push(MissingEvidence::MetricsUnavailable),
        "metrics_unavailable missing entry",
    );
}

#[test]
fn fingerprint_ignores_object_key_order() {
    let mut first = bundle();
    let first_instance = first.instance.as_mut().unwrap();
    first_instance.runtime =
        Some(serde_json::from_str(r#"{"z":1,"a":{"q":2,"b":3}}"#).expect("valid runtime JSON"));
    first_instance.plugins[0].details =
        serde_json::from_str(r#"{"z":1,"a":2,"nested":{"y":3,"x":4}}"#).expect("valid plugin JSON");

    let mut second = bundle();
    let second_instance = second.instance.as_mut().unwrap();
    second_instance.runtime = Some(
        serde_json::from_str(r#"{"a":{"b":3,"q":2},"z":1}"#).expect("valid reordered runtime JSON"),
    );
    second_instance.plugins[0].details =
        serde_json::from_str(r#"{"nested":{"x":4,"y":3},"a":2,"z":1}"#)
            .expect("valid reordered plugin JSON");

    assert_eq!(compute_fingerprint(&first), compute_fingerprint(&second));
}

#[test]
fn fingerprint_ignores_id_bearing_array_order() {
    let original = bundle();
    let mut reordered = original.clone();
    reordered.logs.items.reverse();
    reordered.spans.items.reverse();
    reordered.instance.as_mut().unwrap().plugins.reverse();

    assert_eq!(
        compute_fingerprint(&original),
        compute_fingerprint(&reordered)
    );
}

#[test]
fn fingerprint_is_independent_of_equal_timestamp_log_order() {
    // Equal timestamps but different remaining fields: IDs differ, so
    // canonicalization must sort by ID rather than trust input order.
    let items = vec![
        evidence_log(1_793_443_140_000, "ERROR", "first"),
        evidence_log(1_793_443_140_000, "ERROR", "second"),
    ];

    let mut first = bundle();
    first.logs.items = items.clone();
    first.logs.total = items.len();

    let mut second = first.clone();
    second.logs.items.reverse();

    assert_eq!(
        compute_fingerprint(&first),
        compute_fingerprint(&second),
        "equal-timestamp rows must canonicalize by ID"
    );
}

#[test]
fn fingerprint_preserves_order_for_arrays_without_ids() {
    let mut first = bundle();
    first.missing = vec![
        MissingEvidence::MetricsUnavailable,
        MissingEvidence::SpansUnavailable,
    ];
    let mut second = first.clone();
    second.missing.reverse();

    assert_ne!(
        compute_fingerprint(&first),
        compute_fingerprint(&second),
        "arrays without a uniform string id must keep their order"
    );
}
