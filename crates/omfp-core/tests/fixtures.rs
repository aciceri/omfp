//! Replays of the recorded thread fixtures (real omp output) through the reducer.

mod common;

use common::{FIXTURES, fixture, index_of, message_text, state_through, streaming_text};
use omfp_core::wire::{
    AgentMessage, AssistantContent, ExtensionUiRequest, PromptStatus, StopReason,
};
use omfp_core::{JobState, ProcessState, QueueState, ThreadEvent, ThreadState, ToolStatus, replay};
use proptest::prelude::*;
use serde_json::Value;

fn last_message_text(state: &ThreadState) -> String {
    state
        .transcript
        .last()
        .map(|entry| message_text(&entry.message))
        .unwrap_or_default()
}

fn result_text(value: Option<&Value>) -> String {
    value.map(Value::to_string).unwrap_or_default()
}

// spec: ARC-006, TST-012, DMN-043
#[test]
fn text_reply_ends_with_the_assistant_text_settled() {
    let events = fixture("text_reply");
    let state = replay(&events);

    assert_eq!(state.process, ProcessState::Running);
    assert!(state.settled);
    assert_eq!(state.streaming, None);
    let texts: Vec<String> = state
        .transcript
        .iter()
        .map(|entry| message_text(&entry.message))
        .collect();
    assert_eq!(texts, ["say hi", "Hello from the fake model"]);
    let ids: Vec<Option<&str>> = state
        .transcript
        .iter()
        .map(|entry| entry.message_id.as_deref())
        .collect();
    assert_eq!(ids, [Some("msg-1"), Some("msg-2")]);
    let session = state.session.expect("session from history");
    assert_eq!(session.session_id, "<uuid>");
    assert!(!session.resumed);
    assert_eq!(state.prompts["req_7"].status, PromptStatus::Completed);
    assert!(
        state
            .commands
            .iter()
            .any(|command| command.name == "security")
    );
    assert_eq!(
        state.presentation.widgets.len(),
        0,
        "setWidget without lines clears the widget"
    );
}

// spec: ARC-006, TST-012
#[test]
fn text_reply_streams_text_deltas() {
    let events = fixture("text_reply");

    let started = state_through(&events, index_of(&events, "agent_start", 0));
    assert!(!started.settled);

    let message_started = state_through(&events, index_of(&events, "message_start", 1));
    assert_eq!(
        message_started
            .streaming
            .as_ref()
            .and_then(|s| s.message_id.as_deref()),
        Some("msg-2")
    );

    let text_started = state_through(&events, index_of(&events, "text_start", 0));
    assert_eq!(
        streaming_text(&text_started).as_deref(),
        Some(""),
        "text_start empties the block"
    );

    let after_deltas = state_through(&events, index_of(&events, "text_delta", 2));
    assert_eq!(
        streaming_text(&after_deltas).as_deref(),
        Some("Hello from t")
    );
    assert!(!after_deltas.settled);

    let text_ended = state_through(&events, index_of(&events, "text_end", 0));
    assert_eq!(
        streaming_text(&text_ended).as_deref(),
        Some("Hello from the fake model")
    );

    let ended = state_through(&events, index_of(&events, "message_end", 1));
    assert_eq!(ended.streaming, None);
    assert_eq!(last_message_text(&ended), "Hello from the fake model");
    assert!(!ended.settled, "settled only by session_settled");
}

// spec: ARC-006, TST-012
#[test]
fn bash_tool_call_runs_one_tool_to_success() {
    let events = fixture("bash_tool_call");
    let state = replay(&events);

    assert_eq!(state.tools.len(), 1);
    let run = &state.tools[0];
    assert_eq!(run.tool_call_id, "call_bash_1");
    assert_eq!(run.tool_name, "bash");
    assert_eq!(run.status, ToolStatus::Succeeded);
    assert_eq!(
        run.args
            .as_ref()
            .and_then(|args| args.get("command"))
            .and_then(Value::as_str),
        Some("echo tool-output-123")
    );
    assert!(result_text(run.result.as_ref()).contains("tool-output-123"));
    assert_eq!(
        last_message_text(&state),
        "The command printed tool-output-123."
    );
    let roles: Vec<&str> = state
        .transcript
        .iter()
        .map(|entry| match entry.message {
            AgentMessage::User(_) => "user",
            AgentMessage::Assistant(_) => "assistant",
            AgentMessage::ToolResult(_) => "toolResult",
            _ => "other",
        })
        .collect();
    assert_eq!(roles, ["user", "assistant", "toolResult", "assistant"]);
    assert!(state.settled);
}

// spec: PRT-100, TST-012
#[test]
fn async_bash_tool_call_keeps_the_job_after_the_call_ends() {
    let events = fixture("async_bash_tool_call");
    let ended = state_through(&events, index_of(&events, "tool_execution_end", 0));
    let run = &ended.tools[0];
    assert_eq!(run.status, ToolStatus::Succeeded);
    let job = run.background.as_ref().expect("a background job");
    assert_eq!(
        (job.job_id.as_str(), job.state),
        ("bg_1", JobState::Running)
    );

    let state = replay(&events);
    let job = state.tools[0]
        .background
        .as_ref()
        .expect("a background job");
    assert_eq!(job.state, JobState::Completed);
    assert!(job.output.as_deref().unwrap_or("").contains("bg-output-42"));
    assert_eq!(last_message_text(&state), "The job printed bg-output-42.");
}

// spec: ARC-006, TST-012
#[test]
fn bash_tool_call_streams_arguments_then_runs() {
    let events = fixture("bash_tool_call");

    let deltas_done = state_through(&events, index_of(&events, "toolcall_delta", 5));
    let streaming = deltas_done.streaming.expect("tool call streaming");
    let Some(AssistantContent::ToolCall(call)) = streaming
        .message
        .content
        .as_ref()
        .and_then(|content| content.first())
    else {
        panic!("tool call block expected: {:?}", streaming.message.content);
    };
    assert_eq!(call.id.as_deref(), Some("call_bash_1"));
    assert_eq!(
        call.extra.get("partialArgs").and_then(Value::as_str),
        Some(r#"{"command":"echo tool-output-123"}"#)
    );

    let call_ended = state_through(&events, index_of(&events, "toolcall_end", 0));
    let Some(AssistantContent::ToolCall(call)) = call_ended
        .streaming
        .as_ref()
        .and_then(|s| s.message.content.as_ref())
        .and_then(|content| content.first())
    else {
        panic!("tool call block expected");
    };
    assert_eq!(
        call.arguments
            .as_ref()
            .and_then(|args| args.get("command"))
            .and_then(Value::as_str),
        Some("echo tool-output-123")
    );

    let running = state_through(&events, index_of(&events, "tool_execution_start", 0));
    assert_eq!(running.tools[0].status, ToolStatus::Running);
    assert_eq!(running.tools[0].partial_result, None);

    let updated = state_through(&events, index_of(&events, "tool_execution_update", 0));
    assert!(result_text(updated.tools[0].partial_result.as_ref()).contains("tool-output-123"));
    assert_eq!(updated.tools[0].status, ToolStatus::Running);
}

// spec: ARC-006, TST-012, DMN-050, DMN-051
#[test]
fn approval_granted_shows_then_resolves_the_select() {
    let events = fixture("approval_granted");

    let asked = state_through(&events, index_of(&events, "extension_ui_request", 1));
    assert_eq!(asked.interactions.len(), 1);
    let ExtensionUiRequest::Select(select) = &asked.interactions[0] else {
        panic!("select expected: {:?}", asked.interactions);
    };
    assert!(select.title.starts_with("Allow tool: bash"));
    assert_eq!(
        select.options,
        ["Approve", "Approve bash for this session", "Deny"]
    );

    let resolved = state_through(&events, index_of(&events, "interaction_resolved", 0));
    assert_eq!(resolved.interactions, Vec::new());

    let state = replay(&events);
    assert_eq!(state.interactions, Vec::new());
    assert_eq!(state.tools[0].status, ToolStatus::Succeeded);
    assert_eq!(last_message_text(&state), "Done after approval.");
}

// spec: ARC-006, TST-012, DMN-050, DMN-051, DMN-055
#[test]
fn ask_answered_shows_the_whole_form_once() {
    let events = fixture("ask_answered");

    let pending = (0..events.len())
        .map(|index| state_through(&events, index))
        .find(|state| !state.interactions.is_empty())
        .expect("a pending form");
    let forms: Vec<bool> = pending
        .interactions
        .iter()
        .map(|request| match request {
            ExtensionUiRequest::Form(form) => {
                form.html.contains("Which database?") && form.html.contains("Which features?")
            }
            _ => false,
        })
        .collect();
    assert_eq!(forms, [true], "one request with every field");

    let most_pending = (0..events.len())
        .map(|index| state_through(&events, index).interactions.len())
        .max();
    assert_eq!(most_pending, Some(1));

    let state = replay(&events);
    assert_eq!(state.interactions, Vec::new());
    let run = state
        .tools
        .iter()
        .find(|run| run.tool_call_id == "call_ask_1")
        .expect("ask tool run");
    assert_eq!(run.status, ToolStatus::Succeeded);
    assert_eq!(
        last_message_text(&state),
        "Using Postgres with Auth and Elasticsearch."
    );
}

// spec: ARC-006, TST-012, TST-011
#[test]
fn follow_up_then_abort_tracks_the_queue() {
    let events = fixture("follow_up_then_abort");

    let queued = state_through(&events, index_of(&events, "queue_update", 0));
    assert_eq!(queued.queue.follow_up, ["later please"]);
    assert_eq!(queued.queue.steering, Vec::<String>::new());

    let state = replay(&events);
    assert_eq!(state.queue, QueueState::default());
    let Some(AgentMessage::Assistant(last)) = state.transcript.last().map(|entry| &entry.message)
    else {
        panic!("assistant message expected last");
    };
    assert_eq!(last.stop_reason, Some(StopReason::Aborted));
    assert_eq!(state.prompts["req_7"].status, PromptStatus::Aborted);
    assert_eq!(state.streaming, None);
    assert!(state.settled);
}

// spec: ARC-006, TST-012, DMN-032, DMN-043
#[test]
fn restart_with_history_keeps_history_then_new_messages() {
    let events = fixture("restart_with_history");
    let state = replay(&events);

    let texts: Vec<String> = state
        .transcript
        .iter()
        .map(|entry| message_text(&entry.message))
        .collect();
    assert_eq!(
        texts,
        [
            "first question",
            "first answer",
            "second question",
            "second answer"
        ]
    );
    let ids: Vec<Option<&str>> = state
        .transcript
        .iter()
        .map(|entry| entry.message_id.as_deref())
        .collect();
    assert_eq!(ids, [None, None, Some("msg-1"), Some("msg-2")]);
    assert!(state.session.is_some_and(|session| session.resumed));
}

// spec: PRT-021, TST-011
#[test]
fn snapshot_round_trip_then_remaining_events_equals_full_replay() {
    for name in FIXTURES {
        let events = fixture(name);
        let expected = replay(&events);
        for split in 0..=events.len() {
            let snapshot = replay(&events[..split]);
            let json = serde_json::to_string(&snapshot).expect("serializable state");
            let mut restored: ThreadState =
                serde_json::from_str(&json).expect("deserializable state");
            assert_eq!(restored, snapshot, "{name}: round trip at {split}");
            for event in &events[split..] {
                restored.apply(event);
            }
            assert_eq!(restored, expected, "{name}: split at {split}");
        }
    }
}

fn all_fixture_events() -> Vec<ThreadEvent> {
    FIXTURES.iter().flat_map(|name| fixture(name)).collect()
}

proptest! {
    // spec: ARC-006, TST-010
    #[test]
    fn any_order_of_recorded_events_is_total(indices in prop::collection::vec(any::<prop::sample::Index>(), 0..200)) {
        let pool = all_fixture_events();
        let mut state = ThreadState::default();
        for index in indices {
            state.apply(index.get(&pool));
        }
        let json = serde_json::to_string(&state).expect("serializable state");
        let restored: ThreadState = serde_json::from_str(&json).expect("deserializable state");
        prop_assert_eq!(restored, state);
    }
}
