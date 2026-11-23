//! M1 and M2b acceptance (07-roadmap), L3: the real daemon (`start_with` on the real ports) with the pinned omp, the
//! fake LLM and temporary git repositories, driven by a native protocol client.

mod client;
mod common;

use std::net::SocketAddr;
use std::path::PathBuf;

use client::{Client, Follower};
use common::{Harness, omp_daemon, system_prompt, workspace_roots};
use fake_llm::{Reply, Step, ToolCall};
use omfp_core::wire::{AgentMessage, ExtensionUiRequest, FormUiRequest, SelectUiRequest};
use omfp_core::{ProcessState, ThreadState};
use omfp_daemon::DaemonHandle;
use omfp_testkit::{FAKE_MODEL, FAKE_PROVIDER, TempRepo};
use serde_json::{Value, json};
use tempfile::TempDir;

struct Daemon {
    handle: DaemonHandle,
    data: TempDir,
}

impl Daemon {
    /// A daemon without title generation (DMN-073).
    async fn start(harness: &Harness) -> Self {
        let (handle, data) = omp_daemon(harness, &["--no-title-generation"]).await;
        Self { handle, data }
    }

    /// A daemon that generates titles as `omfp daemon` does.
    async fn start_titling(harness: &Harness) -> Self {
        let (handle, data) = omp_daemon(harness, &[]).await;
        Self { handle, data }
    }

    fn addr(&self) -> SocketAddr {
        self.handle.addr()
    }

    async fn client(&self, name: &str) -> Client {
        Client::hello(self.addr(), self.handle.token().expose(), name).await
    }

    fn omp_log(&self, thread: &str) -> String {
        std::fs::read_to_string(
            self.data
                .path()
                .join("threads")
                .join(thread)
                .join("omp.log"),
        )
        .unwrap_or_default()
    }

    /// Kills the omp process group of `thread` (found by its session directory on the command line).
    fn kill_omp(&self, thread: &str) {
        let sessions = self
            .data
            .path()
            .join("threads")
            .join(thread)
            .join("sessions");
        let needle = sessions.to_string_lossy().into_owned();
        let pid = std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .find(|pid| {
                std::fs::read(format!("/proc/{pid}/cmdline"))
                    .is_ok_and(|cmdline| String::from_utf8_lossy(&cmdline).contains(&needle))
            })
            .expect("the thread's omp process");
        let status = std::process::Command::new("sh")
            .args(["-c", &format!("kill -KILL -- -{pid}")])
            .status()
            .unwrap();
        assert!(status.success());
    }
}

fn model() -> String {
    format!("{FAKE_PROVIDER}/{FAKE_MODEL}")
}

fn repo_with_marker(marker: &str) -> TempRepo {
    let repo = TempRepo::new().unwrap();
    repo.commit_file(
        "AGENTS.md",
        &format!("Marker: {marker}\n"),
        "add agents file",
    )
    .unwrap();
    repo
}

/// Adds a repository to the workspace; returns the workspace and the new repository's path in it.
async fn add_repo(client: &mut Client, workspace: &str, source: Value) -> PathBuf {
    let before = client
        .ok("workspaces.rescan", json!({ "workspaceId": workspace }))
        .await["repos"]
        .as_array()
        .unwrap()
        .len();
    let updated = client
        .ok(
            "workspaces.add_repo",
            json!({ "workspaceId": workspace, "source": source }),
        )
        .await;
    let repos = updated["repos"].as_array().unwrap();
    assert_eq!(repos.len(), before + 1, "{updated}");
    let added = repos
        .iter()
        .find(|repo| repo["kind"] == source["kind"])
        .unwrap();
    PathBuf::from(added["path"].as_str().unwrap())
}

fn assistant_texts(state: &ThreadState) -> Vec<String> {
    state
        .transcript
        .iter()
        .filter(|entry| matches!(entry.message, AgentMessage::Assistant(_)))
        .map(|entry| serde_json::to_string(&entry.message).unwrap())
        .collect()
}

fn settled_after(prompt_id: &str) -> impl Fn(&ThreadState) -> bool + '_ {
    move |state| state.settled && state.prompts.contains_key(prompt_id)
}

async fn prompt(client: &mut Client, thread: &str, text: &str) -> String {
    let result = client
        .ok("thread.prompt", json!({ "threadId": thread, "text": text }))
        .await;
    assert_eq!(result["agentInvoked"], true);
    result["promptId"].as_str().unwrap().to_owned()
}

const USER_APPEND: &str = "User append marker: quince-5";

// spec: ARC-008, DMN-029, DMN-030, DMN-024, DMN-025, PRT-030, ARC-001, DMN-021, DMN-076, AGT-024
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One acceptance journey, step by step.
async fn a_thread_in_a_workspace_folder_sees_its_repositories_the_conventions_and_the_users_append_file()
 {
    let harness = Harness::new(vec![
        Step::reply(Reply::text("Hello from both repositories.")),
        Step::reply(Reply::text("Now with the worktree.")),
        Step::reply(Reply::text("Back to one.")),
    ])
    .await;
    std::fs::write(
        harness.env.agent_dir().join("APPEND_SYSTEM.md"),
        format!("{USER_APPEND}\n"),
    )
    .unwrap();
    let frontend = repo_with_marker("kumquat-17");
    let backend = repo_with_marker("rutabaga-42");
    let daemon = Daemon::start(&harness).await;
    let mut client = daemon.client("acceptance").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "Full stack", "model": model() }),
        )
        .await;
    let (thread, workspace) = (
        thread["id"].as_str().unwrap().to_owned(),
        thread["workspaceId"].as_str().unwrap().to_owned(),
    );
    let link = add_repo(
        &mut client,
        &workspace,
        json!({ "kind": "link", "path": frontend.path() }),
    )
    .await;
    let folder = link.parent().unwrap().to_owned();
    let mut view = Follower::snapshot(&mut client, &thread, None).await;

    let first = prompt(&mut client, &thread, "hello").await;
    view.until(&mut client, settled_after(&first)).await;

    let system = system_prompt(&harness.fake.requests()[0]);
    assert!(
        system.contains("kumquat-17"),
        "the link's AGENTS.md: {system}"
    );
    assert_eq!(workspace_roots(&system), [link.display().to_string()]);
    let daemon_text =
        omfp_daemon::conventions::daemon_text("Full stack", &workspace, &folder, &thread);
    assert!(
        system.ends_with(&format!("{daemon_text}\n\n{USER_APPEND}")),
        "the daemon's conventions, widgets and control notes, then the user's APPEND_SYSTEM.md, end the prompt: {}",
        &system[system.len().saturating_sub(2400)..]
    );

    let worktree = add_repo(
        &mut client,
        &workspace,
        json!({ "kind": "worktree", "repo": backend.path() }),
    )
    .await;
    let second = prompt(&mut client, &thread, "and now?").await;
    view.until(&mut client, settled_after(&second)).await;
    let system = system_prompt(&harness.fake.requests()[1]);
    assert!(
        system.contains("rutabaga-42"),
        "the live /add-dir root: {system}"
    );
    assert_eq!(
        workspace_roots(&system),
        [link.display().to_string(), worktree.display().to_string()]
    );

    client
        .ok(
            "workspaces.remove_repo",
            json!({ "workspaceId": workspace, "name": link.file_name().unwrap().to_str() }),
        )
        .await;
    let third = prompt(&mut client, &thread, "one left?").await;
    view.until(&mut client, settled_after(&third)).await;
    let system = system_prompt(&harness.fake.requests()[2]);
    assert_eq!(
        workspace_roots(&system),
        [worktree.display().to_string()],
        "/remove-dir detached the link from the session"
    );
    assert!(!system.contains("kumquat-17"), "{system}");
    assert!(
        frontend.path().join("AGENTS.md").is_file(),
        "the target stays"
    );
    assert_eq!(assistant_texts(&view.state).len(), 3);
    // The conversation is searchable in omp's own session files, user and assistant text alike (DMN-076).
    for (query, snippet) in [
        ("FROM BOTH", "Hello from both repositories."),
        ("and now", "and now?"),
    ] {
        let found = client.ok("threads.search", json!({ "query": query })).await;
        assert_eq!(
            found,
            json!({ "results": [{ "threadId": thread, "snippet": snippet }] })
        );
    }
    // Both repositories are suggested, the link by its target and the worktree by its repository (DMN-021).
    let suggested = client.ok("repos.suggest", json!({})).await;
    let mut paths: Vec<&str> = suggested["repos"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|repo| repo["path"].as_str())
        .collect();
    paths.sort_unstable();
    let mut expected = [
        frontend.path().canonicalize().unwrap(),
        backend.path().canonicalize().unwrap(),
    ]
    .map(|path| path.display().to_string());
    expected.sort_unstable();
    assert_eq!(paths, expected);
    // The daemon's state equals what the client reduced from the stream (PRT-021).
    let latest = Follower::snapshot(&mut client, &thread, None).await;
    assert_eq!(latest.state, view.state);
    daemon.handle.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}

fn pending_select(state: &ThreadState) -> Option<SelectUiRequest> {
    state.interactions.iter().find_map(|request| match request {
        ExtensionUiRequest::Select(select) => Some(select.clone()),
        _ => None,
    })
}

async fn respond(client: &mut Client, thread: &str, id: &str, value: &str) -> Result<Value, Value> {
    client
        .call(
            "interaction.respond",
            json!({ "threadId": thread, "interactionId": id, "response": { "value": value } }),
        )
        .await
}

// spec: DMN-050, DMN-051, DMN-035, DMN-032, PRT-020, PRT-021, PRT-022
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One acceptance journey, step by step.
async fn approval_reconnects_crash_and_restart_with_history() {
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "echo approved-run" }),
        )])),
        Step::reply(Reply::text("Done after approval.")),
        Step::reply(Reply::text("Back after the crash.")),
    ])
    .await;
    let daemon = Daemon::start(&harness).await;
    let mut web = daemon.client("omfp/web").await;
    let mut desktop = daemon.client("omfp/desktop").await;
    let thread = web
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "approvals", "model": model(), "approvalMode": "always-ask" }),
        )
        .await;
    let thread = thread["id"].as_str().unwrap().to_owned();
    let mut web_view = Follower::snapshot(&mut web, &thread, None).await;
    let mut desktop_view = Follower::snapshot(&mut desktop, &thread, None).await;

    // Approval round trip; the web client disconnects mid-stream, right when the approval is requested.
    let first = prompt(&mut web, &thread, "run it").await;
    web_view
        .until(&mut web, |state| pending_select(state).is_some())
        .await;
    drop(web);
    desktop_view
        .until(&mut desktop, |state| pending_select(state).is_some())
        .await;
    let select = pending_select(&desktop_view.state).unwrap();
    let approve = select
        .options
        .iter()
        .find(|option| option.starts_with("Approve"))
        .unwrap()
        .clone();
    respond(&mut desktop, &thread, &select.id, &approve)
        .await
        .unwrap();
    let late = respond(&mut desktop, &thread, &select.id, "Deny")
        .await
        .unwrap_err();
    assert_eq!(late["code"], "already_resolved");
    desktop_view
        .until(&mut desktop, settled_after(&first))
        .await;
    assert!(
        harness.fake.requests()[1]
            .to_string()
            .contains("approved-run")
    );

    // Reconnect with `since`: the missed events replay without gaps and rebuild the daemon's state.
    let mut web = daemon.client("omfp/web").await;
    let replay = web
        .ok(
            "subscribe",
            json!({ "topic": web_view.topic, "since": { "epoch": web_view.epoch, "seq": web_view.seq } }),
        )
        .await;
    assert_eq!(replay["mode"], "replay");
    assert_eq!(replay["fromSeq"], web_view.seq + 1);
    while web_view.seq < desktop_view.seq {
        let frame = web.event(&web_view.topic).await;
        web_view.apply(&frame);
    }
    assert_eq!(web_view.state, desktop_view.state);
    let before_crash = (web_view.epoch.clone(), web_view.seq);
    drop(web);

    // Crash: the thread fails with omp's stderr tail; nothing restarts on its own.
    daemon.kill_omp(&thread);
    desktop_view
        .until(&mut desktop, |state| {
            matches!(state.process, ProcessState::Failed { .. })
        })
        .await;
    let ProcessState::Failed {
        message,
        stderr_tail,
        ..
    } = &desktop_view.state.process
    else {
        unreachable!()
    };
    assert!(message.contains("exited"), "{message}");
    let log = daemon.omp_log(&thread);
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        *stderr_tail,
        lines[lines.len().saturating_sub(20)..].join("\n")
    );

    // The next prompt restarts omp, which resumes the session with its history (DMN-032).
    // Prompt ids restart with each process, so wait for the new reply itself.
    prompt(&mut desktop, &thread, "are you back?").await;
    desktop_view
        .until(&mut desktop, |state| {
            state.settled
                && assistant_texts(state)
                    .last()
                    .is_some_and(|text| text.contains("Back after the crash."))
        })
        .await;
    let session = desktop_view.state.session.clone().unwrap();
    assert!(session.resumed, "{session:?}");
    let texts = assistant_texts(&desktop_view.state);
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Done after approval.")),
        "{texts:?}"
    );
    assert!(
        texts.last().unwrap().contains("Back after the crash."),
        "{texts:?}"
    );

    // A position of the previous process is gone from the buffer: a snapshot equal to the daemon's state.
    let mut late_client = daemon.client("omfp/web").await;
    let snapshot = Follower::snapshot(
        &mut late_client,
        &thread,
        Some((&before_crash.0, before_crash.1)),
    )
    .await;
    assert_ne!(snapshot.epoch, before_crash.0);
    assert_eq!(snapshot.seq, desktop_view.seq);
    assert_eq!(snapshot.state, desktop_view.state);
    daemon.handle.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}

/// omp 19.1.0's `--approval-mode` values (`packages/coding-agent/src/tools/approval.ts`).
const APPROVAL_MODES: [&str; 4] = ["always-ask", "always-ask-cached", "write", "yolo"];

// spec: DMN-210, TST-106
#[tokio::test(flavor = "multi_thread")]
async fn every_approval_mode_starts_a_working_thread_and_other_values_are_refused() {
    let harness = Harness::new(
        APPROVAL_MODES
            .iter()
            .map(|mode| Step::reply(Reply::text(format!("Answering in {mode}."))))
            .collect(),
    )
    .await;
    let daemon = Daemon::start(&harness).await;
    let mut client = daemon.client("approval-modes").await;

    // omp exits at once on a value it does not know ("server closed"): refused before anything is created.
    let refused = client
        .call(
            "threads.create",
            json!({ "workspace": { "new": {} }, "model": model(), "approvalMode": "ask", "prompt": "hello" }),
        )
        .await
        .unwrap_err();
    assert_eq!(refused["code"], "bad_request", "{refused}");
    let message = refused["message"].as_str().unwrap();
    for mode in APPROVAL_MODES {
        assert!(message.contains(mode), "{message}");
    }
    let index = client.ok("threads.list", json!({})).await;
    assert_eq!(index["threads"], json!([]));
    let workspaces = client.ok("workspaces.list", json!({})).await;
    assert_eq!(workspaces["workspaces"], json!([]));

    for mode in APPROVAL_MODES {
        let thread = client
            .ok(
                "threads.create",
                json!({ "workspace": { "new": {} }, "title": mode, "model": model(), "approvalMode": mode }),
            )
            .await;
        let thread = thread["id"].as_str().unwrap().to_owned();
        let mut view = Follower::snapshot(&mut client, &thread, None).await;
        let id = prompt(&mut client, &thread, "which mode?").await;
        view.until(&mut client, settled_after(&id)).await;
        let texts = assistant_texts(&view.state);
        assert!(
            texts
                .last()
                .is_some_and(|text| text.contains(&format!("Answering in {mode}."))),
            "{mode}: {texts:?}"
        );
        assert_eq!(view.state.process, ProcessState::Running, "{mode}");
    }
    daemon.handle.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}

fn pending_form(state: &ThreadState) -> Option<FormUiRequest> {
    state.interactions.iter().find_map(|request| match request {
        ExtensionUiRequest::Form(form) => Some(form.clone()),
        _ => None,
    })
}

async fn answer_form(
    client: &mut Client,
    thread: &str,
    id: &str,
    data: Value,
) -> Result<Value, Value> {
    client
        .call(
            "interaction.respond",
            json!({ "threadId": thread, "interactionId": id, "response": { "data": data } }),
        )
        .await
}

// spec: DMN-055, DMN-051, DMN-050
#[tokio::test(flavor = "multi_thread")]
async fn ask_forms_round_trip_through_the_protocol() {
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
    let daemon = Daemon::start(&harness).await;
    let mut client = daemon.client("omfp/web").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "ask", "model": model() }),
        )
        .await;
    let thread = thread["id"].as_str().unwrap().to_owned();
    let mut view = Follower::snapshot(&mut client, &thread, None).await;

    let prompt_id = prompt(&mut client, &thread, "set it up").await;
    view.until(&mut client, |state| pending_form(state).is_some())
        .await;
    let request = pending_form(&view.state).unwrap();
    // The whole form reaches the window through the daemon (DMN-050).
    assert_eq!(request.html, form);

    // A value no HTML form submits is refused before omp sees it, and the request stays open (DMN-055).
    let refused = answer_form(
        &mut client,
        &thread,
        &request.id,
        json!({ "db": { "x": 1 } }),
    )
    .await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(pending_form(&view.state).is_some());

    answer_form(
        &mut client,
        &thread,
        &request.id,
        json!({ "db": "sqlite", "features": ["auth", "search"] }),
    )
    .await
    .unwrap();
    // A second answer comes too late (DMN-051).
    let late = answer_form(
        &mut client,
        &thread,
        &request.id,
        json!({ "db": "postgres" }),
    )
    .await;
    assert!(late.is_err(), "{late:?}");
    view.until(&mut client, settled_after(&prompt_id)).await;

    let answers = view
        .state
        .tools
        .iter()
        .find(|run| run.tool_call_id == "call_ask_1")
        .and_then(|run| run.result.as_ref())
        .map(|result| result["details"]["answers"].clone());
    assert_eq!(
        answers,
        Some(json!([
            { "answer": "SQLite", "name": "db", "question": "Which database?" },
            { "answer": "Auth, Search", "name": "features", "question": "Which features?" },
        ]))
    );
    daemon.handle.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}

// spec: DMN-073
#[tokio::test(flavor = "multi_thread")]
async fn the_first_message_names_the_thread_through_an_omp_print_mode_run() {
    // The thread's turn and the title run both ask the fake LLM, in either order: both get the same answer.
    let title = "Checkout flow fix";
    let harness = Harness::new(vec![
        Step::reply(Reply::text(title)),
        Step::reply(Reply::text(title)),
    ])
    .await;
    let daemon = Daemon::start_titling(&harness).await;
    let mut client = daemon.client("omfp/web").await;
    client.ok("subscribe", json!({ "topic": "index" })).await;

    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "model": model(), "prompt": "Please fix the checkout flow" }),
        )
        .await;
    assert_eq!(thread["titleSource"], "placeholder");
    let id = thread["id"].as_str().unwrap().to_owned();
    loop {
        let event = client.event("index").await;
        let summary = &event["event"]["thread"];
        if event["event"]["kind"] == "thread_upserted"
            && summary["id"] == id.as_str()
            && summary["titleSource"] == "generated"
        {
            assert_eq!(summary["title"], title);
            break;
        }
    }

    // The daemon named the omp session after the title.
    let state = client.ok("thread.state", json!({ "threadId": id })).await;
    assert_eq!(state["sessionName"], title);
    let requests = harness.fake.requests();
    let title_run = requests
        .iter()
        .find(|request| system_prompt(request).contains(omfp_daemon::titles::TITLE_INSTRUCTIONS))
        .expect("the print-mode run's request");
    assert!(
        title_run["tools"].as_array().is_none_or(Vec::is_empty),
        "no tools in print mode: {}",
        title_run["tools"]
    );
    assert!(
        title_run
            .to_string()
            .contains("Please fix the checkout flow"),
        "{title_run}"
    );
    assert!(harness.fake.is_exhausted_cleanly());
    daemon.handle.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}

// spec: PRT-125, PRT-126, DMN-213, DMN-214
#[tokio::test(flavor = "multi_thread")]
async fn thread_session_matches_omps_state_in_a_fraction_of_its_size() {
    let harness = Harness::new(vec![Step::reply(Reply::text("Small facts."))]).await;
    let daemon = Daemon::start(&harness).await;
    let mut client = daemon.client("session-facts").await;
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "facts", "model": model() }),
        )
        .await;
    let thread = thread["id"].as_str().unwrap().to_owned();
    let mut view = Follower::snapshot(&mut client, &thread, None).await;
    let id = prompt(&mut client, &thread, "hello").await;
    view.until(&mut client, settled_after(&id)).await;

    let state = client
        .ok("thread.state", json!({ "threadId": thread }))
        .await;
    let session = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    assert_eq!(session["sessionId"], state["sessionId"]);
    assert_eq!(session["sessionFile"], state["sessionFile"]);
    assert!(
        std::path::Path::new(session["sessionFile"].as_str().unwrap()).is_file(),
        "{session}"
    );
    assert_eq!(session["model"], model());
    assert_eq!(session["process"], json!({ "state": "running" }));
    assert_eq!(session["settled"], true);
    assert_eq!(session["messageCount"], state["messageCount"]);
    let usage = &session["usage"];
    assert!(usage["cost"].is_number(), "{session}");
    assert!(usage["tokens"]["total"].as_i64().unwrap() > 0, "{session}");
    let (small, full) = (session.to_string().len(), state.to_string().len());
    assert!(small * 10 < full, "{small} bytes against {full}");

    // Stopped: omp's usage is gone, the rest stays and nothing restarts omp.
    client
        .ok("thread.stop", json!({ "threadId": thread }))
        .await;
    let stopped = client
        .ok("thread.session", json!({ "threadId": thread }))
        .await;
    assert_eq!(stopped["sessionFile"], state["sessionFile"]);
    assert_eq!(stopped["process"], json!({ "state": "stopped" }));
    assert_eq!(stopped.get("usage"), None);
    let index = client.ok("threads.list", json!({})).await;
    assert_eq!(
        index["threads"][0]["process"],
        json!({ "state": "stopped" })
    );
    daemon.handle.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}
