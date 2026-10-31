use std::sync::Arc;

use async_trait::async_trait;
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::instances::MergedInstanceSource;
use doggy_dns_console::datasource::mcp::{InstanceTransport, McpInstanceSource};
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{
    FailureDetail, FailureFilter, FailureGroup, Overview, SeenInstance, Window,
};
use doggy_dns_console::error::DataError;
use serde_json::{Value, json};

#[derive(Default)]
struct FakeTransport {
    runtime_fails: bool,
}

impl FakeTransport {
    fn ok() -> Self {
        Self::default()
    }
}

#[async_trait]
impl InstanceTransport for FakeTransport {
    async fn call(&self, endpoint: &str, tool: &str, _args: Value) -> Result<Value, DataError> {
        if endpoint.contains("bad") {
            return Err(DataError::Upstream("connection refused".into()));
        }
        match tool {
            "get_status" => Ok(json!({
                "instanceId": "dns-node-1",
                "host": "dns-node-1",
                "version": "0.1.0",
                "pid": 42,
                "uptimeSeconds": 3600,
                "startedAt": "2026-10-25T09:00:00Z",
                "listenAddr": "0.0.0.0:15353",
                "protocols": ["udp", "tcp"],
                "status": "running"
            })),
            "list_plugins" => Ok(json!({
                "plugins": [
                    {"order": 0, "kind": "forward", "enabled": true, "details": {"upstream": ["8.8.8.8:53"]}}
                ]
            })),
            "get_config" => Ok(json!({
                "format": "toml",
                "content": "[server]\nport = 15353\n",
                "redactedKeys": ["observability.basic_auth.password"]
            })),
            "get_runtime_status" if self.runtime_fails => {
                Err(DataError::Upstream("tool unavailable".into()))
            }
            "get_runtime_status" => Ok(json!({
                "signals": {
                    "nacos.nacos.local": {"cachedServices": 3, "lastPushSecondsAgo": 2},
                    "xdp": {"attached": true}
                }
            })),
            other => Err(DataError::Invalid(format!("unexpected tool {other}"))),
        }
    }
}

#[derive(Default)]
struct BatchingTransport {
    batches: std::sync::Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl InstanceTransport for BatchingTransport {
    async fn call(&self, _endpoint: &str, tool: &str, _args: Value) -> Result<Value, DataError> {
        if tool == "get_status" {
            Ok(json!({
                "instanceId": "dns-node-1",
                "host": "dns-node-1",
                "version": "0.1.0"
            }))
        } else {
            Err(DataError::Invalid(format!(
                "unexpected individual call {tool}"
            )))
        }
    }

    async fn call_tools(
        &self,
        _endpoint: &str,
        tools: &[&'static str],
    ) -> Vec<Result<Value, DataError>> {
        self.batches
            .lock()
            .unwrap()
            .push(tools.iter().map(|tool| tool.to_string()).collect());
        tools
            .iter()
            .map(|tool| match *tool {
                "list_plugins" => Ok(json!({"plugins": []})),
                "get_config" => Ok(json!({"format": "toml", "content": "", "redactedKeys": []})),
                "get_runtime_status" => Ok(json!({"signals": {}})),
                other => Err(DataError::Invalid(format!(
                    "unexpected batched call {other}"
                ))),
            })
            .collect()
    }
}

struct SeenOnly(Vec<SeenInstance>);

#[async_trait]
impl TelemetrySource for SeenOnly {
    async fn overview(&self, _window: Window) -> Result<Overview, DataError> {
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

#[tokio::test]
async fn telemetry_only_instance_detail_is_degraded_not_not_found() {
    let now = 1_761_436_800_000;
    let fixture = Arc::new(FixtureSource::new(Arc::new(FixedClock(now))));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let mcp = Arc::new(McpInstanceSource::new(
        Vec::new(),
        Arc::new(FakeTransport::ok()),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));
    let detail = merged.detail("dns-node-2").await.unwrap();
    assert_eq!(detail.instance.id, "dns-node-2");
    assert!(detail.plugins.is_empty());
    assert!(!detail.instance.mcp_reachable);
}

#[tokio::test]
async fn telemetry_only_node_is_online_inside_the_three_minute_window() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now - 150_000,
        endpoint: None,
    }]));
    let mcp = Arc::new(McpInstanceSource::new(
        Vec::new(),
        Arc::new(FakeTransport::ok()),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));
    let list = merged.list().await.unwrap();
    let node = list
        .iter()
        .find(|instance| instance.host == "node-a")
        .expect("node-a listed");
    assert!(node.online);
}

#[tokio::test]
async fn lists_instances_and_reads_plugins() {
    let source = McpInstanceSource::new(
        vec!["http://good/mcp".to_string()],
        Arc::new(FakeTransport::ok()),
    );
    let instances = source.list().await.unwrap();
    assert_eq!(instances[0].id, "dns-node-1");
    assert!(instances[0].mcp_reachable);

    let detail = source.detail("dns-node-1").await.unwrap();
    assert_eq!(detail.plugins[0].kind, "forward");
    assert!(detail.config_toml.unwrap().contains("port = 15353"));
    assert_eq!(detail.pid, Some(42));
    assert_eq!(detail.status.as_deref(), Some("running"));
    assert_eq!(
        detail.redacted_keys,
        vec!["observability.basic_auth.password"]
    );
    let runtime = detail.runtime.expect("runtime payload");
    assert_eq!(runtime["signals"]["xdp"]["attached"], true);
    assert_eq!(runtime["signals"]["nacos.nacos.local"]["cachedServices"], 3);
}

#[tokio::test]
async fn runtime_failure_degrades_but_keeps_detail() {
    let source = McpInstanceSource::new(
        vec!["http://good/mcp".to_string()],
        Arc::new(FakeTransport {
            runtime_fails: true,
        }),
    );
    let detail = source.detail("dns-node-1").await.unwrap();
    assert!(detail.runtime.is_none());
    assert!(detail.config_toml.is_some());
    assert_eq!(detail.plugins.len(), 1);
    assert_eq!(detail.pid, Some(42));
}

#[tokio::test]
async fn detail_batches_auxiliary_tools_into_one_transport_call() {
    let transport = Arc::new(BatchingTransport::default());
    let source = McpInstanceSource::new(vec!["http://good/mcp".to_string()], transport.clone());
    source.detail("dns-node-1").await.unwrap();
    let batches = transport.batches.lock().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(
        batches[0],
        vec!["list_plugins", "get_config", "get_runtime_status"]
    );
}

#[tokio::test]
async fn one_unreachable_endpoint_degrades() {
    let now = 1_761_436_800_000;
    let fixture = Arc::new(FixtureSource::new(Arc::new(FixedClock(now))));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let mcp = Arc::new(McpInstanceSource::new(
        vec!["http://bad/mcp".to_string(), "http://good/mcp".to_string()],
        Arc::new(FakeTransport::ok()),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));
    let list = merged.list().await.unwrap();
    assert!(
        list.iter()
            .any(|item| item.id == "dns-node-1" && item.mcp_reachable)
    );
    assert!(
        list.iter()
            .any(|item| item.id == "dns-node-2" && !item.mcp_reachable)
    );
}

#[derive(Default)]
struct FailingTransport;

#[async_trait]
impl InstanceTransport for FailingTransport {
    async fn call(&self, endpoint: &str, _tool: &str, _args: Value) -> Result<Value, DataError> {
        Err(DataError::Upstream(format!(
            "connection refused: {endpoint}"
        )))
    }
}

#[derive(Default)]
struct RecordingTransport {
    calls: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl InstanceTransport for RecordingTransport {
    async fn call(&self, endpoint: &str, tool: &str, _args: Value) -> Result<Value, DataError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{endpoint} {tool}"));
        let host = if endpoint.contains("node-b") {
            "node-b"
        } else {
            "node-a"
        };
        match tool {
            "get_status" => Ok(json!({
                "instanceId": format!("{host}-42"),
                "host": host,
                "version": "0.2.0"
            })),
            "list_plugins" => Ok(json!({"plugins": []})),
            "get_config" => Ok(json!({"format": "toml", "content": "", "redactedKeys": []})),
            "get_runtime_status" => Ok(json!({"signals": {}})),
            other => Err(DataError::Invalid(format!("unexpected tool {other}"))),
        }
    }
}

fn recording() -> Arc<RecordingTransport> {
    Arc::new(RecordingTransport::default())
}

#[tokio::test]
async fn detail_uses_the_endpoint_advertised_in_telemetry() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now,
        endpoint: Some("http://node-a.internal:19999/mcp".to_string()),
    }]));
    let transport = recording();
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), transport.clone()));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let detail = merged.detail("node-a").await.unwrap();
    assert!(detail.instance.mcp_reachable);
    let calls = transport.calls.lock().unwrap();
    assert!(!calls.is_empty());
    assert!(
        calls
            .iter()
            .all(|call| call.starts_with("http://node-a.internal:19999/mcp "))
    );
}

#[tokio::test]
async fn nodes_without_an_advertised_endpoint_fall_back_to_the_control_port() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now,
        endpoint: None,
    }]));
    let transport = recording();
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), transport.clone()));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19191, Arc::new(FixedClock(now)));

    let list = merged.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].mcp_reachable);
    let calls = transport.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("http://node-a:19191/mcp "))
    );
}

#[tokio::test]
async fn unknown_hosts_are_not_probed() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "unknown".to_string(),
        host: "unknown".to_string(),
        last_seen: now,
        endpoint: None,
    }]));
    let transport = recording();
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), transport.clone()));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let list = merged.list().await.unwrap();
    assert_eq!(list[0].host, "unknown");
    assert!(!list[0].mcp_reachable);
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn static_endpoints_are_probed_alongside_discovered_ones() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now,
        endpoint: None,
    }]));
    let transport = recording();
    let mcp = Arc::new(McpInstanceSource::new(
        vec!["http://127.0.0.1:19090/mcp".to_string()],
        transport.clone(),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    merged.list().await.unwrap();
    let calls = transport.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("http://127.0.0.1:19090/mcp "))
    );
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("http://node-a:19090/mcp "))
    );
}

#[tokio::test]
async fn failed_discovered_probe_merges_with_its_telemetry_instance() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now - 30_000,
        endpoint: Some("http://10.0.0.9:19090/mcp".to_string()),
    }]));
    let mcp = Arc::new(McpInstanceSource::new(
        Vec::new(),
        Arc::new(FailingTransport),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let list = merged.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "node-a");
    assert_eq!(list[0].host, "node-a");
    assert_eq!(list[0].last_seen, now - 30_000);
    assert!(list[0].online);
    assert!(!list[0].mcp_reachable);
}

#[tokio::test]
async fn failed_fallback_probe_keeps_telemetry_recency() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now - 10_000,
        endpoint: None,
    }]));
    let mcp = Arc::new(McpInstanceSource::new(
        Vec::new(),
        Arc::new(FailingTransport),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let list = merged.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].host, "node-a");
    assert_eq!(list[0].last_seen, now - 10_000);
    assert!(list[0].online);
    assert!(!list[0].mcp_reachable);
}

#[tokio::test]
async fn unreachable_discovered_endpoint_degrades_detail_with_telemetry_identity() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now - 10_000,
        endpoint: Some("http://10.0.0.9:19090/mcp".to_string()),
    }]));
    let mcp = Arc::new(McpInstanceSource::new(
        Vec::new(),
        Arc::new(FailingTransport),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let detail = merged.detail("node-a").await.unwrap();
    assert_eq!(detail.instance.id, "node-a");
    assert_eq!(detail.instance.host, "node-a");
    assert!(detail.instance.online);
    assert!(!detail.instance.mcp_reachable);
}

#[tokio::test]
async fn detail_probes_only_endpoints_matching_the_requested_instance() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![
        SeenInstance {
            id: "node-b".to_string(),
            host: "node-b".to_string(),
            last_seen: now,
            endpoint: Some("http://node-b.internal:19090/mcp".to_string()),
        },
        SeenInstance {
            id: "node-a".to_string(),
            host: "node-a".to_string(),
            last_seen: now,
            endpoint: Some("http://node-a.internal:19090/mcp".to_string()),
        },
    ]));
    let transport = recording();
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), transport.clone()));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let detail = merged.detail("node-a").await.unwrap();
    assert!(detail.instance.mcp_reachable);
    let calls = transport.calls.lock().unwrap();
    assert!(!calls.is_empty());
    assert!(
        calls
            .iter()
            .all(|call| call.starts_with("http://node-a.internal:19090/mcp "))
    );
}

#[tokio::test]
async fn detail_by_instance_id_probes_the_publishing_host() {
    let now = 1_761_436_800_000;
    let telemetry: Arc<dyn TelemetrySource> = Arc::new(SeenOnly(vec![SeenInstance {
        id: "node-a".to_string(),
        host: "node-a".to_string(),
        last_seen: now,
        endpoint: Some("http://node-a.internal:19090/mcp".to_string()),
    }]));
    let transport = recording();
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), transport.clone()));
    let merged = MergedInstanceSource::new(mcp, telemetry, 19090, Arc::new(FixedClock(now)));

    let detail = merged.detail("node-a-42").await.unwrap();
    assert_eq!(detail.instance.id, "node-a-42");
    let calls = transport.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .all(|call| call.starts_with("http://node-a.internal:19090/mcp "))
    );
}
