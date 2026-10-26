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

struct FakeTransport;

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
                "status": "ok"
            })),
            "list_plugins" => Ok(json!({
                "plugins": [
                    {"order": 0, "kind": "forward", "enabled": true, "details": {"upstream": ["8.8.8.8:53"]}}
                ]
            })),
            "get_config" => Ok(json!({
                "format": "toml",
                "content": "[server]\nport = 15353\n",
                "redactedKeys": []
            })),
            other => Err(DataError::Invalid(format!("unexpected tool {other}"))),
        }
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
    async fn failure_detail(&self, _trace_id: &str) -> Result<FailureDetail, DataError> {
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
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), Arc::new(FakeTransport)));
    let merged = MergedInstanceSource::new(mcp, telemetry, Arc::new(FixedClock(now)));
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
    }]));
    let mcp = Arc::new(McpInstanceSource::new(Vec::new(), Arc::new(FakeTransport)));
    let merged = MergedInstanceSource::new(mcp, telemetry, Arc::new(FixedClock(now)));
    let list = merged.list().await.unwrap();
    assert_eq!(list[0].id, "node-a");
    assert!(list[0].online);
}

#[tokio::test]
async fn lists_instances_and_reads_plugins() {
    let source =
        McpInstanceSource::new(vec!["http://good/mcp".to_string()], Arc::new(FakeTransport));
    let instances = source.list().await.unwrap();
    assert_eq!(instances[0].id, "dns-node-1");
    assert!(instances[0].mcp_reachable);

    let detail = source.detail("dns-node-1").await.unwrap();
    assert_eq!(detail.plugins[0].kind, "forward");
    assert!(detail.config_toml.unwrap().contains("port = 15353"));
}

#[tokio::test]
async fn one_unreachable_endpoint_degrades() {
    let now = 1_761_436_800_000;
    let fixture = Arc::new(FixtureSource::new(Arc::new(FixedClock(now))));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let mcp = Arc::new(McpInstanceSource::new(
        vec!["http://bad/mcp".to_string(), "http://good/mcp".to_string()],
        Arc::new(FakeTransport),
    ));
    let merged = MergedInstanceSource::new(mcp, telemetry, Arc::new(FixedClock(now)));
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
