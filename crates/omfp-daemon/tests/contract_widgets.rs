//! M4 contract tests (L3): the real daemon with the pinned omp and the fake LLM — the `omfp://` host URIs every thread
//! reads (DMN-062, AGT-025), the bash output omp streams while a command runs (AGT-020), and the main agent
//! delegating a widget to the daemon's `widget-builder` subagent (AGT-029, M6b).

mod client;
mod common;

use client::{Client, Follower};
use common::{Harness, omp_daemon};
use fake_llm::{Matcher, Reply, Step, ToolCall};
use omfp_daemon::widgets::{DOCS, SCHEMA_URL};
use omfp_testkit::{FAKE_MODEL, FAKE_PROVIDER};
use serde_json::{Value, json};

/// A new thread on the fake model, followed from its snapshot.
async fn thread(client: &mut Client) -> (String, Follower) {
    let thread = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "widgets", "model": format!("{FAKE_PROVIDER}/{FAKE_MODEL}") }),
        )
        .await;
    let id = thread["id"].as_str().unwrap().to_owned();
    let view = Follower::snapshot(client, &id, None).await;
    (id, view)
}

async fn prompt(client: &mut Client, thread: &str, text: &str) -> String {
    let result = client
        .ok("thread.prompt", json!({ "threadId": thread, "text": text }))
        .await;
    result["promptId"].as_str().unwrap().to_owned()
}

/// Applies the thread's events until one is the omp frame `kind` for which `matches` holds; returns that frame.
async fn until_frame(
    client: &mut Client,
    view: &mut Follower,
    kind: &str,
    matches: impl Fn(&Value) -> bool,
) -> Value {
    loop {
        let event = client.event(&view.topic).await;
        view.apply(&event);
        let frame = &event["event"]["frame"];
        if event["event"]["kind"] == "omp" && frame["type"] == kind && matches(frame) {
            return frame.clone();
        }
    }
}

/// The text of a tool result or partial result (`{ content: [{ type: "text", text }] }`).
fn text(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// The text content of the tool result answering `call_id` in a chat-completions request.
fn tool_result(request: &Value, call_id: &str) -> String {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .map_or_else(
            || panic!("no result for {call_id}: {request:#}"),
            |message| message["content"].as_str().unwrap_or_default().to_owned(),
        )
}

// spec: DMN-062, AGT-025
#[tokio::test(flavor = "multi_thread")]
async fn the_agent_reads_the_widget_docs_and_the_views_schema_from_omfp_urls() {
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![
            ToolCall::new("c1", "read", json!({ "path": "omfp://docs/widgets" })),
            ToolCall::new("c2", "read", json!({ "path": "omfp://schemas/views.json" })),
            ToolCall::new("c3", "read", json!({ "path": "omfp://nothing/here" })),
        ])),
        Step::reply(Reply::text("Read them.")),
    ])
    .await;
    let (daemon, _data) = omp_daemon(&harness, &["--no-title-generation"]).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let (thread, mut view) = thread(&mut client).await;

    let prompt_id = prompt(&mut client, &thread, "learn widgets").await;
    view.until(&mut client, |state| {
        state.settled && state.prompts.contains_key(&prompt_id)
    })
    .await;

    let answer = &harness.fake.requests()[1];
    let docs = tool_result(answer, "c1");
    let first_lines: Vec<&str> = DOCS.lines().take(3).collect();
    for line in first_lines.iter().filter(|line| !line.is_empty()) {
        assert!(
            docs.contains(line),
            "{line:?} missing from the docs read: {docs}"
        );
    }
    let schema = tool_result(answer, "c2");
    assert!(
        schema.contains(&format!("\"$id\": \"{SCHEMA_URL}\"")),
        "{schema}"
    );
    let unknown = tool_result(answer, "c3");
    assert!(
        unknown.contains("omfp://docs/widgets") && unknown.contains(SCHEMA_URL),
        "an unknown URL names both: {unknown}"
    );
    harness.assert_no_stray_network();
}

// spec: AGT-020
#[tokio::test(flavor = "multi_thread")]
async fn omp_streams_the_bash_output_while_the_command_runs() {
    let gate_dir = tempfile::tempdir().unwrap();
    let gate = gate_dir.path().join("gate");
    let command = format!(
        "printf 'one\\n'; while [ ! -e {} ]; do sleep 0.05; done; printf 'two\\n'",
        gate.display()
    );
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "c1",
            "bash",
            json!({ "command": command }),
        )])),
        Step::reply(Reply::text("Both lines.")),
    ])
    .await;
    let (daemon, _data) = omp_daemon(&harness, &["--no-title-generation"]).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let (thread, mut view) = thread(&mut client).await;

    let prompt_id = prompt(&mut client, &thread, "run it").await;
    let update = until_frame(&mut client, &mut view, "tool_execution_update", |frame| {
        text(&frame["partialResult"]).contains("one")
    })
    .await;
    assert_eq!(update["toolCallId"], "c1");
    assert!(!text(&update["partialResult"]).contains("two"));

    std::fs::write(&gate, "").unwrap();
    let end = until_frame(&mut client, &mut view, "tool_execution_end", |frame| {
        frame["toolCallId"] == "c1"
    })
    .await;
    assert_eq!(text(&end["result"]), "one\ntwo\n");
    view.until(&mut client, |state| {
        state.settled && state.prompts.contains_key(&prompt_id)
    })
    .await;
    harness.assert_no_stray_network();
}

/// The names of the tools a chat-completions request offers the model.
fn offered_tools(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

const WIDGET: &str = "import { Card } from \"omfp/kit\";\nexport default function Clock() {\n  return <Card title=\"Clock\">12:00</Card>;\n}\n";

/// The main agent's and the builder's turns; the builder runs beside its parent, so its steps answer in any order.
fn delegation_script() -> Vec<Step> {
    let brief = Matcher::LastUserContains("WIDGET-BRIEF".into());
    vec![
        // The main agent starts the builder in the background...
        Step::when(
            Matcher::LastUserContains("show a clock".into()),
            Reply::tool_calls(vec![ToolCall::new(
                "task1",
                "task",
                json!({
                    "context": "The user wants a clock in their omfp window.",
                    "tasks": [{
                        "agent": "widget-builder",
                        "name": "ClockBuilder",
                        "task": "WIDGET-BRIEF: build ./widgets/clock showing the time; it goes in a side tab.",
                        "solutionSpace": "one widget, design open"
                    }]
                }),
            )]),
        ),
        // ...and keeps talking while the builder runs.
        Step::when(
            Matcher::LastUserContains("show a clock".into()),
            Reply::text("The builder is on it."),
        )
        .any_order(),
        Step::when(
            brief.clone(),
            Reply::tool_calls(vec![ToolCall::new(
                "write1",
                "write",
                json!({ "path": ".omfp/widgets/clock/index.tsx", "content": WIDGET }),
            )]),
        )
        .any_order(),
        Step::when(
            brief.clone(),
            Reply::tool_calls(vec![ToolCall::new(
                "check1",
                "read",
                json!({ "path": "omfp://widgets/check/clock" }),
            )]),
        )
        .any_order(),
        Step::when(
            brief,
            Reply::text("CLOCK-BUILT: ./widgets/clock, no props."),
        )
        .any_order(),
        // The builder's result arrives as a follow-up turn; the main agent places the widget.
        Step::when(
            Matcher::LastUserContains("CLOCK-BUILT".into()),
            Reply::tool_calls(vec![ToolCall::new(
                "add1",
                "omfp_ui_mount",
                json!({ "id": "clock", "widget": "./widgets/clock", "place": "side_tab" }),
            )]),
        ),
        Step::reply(Reply::text("Your clock is in a side tab.")),
    ]
}

// spec: AGT-029, DMN-106, DMN-107, DMN-108, DMN-060, DMN-125, DMN-196
#[tokio::test(flavor = "multi_thread")]
async fn the_main_agent_delegates_a_widget_to_the_widget_builder_which_checks_it_through_the_daemon()
 {
    let harness = Harness::new(delegation_script()).await;
    let (daemon, _data) = omp_daemon(&harness, &["--no-title-generation"]).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let (thread, mut view) = thread(&mut client).await;

    prompt(&mut client, &thread, "show a clock").await;
    let started = until_frame(&mut client, &mut view, "subagent_lifecycle", |_| true).await;
    assert_eq!(started["payload"]["agent"], "widget-builder", "{started:#}");
    assert_eq!(
        started["payload"]["agentSource"], "user",
        "from --extension: {started:#}"
    );
    assert_eq!(
        started["payload"]["detached"], true,
        "a background task: {started:#}"
    );
    let fake = &harness.fake;
    view.until(&mut client, |state| {
        state.settled && fake.requests().len() == 7
    })
    .await;
    harness.assert_no_stray_network();

    let requests = fake.requests();
    let parent = offered_tools(&requests[0]);
    for tool in [
        "task",
        "omfp_open_file",
        "omfp_notify",
        "omfp_widget_check",
        "omfp_ui_mount",
        "omfp_views_errors",
    ] {
        assert!(
            parent.contains(&tool.to_owned()),
            "{tool} is a direct tool of the main agent: {parent:?}"
        );
    }
    let child: Vec<&Value> = requests
        .iter()
        .filter(|request| {
            request["messages"][0]["content"]
                .as_str()
                .is_some_and(|system| system.contains("# omfp widget builder"))
        })
        .collect();
    assert_eq!(
        child.len(),
        3,
        "the builder's three turns carry its definition"
    );
    let tools = offered_tools(child[0]);
    for tool in ["read", "write", "edit"] {
        assert!(tools.contains(&tool.to_owned()), "{tool}: {tools:?}");
    }
    assert!(
        !tools.iter().any(|tool| tool.starts_with("omfp_")),
        "no host tools in a subagent: {tools:?}"
    );
    let report = tool_result(child[2], "check1");
    assert!(
        report.contains("clock") && !report.contains("unknown omfp URL"),
        "the daemon answered the check: {report}"
    );
    let placed = requests
        .iter()
        .find_map(|request| {
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["tool_call_id"] == "add1")
                .then(|| tool_result(request, "add1"))
        })
        .expect("the main agent placed the widget");
    assert!(!placed.contains("not found"), "{placed}");
}
