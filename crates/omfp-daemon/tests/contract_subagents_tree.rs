//! M8 acceptance (07-roadmap), L3: the real daemon with the pinned omp and the fake LLM — a `task` tool spawning a
//! subagent (its roster and its transcript read incrementally), and the session tree (navigate with `editorText`,
//! label, fork from an entry) plus the goal, driven by a native protocol client.

mod client;
mod common;

use clap::Parser;
use client::{Client, Follower};
use common::Harness;
use fake_llm::{Matcher, Reply, Step, ToolCall};
use omfp_core::ThreadState;
use omfp_core::wire::{AgentMessage, SubagentLifecycleStatus};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_protocol::method::ThreadsCreate;
use omfp_protocol::{CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use omfp_testkit::{FAKE_MODEL, FAKE_PROVIDER, omp_bin};
use serde_json::{Value, json};
use tempfile::TempDir;

async fn start(harness: &Harness) -> (DaemonHandle, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let omp = omp_bin();
    let workspaces = data.path().join("workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.path().to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--omp-bin",
        &omp,
        // The fake LLM scripts every model call; a title run would take a step (DMN-073).
        "--no-title-generation",
    ])
    .unwrap();
    let config = Config::from_cli(cli, None, None).unwrap();
    let mut ports = Ports::resolve(&config).unwrap();
    ports.omp_env = harness.env.prepare_omfp_profile(data.path()).unwrap();
    (start_with(config, ports).await.unwrap(), data)
}

/// A thread on the fake model, followed from its snapshot.
async fn thread(client: &mut Client) -> (String, Follower) {
    let thread = client
        .typed::<ThreadsCreate>(CreateThreadParams {
            title: Some("m8".into()),
            workspace: ThreadWorkspace::New(CreateWorkspaceParams::default()),
            model: Some(format!("{FAKE_PROVIDER}/{FAKE_MODEL}")),
            thinking_level: None,
            approval_mode: None,
            prompt: None,
            images: Vec::new(),
        })
        .await
        .id;
    let view = Follower::snapshot(client, &thread, None).await;
    (thread, view)
}

/// Sends `text` and follows the thread until its turn settled with `messages` messages in the transcript.
async fn turn(client: &mut Client, view: &mut Follower, thread: &str, text: &str, messages: usize) {
    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": text, "images": [] }),
        )
        .await;
    view.until(client, |state| {
        state.settled && state.transcript.len() >= messages
    })
    .await;
}

/// The text of a transcript message (user or assistant).
fn text(message: &AgentMessage) -> String {
    match message {
        AgentMessage::User(user) => serde_json::to_value(&user.content)
            .ok()
            .and_then(|content| match content {
                Value::String(text) => Some(text),
                Value::Array(blocks) => Some(
                    blocks
                        .iter()
                        .filter_map(|block| block["text"].as_str())
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default(),
        AgentMessage::Assistant(assistant) => serde_json::to_value(&assistant.content)
            .unwrap_or_default()
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| block["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

fn texts(state: &ThreadState) -> Vec<String> {
    state
        .transcript
        .iter()
        .map(|entry| text(&entry.message))
        .collect()
}

/// Every `message` entry of a `get_tree` forest, depth first: (entry id, role, text).
fn message_entries(nodes: &Value, out: &mut Vec<(String, String, String)>) {
    for node in nodes.as_array().into_iter().flatten() {
        let entry = &node["entry"];
        if entry["type"] == "message" {
            let message: AgentMessage = serde_json::from_value(entry["message"].clone()).unwrap();
            out.push((
                entry["id"].as_str().unwrap().to_owned(),
                entry["message"]["role"].as_str().unwrap().to_owned(),
                text(&message),
            ));
        }
        message_entries(&node["children"], out);
    }
}

/// The id of the tree entry holding the message with `text`.
fn entry_with(tree: &Value, text: &str) -> String {
    let mut entries = Vec::new();
    message_entries(&tree["tree"], &mut entries);
    entries
        .into_iter()
        .find(|(_, _, found)| found == text)
        .unwrap_or_else(|| panic!("no entry with {text:?} in {tree:#}"))
        .0
}

/// The label of entry `id` in a `get_tree` forest.
fn label_of(nodes: &Value, id: &str) -> Option<String> {
    nodes.as_array().into_iter().flatten().find_map(|node| {
        if node["entry"]["id"] == id {
            node["label"].as_str().map(str::to_owned)
        } else {
            label_of(&node["children"], id)
        }
    })
}

// spec: TST-060, PRT-055, UI-081
#[tokio::test(flavor = "multi_thread")]
async fn a_task_subagent_is_listed_and_its_transcript_reads_incrementally() {
    let assignment = "Name the colour of a clear sky";
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_task_1",
            "task",
            json!({
                "context": "Colours.",
                "tasks": [{ "name": "Sky", "task": assignment, "solutionSpace": "one word" }],
                "i": "Delegating the colour"
            }),
        )])),
        // The subagent: its final answer, held so it is seen running.
        Step::when(
            Matcher::LastUserContains(assignment.into()),
            Reply::text("SKY-IS-BLUE"),
        )
        .gated(),
        Step::reply(Reply::text("The sky is blue.")),
    ])
    .await;
    // A blocking task: the parent's turn ends after the subagent's.
    harness
        .env
        .configure(&[], "async:\n  enabled: false\n")
        .unwrap();
    let (daemon, _data) = start(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let (thread, mut view) = thread(&mut client).await;

    client
        .ok(
            "thread.prompt",
            json!({ "threadId": thread, "text": "what colour is the sky?", "images": [] }),
        )
        .await;
    harness.fake.wait_for_gate(1).await;
    view.until(&mut client, |state| !state.subagents.is_empty())
        .await;
    let id = view.state.subagents.keys().next().unwrap().clone();

    // While it runs, omp's roster (`thread.subagents`) lists it; omp drops finished ones from it.
    let roster = client
        .ok("thread.subagents", json!({ "threadId": thread }))
        .await;
    let listed: Vec<&str> = roster["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|subagent| subagent["id"].as_str())
        .collect();
    assert_eq!(listed, [id.as_str()], "{roster:#}");
    harness.fake.release_gate(1);
    // user, assistant (task call), tool result, assistant.
    view.until(&mut client, |state| {
        state.settled && state.transcript.len() >= 4
    })
    .await;

    // The reducer kept the finished subagent from omp's lifecycle and progress frames (PRT-055).
    let subagents = &view.state.subagents;
    assert_eq!(subagents.len(), 1, "{subagents:#?}");
    let subagent = &subagents[&id];
    let lifecycle = subagent.lifecycle.as_ref().unwrap();
    assert_eq!(lifecycle.status, SubagentLifecycleStatus::Completed);
    assert_eq!(
        lifecycle.parent_tool_call_id.as_deref(),
        Some("call_task_1")
    );
    let progress = subagent.progress.as_ref().unwrap();
    assert!(progress.task.contains(assignment), "{progress:#?}");
    let roster = client
        .ok("thread.subagents", json!({ "threadId": thread }))
        .await;
    assert_eq!(roster["subagents"], json!([]), "{roster:#}");

    // Its transcript, then nothing new from where the first read stopped (UI-081).
    let first = client
        .ok(
            "thread.subagent_messages",
            json!({ "threadId": thread, "subagentId": id, "fromByte": 0 }),
        )
        .await;
    let messages: Vec<AgentMessage> = serde_json::from_value(first["messages"].clone()).unwrap();
    let said: Vec<String> = messages.iter().map(text).collect();
    assert!(
        said.iter().any(|text| text.contains(assignment)),
        "{said:?}"
    );
    assert_eq!(said.last().map(String::as_str), Some("SKY-IS-BLUE"));
    let next = first["nextByte"].as_i64().unwrap();
    assert!(next > 0, "{first:#}");
    let again = client
        .ok(
            "thread.subagent_messages",
            json!({ "threadId": thread, "subagentId": id, "fromByte": next }),
        )
        .await;
    assert_eq!(again["messages"], json!([]), "{again:#}");
    assert_eq!(again["nextByte"].as_i64(), Some(next));
    assert_eq!(again["reset"], false);

    harness.assert_no_stray_network();
    daemon.shutdown().await.unwrap();
}

// spec: TST-060, DMN-072, DMN-111, PRT-056, UI-083
#[tokio::test(flavor = "multi_thread")]
async fn navigate_restores_the_editor_text_label_marks_an_entry_and_fork_starts_a_session() {
    let harness = Harness::new(vec![
        Step::reply(Reply::text("first answer")),
        Step::reply(Reply::text("second answer")),
    ])
    .await;
    let (daemon, _data) = start(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let (thread, mut view) = thread(&mut client).await;
    turn(&mut client, &mut view, &thread, "first question", 2).await;
    turn(&mut client, &mut view, &thread, "second question", 4).await;

    let tree = client
        .ok("thread.tree", json!({ "threadId": thread }))
        .await;
    let first_answer = entry_with(&tree, "first answer");
    let second_question = entry_with(&tree, "second question");
    assert_eq!(tree["leafId"], json!(entry_with(&tree, "second answer")));

    // Navigating to a user message gives its text back for the composer and moves the leaf to its parent; the
    // daemon reloads the branch as a `history` event (DMN-111).
    let navigated = client
        .ok(
            "thread.navigate",
            json!({ "threadId": thread, "entryId": second_question }),
        )
        .await;
    assert_eq!(navigated["cancelled"], false, "{navigated:#}");
    assert_eq!(navigated["editorText"], "second question");
    view.until(&mut client, |state| state.transcript.len() == 2)
        .await;
    assert_eq!(texts(&view.state), ["first question", "first answer"]);
    let moved = client
        .ok("thread.tree", json!({ "threadId": thread }))
        .await;
    assert_eq!(moved["leafId"], json!(first_answer));

    // A label shows on its entry in the tree; an absent label clears it.
    client
        .ok(
            "thread.label",
            json!({ "threadId": thread, "entryId": first_answer, "label": "checkpoint" }),
        )
        .await;
    let labelled = client
        .ok("thread.tree", json!({ "threadId": thread }))
        .await;
    assert_eq!(
        label_of(&labelled["tree"], &first_answer).as_deref(),
        Some("checkpoint")
    );
    client
        .ok(
            "thread.label",
            json!({ "threadId": thread, "entryId": first_answer }),
        )
        .await;
    let cleared = client
        .ok("thread.tree", json!({ "threadId": thread }))
        .await;
    assert_eq!(label_of(&cleared["tree"], &first_answer), None);

    // Forking from an entry starts a new session with the branch up to it; the thread follows it (DMN-072).
    let before = view.state.session.clone().unwrap().session_id;
    let forked = client
        .ok(
            "thread.fork",
            json!({ "threadId": thread, "entryId": first_answer }),
        )
        .await;
    assert_eq!(forked["cancelled"], false, "{forked:#}");
    view.until(&mut client, |state| {
        state
            .session
            .as_ref()
            .is_some_and(|session| session.session_id != before)
    })
    .await;
    assert_eq!(texts(&view.state), ["first question", "first answer"]);
    let state = client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    assert_eq!(
        Some(state["sessionId"].as_str().unwrap()),
        view.state
            .session
            .as_ref()
            .map(|session| session.session_id.as_str())
    );

    harness.assert_no_stray_network();
    daemon.shutdown().await.unwrap();
}

// spec: TST-060, PRT-056
#[tokio::test(flavor = "multi_thread")]
async fn a_goal_reaches_the_thread_state() {
    let harness = Harness::new(vec![Step::reply(Reply::text("ok"))]).await;
    let (daemon, _data) = start(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let (thread, mut view) = thread(&mut client).await;
    turn(&mut client, &mut view, &thread, "hello", 2).await;

    let created = client
        .ok(
            "thread.goal",
            json!({ "threadId": thread, "op": "create", "objective": "Ship the tree widget" }),
        )
        .await;
    assert_eq!(
        created["goal"]["objective"], "Ship the tree widget",
        "{created:#}"
    );
    view.until(&mut client, |state| state.goal.is_some()).await;
    let goal = view.state.goal.clone().unwrap();
    assert_eq!(
        (goal.objective.as_str(), goal.status.as_str()),
        ("Ship the tree widget", "active")
    );

    client
        .ok("thread.goal", json!({ "threadId": thread, "op": "drop" }))
        .await;
    view.until(&mut client, |state| {
        state
            .goal
            .as_ref()
            .is_none_or(|goal| goal.status.as_str() == "dropped")
    })
    .await;

    harness.assert_no_stray_network();
    daemon.shutdown().await.unwrap();
}
