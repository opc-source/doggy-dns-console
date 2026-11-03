use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use doggy_dns_console::agent::tools::{self, FailureArgs, InstanceArgs, OverviewArgs, TraceArgs};
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{
    FailureDetail, FailureFilter, FailureGroup, Overview, Range, SeenInstance, Window,
};
use doggy_dns_console::error::DataError;
use doggy_dns_console::service::ConsoleService;

fn fixture_service() -> ConsoleService {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let instances = fixture as Arc<dyn InstanceSource>;
    ConsoleService::new(telemetry, instances, clock)
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
            ..OverviewArgs::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(value["windowMinutes"], 60);
    assert!(value["successRate"].is_number());
}

#[tokio::test]
async fn get_instance_detail_tool_returns_live_runtime_and_redactions() {
    let service = fixture_service();
    let value = tools::get_instance_detail(
        &service,
        InstanceArgs {
            instance_id: "dns-node-1".to_string(),
        },
    )
    .await
    .unwrap();
    assert_eq!(value["pid"], 4242);
    assert_eq!(value["status"], "running");
    assert_eq!(value["runtime"]["signals"]["xdp"]["attached"], true);
    assert!(
        value["redactedKeys"]
            .as_array()
            .is_some_and(|keys| !keys.is_empty())
    );
}

#[tokio::test]
async fn failure_tool_descriptions_state_the_absolute_range_and_the_real_ceilings() {
    let service = fixture_service();
    let tools = tools::build_tools(Arc::new(service.clone()));

    let list = tools
        .iter()
        .find(|tool| tool.name() == "list_failures")
        .expect("list_failures");
    let detail = tools
        .iter()
        .find(|tool| tool.name() == "get_failure_detail")
        .expect("get_failure_detail");

    let list_text = list.description();
    let detail_text = detail.description();
    for text in [&list_text, &detail_text] {
        assert!(text.contains("from"), "{text}");
        assert!(text.contains("to"), "{text}");
        assert!(text.contains("instance"), "{text}");
        assert!(text.contains("10080") || text.contains("7 days"), "{text}");
        assert!(text.contains("epoch milliseconds"), "{text}");
    }
    assert!(
        detail_text.contains("same from/to/domain/responseCode/instance"),
        "the detail tool no longer tells the model to match the list call: {detail_text}"
    );
    assert!(
        detail_text.contains("rejected"),
        "the detail tool still promises a clamp: {detail_text}"
    );
    assert!(!detail_text.contains("capped"), "{detail_text}");
    assert!(
        detail_text.contains("windowMinutes"),
        "the detail tool omits the relative shortcut the model actually has: {detail_text}"
    );
}

#[tokio::test]
async fn list_failures_resolves_an_absolute_bracket_and_rejects_an_impossible_one() {
    let service = fixture_service();

    let value = tools::list_failures(
        &service,
        FailureArgs {
            from: Some(1_761_436_800_000 - 3 * 24 * 60 * 60_000),
            to: Some(1_761_436_800_000),
            ..FailureArgs::default()
        },
    )
    .await
    .unwrap();
    assert!(value.is_array(), "expected a group array, got {value}");

    let error = tools::list_failures(
        &service,
        FailureArgs {
            from: Some(1_761_436_800_000 - 8 * 24 * 60 * 60_000),
            to: Some(1_761_436_800_000),
            ..FailureArgs::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, DataError::Invalid(_)),
        "expected a rejection, got {error:?}"
    );
}

/// Telemetry double that records the exact filter the tools hand to the
/// service. The pin is at the tool layer — the resolved bracket, the caller's
/// instance and the limit — not at the fixture, which only starts filtering
/// its own rows in Task 7.
struct RecordingTelemetry {
    inner: FixtureSource,
    last_filter: Arc<Mutex<Option<FailureFilter>>>,
}

#[async_trait]
impl TelemetrySource for RecordingTelemetry {
    async fn overview(&self, range: Range) -> Result<Overview, DataError> {
        self.inner.overview(range).await
    }
    async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        *self.last_filter.lock().unwrap() = Some(filter);
        Ok(vec![])
    }
    async fn failure_detail(
        &self,
        trace_id: &str,
        filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
        *self.last_filter.lock().unwrap() = Some(filter.clone());
        Ok(FailureDetail {
            group: FailureGroup {
                key: trace_id.to_owned(),
                trace_id: Some(trace_id.to_owned()),
                query: "failure.example".to_owned(),
                qtype: "A".to_owned(),
                instance: filter.instance.clone(),
                authority: None,
                response_code: "SERVFAIL".to_owned(),
                error: None,
                count: 1,
                first_seen: 0,
                last_seen: 0,
                duration_ms: None,
            },
            logs: vec![],
            spans: vec![],
            spans_unavailable: false,
        })
    }
    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError> {
        self.inner.seen_instances(window).await
    }
}

fn recording_service() -> (ConsoleService, Arc<Mutex<Option<FailureFilter>>>) {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let recorder = Arc::new(RecordingTelemetry {
        inner: FixtureSource::new(clock.clone()),
        last_filter: Arc::new(Mutex::new(None)),
    });
    let recorded = recorder.last_filter.clone();
    let instances = Arc::new(FixtureSource::new(clock.clone())) as Arc<dyn InstanceSource>;
    let service = ConsoleService::new(recorder as Arc<dyn TelemetrySource>, instances, clock);
    (service, recorded)
}

#[tokio::test]
async fn failure_tools_treat_a_blank_instance_as_no_filter() {
    let (service, recorded) = recording_service();

    for args in [
        FailureArgs {
            instance: Some("  ".to_owned()),
            ..FailureArgs::default()
        },
        FailureArgs::default(),
    ] {
        tools::list_failures(&service, args).await.unwrap();
        let filter = recorded
            .lock()
            .unwrap()
            .clone()
            .expect("list_failures handed the service a filter");
        assert_eq!(filter.instance, None);
    }

    tools::get_failure_detail(
        &service,
        TraceArgs {
            trace_id: "trace-1".to_owned(),
            window_minutes: None,
            from: None,
            to: None,
            domain: None,
            response_code: None,
            instance: Some("  ".to_owned()),
        },
    )
    .await
    .unwrap();
    let filter = recorded
        .lock()
        .unwrap()
        .clone()
        .expect("get_failure_detail handed the service a filter");
    assert_eq!(filter.instance, None);
}

#[tokio::test]
async fn failure_tools_pass_the_resolved_bracket_and_instance_into_the_filter() {
    let (service, recorded) = recording_service();
    let from = 1_761_436_800_000 - 7 * 1440 * 60_000;
    let to = 1_761_436_800_000;

    tools::list_failures(
        &service,
        FailureArgs {
            from: Some(from),
            to: Some(to),
            instance: Some("dns-node-3".to_owned()),
            ..FailureArgs::default()
        },
    )
    .await
    .unwrap();
    let filter = recorded
        .lock()
        .unwrap()
        .clone()
        .expect("list_failures handed the service a filter");
    assert_eq!(filter.instance.as_deref(), Some("dns-node-3"));
    assert_eq!(filter.range, Range::new(from, to));

    tools::get_failure_detail(
        &service,
        TraceArgs {
            trace_id: "trace-1".to_owned(),
            window_minutes: None,
            from: Some(from),
            to: Some(to),
            domain: None,
            response_code: None,
            instance: Some("dns-node-3".to_owned()),
        },
    )
    .await
    .unwrap();
    let filter = recorded
        .lock()
        .unwrap()
        .clone()
        .expect("get_failure_detail handed the service a filter");
    assert_eq!(filter.instance.as_deref(), Some("dns-node-3"));
    assert_eq!(filter.range, Range::new(from, to));
    assert_eq!(filter.limit, 1);
}

#[tokio::test]
async fn the_overview_tool_promises_the_bracket_it_actually_takes() {
    let service = fixture_service();
    let tools = tools::build_tools(Arc::new(service.clone()));
    let overview = tools
        .iter()
        .find(|tool| tool.name() == "get_overview")
        .expect("get_overview is registered");
    let text = overview.description();
    for needle in [
        "from",
        "to",
        "windowMinutes",
        "epoch milliseconds",
        "7 days",
        "10080",
        "worst bucket",
        "rejected",
    ] {
        assert!(
            text.contains(needle),
            "get_overview description lost `{needle}`: {text}"
        );
    }
    // 反向钉子:描述不许再承诺 clamp,也不许再提 1440(那是看板不再有
    // 的旧 live 类上限)。
    assert!(!text.contains("1440"), "{text}");
    assert!(!text.contains("clamps"), "{text}");
}

#[tokio::test]
async fn get_overview_tool_takes_a_week_without_clamping_it() {
    let service = fixture_service();
    let value = tools::get_overview(
        &service,
        OverviewArgs {
            window_minutes: Some(10_080),
            ..OverviewArgs::default()
        },
    )
    .await
    .expect("a week is inside the ceiling");
    // 桥接实现会在这里报 1440。
    assert_eq!(value["windowMinutes"], 10_080);
}

#[tokio::test]
async fn get_overview_tool_refuses_a_week_plus_a_day() {
    let service = fixture_service();
    let error = tools::get_overview(
        &service,
        OverviewArgs {
            from: Some(1_761_436_800_000 - 11_000 * 60_000),
            to: Some(1_761_436_800_000),
            ..OverviewArgs::default()
        },
    )
    .await
    .expect_err("a bracket over the ceiling must not be clamped");
    assert!(
        error.to_string().contains("10080-minute ceiling"),
        "{error}"
    );
}
