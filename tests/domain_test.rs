use doggy_dns_console::domain::{Overview, Window};

#[test]
fn success_rate_is_none_without_queries() {
    assert_eq!(Overview::success_rate(0, 0), None);
}

#[test]
fn success_rate_excludes_nxdomain_by_contract() {
    assert_eq!(Overview::success_rate(1000, 12), Some(0.988));
}

#[test]
fn success_rate_is_clamped_when_failures_exceed_queries() {
    assert_eq!(Overview::success_rate(10, 20), Some(0.0));
}

#[test]
fn window_is_clamped_to_five_and_1440_minutes() {
    assert_eq!(Window::new(0).minutes, 5);
    assert_eq!(Window::new(99999).minutes, 1440);
}
