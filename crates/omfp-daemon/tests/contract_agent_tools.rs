//! M10 contract test (L3): the real daemon with the pinned omp and the fake LLM — an orchestrating thread reads the
//! method catalogue (AGT-042), lists and creates threads with `omfp_call` (AGT-041) as its agent principal (DMN-150),
//! and waits for the thread it spawned with `omfp_thread_wait` (AGT-043).

mod client;
mod common;

use client::{Client, Follower};
use common::{Harness, omp_daemon, system_prompt};
use fake_llm::{Matcher, Reply, Step, ToolCall};
use omfp_testkit::{FAKE_MODEL, FAKE_PROVIDER};
use serde_json::{Value, json};

/// The orchestrator's prompt and the first message it gives the thread it spawns.
const ORCHESTRATE: &str = "ORCHESTRATE: spawn a child and wait for it";
const CHILD_TASK: &str = "CHILD-TASK: say you are done";

/// The gated step answering the orchestrator's second request: the test releases it with the child's id.
const WAIT_STEP: usize = 2;

fn model() -> String {
    format!("{FAKE_PROVIDER}/{FAKE_MODEL}")
}

/// The orchestrator's three turns and the child's one; the child runs beside the orchestrator, so the steps after
/// the first answer in any order.
fn script() -> Vec<Step> {
    let orchestrator = Matcher::LastUserContains(ORCHESTRATE.into());
    vec![
        Step::when(
            orchestrator.clone(),
            Reply::tool_calls(vec![
                ToolCall::new("m1", "read", json!({ "path": "omfp://protocol/methods" })),
                ToolCall::new(
                    "m2",
                    "read",
                    json!({ "path": "omfp://protocol/methods/threads.create" }),
                ),
                ToolCall::new("c1", "omfp_call", json!({ "method": "threads.list" })),
                ToolCall::new(
                    "c2",
                    "omfp_call",
                    json!({
                        "method": "threads.create",
                        "params": { "workspace": { "new": {} }, "title": "child", "model": model(), "prompt": CHILD_TASK }
                    }),
                ),
            ]),
        ),
        Step::when(
            Matcher::LastUserContains(CHILD_TASK.into()),
            Reply::text("CHILD-DONE"),
        )
        .any_order(),
        // Released with an `omfp_thread_wait` for the child, whose id only the daemon knows.
        Step::when(orchestrator.clone(), Reply::text("unused"))
            .gated()
            .any_order(),
        Step::when(orchestrator, Reply::text("ALL-DONE")).any_order(),
    ]
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

// spec: AGT-040, AGT-041, AGT-042, AGT-043, AGT-045, DMN-150, DMN-151, DMN-152, PRT-090
#[tokio::test(flavor = "multi_thread")]
async fn an_orchestrator_lists_and_creates_threads_with_omfp_call_and_waits_for_its_child() {
    let harness = Harness::new(script()).await;
    let (daemon, _data) = omp_daemon(&harness, &["--no-title-generation"]).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let orchestrator = client
        .ok(
            "threads.create",
            json!({ "workspace": { "new": {} }, "title": "orchestrator", "model": model() }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut view = Follower::snapshot(&mut client, &orchestrator, None).await;
    let prompt_id = client
        .ok(
            "thread.prompt",
            json!({ "threadId": orchestrator, "text": ORCHESTRATE }),
        )
        .await["promptId"]
        .as_str()
        .unwrap()
        .to_owned();

    // The orchestrator called its tools and asks again: the child exists, created by the orchestrator's agent.
    harness.fake.wait_for_gate(WAIT_STEP).await;
    let threads = client.ok("threads.list", json!({})).await;
    let child = threads["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["title"] == "child")
        .unwrap_or_else(|| panic!("no child: {threads:#}"))
        .clone();
    assert_eq!(
        child["provenance"],
        json!({ "createdBy": { "kind": "thread", "threadId": orchestrator }, "client": "omp" })
    );
    let child_id = child["id"].as_str().unwrap().to_owned();
    harness.fake.release_gate_with(
        WAIT_STEP,
        Reply::tool_calls(vec![ToolCall::new(
            "w1",
            "omfp_thread_wait",
            json!({ "threadId": child_id, "timeoutSeconds": 120 }),
        )]),
    );
    view.until(&mut client, |state| {
        state.settled && state.prompts.contains_key(&prompt_id)
    })
    .await;
    harness.assert_no_stray_network();
    assert!(harness.fake.is_exhausted_cleanly());

    let requests = harness.fake.requests();
    let first = &requests[0];
    let offered = offered_tools(first);
    for tool in ["omfp_call", "omfp_thread_wait"] {
        assert!(
            offered.contains(&tool.to_owned()),
            "{tool} is a direct tool: {offered:?}"
        );
    }
    let system = system_prompt(first);
    assert!(
        system.contains(&format!("your thread id is {orchestrator}")),
        "the control note: {system}"
    );

    // The last request answers the wait; it carries every tool result of the conversation.
    let last = requests.last().unwrap();
    let methods = tool_result(last, "m1");
    assert!(
        methods.contains("`threads.create` — Creates a thread in an existing or a new workspace."),
        "{methods}"
    );
    assert!(
        methods.contains("`threads.list` — ") && !methods.contains("`subscribe`"),
        "{methods}"
    );
    let schema = tool_result(last, "m2");
    assert!(
        schema.contains("\"method\": \"threads.create\"") && schema.contains("\"prompt\""),
        "{schema}"
    );
    let listed = tool_result(last, "c1");
    assert!(listed.contains(&orchestrator), "{listed}");
    let created = tool_result(last, "c2");
    assert!(
        created.contains(&child_id)
            && created.contains(&format!("\"threadId\": \"{orchestrator}\"")),
        "{created}"
    );
    let waited = tool_result(last, "w1");
    assert!(
        waited.contains("\"status\": \"done\"") && waited.contains("CHILD-DONE"),
        "{waited}"
    );
    assert!(waited.contains(&child_id), "{waited}");
}
