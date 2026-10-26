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
