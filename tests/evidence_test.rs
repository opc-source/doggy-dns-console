use std::collections::HashSet;

use doggy_dns_console::domain::LogRecord;
use doggy_dns_console::evidence::hash::{log_hash, log_hash_input, short_hash};
use doggy_dns_console::evidence::ids::IdAllocator;
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
