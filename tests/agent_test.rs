use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use adk_rust::model::{ScriptedLlm, ScriptedResponse};
use adk_rust::runner::Runner;
use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};
use adk_rust::{Content, Part};
use async_trait::async_trait;
use doggy_dns_console::agent::build_agent;
use doggy_dns_console::datasource::fixture::FixtureSource;
use doggy_dns_console::datasource::{FixedClock, InstanceSource, TelemetrySource};
use doggy_dns_console::domain::{
    FailureDetail, FailureFilter, FailureGroup, Overview, Range, SeenInstance, Window,
};
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
    async fn overview(&self, range: Range) -> Result<Overview, DataError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.overview(range).await
    }
    async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
        self.inner.failures(filter).await
    }
    async fn failure_detail(
        &self,
        trace_id: &str,
        filter: FailureFilter,
    ) -> Result<FailureDetail, DataError> {
        self.inner.failure_detail(trace_id, filter).await
    }
    async fn seen_instances(&self, window: Window) -> Result<Vec<SeenInstance>, DataError> {
        self.inner.seen_instances(window).await
    }
}

fn service(calls: Arc<AtomicUsize>) -> Arc<ConsoleService> {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let inner = FixtureSource::new(clock.clone());
    let instances = Arc::new(FixtureSource::new(Arc::new(FixedClock(1_761_436_800_000))));
    Arc::new(ConsoleService::new(
        Arc::new(CountingTelemetry { inner, calls }),
        instances as Arc<dyn InstanceSource>,
        clock,
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

struct FailingControl;

#[async_trait]
impl doggy_dns_console::datasource::InstanceControl for FailingControl {
    async fn set_log_level(
        &self,
        _id: &str,
        _change: doggy_dns_console::domain::LogLevelChange,
    ) -> Result<doggy_dns_console::domain::LogLevelState, DataError> {
        Err(DataError::Control(
            "instance dns-node-1 is not reachable at its control endpoint".to_string(),
        ))
    }
}

fn service_with_control(
    control: Arc<dyn doggy_dns_console::datasource::InstanceControl>,
) -> Arc<ConsoleService> {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));
    Arc::new(
        ConsoleService::new(
            fixture.clone() as Arc<dyn TelemetrySource>,
            fixture.clone() as Arc<dyn InstanceSource>,
            clock,
        )
        .with_control(control),
    )
}

async fn run_scripted(
    model: Arc<ScriptedLlm>,
    service: Arc<ConsoleService>,
    prompt: &str,
) -> String {
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
    run_once(&runner, prompt).await
}

fn second_request_text(model: &ScriptedLlm) -> String {
    let requests = model.requests();
    assert_eq!(
        requests.len(),
        2,
        "the tool result must trigger a second model call"
    );
    serde_json::to_string(&requests[1].contents).unwrap()
}

#[tokio::test]
async fn the_model_receives_the_specific_invalid_arguments_message() {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));
    let service = service_with_control(fixture);
    let model = Arc::new(ScriptedLlm::new(
        "test",
        [
            ScriptedResponse::function_call(
                "set_instance_log_level",
                json!({ "instanceId": "dns-node-1", "directives": "debug", "reset": true }),
            ),
            ScriptedResponse::text("调整失败。"),
        ],
    ));

    let output = run_scripted(model.clone(), service, "把日志级别改成 debug 并重置").await;
    assert_eq!(output, "调整失败。");
    let request = second_request_text(&model);
    assert!(
        request.contains("pass either directives or reset, not both"),
        "the model request must carry the specific validation failure: {request}"
    );
}

#[tokio::test]
async fn the_model_receives_the_specific_control_failure() {
    let service = service_with_control(Arc::new(FailingControl));
    let model = Arc::new(ScriptedLlm::new(
        "test",
        [
            ScriptedResponse::function_call(
                "set_instance_log_level",
                json!({ "instanceId": "dns-node-1", "reset": true }),
            ),
            ScriptedResponse::text("节点不可达。"),
        ],
    ));

    let output = run_scripted(model.clone(), service, "重置日志级别").await;
    assert_eq!(output, "节点不可达。");
    let request = second_request_text(&model);
    assert!(
        request.contains("instance dns-node-1 is not reachable at its control endpoint"),
        "the model request must carry the control failure message: {request}"
    );
}
