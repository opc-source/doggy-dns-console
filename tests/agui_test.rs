use std::sync::Arc;

use async_trait::async_trait;
use doggy_dns_console::agent::build_agent;
use doggy_dns_console::api::state::Probe;
use doggy_dns_console::config::Config;
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::error::DataError;
use doggy_dns_console::server::{AppDeps, build_app};
use doggy_dns_console::service::ConsoleService;

struct Healthy;

#[async_trait]
impl Probe for Healthy {
    async fn probe(&self) -> Result<(), DataError> {
        Ok(())
    }
}

fn config() -> Config {
    Config::from_env_with(|key| match key {
        "CONSOLE_DEMO" => Some("1".to_string()),
        "CONSOLE_DB" => Some("sqlite::memory:".to_string()),
        _ => None,
    })
    .unwrap()
}

fn demo_model() -> Arc<dyn adk_rust::prelude::Llm> {
    use adk_rust::model::{ScriptedLlm, ScriptedResponse};
    let mut script = Vec::new();
    for _ in 0..10 {
        script.push(ScriptedResponse::function_call(
            "get_overview",
            serde_json::json!({"windowMinutes": 60}),
        ));
        script.push(ScriptedResponse::text("最近 1 小时成功率 99.7%。"));
    }
    Arc::new(ScriptedLlm::new("demo", script))
}

async fn spawn_test_app() -> (String, tokio::task::JoinHandle<()>) {
    let fixture = Arc::new(FixtureSource::new(Arc::new(FixedClock(1_761_436_800_000))));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let instances = fixture as Arc<dyn InstanceSource>;
    let service = Arc::new(ConsoleService::new(telemetry, instances.clone()));
    let sessions = Arc::new(adk_rust::session::InMemorySessionService::new());
    let cfg = config();
    let agent = build_agent(demo_model(), service.clone(), Vec::new()).unwrap();
    let deps = AppDeps {
        service,
        instances,
        model: demo_model(),
        sessions,
        extra_tools: Vec::new(),
        openobserve_health: Arc::new(Healthy),
        oo_mcp_health: None,
        model_configured: true,
    };
    let _ = agent;
    let _ = cfg.clone();
    let app = build_app(cfg, deps).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn run_sse_emits_ag_ui_events() {
    let (base, handle) = spawn_test_app().await;
    let client = reqwest::Client::new();
    let created: serde_json::Value = client
        .post(format!(
            "{base}/api/apps/doggy-dns-console/users/local/sessions"
        ))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = created["id"].as_str().unwrap().to_string();

    let body = serde_json::json!({
        "appName": "doggy-dns-console",
        "userId": "local",
        "sessionId": session_id,
        "newMessage": { "role": "user", "parts": [{ "text": "现在成功率多少？" }] },
        "uiProtocol": "ag_ui",
        "uiTransport": "protocol_native"
    });
    let text = client
        .post(format!("{base}/api/run_sse"))
        .header("x-adk-ui-protocol", "ag_ui")
        .header("x-adk-ui-transport", "protocol_native")
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(text.contains("\"type\":\"RUN_STARTED\""), "{text}");
    assert!(text.contains("\"type\":\"TEXT_MESSAGE_CONTENT\""), "{text}");
    assert!(text.contains("\"type\":\"RUN_FINISHED\""), "{text}");
    handle.abort();
}

#[tokio::test]
async fn overview_works_when_oo_mcp_is_down() {
    let (base, handle) = spawn_test_app().await;
    let response = reqwest::get(format!("{base}/api/overview")).await.unwrap();
    assert!(response.status().is_success());
    handle.abort();
}
