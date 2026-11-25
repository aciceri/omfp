//! Contract scenarios (L3): realistic thread runs against the pinned omp, recorded as reducer fixtures under
//! `fixtures/threads/<omp version>/` with `just record-fixtures` (TST-012). The assertions pin down the omp behavior
//! the daemon and the reducer rely on.

mod common;

use common::{Harness, Recorder, TIMEOUT, frame_type, prompt, start};
use fake_llm::{Reply, Step, ToolCall};
use omfp_core::{Resolution, ThreadEvent};
use omp_rpc::{
    AbortAndRestoreQueueCommand, DataUiResponse, ExtensionUiRequest, ExtensionUiResponse,
    FollowUpCommand, PromptStatus, RpcAgentEvent, RpcInbound, RpcNotification, ValueUiResponse,
};
use serde_json::json;

fn settled(count: usize) -> impl Fn(&[ThreadEvent]) -> bool {
    move |events| Recorder::count(events, "session_settled") >= count
}

/// The first pending dialog request of `events` matching `pick`.
fn find_request<T>(
    events: &[ThreadEvent],
    pick: impl Fn(&ExtensionUiRequest) -> Option<T>,
) -> Option<T> {
    events.iter().find_map(|event| match event {
        ThreadEvent::Omp {
            frame: RpcNotification::ExtensionUiRequest(request),
        } => pick(request),
        _ => None,
    })
}

fn resolved(id: &str) -> ThreadEvent {
    ThreadEvent::InteractionResolved {
        id: id.into(),
        resolution: Resolution::Client {
            client: "contract-test".into(),
        },
    }
}

// spec: TST-012
#[tokio::test(flavor = "multi_thread")]
async fn text_reply() {
    let harness = Harness::new(vec![Step::reply(Reply::text("Hello from the fake model"))]).await;
    let spec = harness.spec(&[]);

    let recorder = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        let turn = process
            .client()
            .prompt_and_wait(&prompt("say hi"), TIMEOUT)
            .unwrap();
        assert_eq!(turn.result.unwrap().status, PromptStatus::Completed);
        recorder.wait_until("session_settled", settled(1));
        recorder
    })
    .await
    .unwrap();

    let types: Vec<String> = recorder.events().iter().filter_map(frame_type).collect();
    assert!(
        types.iter().any(|kind| kind == "message_update"),
        "{types:?}"
    );
    recorder.record(&harness, "text_reply");
    harness.assert_no_stray_network();
}

// spec: TST-012
#[tokio::test(flavor = "multi_thread")]
async fn bash_tool_call() {
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "echo tool-output-123" }),
        )])),
        Step::reply(Reply::text("The command printed tool-output-123.")),
    ])
    .await;
    let mut spec = harness.spec(&[]);
    spec.approval_mode = Some("yolo".into());

    let recorder = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process
            .client()
            .prompt_and_wait(&prompt("run echo"), TIMEOUT)
            .unwrap();
        recorder.wait_until("session_settled", settled(1));
        recorder
    })
    .await
    .unwrap_or_else(|_| panic!("stderr:\n{}", harness.stderr()));

    let types: Vec<String> = recorder.events().iter().filter_map(frame_type).collect();
    assert!(
        types.iter().any(|kind| kind == "tool_execution_end"),
        "{types:?}"
    );
    let second = harness.fake.requests()[1].to_string();
    assert!(
        second.contains("tool-output-123"),
        "the tool result reaches the model"
    );
    recorder.record(&harness, "bash_tool_call");
    harness.assert_no_stray_network();
}

/// Whether `events` hold a `tool_execution_update` whose job frame (`details.async`) is in `state`.
fn job_update(events: &[ThreadEvent], state: &str) -> bool {
    events.iter().any(|event| {
        serde_json::to_value(event).is_ok_and(|value| {
            value["frame"]["type"] == "tool_execution_update"
                && value["frame"]["partialResult"]["details"]["async"]["state"] == state
        })
    })
}

// spec: TST-012, PRT-100
#[tokio::test(flavor = "multi_thread")]
async fn async_bash_tool_call() {
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_bg",
            "bash",
            json!({ "command": "sleep 1; echo bg-output-42", "async": true }),
        )])),
        Step::reply(Reply::text("Started it in the background.")),
        Step::reply(Reply::text("The job printed bg-output-42.")),
    ])
    .await;
    let mut spec = harness.spec(&[]);
    spec.approval_mode = Some("yolo".into());

    let recorder = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process
            .client()
            .prompt_and_wait(&prompt("run it in the background"), TIMEOUT)
            .unwrap();
        recorder.wait_until("the job's completed frame", |events| {
            job_update(events, "completed")
        });
        recorder.wait_until("session_settled", settled(1));
        recorder
    })
    .await
    .unwrap_or_else(|_| panic!("stderr:\n{}", harness.stderr()));

    let types: Vec<String> = recorder.events().iter().filter_map(frame_type).collect();
    let end = types.iter().position(|kind| kind == "tool_execution_end");
    let last_update = types
        .iter()
        .rposition(|kind| kind == "tool_execution_update");
    assert!(
        end < last_update,
        "job frames follow the call's end: {types:?}"
    );
    recorder.record(&harness, "async_bash_tool_call");
    harness.assert_no_stray_network();
}

// spec: TST-012, DMN-050
#[tokio::test(flavor = "multi_thread")]
async fn approval_requested_and_granted() {
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_2",
            "bash",
            json!({ "command": "echo approved-run" }),
        )])),
        Step::reply(Reply::text("Done after approval.")),
    ])
    .await;
    let mut spec = harness.spec(&[]);
    spec.approval_mode = Some("always-ask".into());

    let recorder = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process.client().call(&prompt("run it")).unwrap();
        recorder.wait_until("an approval select", |events| {
            find_request(events, |request| match request {
                ExtensionUiRequest::Select(_) => Some(()),
                _ => None,
            })
            .is_some()
        });
        let (id, approve) = find_request(&recorder.events(), |request| match request {
            ExtensionUiRequest::Select(select) => select
                .options
                .iter()
                .find(|option| option.starts_with("Approve"))
                .map(|option| (select.id.clone(), option.clone())),
            _ => None,
        })
        .expect("an Approve option is offered");
        process
            .client()
            .send(&RpcInbound::ExtensionUiResponse(
                ExtensionUiResponse::ValueUiResponse(ValueUiResponse {
                    id: id.clone(),
                    value: approve,
                }),
            ))
            .unwrap();
        recorder.push(resolved(&id));
        recorder.wait_until("session_settled", settled(1));
        recorder
    })
    .await
    .unwrap();

    let second = harness.fake.requests()[1].to_string();
    assert!(second.contains("approved-run"), "the approved command ran");
    recorder.record(&harness, "approval_granted");
    harness.assert_no_stray_network();
}

// spec: TST-012, DMN-050, DMN-055
#[tokio::test(flavor = "multi_thread")]
async fn ask_answered() {
    // omp 19.2's `ask` sends its whole HTML form as one `form` request: context, a radio group, a checkbox group
    // with a free "Other" answer, a required text field and a number.
    let form = r#"<form>
  <p>The service stores <strong>orders</strong> and needs a database.</p>
  <fieldset><legend>Which database?</legend>
    <label><input type="radio" name="db" value="postgres"> Postgres</label>
    <label><input type="radio" name="db" value="sqlite" checked> SQLite</label>
  </fieldset>
  <fieldset><legend>Which features?</legend>
    <label><input type="checkbox" name="features" value="auth"> Auth</label>
    <label><input type="checkbox" name="features" value="billing"> Billing</label>
    <label><input type="checkbox" name="features" value="search"> Search</label>
  </fieldset>
  <label>Service name <input type="text" name="service" required></label>
  <label>Replicas <input type="number" name="replicas" min="1" max="5" value="1"></label>
</form>"#;
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_ask_1",
            "ask",
            json!({ "html": form }),
        )])),
        Step::reply(Reply::text("Using Postgres with Auth and Elasticsearch.")),
    ])
    .await;
    let spec = harness.spec(&[]);

    let (recorder, request) = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process.client().call(&prompt("set it up")).unwrap();
        let pending = |events: &[ThreadEvent]| {
            find_request(events, |request| match request {
                ExtensionUiRequest::Form(form) => Some(form.clone()),
                _ => None,
            })
        };
        recorder.wait_until("the form request", |events| pending(events).is_some());
        let request = pending(&recorder.events()).unwrap();
        // Recorded before the answer goes out, so omp's reaction always follows it (stable fixture).
        recorder.push(resolved(&request.id));
        let data = json!({
            "db": "postgres",
            "features": ["auth", "Elasticsearch"],
            "service": "orders",
            "replicas": 3,
        });
        process
            .client()
            .send(&RpcInbound::ExtensionUiResponse(
                ExtensionUiResponse::DataUiResponse(DataUiResponse {
                    id: request.id.clone(),
                    data: data.as_object().unwrap().clone(),
                }),
            ))
            .unwrap();
        recorder.wait_until("session_settled", settled(1));
        (recorder, request)
    })
    .await
    .unwrap();

    assert_eq!(request.html, form, "the form arrives whole: {request:#?}");
    let answers = recorder.events().iter().find_map(|event| match event {
        ThreadEvent::Omp {
            frame: RpcNotification::RpcAgentEvent(RpcAgentEvent::ToolExecutionEnd(end)),
        } if end.tool_name == "ask" => end.result.clone(),
        _ => None,
    });
    assert_eq!(
        answers.as_ref().map(|result| &result["details"]["answers"]),
        Some(&json!([
            { "answer": "Postgres", "name": "db", "question": "Which database?" },
            { "answer": "Auth, Other: Elasticsearch", "name": "features", "question": "Which features?" },
            { "answer": "orders", "name": "service", "question": "Service name" },
            { "answer": "3", "name": "replicas", "question": "Replicas" },
        ])),
        "{answers:#?}"
    );
    let second = harness.fake.requests()[1].to_string();
    assert!(
        second.contains("Elasticsearch") && second.contains("orders"),
        "answers reach the model: {second}"
    );
    recorder.record(&harness, "ask_answered");
    harness.assert_no_stray_network();
}

// spec: TST-012
#[tokio::test(flavor = "multi_thread")]
async fn follow_up_queued_then_abort_restores_it() {
    let harness = Harness::new(vec![Step::reply(Reply::text("slow answer")).gated()]).await;
    let spec = harness.spec(&[]);
    let (process, recorder) = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process.client().call(&prompt("first")).unwrap();
        (process, recorder)
    })
    .await
    .unwrap();
    harness.fake.wait_for_gate(0).await;

    let (process, restored, recorder) = tokio::task::spawn_blocking(move || {
        process
            .client()
            .call(&FollowUpCommand {
                message: "later please".into(),
                images: None,
            })
            .unwrap();
        recorder.wait_until("a queue_update with the follow-up", |events| {
            Recorder::count(events, "queue_update") >= 1
        });
        let restored = process
            .client()
            .call(&AbortAndRestoreQueueCommand {})
            .unwrap();
        (process, restored, recorder)
    })
    .await
    .unwrap();
    harness.fake.release_gate(0);
    let recorder = tokio::task::spawn_blocking(move || {
        recorder.wait_until("session_settled", settled(1));
        drop(process);
        recorder
    })
    .await
    .unwrap();

    let texts: Vec<&str> = restored
        .follow_up
        .iter()
        .map(|message| message.text.as_str())
        .collect();
    assert_eq!(texts, ["later please"]);
    assert_eq!(restored.steering, Vec::new());
    recorder.record(&harness, "follow_up_then_abort");
}

// spec: PRT-100, UI-270
#[tokio::test(flavor = "multi_thread")]
async fn abort_with_a_queued_follow_up_moves_the_running_command_to_the_background() {
    // omp 19.2: `abort_and_restore_queue` with a non-empty queue detaches the running tools, so a `bash` call goes on
    // as a background job (its card shows the job, Stop ends it) instead of being killed with the turn.
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "while [ ! -e release ]; do sleep 0.05; done; echo detached-77" }),
        )])),
        Step::reply(Reply::text("The job printed detached-77.")),
    ])
    .await;
    let mut spec = harness.spec(&[]);
    spec.approval_mode = Some("yolo".into());
    let (process, recorder) = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process.client().call(&prompt("run it")).unwrap();
        recorder.wait_until("the bash call's start", |events| {
            Recorder::count(events, "tool_execution_start") >= 1
        });
        process
            .client()
            .call(&FollowUpCommand {
                message: "later please".into(),
                images: None,
            })
            .unwrap();
        recorder.wait_until("a queue_update with the follow-up", |events| {
            Recorder::count(events, "queue_update") >= 1
        });
        let restored = process
            .client()
            .call(&AbortAndRestoreQueueCommand {})
            .unwrap();
        assert_eq!(restored.follow_up.len(), 1);
        recorder.wait_until("the detached job's running frame", |events| {
            job_update(events, "running")
        });
        (process, recorder)
    })
    .await
    .unwrap_or_else(|_| panic!("stderr:\n{}", harness.stderr()));

    std::fs::write(harness.workspace().join("release"), "").unwrap();
    let recorder = tokio::task::spawn_blocking(move || {
        recorder.wait_until("the job's completed frame", |events| {
            job_update(events, "completed")
        });
        drop(process);
        recorder
    })
    .await
    .unwrap_or_else(|_| panic!("stderr:\n{}", harness.stderr()));
    let output = recorder.events().iter().rev().find_map(|event| {
        let value = serde_json::to_value(event).ok()?;
        let job = &value["frame"]["partialResult"]["details"]["async"];
        (job["state"] == "completed").then(|| value["frame"]["partialResult"].to_string())
    });
    assert!(
        output
            .as_deref()
            .is_some_and(|text| text.contains("detached-77")),
        "{output:?}"
    );
    harness.assert_no_stray_network();
}

// spec: TST-012, DMN-032
#[tokio::test(flavor = "multi_thread")]
async fn restart_resumes_with_history() {
    let harness = Harness::new(vec![
        Step::reply(Reply::text("first answer")),
        Step::reply(Reply::text("second answer")),
    ])
    .await;
    let spec = harness.spec(&[]);

    let recorder = tokio::task::spawn_blocking(move || {
        let (process, _) = start(&spec);
        process
            .client()
            .prompt_and_wait(&prompt("first question"), TIMEOUT)
            .unwrap();
        drop(process);
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process
            .client()
            .prompt_and_wait(&prompt("second question"), TIMEOUT)
            .unwrap();
        recorder.wait_until("session_settled", settled(1));
        recorder
    })
    .await
    .unwrap();

    let ThreadEvent::History { session, messages } = &recorder.events()[0] else {
        panic!("recording starts with the history");
    };
    assert!(session.resumed);
    assert_eq!(messages.len(), 2);
    recorder.record(&harness, "restart_with_history");
    harness.assert_no_stray_network();
}

// spec: TST-012, TST-105, PRT-120
#[tokio::test(flavor = "multi_thread")]
async fn local_command_output() {
    let harness = Harness::new(Vec::new()).await;
    let spec = harness.spec(&[]);

    let recorder = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        let turn = process
            .client()
            .prompt_and_wait(&prompt("/dirs"), TIMEOUT)
            .unwrap();
        // A local command: no model turn, so no `prompt_result`.
        assert!(turn.result.is_none());
        recorder.wait_until("command_output", |events| {
            Recorder::count(events, "command_output") >= 1
        });
        recorder
    })
    .await
    .unwrap();

    let outputs: Vec<String> = recorder
        .events()
        .iter()
        .filter_map(|event| match event {
            ThreadEvent::Omp {
                frame: RpcNotification::CommandOutput(output),
            } => Some(output.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(outputs.len(), 1, "{outputs:?}");
    recorder.record(&harness, "local_command_output");
    harness.assert_no_stray_network();
}
