use doggy_dns_console::agent::prompt::SYSTEM_PROMPT;

#[test]
fn prompt_states_nxdomain_is_not_failure() {
    assert!(SYSTEM_PROMPT.contains("NXDOMAIN"));
    assert!(SYSTEM_PROMPT.contains("不计入失败"));
    assert!(SYSTEM_PROMPT.contains("必须先调用工具"));
}

#[test]
fn prompt_routes_raw_drilldown_to_oo_tools() {
    assert!(SYSTEM_PROMPT.contains("oo_stream_schema"));
    assert!(SYSTEM_PROMPT.contains("oo_search_sql"));
    assert!(SYSTEM_PROMPT.contains("先用 DNS 语义工具"));
}
