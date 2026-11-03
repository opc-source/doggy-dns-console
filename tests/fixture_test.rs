use std::sync::Arc;

use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{FailureFilter, Range, Window};
use doggy_dns_console::error::DataError;
use doggy_dns_console::service::ConsoleService;

fn service() -> ConsoleService {
    let clock = Arc::new(doggy_dns_console::datasource::FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let instances = fixture as Arc<dyn InstanceSource>;
    ConsoleService::new(telemetry, instances, clock)
}

/// A bracket of `minutes` ending at the fixture clock, with every other field
/// at the defaults (limit 50, no domain/code/instance narrowing).
fn bracket(minutes: i64) -> FailureFilter {
    let end = 1_761_436_800_000;
    FailureFilter {
        range: Range::new(end - minutes * 60_000, end),
        ..FailureFilter::new(Range::default())
    }
}

#[tokio::test]
async fn fixture_overview_has_nodes_and_success_rate() {
    let overview = service().overview(Window::new(60)).await.unwrap();
    assert_eq!(overview.instances_online, 3);
    assert_eq!(overview.instances_total, 4);
    assert!(overview.success_rate.unwrap() > 0.9);
    assert!(!overview.series.is_empty());
}

#[tokio::test]
async fn fixture_failure_detail_contains_trace_evidence() {
    let groups = service().failures(bracket(7 * 1440)).await.unwrap();
    let trace_id = groups[0].trace_id.clone().unwrap();
    let detail = service()
        .failure_detail(&trace_id, bracket(7 * 1440))
        .await
        .unwrap();
    assert_eq!(detail.group.trace_id.as_deref(), Some(trace_id.as_str()));
    assert!(!detail.logs.is_empty());
    assert!(!detail.spans.is_empty());
    assert!(!detail.spans_unavailable);
}

#[tokio::test]
async fn fixture_instance_detail_counts_recent_failures() {
    let detail = service().instance_detail("dns-node-1").await.unwrap();
    assert_eq!(detail.instance.id, "dns-node-1");
    assert!(!detail.plugins.is_empty());
    assert!(detail.recent_failures > 0);
}

#[tokio::test]
async fn failures_narrow_to_the_requested_bracket() {
    let service = service();

    let one_hour = service
        .failures(bracket(60))
        .await
        .unwrap()
        .into_iter()
        .map(|group| group.trace_id.unwrap())
        .collect::<Vec<_>>();
    let one_week = service
        .failures(bracket(7 * 1440))
        .await
        .unwrap()
        .into_iter()
        .map(|group| group.trace_id.unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        one_hour,
        ["4bf92f3577b34da6", "b1f92f3577b34db3", "8ff92f3577b34da0"]
    );
    // 5cf9 (3h) and 7ef9 (3d) only appear once the bracket widens; a0f9 (8d)
    // never does.
    assert!(one_week.contains(&"5cf92f3577b34da7".to_owned()));
    assert!(one_week.contains(&"7ef92f3577b34da9".to_owned()));
    assert!(one_week.contains(&"9af92f3577b34db1".to_owned()));
    assert!(!one_week.contains(&"a0f92f3577b34db2".to_owned()));
    assert!(!one_week.contains(&"6df92f3577b34da8".to_owned()));
    assert!(one_week.len() > one_hour.len());
}

#[tokio::test]
async fn failures_narrow_to_the_instance_case_insensitively() {
    let service = service();

    let filter = FailureFilter {
        instance: Some("DNS-NODE-2".to_owned()),
        ..bracket(7 * 1440)
    };
    let groups = service.failures(filter).await.unwrap();
    let traces: Vec<_> = groups
        .iter()
        .map(|group| group.trace_id.clone().unwrap())
        .collect();

    assert_eq!(
        traces,
        ["5cf92f3577b34da7", "9af92f3577b34db1", "8ff92f3577b34da0"]
    );
    // The unlabeled row is unreachable by a host equality, in the fixture exactly
    // as it is in OpenObserve.
    assert!(!traces.contains(&"b1f92f3577b34db3".to_owned()));
}

#[tokio::test]
async fn failures_report_unlabeled_rows_the_instance_filter_cannot_reach() {
    let service = service();

    let filter = FailureFilter {
        instance: Some("dns-node-2".to_owned()),
        ..bracket(7 * 1440)
    };
    assert_eq!(service.unlabeled_failures(filter).await.unwrap(), 1);

    // Out of the bracket, out of the count.
    let narrow = FailureFilter {
        instance: Some("dns-node-2".to_owned()),
        ..bracket(15)
    };
    assert_eq!(service.unlabeled_failures(narrow).await.unwrap(), 0);
}

#[tokio::test]
async fn failure_detail_respects_the_bracket() {
    let service = service();

    // 7ef9 happened 3 days ago: visible in a week, a 404 inside an hour.
    let wide = service
        .failure_detail("7ef92f3577b34da9", bracket(7 * 1440))
        .await;
    assert!(wide.is_ok());

    let error = service
        .failure_detail("7ef92f3577b34da9", bracket(60))
        .await
        .unwrap_err();
    assert!(matches!(error, DataError::NotFound(_)), "got {error:?}");
}
