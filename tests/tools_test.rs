use std::sync::Arc;

use doggy_dns_console::agent::tools::{self, OverviewArgs};
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::service::ConsoleService;

fn fixture_service() -> ConsoleService {
    let fixture = Arc::new(FixtureSource::new(Arc::new(FixedClock(1_761_436_800_000))));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let instances = fixture as Arc<dyn InstanceSource>;
    ConsoleService::new(telemetry, instances)
}

#[tokio::test]
async fn get_overview_tool_returns_service_payload() {
    let service = fixture_service();
    let tools = tools::build_tools(Arc::new(service.clone()));
    assert!(tools.iter().any(|tool| tool.name() == "get_overview"));
    assert_eq!(tools.len(), 5);
    let value = tools::get_overview(
        &service,
        OverviewArgs {
            window_minutes: Some(60),
        },
    )
    .await
    .unwrap();
    assert_eq!(value["windowMinutes"], 60);
    assert!(value["successRate"].is_number());
}
