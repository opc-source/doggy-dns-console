use doggy_dns_console::domain::{Overview, Range, Window};

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

#[test]
fn range_from_window_expands_backwards_from_now() {
    let range = Range::from_window(Window::new(60), 1_761_436_800_000);
    assert_eq!(range.end_ms, 1_761_436_800_000);
    assert_eq!(range.start_ms, 1_761_436_800_000 - 60 * 60_000);
}

#[test]
fn range_resolve_prefers_explicit_from_and_to() {
    // The brief's original span (1_000..2_000) was one second, rejected by the
    // five-minute floor this same task documents; widened to ten minutes so the
    // test checks what it names: the explicit pair wins over `windowMinutes`.
    let range = Range::resolve(Some(1_000), Some(601_000), Some(60), 9_000_000_000).unwrap();
    assert_eq!((range.start_ms, range.end_ms), (1_000, 601_000));
}

#[test]
fn range_resolve_falls_back_to_window_minutes_anchored_at_to() {
    // `to` is given, `from` is not: the window is measured backwards from `to`.
    let range = Range::resolve(None, Some(600_000), Some(10), 9_000_000_000).unwrap();
    assert_eq!((range.start_ms, range.end_ms), (0, 600_000));

    // Neither bound is given: `to` defaults to now and the window to 60 minutes.
    let range = Range::resolve(None, None, None, 1_761_436_800_000).unwrap();
    assert_eq!(range.end_ms, 1_761_436_800_000);
    assert_eq!(range.start_ms, 1_761_436_800_000 - 60 * 60_000);
}

#[test]
fn range_checked_rejects_the_four_documented_rules() {
    let now = 1_761_436_800_000;
    let ten_minutes = 10 * 60_000;

    // start >= end
    assert!(Range::checked(now, now, now).is_err());
    assert!(Range::checked(now, now - ten_minutes, now).is_err());

    // span shorter than the 5-minute floor
    assert!(Range::checked(now - 4 * 60_000, now, now).is_err());
    assert!(Range::checked(now - 5 * 60_000, now, now).is_ok());

    // span longer than the 7-day ceiling
    assert!(Range::checked(now - 7 * 1440 * 60_000 - 1, now, now).is_err());
    assert!(Range::checked(now - 7 * 1440 * 60_000, now, now).is_ok());

    // `to` beyond the future tolerance
    assert!(Range::checked(now - ten_minutes, now + 60_001, now).is_err());
    assert!(Range::checked(now - ten_minutes, now + 60_000, now).is_ok());
}

#[test]
fn range_resolve_reports_the_rejection_reason() {
    let now = 1_761_436_800_000;
    let error = Range::resolve(Some(now), Some(now), None, now).unwrap_err();
    assert!(error.contains("start"), "unexpected error: {error}");

    let error =
        Range::resolve(Some(now - 60 * 60_000), Some(now + 120_000), None, now).unwrap_err();
    assert!(error.contains("future"), "unexpected error: {error}");
}
