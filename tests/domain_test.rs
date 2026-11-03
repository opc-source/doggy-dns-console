use doggy_dns_console::domain::{FailureFilter, Overview, Range, Window, host_filter};

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
fn range_rejects_extreme_bounds_instead_of_overflowing() {
    let now = 1_761_436_800_000;
    // Both directions overflow `end - start`; each must be an Err, not a panic.
    assert!(Range::resolve(Some(i64::MIN), Some(0), None, now).is_err());
    assert!(Range::resolve(None, Some(i64::MIN), None, now).is_err());
    assert!(Range::checked(i64::MIN, 1_000, now).is_err());
}

#[test]
fn range_resolve_honours_a_lone_from() {
    // `windowMinutes` must not move a start the caller stated, and the resolved
    // end stays the server clock. The span has to stay inside the 7-day ceiling
    // for `checked` to accept it, so `now` is one hour past `from`; the window
    // is half that hour, so a resolver that ignored `from` would produce a
    // different start.
    let now = 1_000 + 60 * 60_000;
    let range = Range::resolve(Some(1_000), None, Some(30), now).unwrap();
    assert_eq!(range.end_ms, now);
    assert_eq!(range.start_ms, 1_000);
}

#[test]
fn host_filter_trims_and_drops_an_empty_label() {
    assert_eq!(host_filter(None), None);
    assert_eq!(host_filter(Some(String::new())), None);
    assert_eq!(host_filter(Some("   ".to_owned())), None);
    assert_eq!(
        host_filter(Some(" dns-node-2 ".to_owned())).as_deref(),
        Some("dns-node-2")
    );
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

#[test]
fn legacy_failure_filter_json_still_decodes() {
    // What src/analysis/store.rs wrote before the absolute range existed.
    // `SqliteInvestigationStore::decode_record` runs this same `from_str`, so a
    // failure here would be a 500 on every historical row.
    let legacy = r#"{"window":{"minutes":60},"domain":null,"responseCode":null,"limit":50}"#;
    let filter: FailureFilter = serde_json::from_str(legacy).unwrap();

    assert_eq!(filter.limit, 50);
    assert_eq!(filter.domain, None);
    assert_eq!(filter.response_code, None);
    assert_eq!(filter.instance, None);
    // The unknown `window` key is ignored and the missing `range` decodes to the
    // inert 0..0 bracket; nothing reads it off the evidence echo after the
    // fingerprint decoupling.
    assert_eq!(filter.range, Range::default());
    assert_eq!(filter.range.start_ms, 0);
    assert_eq!(filter.range.end_ms, 0);
}

#[test]
fn failure_filter_new_defaults_the_limit() {
    let filter = FailureFilter::new(Range::new(1_000, 2_000));
    assert_eq!(filter.range, Range::new(1_000, 2_000));
    assert_eq!(filter.limit, 50);
    assert_eq!(filter.domain, None);
    assert_eq!(filter.response_code, None);
    assert_eq!(filter.instance, None);

    // Default stays available for tests and is defined as `new(0..0)`; the live
    // limit of 50 keeps it out of `#[derive(Default)]`'s reach.
    assert_eq!(
        FailureFilter::default(),
        FailureFilter::new(Range::default())
    );
}

#[test]
fn bucket_width_tiles_a_bracket_into_at_most_168_points() {
    let bracket = |minutes: i64| Range::new(0, minutes * 60_000);
    assert_eq!(bracket(5).bucket_minutes(), 1);
    assert_eq!(bracket(60).bucket_minutes(), 1);
    assert_eq!(bracket(168).bucket_minutes(), 1);
    assert_eq!(bracket(169).bucket_minutes(), 2);
    assert_eq!(bracket(1_440).bucket_minutes(), 9);
    assert_eq!(bracket(10_080).bucket_minutes(), 60);
    assert_eq!(bracket(10_080).step_seconds(), 3_600);
}

#[test]
fn rate_window_never_drops_below_five_minutes() {
    let bracket = |minutes: i64| Range::new(0, minutes * 60_000);
    assert_eq!(bracket(60).rate_window_minutes(), 5);
    assert_eq!(bracket(1_440).rate_window_minutes(), 9);
    assert_eq!(bracket(10_080).rate_window_minutes(), 60);
}

#[test]
fn totals_are_sampled_once_over_the_whole_bracket() {
    assert_eq!(Range::new(0, 60 * 60_000).whole_step_seconds(), 3_600);
    assert_eq!(Range::new(0, 10_080 * 60_000).whole_step_seconds(), 604_800);
    assert_eq!(Range::new(0, 5 * 60_000).span_minutes(), 5);
    assert_eq!(Range::new(0, 90_000).span_minutes(), 2);
}
