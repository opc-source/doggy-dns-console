use std::sync::Arc;

use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{FailureFilter, Range, Window};
use doggy_dns_console::service::ConsoleService;

fn service() -> ConsoleService {
    let clock = Arc::new(doggy_dns_console::datasource::FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let instances = fixture as Arc<dyn InstanceSource>;
    ConsoleService::new(telemetry, instances, clock)
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
    let groups = service()
        .failures(FailureFilter {
            range: Range::new(0, 1_761_436_800_000 + 60 * 60_000),
            instance: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let trace_id = groups[0].trace_id.clone().unwrap();
    let detail = service()
        .failure_detail(
            &trace_id,
            FailureFilter {
                range: Range::new(0, 1_761_436_800_000 + 60 * 60_000),
                instance: None,
                ..Default::default()
            },
        )
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
