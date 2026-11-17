//! Contract scenarios (L3): realistic thread runs against the pinned omp, recorded as reducer fixtures under
//! `fixtures/threads/<omp version>/` with `just record-fixtures` (TST-012). The assertions pin down the omp behavior
//! the daemon and the reducer rely on.

mod common;

use common::{Harness, Recorder, TIMEOUT, frame_type, prompt, start};
use fake_llm::{Reply, Step, ToolCall};
use omfp_core::{Resolution, ThreadEvent};
use omp_rpc::{
    AbortAndRestoreQueueCommand, ExtensionUiRequest, ExtensionUiResponse, FollowUpCommand,
    PromptStatus, RpcAgentEvent, RpcInbound, RpcNotification, SelectUiRequest,
    SelectUiRequestSelectionMarker, ValueUiResponse,
};
use serde_json::json;

fn settled(count: usize) -> impl Fn(&[ThreadEvent]) -> bool {
    move |events| Recorder::count(events, "session_settled") >= count
}

/// Answer for the form fallback's `select` dialogs: SQLite for the radio group; for the checkbox loop, the first of
/// Auth and Search not checked yet, then Done, as omp reports the checked options (`checkedIndices`, omp 18.20).
fn choose(select: &SelectUiRequest) -> String {
    let option = |needle: &str| {
        select
            .options
            .iter()
            .find(|option| option.contains(needle))
            .cloned()
            .unwrap_or_else(|| panic!("no `{needle}` option in {:?}", select.options))
    };
    if select.title.contains("database") {
        return option("SQLite");
    }
    let checked = checked_options(select);
    let next = ["Auth", "Search"]
        .into_iter()
        .find(|wanted| !checked.contains(&(*wanted).to_owned()))
        .unwrap_or("Done");
    option(next)
}

/// The options of `select` its `checkedIndices` name.
fn checked_options(select: &SelectUiRequest) -> Vec<String> {
    select
        .checked_indices
        .iter()
        .flatten()
        .map(|index| select.options[usize::try_from(*index).unwrap()].clone())
        .collect()
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

// spec: TST-012, DMN-050, DMN-054
#[tokio::test(flavor = "multi_thread")]
async fn ask_answered() {
    // omp 18.22.0's `ask` takes one HTML form and walks it with `select` dialogs over RPC (not the `ask` dialog).
    let form = r#"<form>
  <fieldset><legend>Which database?</legend>
    <label><input type="radio" name="db" value="postgres"> Postgres</label>
    <label><input type="radio" name="db" value="sqlite" checked> SQLite</label>
  </fieldset>
  <fieldset><legend>Which features?</legend>
    <label><input type="checkbox" name="features" value="auth"> Auth</label>
    <label><input type="checkbox" name="features" value="billing"> Billing</label>
    <label><input type="checkbox" name="features" value="search"> Search</label>
  </fieldset>
</form>"#;
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_ask_1",
            "ask",
            json!({ "html": form }),
        )])),
        Step::reply(Reply::text("Using SQLite with Auth and Search.")),
    ])
    .await;
    let spec = harness.spec(&[]);

    let (recorder, selects) = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process.client().call(&prompt("set it up")).unwrap();
        // omp 18.22.0 walks the form field by field with `select` dialogs (it does not use the RPC `ask` dialog):
        // the radio group, then the checkbox group as a loop that ends with "Done selecting", sent again after every
        // toggle with the options checked so far.
        let mut answered: Vec<String> = Vec::new();
        let mut selects = Vec::new();
        loop {
            recorder.wait_until("a new select or the end of the turn", |events| {
                Recorder::count(events, "session_settled") >= 1
                    || find_request(events, |request| match request {
                        ExtensionUiRequest::Select(select) if !answered.contains(&select.id) => {
                            Some(())
                        }
                        _ => None,
                    })
                    .is_some()
            });
            let next = find_request(&recorder.events(), |request| match request {
                ExtensionUiRequest::Select(select) if !answered.contains(&select.id) => {
                    Some(select.clone())
                }
                _ => None,
            });
            let Some(select) = next else { break };
            process
                .client()
                .send(&RpcInbound::ExtensionUiResponse(
                    ExtensionUiResponse::ValueUiResponse(ValueUiResponse {
                        id: select.id.clone(),
                        value: choose(&select),
                    }),
                ))
                .unwrap();
            recorder.push(resolved(&select.id));
            answered.push(select.id.clone());
            selects.push(select);
        }
        recorder.wait_until("session_settled", settled(1));
        (recorder, selects)
    })
    .await
    .unwrap();

    assert_form_selects(&selects);
    let answers = recorder.events().iter().find_map(|event| match event {
        ThreadEvent::Omp {
            frame: RpcNotification::RpcAgentEvent(RpcAgentEvent::ToolExecutionEnd(end)),
        } if end.tool_name == "ask" => end.result.clone(),
        _ => None,
    });
    assert_eq!(
        answers.as_ref().map(|result| &result["details"]["answers"]),
        Some(&json!([
            { "answer": "SQLite", "name": "db", "question": "Which database?" },
            { "answer": "Auth, Search", "name": "features", "question": "Which features?" },
        ])),
        "{answers:#?}"
    );
    let second = harness.fake.requests()[1].to_string();
    assert!(
        second.contains("User answers:") && second.contains(r#"\"search\""#),
        "answers reach the model: {second}"
    );
    recorder.record(&harness, "ask_answered");
    harness.assert_no_stray_network();
}

/// The `select` dialogs of `ask_answered`'s form, with the selection fields of omp 18.20: a radio group, then the
/// checkbox loop (Auth checked, then Search, then Done), each select naming what is checked.
fn assert_form_selects(selects: &[SelectUiRequest]) {
    let (database, features) = selects.split_first().expect("a select per field");
    assert_eq!(database.title, "Which database?");
    assert_eq!(
        database.selection_marker,
        Some(SelectUiRequestSelectionMarker::Radio)
    );
    assert_eq!(
        features.iter().map(checked_options).collect::<Vec<_>>(),
        [
            vec![],
            vec!["Auth".to_owned()],
            vec!["Auth".to_owned(), "Search".to_owned()]
        ],
        "{features:#?}"
    );
    for select in features {
        assert!(
            select.title.ends_with("Which features?"),
            "{}",
            select.title
        );
        assert_eq!(
            select.selection_marker,
            Some(SelectUiRequestSelectionMarker::Checkbox)
        );
        // Every option but "Done selecting" gets a marker, "Other" included.
        assert_eq!(select.markable_count, Some(4), "{:?}", select.options);
    }
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
