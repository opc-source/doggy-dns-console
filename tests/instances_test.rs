use std::sync::Arc;

use async_trait::async_trait;
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::instances::MergedInstanceSource;
use doggy_dns_console::datasource::mcp::{InstanceTransport, McpInstanceSource};
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::error::DataError;
use serde_json::{Value, json};

struct FakeTransport;

#[async_trait]
impl InstanceTransport for FakeTransport {
    async fn call(
        &self,
        endpoint: &str,
        tool: &str,
        _args: Value,
    ) -> Result<Value, DataError> {
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

#[tokio::test]
async fn lists_instances_and_reads_plugins() {
    let source = McpInstanceSource::new(
        vec!["http://good/mcp".to_string()],
        Arc::new(FakeTransport),
    );
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
    assert!(list.iter().any(|item| item.id == "dns-node-1" && item.mcp_reachable));
    assert!(list.iter().any(|item| item.id == "dns-node-2" && !item.mcp_reachable));
}
