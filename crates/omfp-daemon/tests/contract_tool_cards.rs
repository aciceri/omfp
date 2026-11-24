//! Contract scenarios for the transcript's tool cards (L3): one run per card family against the pinned omp, each
//! with a succeeding and a failing call, recorded as reducer fixtures under `fixtures/threads/<omp version>/`
//! (TST-012). The card tests replay them in their streaming, success and error states, so argument and result
//! shapes come from real omp output (UI-051). The assertions pin down the facts the cards rely on.

mod common;

use common::{Harness, Recorder, TIMEOUT, prompt, start};
use fake_llm::{Matcher, Reply, Step, ToolCall};
use omfp_core::{Resolution, ThreadEvent};
use omfp_testkit::normalize_frame;
use omp_rpc::{
    CancelUiResponse, ExtensionUiRequest, ExtensionUiResponse, LitTrue, RpcInbound, RpcNotification,
};
use serde_json::{Value, json};

/// Loopback address of the test web server (`read` of a URL, the `SearXNG` engine of `web_search`). A fixed port
/// keeps the URLs in tool arguments (streamed in 8-character deltas) stable across recordings; it lies below
/// Linux's ephemeral range, so outgoing connections never hold it.
const WEB_ADDR: &str = "127.0.0.1:29417";

/// `src/lib.rs` of the workspace; omp's read tags it `#72CA`.
const LIB_RS: &str = "//! Tool card fixture crate.

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

pub fn greet(name: &str) -> String {
    format!(\"Hello, {name}!\")
}
";

const PAGE_HTML: &str = "<!doctype html>
<html><head><title>Tool cards</title></head>
<body><main>
<h1>Tool cards</h1>
<p>This page is served on loopback by the contract test so the read tool has a real URL to fetch.</p>
<p>Each card family gets a succeeding and a failing call, recorded from the pinned omp release.</p>
</main></body></html>
";

fn call(id: &str, name: &str, args: Value) -> Step {
    Step::reply(Reply::tool_calls(vec![ToolCall::new(id, name, args)]))
}

fn settled(count: usize) -> impl Fn(&[ThreadEvent]) -> bool {
    move |events| Recorder::count(events, "session_settled") >= count
}

/// Writes `files` (relative path, content) into the harness workspace, creating it.
fn workspace_with(harness: &Harness, files: &[(&str, &str)]) {
    std::fs::create_dir_all(harness.workspace()).unwrap();
    for (path, content) in files {
        let path = harness.workspace().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

/// Runs one prompt with every tool approved (yolo) until the session settles.
async fn run(harness: &Harness, text: &'static str) -> Recorder {
    let mut spec = harness.spec(&[]);
    spec.approval_mode = Some("yolo".into());
    tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process
            .client()
            .prompt_and_wait(&prompt(text), TIMEOUT)
            .unwrap();
        recorder.wait_until("session_settled", settled(1));
        recorder
    })
    .await
    .unwrap_or_else(|_| panic!("stderr:\n{}", harness.stderr()))
}

/// Recorded omp frames of `kind` for the tool call `id`, as JSON.
fn tool_frames(events: &[ThreadEvent], kind: &str, id: &str) -> Vec<Value> {
    events
        .iter()
        .filter(|event| matches!(event, ThreadEvent::Omp { .. }))
        .map(|event| serde_json::to_value(event).unwrap()["frame"].clone())
        .filter(|frame| frame["type"] == kind && frame["toolCallId"] == id)
        .collect()
}

/// The `result` of the call's `tool_execution_end`, after checking its `isError` flag.
fn ended(events: &[ThreadEvent], id: &str, is_error: bool) -> Value {
    let ends = tool_frames(events, "tool_execution_end", id);
    assert_eq!(ends.len(), 1, "one tool_execution_end for {id}: {ends:#?}");
    let end = &ends[0];
    assert_eq!(end["isError"], is_error, "isError of {id}: {end:#}");
    end["result"].clone()
}

fn text(result: &Value) -> String {
    result["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect()
}

/// Starts the loopback web server: `/page` (HTML), `/search` (the `SearXNG` JSON API; a query containing `fail`
/// answers 500), anything else 404.
async fn serve_web() {
    use axum::extract::Query;
    use axum::http::StatusCode;
    use axum::response::Html;
    use axum::routing::get;
    use axum::{Json, Router};
    use std::collections::HashMap;

    let app = Router::new()
        .route("/page", get(|| async { Html(PAGE_HTML) }))
        .route(
            "/search",
            get(|Query(params): Query<HashMap<String, String>>| async move {
                let query = params.get("q").cloned().unwrap_or_default();
                if query.contains("fail") {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
                Ok(Json(json!({
                    "query": query,
                    "number_of_results": 2,
                    "results": [
                        {
                            "title": "Tool cards",
                            "url": format!("http://{WEB_ADDR}/page"),
                            "content": "How the transcript shows tool calls.",
                            "engine": "duckduckgo"
                        },
                        {
                            "title": "omp tools",
                            "url": "http://127.0.0.1/tools",
                            "content": "Reference of the agent's tools.",
                            "engine": "brave"
                        }
                    ]
                })))
            }),
        );
    let listener = tokio::net::TcpListener::bind(WEB_ADDR)
        .await
        .unwrap_or_else(|error| panic!("cannot bind the test web server on {WEB_ADDR}: {error}"));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn read_tool_calls() {
    let harness = Harness::new(vec![
        call(
            "call_read_1",
            "read",
            json!({ "path": "src/lib.rs", "i": "Reading the library" }),
        ),
        call(
            "call_read_2",
            "read",
            json!({ "path": "src/missing.rs", "i": "Reading a missing file" }),
        ),
        call(
            "call_read_3",
            "read",
            json!({ "path": "src", "i": "Listing the sources" }),
        ),
        Step::reply(Reply::text("The library defines add and greet.")),
    ])
    .await;
    workspace_with(&harness, &[("src/lib.rs", LIB_RS)]);

    let recorder = run(&harness, "read the library").await;
    let events = recorder.events();

    let file = ended(&events, "call_read_1", false);
    assert!(
        text(&file).starts_with("[src/lib.rs#72CA]\n1://! Tool card"),
        "{file:#}"
    );
    assert_eq!(file["details"]["totalLines"], 9);
    assert_eq!(file["details"]["displayContent"]["startLine"], 1);
    let missing = ended(&events, "call_read_2", true);
    assert_eq!(text(&missing), "Path 'src/missing.rs' not found");
    let listing = ended(&events, "call_read_3", false);
    assert!(text(&listing).contains("lib.rs"), "{listing:#}");
    finish(&harness, &recorder, "read_tool_calls");
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn edit_tool_calls() {
    let harness = Harness::new(vec![
        call(
            "call_read_1",
            "read",
            json!({ "path": "src/lib.rs", "i": "Reading the library" }),
        ),
        call(
            "call_edit_1",
            "edit",
            json!({
                "input": "[src/lib.rs#72CA]\nPUT 4.=4:\n+    left.saturating_add(right)\n",
                "i": "Saturating the addition"
            }),
        ),
        call(
            "call_edit_2",
            "edit",
            json!({
                "input": "[src/lib.rs#0000]\nPUT 8.=8:\n+    format!(\"Hi, {name}!\")\n",
                "i": "Shortening the greeting"
            }),
        ),
        Step::reply(Reply::text(
            "add saturates now; the greeting edit used a stale tag.",
        )),
    ])
    .await;
    workspace_with(&harness, &[("src/lib.rs", LIB_RS)]);

    let recorder = run(&harness, "make add saturate").await;
    let events = recorder.events();

    let edited = ended(&events, "call_edit_1", false);
    let diff = edited["details"]["diff"].as_str().unwrap();
    assert!(
        diff.contains("left + right") && diff.contains("left.saturating_add(right)"),
        "{edited:#}"
    );
    let rejected = ended(&events, "call_edit_2", true);
    assert_eq!(
        rejected["details"]["errorKind"], "unknown_tag",
        "{rejected:#}"
    );
    assert!(
        text(&rejected).starts_with("Edit rejected for src/lib.rs"),
        "{rejected:#}"
    );
    let lib = std::fs::read_to_string(harness.workspace().join("src/lib.rs")).unwrap();
    assert!(
        lib.contains("left.saturating_add(right)") && lib.contains("Hello, {name}!"),
        "{lib}"
    );
    finish(&harness, &recorder, "edit_tool_calls");
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn write_tool_calls() {
    let harness = Harness::new(vec![
        call(
            "call_write_1",
            "write",
            json!({
                "path": "notes.md",
                "content": "# Notes\n\n- add saturates\n- greet says hello\n",
                "i": "Writing notes"
            }),
        ),
        call(
            "call_write_2",
            "write",
            json!({ "path": "src", "content": "not a file\n", "i": "Writing over a directory" }),
        ),
        Step::reply(Reply::text("Notes written; src is a directory.")),
    ])
    .await;
    workspace_with(&harness, &[("src/lib.rs", LIB_RS)]);

    let recorder = run(&harness, "write notes").await;
    let events = recorder.events();

    let written = ended(&events, "call_write_1", false);
    assert!(
        text(&written).starts_with("[notes.md#") && text(&written).contains("Successfully wrote"),
        "{written:#}"
    );
    let failed = ended(&events, "call_write_2", true);
    assert!(text(&failed).contains("EISDIR"), "{failed:#}");
    assert_eq!(
        std::fs::read_to_string(harness.workspace().join("notes.md")).unwrap(),
        "# Notes\n\n- add saturates\n- greet says hello\n"
    );
    finish(&harness, &recorder, "write_tool_calls");
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn search_tool_calls() {
    // omp 19.1 advertises `grep` and `glob`; `find` and `ast_grep` are not offered as tools.
    let harness = Harness::new(vec![
        call(
            "call_grep_1",
            "grep",
            json!({ "pattern": "pub fn", "path": "src", "i": "Finding functions" }),
        ),
        call(
            "call_grep_2",
            "grep",
            json!({ "pattern": "pub fn", "path": "missing", "i": "Searching a missing folder" }),
        ),
        call(
            "call_glob_1",
            "glob",
            json!({ "path": "src/**/*.rs", "i": "Listing Rust files" }),
        ),
        call(
            "call_glob_2",
            "glob",
            json!({ "path": "missing/**/*.rs", "i": "Listing a missing folder" }),
        ),
        Step::reply(Reply::text("Two functions in src/lib.rs.")),
    ])
    .await;
    workspace_with(
        &harness,
        &[("src/lib.rs", LIB_RS), ("notes.md", "# Notes\n")],
    );

    let recorder = run(&harness, "find the functions").await;
    let events = recorder.events();

    let grep = ended(&events, "call_grep_1", false);
    assert_eq!(grep["details"]["matchCount"], 2, "{grep:#}");
    assert_eq!(grep["details"]["files"], json!(["src/lib.rs"]));
    ended(&events, "call_grep_2", true);
    let glob = ended(&events, "call_glob_1", false);
    assert_eq!(glob["details"]["files"], json!(["src/lib.rs"]), "{glob:#}");
    let missing = ended(&events, "call_glob_2", true);
    assert_eq!(text(&missing), "Path not found: missing");
    finish(&harness, &recorder, "search_tool_calls");
}

/// A `bash` call running the tool command `tool` with the JSON arguments `args` (`-j`): how omp 19.1 runs its
/// discoverable tools (`lsp`, `ast_grep`, `ast_edit`) while `tools.commands` (on by default) mounts them as commands.
fn command_call(id: &str, tool: &str, args: &Value, intent: &str) -> Step {
    let json = args.to_string();
    assert!(!json.contains('\''), "single-quoted for the shell: {json}");
    call(
        id,
        "bash",
        json!({ "command": format!("{tool} -j '{json}'"), "i": intent }),
    )
}

/// The dispatch of the single tool command a `bash` result ran (`details.toolCommands`, omp 19.1).
fn dispatch(result: &Value) -> &Value {
    let dispatches = result["details"]["toolCommands"]
        .as_array()
        .unwrap_or_else(|| panic!("no tool commands: {result:#}"));
    assert_eq!(dispatches.len(), 1, "{result:#}");
    &dispatches[0]
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn code_intel_tool_calls() {
    let harness = Harness::new(vec![
        command_call(
            "call_ast_grep_1",
            "ast_grep",
            &json!({ "pat": "pub fn $NAME($$$ARGS) -> $RET { $$$BODY }", "path": "src" }),
            "Finding the functions",
        ),
        command_call(
            "call_ast_grep_2",
            "ast_grep",
            &json!({ "pat": "pub fn $NAME($$$ARGS)", "path": "missing" }),
            "Searching a missing folder",
        ),
        command_call(
            "call_lsp_1",
            "lsp",
            &json!({ "action": "diagnostics", "file": "notes.txt" }),
            "Checking the notes",
        ),
        command_call(
            "call_ast_edit_1",
            "ast_edit",
            &json!({ "ops": [{ "pat": "left + right", "out": "left.saturating_add(right)" }], "paths": ["src"] }),
            "Saturating the addition",
        ),
        // An `ast_edit` is a preview pending until the agent resolves it (`resolve <reason>`).
        call(
            "call_resolve_1",
            "bash",
            json!({ "command": "resolve 'Saturating is safer.'", "i": "Applying the rewrite" }),
        ),
        Step::reply(Reply::text("Two functions; the addition saturates now.")),
    ])
    .await;
    // omfp's profile turns the AST tools on; omp offers them as bash tool commands (`tools.commands`).
    harness
        .env
        .configure(
            &[],
            "astGrep:\n  enabled: true\nastEdit:\n  enabled: true\n",
        )
        .unwrap();
    workspace_with(
        &harness,
        &[
            ("src/lib.rs", LIB_RS),
            ("notes.txt", "no language server reads this\n"),
        ],
    );

    let recorder = run(&harness, "find the functions").await;
    let events = recorder.events();

    assert_ast_grep(&events);
    let diagnostics = ended(&events, "call_lsp_1", false);
    assert_eq!(
        dispatch(&diagnostics)["inner"]["action"],
        "diagnostics",
        "{diagnostics:#}"
    );
    assert!(
        text(&diagnostics).contains("notes.txt: No language server handles .txt files"),
        "{diagnostics:#}"
    );
    let preview = ended(&events, "call_ast_edit_1", false);
    let inner = &dispatch(&preview)["inner"];
    assert_eq!(
        (&inner["totalReplacements"], &inner["applied"]),
        (&json!(1), &json!(false)),
        "{preview:#}"
    );
    assert!(
        inner["displayContent"]
            .as_str()
            .unwrap()
            .contains("left.saturating_add(right)"),
        "{preview:#}"
    );
    let applied = ended(&events, "call_resolve_1", false);
    assert!(
        text(&applied).contains("Applied 1 replacement in 1 file."),
        "{applied:#}"
    );
    assert!(
        std::fs::read_to_string(harness.workspace().join("src/lib.rs"))
            .unwrap()
            .contains("left.saturating_add(right)")
    );
    finish(&harness, &recorder, "code_intel_tool_calls");
}

/// The `ast_grep` command calls of [`code_intel_tool_calls`]: the `bash` result's dispatch names the tool and its
/// arguments and carries the tool's own details (`inner`).
fn assert_ast_grep(events: &[ThreadEvent]) {
    let found = ended(events, "call_ast_grep_1", false);
    let dispatched = dispatch(&found);
    assert_eq!(
        (&dispatched["tool"], &dispatched["mode"]),
        (&json!("ast_grep"), &json!("execute")),
        "{found:#}"
    );
    assert_eq!(
        dispatched["args"]["pat"],
        "pub fn $NAME($$$ARGS) -> $RET { $$$BODY }"
    );
    assert_eq!(dispatched["inner"]["matchCount"], 2, "{found:#}");
    assert_eq!(
        dispatched["inner"]["fileMatches"],
        json!([{ "path": "src/lib.rs", "count": 2 }])
    );
    assert!(
        dispatched["inner"]["displayContent"]
            .as_str()
            .unwrap()
            .contains("pub fn add(left: u64, right: u64) -> u64 {"),
        "{found:#}"
    );
    let missing = ended(events, "call_ast_grep_2", true);
    assert_eq!(dispatch(&missing)["tool"], "ast_grep", "{missing:#}");
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn bash_tool_error() {
    let harness = Harness::new(vec![
        call(
            "call_bash_1",
            "bash",
            json!({ "command": "echo oops >&2; exit 3", "i": "Running a failing command" }),
        ),
        Step::reply(Reply::text("The command failed with exit code 3.")),
    ])
    .await;
    workspace_with(&harness, &[]);

    let recorder = run(&harness, "run the failing command").await;
    let events = recorder.events();

    let failed = ended(&events, "call_bash_1", true);
    assert_eq!(failed["details"]["exitCode"], 3, "{failed:#}");
    // omp puts a `Wall time: …` notice before the exit line once the call took 1 s or more, its shell's setup
    // included, and says so in `details.wallTimeNoticeEmitted`: a loaded machine gets there even with this command.
    let wall_time = if failed["details"]["wallTimeNoticeEmitted"] == true {
        "Wall time: <duration>\n\n"
    } else {
        ""
    };
    let mut shown = failed.clone();
    normalize_frame(&mut shown, "");
    assert_eq!(
        text(&shown),
        format!("oops\n\n\n{wall_time}Command exited with code 3"),
        "{failed:#}"
    );
    // Over RPC omp 19.1 runs a foreground call as a job (`bash.autoBackground`), which reports a failed command, output
    // included, before the call ends: an update carrying the output comes however fast the command exits.
    let updates = tool_frames(&events, "tool_execution_update", "call_bash_1");
    assert!(
        updates
            .iter()
            .any(|update| text(&update["partialResult"]).starts_with("oops\n")),
        "bash streams its output: {updates:#?}"
    );
    finish(&harness, &recorder, "bash_tool_error");
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn todo_tool_calls() {
    let harness = Harness::new(vec![
        call(
            "call_todo_1",
            "todo",
            json!({ "op": "init", "items": ["Read the code", "Fix the bug"], "i": "Planning" }),
        ),
        call(
            "call_todo_2",
            "todo",
            json!({ "op": "start", "task": "Write the docs", "i": "Starting an unknown task" }),
        ),
        call(
            "call_todo_3",
            "todo",
            json!({ "op": "done", "task": "Read the code", "i": "Finishing the reading" }),
        ),
        call(
            "call_todo_4",
            "todo",
            json!({ "op": "done", "task": "Fix the bug", "i": "Finishing the fix" }),
        ),
        Step::reply(Reply::text("All tasks done.")),
    ])
    .await;
    workspace_with(&harness, &[]);

    let recorder = run(&harness, "plan the work").await;
    let events = recorder.events();

    let init = ended(&events, "call_todo_1", false);
    assert_eq!(init["details"]["op"], "init");
    assert_eq!(
        init["details"]["phases"][0]["tasks"],
        json!([
            { "content": "Read the code", "status": "in_progress" },
            { "content": "Fix the bug", "status": "pending" }
        ])
    );
    let unknown = ended(&events, "call_todo_2", true);
    assert!(
        text(&unknown).starts_with("Errors: Task \"Write the docs\" not found"),
        "{unknown:#}"
    );
    ended(&events, "call_todo_3", false);
    let last = ended(&events, "call_todo_4", false);
    assert_eq!(
        last["details"]["phases"][0]["tasks"],
        json!([
            { "content": "Read the code", "status": "completed" },
            { "content": "Fix the bug", "status": "completed" }
        ]),
        "{last:#}"
    );
    finish(&harness, &recorder, "todo_tool_calls");
}

// spec: TST-012, UI-051, TST-021
#[tokio::test(flavor = "multi_thread")]
async fn web_tool_calls() {
    serve_web().await;
    let harness = Harness::new(vec![
        call(
            "call_web_search_1",
            "web_search",
            json!({ "query": "omfp tool cards", "i": "Searching the web" }),
        ),
        call(
            "call_web_search_2",
            "web_search",
            json!({ "query": "fail on purpose", "i": "Searching a failing engine" }),
        ),
        call(
            "call_read_1",
            "read",
            json!({ "path": format!("http://{WEB_ADDR}/page"), "i": "Fetching the page" }),
        ),
        call(
            "call_read_2",
            "read",
            json!({ "path": format!("http://{WEB_ADDR}/missing"), "i": "Fetching a missing page" }),
        ),
        call(
            "call_read_3",
            "read",
            json!({ "path": format!("http://{WEB_ADDR}/page:1-2:4-5"), "i": "Fetching with a bad selector" }),
        ),
        Step::reply(Reply::text("Found the tool cards page.")),
    ])
    .await;
    // SearXNG on loopback is the only search engine: an empty fallback chain keeps omp from trying the public
    // engines (and a headless browser) when it fails.
    harness
        .env
        .configure(
            &[("web", "web/searxng")],
            &format!(
                "searxng:\n  endpoint: http://{WEB_ADDR}\nretry:\n  fallbackChains:\n    web: []\n"
            ),
        )
        .unwrap();
    workspace_with(&harness, &[]);

    let recorder = run(&harness, "search the web").await;
    let events = recorder.events();

    let found = ended(&events, "call_web_search_1", false);
    assert_eq!(
        found["details"]["response"]["provider"], "searxng",
        "{found:#}"
    );
    assert_eq!(
        found["details"]["response"]["sources"][0]["title"],
        "Tool cards"
    );
    // omp reports a failed search as a normal result: `isError` false, the reason in `details.error`.
    let failed = ended(&events, "call_web_search_2", false);
    assert!(
        failed["details"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("SearXNG API error (500)")),
        "{failed:#}"
    );
    let page = ended(&events, "call_read_1", false);
    assert_eq!(page["details"]["kind"], "url", "{page:#}");
    assert!(text(&page).contains("served on loopback"), "{page:#}");
    // A URL that answers 404 is not a tool error either: `details.method` is `failed`, the reason in `notes`.
    let missing = ended(&events, "call_read_2", false);
    assert_eq!(missing["details"]["method"], "failed", "{missing:#}");
    assert_eq!(
        missing["details"]["notes"],
        json!(["Failed to fetch URL (HTTP 404)"])
    );
    ended(&events, "call_read_3", true);
    finish(&harness, &recorder, "web_tool_calls");
}

// spec: TST-012, UI-051
#[tokio::test(flavor = "multi_thread")]
async fn task_tool_call() {
    let assignment = "Count the functions in src/lib.rs";
    let harness = Harness::new(vec![
        call(
            "call_task_1",
            "task",
            json!({
                "context": "The workspace holds a small Rust library.",
                "tasks": [{ "name": "Counter", "task": assignment, "solutionSpace": "src/lib.rs only" }],
                "i": "Delegating the count"
            }),
        ),
        // The subagent: one tool call, then its final answer (a plain reply ends it).
        Step::when(
            Matcher::LastUserContains(assignment.into()),
            Reply::tool_calls(vec![ToolCall::new(
                "call_read_1",
                "read",
                json!({ "path": "src/lib.rs", "i": "Reading the library" }),
            )]),
        ),
        Step::when(
            Matcher::LastUserContains(assignment.into()),
            Reply::text("src/lib.rs defines 2 functions: add and greet."),
        ),
        call(
            "call_task_2",
            "task",
            json!({
                "context": "The workspace holds a small Rust library.",
                "tasks": [{ "name": "Ghost", "agent": "no-such-agent", "task": assignment, "solutionSpace": "none" }],
                "i": "Delegating to an unknown agent"
            }),
        ),
        Step::reply(Reply::text("The library has 2 functions.")),
    ])
    .await;
    // Blocking subagents: with async jobs (on by default over RPC) the task returns at once and its job reports later.
    harness
        .env
        .configure(&[], "async:\n  enabled: false\n")
        .unwrap();
    workspace_with(&harness, &[("src/lib.rs", LIB_RS)]);

    let recorder = run(&harness, "count the functions").await;
    let events = recorder.events();

    let done = ended(&events, "call_task_1", false);
    assert!(text(&done).contains("2 functions"), "{done:#}");
    assert!(
        !tool_frames(&events, "tool_execution_update", "call_task_1").is_empty(),
        "the task streams its progress"
    );
    let unknown = ended(&events, "call_task_2", true);
    assert!(
        text(&unknown).contains("Unknown agent \"no-such-agent\""),
        "{unknown:#}"
    );
    finish(&harness, &recorder, "task_tool_call");
}

// spec: TST-012, UI-051, DMN-050
#[tokio::test(flavor = "multi_thread")]
async fn ask_cancelled() {
    let form = r#"<form>
  <fieldset><legend>Which database?</legend>
    <label><input type="radio" name="db" value="postgres"> Postgres</label>
    <label><input type="radio" name="db" value="sqlite" checked> SQLite</label>
  </fieldset>
</form>"#;
    let harness = Harness::new(vec![call(
        "call_ask_1",
        "ask",
        json!({ "html": form, "i": "Choosing the database" }),
    )])
    .await;
    workspace_with(&harness, &[]);
    let spec = harness.spec(&[]);

    let recorder = tokio::task::spawn_blocking(move || {
        let (mut process, startup) = start(&spec);
        let recorder = Recorder::attach(&mut process, &startup);
        process.client().call(&prompt("set it up")).unwrap();
        // omp 19.1 sends the whole form as one `form` request.
        let form_id = |events: &[ThreadEvent]| {
            events.iter().find_map(|event| match event {
                ThreadEvent::Omp {
                    frame: RpcNotification::ExtensionUiRequest(ExtensionUiRequest::Form(form)),
                } => Some(form.id.clone()),
                _ => None,
            })
        };
        recorder.wait_until("the form dialog", |events| form_id(events).is_some());
        let id = form_id(&recorder.events()).unwrap();
        // Recorded before the answer goes out, so omp's reaction always follows it (stable fixture).
        recorder.push(ThreadEvent::InteractionResolved {
            id: id.clone(),
            resolution: Resolution::Client {
                client: "contract-test".into(),
            },
        });
        process
            .client()
            .send(&RpcInbound::ExtensionUiResponse(
                ExtensionUiResponse::CancelUiResponse(CancelUiResponse {
                    id,
                    cancelled: LitTrue,
                    timed_out: None,
                }),
            ))
            .unwrap();
        recorder.wait_until("session_settled", settled(1));
        recorder
    })
    .await
    .unwrap_or_else(|_| panic!("stderr:\n{}", harness.stderr()));

    let events = recorder.events();
    let cancelled = ended(&events, "call_ask_1", true);
    assert_eq!(
        text(&cancelled),
        "Ask tool was cancelled by the user",
        "{cancelled:#}"
    );
    finish(&harness, &recorder, "ask_cancelled");
}

/// Writes the fixture (with `OMFP_RECORD_FIXTURES`) and checks the run stayed on loopback (TST-021).
fn finish(harness: &Harness, recorder: &Recorder, name: &str) {
    recorder.record(harness, name);
    harness.assert_no_stray_network();
}
