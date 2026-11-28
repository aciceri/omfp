//! The widget journeys of M4 (06-testing.md, E2E-10..12): an agent-made widget following a daemon job, views from
//! `views.json` with their error banner and the mode line's view tabs, and `:widget add` with a copied built-in.
//! They also take the M4 screenshots (`omfp-m4-*.png` in the run's screenshot directory). Run with `just test-e2e`.

use std::path::Path;

use e2e::Journey;
use fake_llm::{Reply, Step, ToolCall};
use serde_json::{Value, json};

/// A job that prints `PASS <n>` once the gate file `.gate<n>` exists in the workspace folder, for n = 1..3: the
/// journey decides when each line comes, so nothing depends on timing.
const JOB: &str =
    "for i in 1 2 3; do while [ ! -e .gate$i ]; do sleep 0.05; done; echo PASS $i; done";

/// The widget the scripted agent writes: a progress bar over the job's `PASS` lines.
fn tests_widget(title: &str) -> String {
    format!(
        r#"import {{ Card, Log, Progress }} from "omfp/kit";
import {{ useJob }} from "omfp/sdk";

const COMMAND = {command};

export default function Tests() {{
  const job = useJob(COMMAND);
  const done = job.stdout.filter((line: string) => line.startsWith("PASS")).length;
  return (
    <Card title={{job.status === "running" ? `{title} ${{done}}/3` : `{title} ${{job.status}} (exit ${{job.exitCode}})`}}>
      <div data-testid="tests" data-done={{done}} data-status={{job.status}}>
        <Progress label="{title}" value={{done}} max={{3}} />
        <Log lines={{job.lines}} />
      </div>
    </Card>
  );
}}
"#,
        command = json!(JOB),
    )
}

/// The `data-done` count the tests widget shows, once it shows one.
async fn tests_done(journey: &Journey, done: u32) -> anyhow::Result<()> {
    let css = "[data-testid=widget][data-widget=\"./widgets/tests\"] [data-testid=tests]";
    journey
        .until(&format!("the tests widget at {done}/3"), async |page| {
            Ok(
                (page.attr(css, "data-done").await?.as_deref() == Some(&done.to_string()))
                    .then_some(()),
            )
        })
        .await
}

fn open_gate(folder: &Path, n: u32) -> anyhow::Result<()> {
    std::fs::write(folder.join(format!(".gate{n}")), "")?;
    Ok(())
}

/// The text of the tool result answering `call_id`, from the first chat-completions request carrying it.
fn tool_result(requests: &[Value], call_id: &str) -> String {
    requests
        .iter()
        .filter_map(|request| request["messages"].as_array())
        .flatten()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .map_or_else(
            || panic!("no result for {call_id} in {} requests", requests.len()),
            |message| message["content"].as_str().unwrap_or_default().to_owned(),
        )
}

fn assert_contains(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(text.contains(needle), "{needle:?} missing: {text}");
    }
}

// spec: E2E-10, AGT-012, AGT-013, AGT-014, AGT-019, AGT-026, AGT-061, DMN-063, UI-105
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_widget_follows_a_daemon_job_live_and_recovers_from_a_syntax_error()
-> anyhow::Result<()> {
    let check = |id: &str| {
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            id,
            "omfp_widget_check",
            json!({ "name": "tests" }),
        )]))
    };
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_write_widget",
            "write",
            json!({ "path": ".omfp/widgets/tests/index.tsx", "content": tests_widget("Tests") }),
        )])),
        check("call_check_hidden"),
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_mount",
            "omfp_ui_mount",
            json!({ "id": "tests", "widget": "./widgets/tests", "place": "end" }),
        )])),
        check("call_check_shown"),
        Step::reply(Reply::text("Added a tests widget.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("widgets journey").await?;
    let folder = journey.workspace_folder(&thread).await?;
    journey
        .element("[data-testid=view][data-view=default] [data-widget=\"omfp:canvas\"]")
        .await?;
    journey.screenshot("m4-default-view").await?;

    // The agent writes the widget, checks it (it compiles; the window shows the workspace but not the widget), mounts
    // it in the view and checks again: the window rendered it and its job runs, started by the daemon.
    journey.send("add a tests widget").await?;
    journey.reply("Added a tests widget.").await?;
    let requests = journey.fake.requests();
    assert_contains(
        &tool_result(&requests, "call_check_hidden"),
        &[
            "Widget ./widgets/tests (folder ",
            "Daemon compile: every file compiles",
            "1 window(s) show this workspace, but none shows ./widgets/tests",
            "omfp_ui_mount { \"id\": \"tests\", \"widget\": \"./widgets/tests\" }",
        ],
    );
    assert_contains(
        &tool_result(&requests, "call_mount"),
        &["Mounted ./widgets/tests as \"tests\" in view \"default\"; open windows show it now."],
    );
    let shown = tool_result(&requests, "call_check_shown");
    assert_contains(
        &shown,
        &[
            "Daemon compile: every file compiles",
            "Window 1 (",
            "- tests: rendered (version ",
            "  errors: none\n",
            &format!("`{JOB}`: running, run 1"),
            "Tests 0/3",
        ],
    );
    assert!(
        !shown.contains("after waiting"),
        "the window settled: {shown}"
    );
    journey
        .element("[data-testid=view][data-view=default] [data-widget=\"./widgets/tests\"]")
        .await?;
    tests_done(&journey, 0).await?;
    open_gate(&folder, 1)?;
    tests_done(&journey, 1).await?;
    journey.screenshot("m4-widget-mid-run").await?;
    open_gate(&folder, 2)?;
    tests_done(&journey, 2).await?;

    // A syntax error saved into the widget shows in its place with the file and line; the next save recovers, and
    // the widget follows the same job (its id is stored by the daemon), so the job is not started again.
    let widget = folder.join(".omfp/widgets/tests/index.tsx");
    let broken = tests_widget("Tests").replace("const COMMAND", "const = COMMAND");
    std::fs::write(&widget, broken)?;
    let error =
        "[data-widget=\"./widgets/tests\"] [data-testid=widget-error][data-error-kind=compile]";
    journey.element(error).await?;
    let shown = journey.text(error).await?;
    assert!(shown.contains("index.tsx:4"), "file and line: {shown}");
    std::fs::write(&widget, tests_widget("Tests (fixed)"))?;
    journey.gone(error).await?;
    tests_done(&journey, 2).await?;
    open_gate(&folder, 3)?;
    tests_done(&journey, 3).await?;
    journey
        .until("the job to succeed", async |page| {
            let css = "[data-widget=\"./widgets/tests\"] [data-testid=tests]";
            Ok(
                (page.attr(css, "data-status").await?.as_deref() == Some("succeeded"))
                    .then_some(()),
            )
        })
        .await?;
    journey.finish("E2E-10").await
}

fn write_manifest(folder: &Path, text: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(folder.join(".omfp"))?;
    std::fs::write(folder.join(".omfp/views.json"), text)?;
    Ok(())
}

// spec: E2E-11, AGT-011, UI-101, UI-102
#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_manifest_shows_the_default_view_and_two_views_switch() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("views journey").await?;
    let folder = journey.workspace_folder(&thread).await?;

    // An invalid manifest: the default view and a banner with the daemon's error.
    write_manifest(
        &folder,
        r#"{ "views": { "a": { "layout": ["row", {}] } } }"#,
    )?;
    journey.element("[data-testid=view-error]").await?;
    let banner = journey.text("[data-testid=view-error]").await?;
    assert!(
        banner.contains("views.a.layout: row has no children"),
        "{banner}"
    );
    journey
        .element("[data-testid=view][data-view=default]")
        .await?;
    journey.screenshot("m4-error-banner").await?;

    // Fixed with two views: the banner goes, the default view of the manifest shows.
    let manifest = json!({
        "default": "a",
        "views": {
            "a": { "title": "Alpha", "layout": ["row", {}, ["col", {}, "omfp:header", "omfp:chat"], "omfp:workspace"] },
            "b": { "title": "Beta", "layout": ["col", {}, "omfp:header", "omfp:workspace"] }
        }
    });
    write_manifest(&folder, &serde_json::to_string_pretty(&manifest)?)?;
    journey.gone("[data-testid=view-error]").await?;
    journey.element("[data-testid=view][data-view=a]").await?;

    // The mode line's view tabs show the other view, and `:view` brings the first one back.
    journey
        .click("[data-testid=mode-view][data-view=b]")
        .await?;
    journey.element("[data-testid=view][data-view=b]").await?;
    assert_eq!(
        journey
            .count("[data-testid=view] [data-widget=\"omfp:chat\"]")
            .await?,
        0
    );
    journey.command("view a").await?;
    journey.element("[data-testid=view][data-view=a]").await?;
    journey.finish("E2E-11").await
}

/// The manifest omfp wrote to the workspace's `views.json`, once it holds a view whose layout satisfies `check`.
async fn written_layout(
    journey: &Journey,
    folder: &Path,
    what: &str,
    check: impl Fn(&Value) -> bool,
) -> anyhow::Result<Value> {
    let file = folder.join(".omfp/views.json");
    journey
        .until(what, async |_| {
            let Ok(text) = std::fs::read_to_string(&file) else {
                return Ok(None);
            };
            let manifest: Value = serde_json::from_str(&text)?;
            let layout = manifest["views"]["default"]["layout"].clone();
            Ok(check(&layout).then_some(layout))
        })
        .await
}

fn references(layout: &Value) -> Vec<String> {
    match layout {
        Value::String(reference) => vec![reference.clone()],
        Value::Array(items) => match items.first() {
            Some(Value::String(head))
                if ["row", "col", "tabs", "stack"].contains(&head.as_str()) =>
            {
                items.iter().skip(2).flat_map(references).collect()
            }
            Some(Value::String(head)) => vec![head.clone()],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

// spec: E2E-12, UI-300, UI-106, AGT-021, AGT-022, DMN-088
#[tokio::test(flavor = "multi_thread")]
async fn the_user_adds_a_widget_and_replaces_a_built_in_with_a_copy() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    let thread = journey.create_thread("layout journey").await?;
    let folder = journey.workspace_folder(&thread).await?;
    // A new workspace has the default views.json (DMN-088).
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join(".omfp/views.json"))?)?;
    assert_eq!(
        references(&written["views"]["default"]["layout"]).join(" "),
        "omfp:header omfp:chat omfp:canvas"
    );

    // `:widget add` puts a widget in the view: the last tab of the side tabs, written to views.json.
    journey.command("widget add omfp:todos").await?;
    let layout = written_layout(&journey, &folder, "the added todos", |layout| {
        references(layout).join(" ") == "omfp:header omfp:chat omfp:canvas omfp:todos"
    })
    .await?;
    assert_eq!(
        layout,
        json!([
            "row",
            { "sizes": [3, 1] },
            ["col", {}, "omfp:header", ["tabs", { "id": "main" }, "omfp:chat"]],
            ["tabs", { "id": "side" }, "omfp:canvas", "omfp:todos"]
        ])
    );
    journey
        .until("the Todos tab", async |page| {
            let tabs = page
                .texts("[data-testid=view] [data-testid=view-tab]")
                .await?;
            Ok(tabs.iter().any(|tab| tab.trim() == "Todos").then_some(()))
        })
        .await?;

    // A copy of the built-in workspace widget, added to the view, replaces it in this workspace; a change to the copy
    // shows.
    journey.command("widget add omfp:workspace").await?;
    journey
        .element("[data-testid=view] [data-testid=widget][data-widget=\"omfp:workspace\"]")
        .await?;
    journey.command("widget copy omfp:workspace").await?;
    let copy = folder.join(".omfp/widgets/workspace");
    journey
        .until("the copied widget folder", async |_| {
            Ok(copy.join("index.ts").is_file().then_some(()))
        })
        .await?;
    let slot = "[data-testid=widget][data-widget=\"omfp:workspace\"][data-source=workspace]";
    journey
        .element(&format!("{slot} [data-testid=rename-workspace]"))
        .await?;
    std::fs::write(
        copy.join("index.tsx"),
        "import Original from \"./index.ts\";\n\n\
         export default function Modified(props: Record<string, unknown>) {\n\
         \x20 return (\n\
         \x20   <div data-testid=\"modified-workspace\">\n\
         \x20     <Original {...props} />\n\
         \x20   </div>\n\
         \x20 );\n\
         }\n",
    )?;
    journey
        .element(&format!(
            "{slot} [data-testid=modified-workspace] [data-testid=rename-workspace]"
        ))
        .await?;
    journey.finish("E2E-12").await
}
