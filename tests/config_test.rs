use doggy_dns_console::config::Config;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: std::collections::HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| map.get(k).cloned()
}

#[test]
fn defaults_are_loopback_and_local_sqlite() {
    let cfg = Config::from_env_with(env(&[
        ("OPENOBSERVE_BASE_URL", "http://127.0.0.1:5080"),
        ("MODEL_API_KEY", "test-key"),
    ]))
    .unwrap();
    assert_eq!(cfg.addr.to_string(), "127.0.0.1:8787");
    assert_eq!(cfg.db_url, "sqlite:data/console.db");
    assert_eq!(cfg.app_name, "doggy-dns-console");
    assert_eq!(cfg.user_id, "local");
    assert!(!cfg.demo);
    assert!(cfg.openobserve_mcp.enabled);
    assert_eq!(cfg.openobserve.metric_queries, "dns_queries");
    assert_eq!(cfg.openobserve.metric_duration, "dns_query_duration_bucket");
    assert_eq!(cfg.openobserve.log_stream, "doggy_dns");
    assert_eq!(cfg.openobserve.trace_stream, "doggy_dns");
    assert_eq!(cfg.model.name, "qwen3.8-flash");
    assert_eq!(cfg.mcp.control_port, 19090);
}

#[test]
fn mcp_control_port_overrides_the_discovery_fallback() {
    let cfg = Config::from_env_with(env(&[("CONSOLE_DEMO", "1"), ("MCP_CONTROL_PORT", "19191")]))
        .unwrap();
    assert_eq!(cfg.mcp.control_port, 19191);
}

#[test]
fn invalid_mcp_control_port_is_rejected() {
    for raw in ["not-a-port", "0", "70000"] {
        let err = Config::from_env_with(env(&[("CONSOLE_DEMO", "1"), ("MCP_CONTROL_PORT", raw)]))
            .unwrap_err();
        assert!(
            err.to_string().contains("MCP_CONTROL_PORT"),
            "unexpected error for {raw}: {err}"
        );
    }
}

#[test]
fn openobserve_url_is_required_outside_demo() {
    let err = Config::from_env_with(env(&[("MODEL_API_KEY", "test-key")])).unwrap_err();
    assert!(err.to_string().contains("OPENOBSERVE_BASE_URL"));
}

#[test]
fn demo_mode_needs_no_model_key_or_openobserve() {
    let cfg = Config::from_env_with(env(&[("CONSOLE_DEMO", "1")])).unwrap();
    assert!(cfg.demo);
    assert!(cfg.model.api_key.is_empty());
    assert!(!cfg.openobserve_mcp.enabled);
}

#[test]
fn model_config_falls_back_to_openai_and_qwen_env() {
    let cfg = Config::from_env_with(env(&[
        ("OPENOBSERVE_BASE_URL", "http://127.0.0.1:5080"),
        (
            "OPENAI_BASE_URL",
            "https://token-plan.example/compatible-mode/v1",
        ),
        ("QWEN_API_KEY", "qwen-key"),
    ]))
    .unwrap();
    assert_eq!(
        cfg.model.base_url.as_deref(),
        Some("https://token-plan.example/compatible-mode/v1")
    );
    assert_eq!(cfg.model.api_key, "qwen-key");
}

#[test]
fn model_config_falls_back_to_openai_key() {
    let cfg = Config::from_env_with(env(&[
        ("OPENOBSERVE_BASE_URL", "http://127.0.0.1:5080"),
        ("OPENAI_BASE_URL", "https://openai.example/v1"),
        ("OPENAI_API_KEY", "openai-key"),
    ]))
    .unwrap();
    assert_eq!(
        cfg.model.base_url.as_deref(),
        Some("https://openai.example/v1")
    );
    assert_eq!(cfg.model.api_key, "openai-key");
}

#[test]
fn qwen_api_key_wins_over_openai_key() {
    let cfg = Config::from_env_with(env(&[
        ("OPENOBSERVE_BASE_URL", "http://127.0.0.1:5080"),
        ("QWEN_API_KEY", "qwen-key"),
        ("OPENAI_API_KEY", "openai-key"),
    ]))
    .unwrap();
    assert_eq!(cfg.model.api_key, "qwen-key");
}

#[test]
fn model_config_prefers_explicit_model_env() {
    let cfg = Config::from_env_with(env(&[
        ("OPENOBSERVE_BASE_URL", "http://127.0.0.1:5080"),
        ("MODEL_BASE_URL", "https://explicit.example/v1"),
        ("MODEL_API_KEY", "explicit-key"),
        ("OPENAI_BASE_URL", "https://fallback.example/v1"),
        ("QWEN_API_KEY", "qwen-key"),
    ]))
    .unwrap();
    assert_eq!(
        cfg.model.base_url.as_deref(),
        Some("https://explicit.example/v1")
    );
    assert_eq!(cfg.model.api_key, "explicit-key");
}

#[test]
fn bearer_token_wins_over_basic_auth() {
    let cfg = Config::from_env_with(env(&[
        ("CONSOLE_DEMO", "1"),
        ("OPENOBSERVE_TOKEN", "bearer-token"),
        ("OPENOBSERVE_USER", "u"),
        ("OPENOBSERVE_PASSWORD", "p"),
    ]))
    .unwrap();
    assert!(matches!(
        cfg.openobserve.auth,
        doggy_dns_console::config::OpenObserveAuth::Bearer(ref t) if t == "bearer-token"
    ));
}

#[test]
fn openobserve_mcp_url_defaults_from_base_and_org() {
    let cfg = Config::from_env_with(env(&[
        ("CONSOLE_DEMO", "1"),
        ("OPENOBSERVE_BASE_URL", "http://127.0.0.1:5080"),
        ("OPENOBSERVE_ORG", "default"),
    ]))
    .unwrap();
    assert_eq!(
        cfg.openobserve_mcp.url,
        "http://127.0.0.1:5080/api/default/mcp"
    );
}

#[test]
fn openobserve_mcp_token_overrides_basic_auth() {
    let cfg = Config::from_env_with(env(&[
        ("CONSOLE_DEMO", "1"),
        ("OPENOBSERVE_USER", "u"),
        ("OPENOBSERVE_PASSWORD", "p"),
        ("OPENOBSERVE_MCP_TOKEN", "mcp-token"),
    ]))
    .unwrap();
    assert!(matches!(
        cfg.openobserve_mcp.auth,
        doggy_dns_console::config::OpenObserveAuth::Bearer(ref t) if t == "mcp-token"
    ));
}

#[test]
fn analysis_config_defaults_are_secure() {
    let cfg = Config::from_env_with(env(&[("CONSOLE_DEMO", "1")])).unwrap();
    assert!(cfg.analysis.enabled);
    assert_eq!(cfg.analysis.retention_days, 30);
    assert_eq!(cfg.analysis.rate_per_minute, 10);
    assert_eq!(cfg.analysis.timeout, std::time::Duration::from_secs(60));
    assert_eq!(cfg.analysis.max_tool_calls, 6);
}

#[test]
fn analysis_config_reads_overrides() {
    let cfg = Config::from_env_with(env(&[
        ("CONSOLE_DEMO", "1"),
        ("CONSOLE_ANALYSIS_ENABLED", "false"),
        ("CONSOLE_ANALYSIS_RETENTION_DAYS", "7"),
        ("CONSOLE_ANALYSIS_RATE_PER_MIN", "2"),
        ("CONSOLE_ANALYSIS_TIMEOUT_SECONDS", "15"),
        ("CONSOLE_ANALYSIS_MAX_TOOL_CALLS", "3"),
    ]))
    .unwrap();
    assert!(!cfg.analysis.enabled);
    assert_eq!(cfg.analysis.retention_days, 7);
    assert_eq!(cfg.analysis.rate_per_minute, 2);
    assert_eq!(cfg.analysis.timeout, std::time::Duration::from_secs(15));
    assert_eq!(cfg.analysis.max_tool_calls, 3);
}

#[test]
fn analysis_config_rejects_invalid_values() {
    for (key, value) in [
        ("CONSOLE_ANALYSIS_ENABLED", "sometimes"),
        ("CONSOLE_ANALYSIS_RETENTION_DAYS", "0"),
        ("CONSOLE_ANALYSIS_RATE_PER_MIN", "abc"),
        ("CONSOLE_ANALYSIS_TIMEOUT_SECONDS", "0"),
        ("CONSOLE_ANALYSIS_MAX_TOOL_CALLS", "0"),
    ] {
        let err = Config::from_env_with(env(&[("CONSOLE_DEMO", "1"), (key, value)])).unwrap_err();
        assert!(
            err.to_string().contains(key),
            "unexpected error for {key}={value}: {err}"
        );
    }
}
