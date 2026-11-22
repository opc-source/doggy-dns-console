use doggy_dns_console::agent::prompt::SYSTEM_PROMPT;

#[test]
fn prompt_states_nxdomain_is_not_failure() {
    assert!(SYSTEM_PROMPT.contains("NXDOMAIN"));
    assert!(SYSTEM_PROMPT.contains("不计入失败"));
    assert!(SYSTEM_PROMPT.contains("必须先调用工具"));
}

#[test]
fn prompt_routes_raw_drilldown_to_openobserve_tools() {
    assert!(SYSTEM_PROMPT.contains("openobserve_stream_schema"));
    assert!(SYSTEM_PROMPT.contains("openobserve_search_sql"));
    assert!(SYSTEM_PROMPT.contains("先用 DNS 语义工具"));
}

#[test]
fn prompt_points_runtime_signals_at_instance_detail() {
    assert!(SYSTEM_PROMPT.contains("runtime signals"));
    assert!(SYSTEM_PROMPT.contains("get_instance_detail"));
}

#[test]
fn prompt_limits_writes_to_the_explicit_log_level_control() {
    assert!(SYSTEM_PROMPT.contains("set_instance_log_level"));
    assert!(SYSTEM_PROMPT.contains("明确要求"));
    assert!(SYSTEM_PROMPT.contains("get_instance_detail"));
    assert!(SYSTEM_PROMPT.contains("重启即恢复"));
    // The widened surface must not read as a general write grant.
    assert!(SYSTEM_PROMPT.contains("不改配置"));
    assert!(SYSTEM_PROMPT.contains("不写配置文件"));
    // A read-only console does not register the tool; the prompt must say so
    // instead of promising a control the model cannot call.
    assert!(SYSTEM_PROMPT.contains("不在工具列表中"));
    assert!(SYSTEM_PROMPT.contains("MCP_TOKEN"));
    assert!(SYSTEM_PROMPT.contains("精确 id"));
}
