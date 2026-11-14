//! `threads.fork` against the pinned omp and the fake LLM (L3, DMN-300..DMN-306): a fork at a user message is a new
//! thread whose omp resumes the conversation up to that message, goes on by itself and keeps its own session across a
//! restart, while the source thread, its omp and its session files stay as they were.

mod client;
mod common;

use std::path::{Path, PathBuf};

use clap::Parser;
use client::{Client, Follower};
use common::Harness;
use fake_llm::{Reply, Step};
use omfp_core::ThreadState;
use omfp_core::wire::AgentMessage;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::{DaemonHandle, Ports, start_with};
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
    ports.omp_env = harness.env.env_overrides();
    (start_with(config, ports).await.unwrap(), data)
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

/// The text of a transcript message (user or assistant), `""` for others.
fn text(message: &AgentMessage) -> String {
    let content = match message {
        AgentMessage::User(user) => serde_json::to_value(&user.content).unwrap_or_default(),
        AgentMessage::Assistant(assistant) => {
            serde_json::to_value(&assistant.content).unwrap_or_default()
        }
        _ => Value::Null,
    };
    match content {
        Value::String(text) => text,
        Value::Array(blocks) => blocks
            .iter()
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

/// The `message` entry of a `get_tree` forest whose message has `text`.
fn entry_with(nodes: &Value, wanted: &str) -> Option<Value> {
    nodes.as_array().into_iter().flatten().find_map(|node| {
        let entry = &node["entry"];
        let found = entry["type"] == "message"
            && serde_json::from_value::<AgentMessage>(entry["message"].clone())
                .is_ok_and(|message| text(&message) == wanted);
        if found {
            Some(entry.clone())
        } else {
            entry_with(&node["children"], wanted)
        }
    })
}

/// The `.jsonl` files of a thread's session directory, sorted, with their contents.
fn session_files(data: &Path, thread: &str) -> Vec<(PathBuf, String)> {
    let dir = data.join("threads").join(thread).join("sessions");
    let mut files: Vec<(PathBuf, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            (path, text)
        })
        .collect();
    files.sort();
    files
}

fn session_id(state: &ThreadState) -> String {
    state.session.as_ref().unwrap().session_id.clone()
}

// spec: TST-060, DMN-300, DMN-301, DMN-302, DMN-306, PRT-180, PRT-181, UI-661
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_at_a_user_message_resumes_up_to_it_in_a_new_thread_and_the_source_is_unchanged() {
    let harness = Harness::new(vec![
        Step::reply(Reply::text("first answer")),
        Step::reply(Reply::text("second answer")),
        Step::reply(Reply::text("fork answer")),
    ])
    .await;
    let (daemon, data) = start(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let (source, view) = conversation(&mut client).await;

    // The UI finds a transcript message's entry by role and timestamp: omp keeps both in the tree (UI-661).
    let tree = client
        .ok("thread.tree", json!({ "threadId": source }))
        .await;
    let question = entry_with(&tree["tree"], "second question").expect("the entry");
    let AgentMessage::User(user) = &view.state.transcript[2].message else {
        panic!("{:?}", view.state.transcript[2]);
    };
    assert_eq!(
        question["message"]["timestamp"].as_f64(),
        user.timestamp,
        "{question:#}"
    );
    let source_files = session_files(data.path(), &source);
    let source_session = session_id(&view.state);

    let fork = client
        .ok(
            "threads.fork",
            json!({ "threadId": source, "entryId": question["id"] }),
        )
        .await;
    let forked = fork["id"].as_str().unwrap().to_owned();
    assert_eq!(
        fork["forkedFrom"],
        json!({ "threadId": source, "title": "m17", "entryId": question["id"], "message": "second question" })
    );

    // The fork resumes the conversation up to the message, in one session file of its own (the full copy is gone).
    let mut fork_view = Follower::snapshot(&mut client, &forked, None).await;
    assert_eq!(
        texts(&fork_view.state),
        ["first question", "first answer", "second question"]
    );
    let fork_session = session_id(&fork_view.state);
    assert_ne!(fork_session, source_session);
    let fork_files = session_files(data.path(), &forked);
    assert_eq!(fork_files.len(), 1, "{fork_files:#?}");
    assert!(!fork_files[0].1.contains("second answer"));

    goes_on_and_restarts(&mut client, &mut fork_view, &forked).await;

    // The source: same session, same transcript, session files untouched.
    let source_view = Follower::snapshot(&mut client, &source, None).await;
    assert_eq!(session_id(&source_view.state), source_session);
    assert_eq!(
        texts(&source_view.state),
        [
            "first question",
            "first answer",
            "second question",
            "second answer"
        ]
    );
    assert_eq!(session_files(data.path(), &source), source_files);

    // Without an entry the whole session is copied.
    let whole = client
        .ok("threads.fork", json!({ "threadId": source }))
        .await;
    let whole_view = Follower::snapshot(&mut client, whole["id"].as_str().unwrap(), None).await;
    assert_eq!(texts(&whole_view.state), texts(&source_view.state));

    harness.assert_no_stray_network();
    daemon.shutdown().await.unwrap();
}

/// Thread "m17" on the fake model after two turns, followed.
async fn conversation(client: &mut Client) -> (String, Follower) {
    let source = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "m17", "model": format!("{FAKE_PROVIDER}/{FAKE_MODEL}") }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut view = Follower::snapshot(client, &source, None).await;
    turn(client, &mut view, &source, "first question", 2).await;
    turn(client, &mut view, &source, "second question", 4).await;
    (source, view)
}

/// The fork goes on by itself and keeps its session across a restart (`--continue`, DMN-301).
async fn goes_on_and_restarts(client: &mut Client, view: &mut Follower, forked: &str) {
    let session = session_id(&view.state);
    turn(client, view, forked, "fork question", 5).await;
    client
        .ok("thread.restart", json!({ "threadId": forked }))
        .await;
    let restarted = client
        .ok("thread.state", json!({ "threadId": forked }))
        .await;
    assert_eq!(restarted["sessionId"], json!(session));
    let after = Follower::snapshot(client, forked, None).await;
    assert_eq!(
        texts(&after.state),
        [
            "first question",
            "first answer",
            "second question",
            "fork question",
            "fork answer"
        ]
    );
}
