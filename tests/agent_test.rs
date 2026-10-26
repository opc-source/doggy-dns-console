use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use adk_rust::model::{ScriptedLlm, ScriptedResponse};
use adk_rust::runner::Runner;
use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};
use adk_rust::{Content, Part};
use async_trait::async_trait;
use doggy_dns_console::agent::build_agent;
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{FailureDetail, FailureFilter, FailureGroup, Overview, SeenInstance, Window};
use doggy_dns_console::error::DataError;
use doggy_dns_console::service::ConsoleService;
use futures::StreamExt;
use serde_json::json;

struct CountingTelemetry {
    inner: FixtureSource,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl TelemetrySource for CountingTelemetry {
    async fn overview(&self, window: Window) -> Result<Overview, DataError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.overview(window).await
    }
    async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        self.inner.failures(filter).await
    }
    async fn failure_detail(&self, trace_id: &str) -> Result<FailureDetail, DataError> {
        self.inner.failure_detail(trace_id).await
    }
    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError> {
        self.inner.seen_instances(window).await
    }
}

fn service(calls: Arc<AtomicUsize>) -> Arc<ConsoleService> {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let inner = FixtureSource::new(clock);
    let instances = Arc::new(FixtureSource::new(Arc::new(FixedClock(1_761_436_800_000))));
    Arc::new(ConsoleService::new(
        Arc::new(CountingTelemetry { inner, calls }),
        instances as Arc<dyn InstanceSource>,
    ))
}

async fn run_once(runner: &Runner, text: &str) -> String {
    let content = Content::new("user").with_text(text);
    let mut stream = runner.run_str("user", "session-1", content).await.unwrap();
    let mut output = String::new();
    while let Some(event) = stream.next().await {
        let event = event.unwrap();
        if let Some(content) = event.content() {
            for part in &content.parts {
                if let Part::Text { text } = part {
                    output.push_str(text);
                }
            }
        }
    }
    output
}

#[tokio::test]
async fn agent_calls_get_overview_before_answering() {
    let calls = Arc::new(AtomicUsize::new(0));
    let service = service(calls.clone());
    let model = Arc::new(ScriptedLlm::new(
        "test",
        [
            ScriptedResponse::function_call("get_overview", json!({"windowMinutes": 60})),
            ScriptedResponse::text("最近 1 小时成功率 99.8%。"),
        ],
    ));
    let agent = build_agent(model, service, Vec::new()).unwrap();
    let sessions = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "test".to_string(),
            user_id: "user".to_string(),
            session_id: Some("session-1".to_string()),
            state: Default::default(),
        })
        .await
        .unwrap();
    let runner = Runner::builder()
        .app_name("test")
        .agent(agent)
        .session_service(sessions)
        .build()
        .unwrap();
    let output = run_once(&runner, "现在成功率多少？").await;
    assert!(output.contains("99.8%"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
