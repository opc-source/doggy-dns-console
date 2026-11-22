use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::instances::MergedInstanceSource;
use doggy_dns_console::datasource::mcp::{InstanceTransport, McpInstanceSource, Probe};
use doggy_dns_console::datasource::{FixedClock, InstanceControl, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{
    FailureDetail, FailureFilter, FailureGroup, LogLevelChange, LogLevelState, Overview, Range,
    SeenInstance, Window,
};
use doggy_dns_console::error::DataError;
use doggy_dns_console::service::ConsoleService;
use serde_json::{Value, json};

struct SeenOnly(Vec<SeenInstance>);

#[async_trait]
impl TelemetrySource for SeenOnly {
    async fn overview(&self, _range: Range) -> Result<Overview, DataError> {
        Err(DataError::NotConfigured("unused".into()))
    }
    async fn failures(&self, _filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        Err(DataError::NotConfigured("unused".into()))
    }
    async fn failure_detail(
        &self,
        _trace_id: &str,
        _filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
        Err(DataError::NotConfigured("unused".into()))
    }
    async fn seen_instances(&self, _window: Window) -> Result<Vec<SeenInstance>, DataError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct ControlTransport {
    calls: Mutex<Vec<(String, String, Value)>>,
}

impl ControlTransport {
    fn calls(&self) -> Vec<(String, String, Value)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl InstanceTransport for ControlTransport {
    async fn call(&self, endpoint: &str, tool: &str, args: Value) -> Result<Value, DataError> {
        self.calls
            .lock()
            .unwrap()
            .push((endpoint.to_string(), tool.to_string(), args.clone()));
        match tool {
            "get_status" => Ok(json!({
                "instanceId": "node-a",
                "host": "node-a",
                "version": "0.2.0"
            })),
            "set_log_level" => {
                let filters = args
                    .get("directives")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        args.get("reset")
                            .and_then(Value::as_bool)
                            .filter(|reset| *reset)
                            .map(|_| "info".to_string())
                    })
                    .ok_or_else(|| DataError::Invalid("bad args".to_string()))?;
                Ok(json!({ "stdout": filters, "otlp": filters }))
            }
            other => Err(DataError::Invalid(format!("unexpected tool {other}"))),
        }
    }
}

fn merged_with(
    transport: Arc<dyn InstanceTransport>,
) -> (MergedInstanceSource, Arc<dyn TelemetrySource>) {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now,
        endpoint: Some("http://node-a.internal:19999/mcp".to_string()),
    }]));
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), transport));
    (
        MergedInstanceSource::new(mcp, telemetry.clone(), 19090, Arc::new(FixedClock(now))),
        telemetry,
    )
}

#[tokio::test]
async fn control_targets_the_discovered_endpoint_and_sends_directives() {
    let transport = Arc::new(ControlTransport::default());
    let (merged, _) = merged_with(transport.clone());

    let state = InstanceControl::set_log_level(
        &merged,
        "node-a",
        LogLevelChange::Set("doggy_dns=debug".to_string()),
    )
    .await
    .unwrap();

    assert_eq!(
        state,
        LogLevelState {
            stdout: Some("doggy_dns=debug".to_string()),
            otlp: Some("doggy_dns=debug".to_string()),
        }
    );
    let calls = transport.calls();
    assert_eq!(calls[0].0, "http://node-a.internal:19999/mcp");
    assert_eq!(calls[0].1, "get_status");
    assert_eq!(calls[1].1, "set_log_level");
    assert_eq!(calls[1].2, json!({ "directives": "doggy_dns=debug" }));
}

#[tokio::test]
async fn control_sends_the_reset_flag() {
    let transport = Arc::new(ControlTransport::default());
    let (merged, _) = merged_with(transport.clone());

    let state = InstanceControl::set_log_level(&merged, "node-a", LogLevelChange::Reset)
        .await
        .unwrap();

    assert_eq!(state.stdout.as_deref(), Some("info"));
    assert_eq!(transport.calls()[1].2, json!({ "reset": true }));
}

#[tokio::test]
async fn control_without_a_handle_is_not_configured() {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));
    let service = ConsoleService::new(
        fixture.clone() as Arc<dyn TelemetrySource>,
        fixture as Arc<dyn InstanceSource>,
        clock,
    );

    assert!(!service.control_enabled());
    let error = service
        .set_log_level("dns-node-1", LogLevelChange::Reset)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DataError::NotConfigured(_)),
        "expected not configured, got {error:?}"
    );
}

#[tokio::test]
async fn fixture_control_is_visible_in_the_detail_and_reset_restores_it() {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));

    let before = fixture
        .detail("dns-node-1")
        .await
        .unwrap()
        .runtime
        .expect("fixture runtime");
    assert_eq!(before["logs"]["stdout"], "info");

    InstanceControl::set_log_level(
        &*fixture,
        "dns-node-1",
        LogLevelChange::Set("doggy_dns=debug".to_string()),
    )
    .await
    .unwrap();
    let after = fixture
        .detail("dns-node-1")
        .await
        .unwrap()
        .runtime
        .expect("fixture runtime");
    assert_eq!(after["logs"]["stdout"], "doggy_dns=debug");
    assert_eq!(after["logs"]["otlp"], "doggy_dns=debug");

    InstanceControl::set_log_level(&*fixture, "dns-node-1", LogLevelChange::Reset)
        .await
        .unwrap();
    let reset = fixture
        .detail("dns-node-1")
        .await
        .unwrap()
        .runtime
        .expect("fixture runtime");
    assert_eq!(reset["logs"]["stdout"], "info");
}

#[tokio::test]
async fn fixture_control_rejects_an_unknown_instance() {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = FixtureSource::new(clock);

    let error = InstanceControl::set_log_level(&fixture, "missing", LogLevelChange::Reset)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DataError::NotFound(_)),
        "expected not found, got {error:?}"
    );
}

struct EmptyStateTransport;

#[async_trait]
impl InstanceTransport for EmptyStateTransport {
    async fn call(&self, _endpoint: &str, tool: &str, _args: Value) -> Result<Value, DataError> {
        match tool {
            "get_status" => Ok(json!({ "instanceId": "node-a", "host": "node-a" })),
            "set_log_level" => Ok(json!({})),
            other => Err(DataError::Invalid(format!("unexpected tool {other}"))),
        }
    }
}

#[tokio::test]
async fn control_rejects_a_body_without_the_state_fields() {
    let (merged, _) = merged_with(Arc::new(EmptyStateTransport));

    let error = InstanceControl::set_log_level(&merged, "node-a", LogLevelChange::Reset)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DataError::Control(_)),
        "an empty state body must not read as two disabled outputs, got {error:?}"
    );
}

struct SiblingTransport {
    calls: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl InstanceTransport for SiblingTransport {
    async fn call(&self, endpoint: &str, tool: &str, _args: Value) -> Result<Value, DataError> {
        self.calls
            .lock()
            .unwrap()
            .push((endpoint.to_string(), tool.to_string()));
        match tool {
            "get_status" if endpoint.contains("sibling") => Ok(json!({
                "instanceId": "node-a-999",
                "host": "node-a"
            })),
            "get_status" => Ok(json!({
                "instanceId": "node-a-123",
                "host": "node-a"
            })),
            "set_log_level" => Ok(json!({ "stdout": "debug", "otlp": "debug" })),
            other => Err(DataError::Invalid(format!("unexpected tool {other}"))),
        }
    }
}

#[tokio::test]
async fn control_targets_the_exact_process_and_never_a_host_alias() {
    let transport = Arc::new(SiblingTransport {
        calls: Mutex::new(Vec::new()),
    });
    let source = McpInstanceSource::new(Vec::new(), transport.clone());
    let probes = [
        Probe::new("http://sibling/mcp"),
        Probe::new("http://target/mcp"),
    ];

    let state = source
        .set_log_level_at(
            &probes,
            "node-a-123",
            LogLevelChange::Set("debug".to_string()),
        )
        .await
        .unwrap();
    assert_eq!(state.stdout.as_deref(), Some("debug"));
    let writes: Vec<_> = transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, tool)| tool == "set_log_level")
        .cloned()
        .collect();
    assert_eq!(
        writes,
        vec![("http://target/mcp".to_string(), "set_log_level".to_string())],
        "the write must land on the exact process, not a sibling on the same host"
    );

    let error = source
        .set_log_level_at(&probes, "node-a", LogLevelChange::Reset)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DataError::Invalid(_)),
        "a host-only id must be an invalid request, got {error:?}"
    );
    assert_eq!(
        transport
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, tool)| tool == "set_log_level")
            .count(),
        1,
        "the host-only request must not issue a write"
    );
}

struct UnreachableTransport;

#[async_trait]
impl InstanceTransport for UnreachableTransport {
    async fn call(&self, _endpoint: &str, _tool: &str, _args: Value) -> Result<Value, DataError> {
        Err(DataError::Upstream("connection refused".to_string()))
    }
}

#[tokio::test]
async fn control_of_a_known_but_unreachable_instance_is_a_control_failure() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now,
        endpoint: Some("http://node-a.internal:19999/mcp".to_string()),
    }]));
    let mcp = Arc::new(McpInstanceSource::new(
        Vec::new(),
        Arc::new(UnreachableTransport),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let error = InstanceControl::set_log_level(&merged, "node-a-123", LogLevelChange::Reset)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DataError::Control(_)),
        "a known instance with a dead control port is a 502, not a 404: {error:?}"
    );
}

#[tokio::test]
async fn fixture_control_is_scoped_to_one_instance() {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock));

    InstanceControl::set_log_level(
        &*fixture,
        "dns-node-1",
        LogLevelChange::Set("doggy_dns=debug".to_string()),
    )
    .await
    .unwrap();

    let node1 = fixture.detail("dns-node-1").await.unwrap().runtime.unwrap();
    let node2 = fixture.detail("dns-node-2").await.unwrap().runtime.unwrap();
    assert_eq!(node1["logs"]["stdout"], "doggy_dns=debug");
    assert_eq!(
        node2["logs"]["stdout"], "info",
        "a demo change on one node must not leak to another"
    );
}

struct ProducerTransport;

#[async_trait]
impl InstanceTransport for ProducerTransport {
    async fn call(&self, _endpoint: &str, tool: &str, _args: Value) -> Result<Value, DataError> {
        match tool {
            "get_status" => Ok(json!({ "instanceId": "node-a-123", "host": "node-a" })),
            "set_log_level" => Ok(json!({ "stdout": "debug", "otlp": "debug" })),
            other => Err(DataError::Invalid(format!("unexpected tool {other}"))),
        }
    }
}

#[tokio::test]
async fn control_resolves_a_producer_instance_id_through_host_telemetry() {
    // Production telemetry identifies a node by host, while its control plane
    // reports `host-pid`; the write must follow the exact producer id.
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now,
        endpoint: Some("http://node-a.internal:19999/mcp".to_string()),
    }]));
    let mcp = Arc::new(McpInstanceSource::new(
        Vec::new(),
        Arc::new(ProducerTransport),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let state = InstanceControl::set_log_level(
        &merged,
        "node-a-123",
        LogLevelChange::Set("debug".to_string()),
    )
    .await
    .unwrap();
    assert_eq!(state.stdout.as_deref(), Some("debug"));

    let error = InstanceControl::set_log_level(&merged, "node-a", LogLevelChange::Reset)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DataError::Invalid(_)),
        "a host alias must be a 400, got {error:?}"
    );
}
