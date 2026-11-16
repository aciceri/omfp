//! Transcript, composer and file performance (M9, UI-118) against a daemon on the fake launcher: a thread resumes a
//! session of 10,000 messages (prose, Markdown with code blocks, tool calls with their results), and the test plays
//! omp's streaming frames into it; a linked repository holds big files changed since `HEAD`. A probe installed before
//! the page's scripts (`perf/probe.js`) stands in for the React developer tools to count commits and component renders
//! per transcript item, and records frames, long tasks, the time spent handling WebSocket messages, and per key press
//! the time to the next frame and the event timing entries.
//!
//! The tests run in headless Chromium and assert structure only (TST-052, TST-107): the DOM stays bounded with 10,000
//! messages (UI-190), a streamed delta re-renders the streaming row and no other transcript item (UI-191), a key press
//! in the composer renders the input and nothing else (UI-197), and a 20,000-line file scrolls in the editor's own
//! scroller, inside a layout root, with a bounded DOM, read-only and as a diff (UI-118). The benchmarks (`benchmark_*`)
//! print timings and assert nothing; they are ignored, and `just bench` runs them by hand (TST-115). The transcript
//! benchmark also runs on a short thread in `just test-e2e`, so its code keeps working (TST-100).
//!
//! `OMFP_PERF_MESSAGES` (default 10000 for the transcript, 300 for typing), `OMFP_PERF_RATE` (events per second,
//! default 100) and `OMFP_PERF_SECONDS` (default 10) change the load; with `OMFP_PERF_PROFILE=1` the Chromium
//! benchmarks also print the functions with the most self time while scrolling, streaming or pressing `j` (Chromium's
//! sampling profiler; build the bundle without `minify` for readable names, and point `OMFP_WEB_BUNDLE` at it). The
//! file benchmarks open `small.ts` (60 lines), `hunks.ts` and `rewrite.ts` (`OMFP_PERF_HUNKS_LINES`,
//! `OMFP_PERF_REWRITE_LINES`, default 1000: a changed line in every 20, one rewritten block) and `big.ts`
//! (`OMFP_PERF_BIG_LINES`, default 20000, long lines among them), or those of `OMFP_PERF_FILES`, in the modes of
//! `OMFP_PERF_MODES` (default `diff,view,edit`), scrolling `OMFP_PERF_FRAMES` frames (default 240) each way. The
//! `webkit` benchmarks drive nixpkgs' `MiniBrowser` (the engine of `omfp desktop`) through `WebKitWebDriver` under
//! `xvfb-run`, never the user's display, on a copy of the bundle whose `index.html` loads the probe (no CDP there);
//! their wheel turns are wheel buttons clicked on that X server with `xdotool`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use e2e::{Journey, WebKitDriver, row};
use omfp_core::wire::{AgentMessage, RpcNotification};
use omfp_daemon::agent::{FakeLauncher, FakeProcess, FakeRun};
use omfp_protocol::method::ThreadsCreate;
use omfp_protocol::{CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use serde_json::{Value, json};
use thirtyfour::prelude::*;

const PROBE: &str = include_str!("perf/probe.js");
const TRANSCRIPT: &str = "[data-testid=transcript]";
const STREAMING: &str = "[data-testid=transcript-item][data-key=streaming]";
const COMPOSER: &str = "[data-testid=composer-input]";
/// The streamed reply's omp message id.
const STREAM_ID: &str = "perf-stream";

/// Transcript items in the DOM with 10,000 messages: the viewport's and the overscan on each side (UI-190).
const MAX_ITEMS: u64 = 32;
/// Elements under the transcript with 10,000 messages (UI-190).
const MAX_NODES: u64 = 1500;
/// Components one key press in the composer renders: the input with its React Aria field parts (UI-197).
const MAX_RENDERS_PER_KEY: u64 = 24;

fn assistant(content: &Value) -> Value {
    json!({
        "role": "assistant",
        "api": "openai-completions",
        "provider": "fake",
        "model": "scripted",
        "content": content,
        "stopReason": "stop",
        "timestamp": 0,
        "completedAt": 0,
        "usage": {
            "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
            "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 },
        },
    })
}

/// One turn of four messages: a question, a reply with a tool call, the tool's result, and a Markdown answer with a
/// list and a code block.
fn turn(index: usize) -> [Value; 4] {
    let call = format!("call_{index}");
    let (tool, arguments, output) = match index % 4 {
        0 => (
            "bash",
            json!({ "command": format!("cargo test -p crate{index} -- --nocapture") }),
            (0..12)
                .map(|line| format!("test module{line}::case_{index} ... ok\n"))
                .collect::<Vec<_>>()
                .concat(),
        ),
        1 => (
            "read",
            json!({ "path": format!("src/module{index}.rs") }),
            (1..=20)
                .map(|line| format!("{line}:    let value_{line} = compute({index}, {line});\n"))
                .collect::<Vec<_>>()
                .concat(),
        ),
        2 => (
            "grep",
            json!({ "pattern": format!("fn handler_{index}"), "path": "src" }),
            (0..6)
                .map(|line| format!("src/handlers/h{line}.rs:{index}: fn handler_{index}() {{\n"))
                .collect::<Vec<_>>()
                .concat(),
        ),
        _ => (
            "write",
            json!({ "path": format!("notes/n{index}.md"), "content": format!("# Note {index}\n\nSome text.\n") }),
            format!("Wrote notes/n{index}.md"),
        ),
    };
    [
        json!({
            "role": "user",
            "attribution": "user",
            "timestamp": 0,
            "content": [{ "type": "text", "text": format!("Question {index}: why does `handler_{index}` return early when the cache is cold?") }],
        }),
        assistant(&json!([
            { "type": "text", "text": format!("Let me check `src/module{index}.rs` first.") },
            { "type": "toolCall", "id": call, "name": tool, "arguments": arguments },
        ])),
        json!({
            "role": "toolResult",
            "toolCallId": call,
            "toolName": tool,
            "isError": index.is_multiple_of(17),
            "timestamp": 0,
            "content": [{ "type": "text", "text": output }],
        }),
        assistant(&json!([{ "type": "text", "text": format!(
            "## Finding {index}\n\nThe handler returns early because the **cache** is checked before the lock is \
             taken, so a cold cache skips the refill. Three things follow:\n\n\
             - the first request after a restart always misses;\n\
             - `refill()` runs once per worker, not once per process;\n\
             - the metric `cache_cold_{index}` counts the misses.\n\n\
             A fix takes the lock first:\n\n\
             ```rust\nfn handler_{index}(cache: &Cache) -> Result<Value> {{\n    let guard = cache.lock()?;\n    \
             if guard.is_cold() {{\n        guard.refill()?;\n    }}\n    Ok(guard.get({index}))\n}}\n```\n\n\
             After that change the early return only happens on a real error."
        ) }])),
    ]
}

/// `messages` messages of history (whole turns).
fn history(messages: usize) -> Vec<AgentMessage> {
    (0..messages.div_ceil(4))
        .flat_map(turn)
        .take(messages)
        .map(|message| serde_json::from_value(message).expect("a valid agent message"))
        .collect()
}

fn frame(value: &Value) -> RpcNotification {
    serde_json::from_value(value.clone()).expect("a valid omp frame")
}

/// omp's frames that open a streamed reply with an empty text block.
fn stream_start() -> Vec<RpcNotification> {
    vec![
        frame(
            &json!({ "type": "message_start", "messageId": STREAM_ID, "message": assistant(&json!([])) }),
        ),
        frame(&json!({
            "type": "message_update",
            "messageId": STREAM_ID,
            "message": { "role": "assistant" },
            "assistantMessageEvent": { "type": "text_start", "contentIndex": 0, "block": { "type": "text", "text": "" } },
        })),
    ]
}

fn text_delta(delta: &str) -> RpcNotification {
    frame(&json!({
        "type": "message_update",
        "messageId": STREAM_ID,
        "message": { "role": "assistant" },
        "assistantMessageEvent": { "type": "text_delta", "contentIndex": 0, "delta": delta },
    }))
}

/// omp's frames that end the streamed reply with `text`.
fn stream_end(text: &str) -> Vec<RpcNotification> {
    vec![
        frame(&json!({
            "type": "message_update",
            "messageId": STREAM_ID,
            "message": { "role": "assistant" },
            "assistantMessageEvent": { "type": "text_end", "contentIndex": 0, "content": text, "block": { "type": "text" } },
        })),
        frame(&json!({
            "type": "message_end",
            "messageId": STREAM_ID,
            "message": assistant(&json!([{ "type": "text", "text": text }])),
        })),
    ]
}

/// The streamed reply, cut into deltas of a few characters like a model's tokens: prose, lists and code blocks.
fn streamed_deltas(count: usize) -> Vec<String> {
    let source = "The streamed answer walks through the change step by step, with `inline code`, **bold** words \
                  and a list:\n\n- first, the reducer keeps the transcript;\n- then, the view model indexes it;\n\
                  - last, the rows render.\n\n```ts\nexport function sum(values: readonly number[]): number {\n  \
                  return values.reduce((total, value) => total + value, 0);\n}\n```\n\nAnd a closing paragraph \
                  that keeps going for a while so the message grows well past a screen.\n\n";
    let chars: Vec<char> = source.chars().collect();
    let mut deltas = Vec::with_capacity(count);
    let mut at = 0;
    for index in 0..count {
        let size = 3 + index % 5;
        deltas.push(
            (0..size)
                .map(|offset| chars[(at + offset) % chars.len()])
                .collect(),
        );
        at += size;
    }
    deltas
}

/// Installs the probe for every later page load and reloads the page under it.
async fn install_probe(journey: &Journey) -> anyhow::Result<()> {
    let cdp = journey.driver.cdp();
    cdp.send_raw(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": PROBE }),
    )
    .await?;
    cdp.send_raw("Performance.enable", json!({})).await?;
    journey.driver.refresh().await?;
    journey.connected().await
}

/// Creates a thread in a new workspace through the protocol (the launcher's next run plays it); returns its id.
async fn create_thread(journey: &Journey, workspace: &str) -> anyhow::Result<String> {
    let mut connection = journey.connect().await?;
    let thread = connection
        .call::<ThreadsCreate>(CreateThreadParams {
            title: None,
            workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                name: Some(workspace.to_owned()),
                link: None,
            }),
            model: None,
            thinking_level: None,
            approval_mode: None,
            prompt: None,
            images: Vec::new(),
        })
        .await?;
    connection.close().await?;
    until_present(journey, &row(&thread.id)).await?;
    Ok(thread.id)
}

/// Waits until `css` matches. Queries in the page each time: an element handle goes stale when React replaces the node
/// (the virtualized list re-renders its items as they are measured).
async fn until_present(journey: &Journey, css: &str) -> anyhow::Result<()> {
    journey
        .until(css, async |page| {
            Ok((page.count(css).await? > 0).then_some(()))
        })
        .await
}

/// Clicks the first element matching `css` in the page (no element handle to go stale).
async fn click(journey: &Journey, css: &str) -> anyhow::Result<()> {
    until_present(journey, css).await?;
    journey
        .eval(
            "document.querySelector(arguments[0]).click();",
            vec![json!(css)],
        )
        .await?;
    Ok(())
}

/// The selector of thread `id`'s transcript items.
fn items_of(id: &str) -> String {
    format!("{TRANSCRIPT}[data-thread-id=\"{id}\"] [data-testid=transcript-item]")
}

/// Clicks thread `id`'s row in the page and measures until `shown` (a CSS selector) is in the DOM, and until the
/// frame after it: `(in_dom_ms, painted_ms)`.
async fn open_timed(journey: &Journey, id: &str, shown: &str) -> anyhow::Result<(f64, f64)> {
    let script = "const [rowCss, itemCss, done] = arguments;
        const started = performance.now();
        const check = () => {
          if (document.querySelector(itemCss) === null) { return false; }
          const inDom = performance.now() - started;
          requestAnimationFrame(() => setTimeout(() => done([inDom, performance.now() - started]), 0));
          return true;
        };
        const observer = new MutationObserver(() => { if (check()) { observer.disconnect(); } });
        observer.observe(document.body, { childList: true, subtree: true });
        document.querySelector(rowCss).click();";
    let value = journey
        .driver
        .execute_async(script, vec![json!(row(id)), json!(shown)])
        .await?
        .json()
        .clone();
    Ok(serde_json::from_value(value)?)
}

/// What the transcript keeps in the DOM.
#[derive(Debug)]
struct Dom {
    items: u64,
    nodes: u64,
    page: u64,
}

fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap_or_default()
}

fn count(value: &Value, key: &str) -> u64 {
    value[key].as_u64().unwrap_or_default()
}

async fn dom(journey: &Journey) -> anyhow::Result<Dom> {
    let value = journey
        .eval(
            "const t = document.querySelector(arguments[0]);
             return { items: t.querySelectorAll('[data-testid=transcript-item]').length,
                      nodes: t.querySelectorAll('*').length,
                      page: document.getElementsByTagName('*').length };",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    Ok(Dom {
        items: count(&value, "items"),
        nodes: count(&value, "nodes"),
        page: count(&value, "page"),
    })
}

/// What the probe recorded.
#[derive(Debug)]
struct Recording {
    commits: u64,
    /// Component renders by transcript item key (`outside` for the rest).
    renders: BTreeMap<String, u64>,
    /// Component renders outside the transcript items, by component name.
    outside: BTreeMap<String, u64>,
    /// Components rendered per commit (when renders are tracked).
    commit_sizes: Vec<u64>,
    /// `requestAnimationFrame` timestamps (ms).
    frames: Vec<f64>,
    /// Durations of the long tasks (ms).
    long_tasks: Vec<f64>,
    /// WebSocket messages handled: count, total and longest ms, and the text received (UTF-16 code units).
    messages: (u64, f64, f64, u64),
    /// Per key press: ms from the key to the task after the next frame.
    keys: Vec<f64>,
    /// Keyboard and input events of 16 ms or more (Chromium's event timing): name and duration.
    slow_events: Vec<(String, f64)>,
}

impl Recording {
    fn from_json(value: &Value) -> Self {
        let list = |key: &str| value[key].as_array().cloned().unwrap_or_default();
        let counts = |key: &str| {
            value[key]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, count)| (name.clone(), count.as_u64().unwrap_or_default()))
                .collect()
        };
        let messages = &value["messages"];
        Self {
            commits: count(value, "commits"),
            renders: counts("renders"),
            outside: counts("outside"),
            commit_sizes: list("commitSizes")
                .iter()
                .filter_map(Value::as_u64)
                .collect(),
            frames: list("frames").iter().filter_map(Value::as_f64).collect(),
            long_tasks: list("longTasks")
                .iter()
                .map(|task| number(task, "duration"))
                .collect(),
            messages: (
                count(messages, "count"),
                number(messages, "total"),
                number(messages, "max"),
                count(messages, "bytes"),
            ),
            keys: list("keys").iter().filter_map(Value::as_f64).collect(),
            slow_events: list("events")
                .iter()
                .map(|event| {
                    (
                        event["name"].as_str().unwrap_or_default().to_owned(),
                        number(event, "duration"),
                    )
                })
                .collect(),
        }
    }
}

async fn start_recording(driver: &WebDriver, track_renders: bool) -> anyhow::Result<()> {
    driver
        .execute(
            "window.__omfpPerf.start(arguments[0]);",
            vec![json!(track_renders)],
        )
        .await?;
    Ok(())
}

async fn stop_recording(driver: &WebDriver) -> anyhow::Result<Recording> {
    let value = driver
        .execute("return window.__omfpPerf.stop();", Vec::new())
        .await?;
    Ok(Recording::from_json(value.json()))
}

/// Opens the streamed reply and waits until its row shows.
async fn open_stream(journey: &Journey, process: &FakeProcess) -> anyhow::Result<()> {
    process.emit(stream_start());
    until_present(journey, STREAMING).await?;
    Ok(())
}

/// Ends the streamed reply and waits until its row is gone (the message joined the transcript).
async fn close_stream(
    journey: &Journey,
    process: &FakeProcess,
    deltas: &[String],
) -> anyhow::Result<()> {
    process.emit(stream_end(&deltas.concat()));
    journey.gone(STREAMING).await
}

/// Waits until the streaming row's text ends with the end of `text` (whitespace aside).
async fn until_streamed(journey: &Journey, text: &str) -> anyhow::Result<()> {
    let tail: String = text
        .chars()
        .filter(|c| c.is_alphanumeric())
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    journey
        .until("the streamed text", async |page| {
            let shown: String = page
                .text(STREAMING)
                .await?
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            Ok(shown.ends_with(&tail).then_some(()))
        })
        .await
}

/// Scrolls the transcript to its top as the user would (a wheel up unpins it) and waits until item 0 shows.
async fn scroll_to_top(journey: &Journey) -> anyhow::Result<()> {
    journey
        .eval(
            "const t = document.querySelector(arguments[0]);
             t.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, bubbles: true }));
             t.scrollTop = 0;",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    journey
        .until("the first item", async |page| {
            let first = page
                .count(&format!("{TRANSCRIPT} [data-index=\"0\"]"))
                .await?;
            Ok((first == 1).then_some(()))
        })
        .await
}

/// Goes back to the bottom with `End` and waits until the view is pinned there.
async fn scroll_to_bottom(journey: &Journey) -> anyhow::Result<()> {
    until_present(journey, TRANSCRIPT).await?;
    journey
        .eval(
            "document.querySelector(arguments[0]).focus();",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    journey.keys(Key::End).await?;
    journey
        .until("the pinned transcript", async |page| {
            let pinned = page.attr(TRANSCRIPT, "data-pinned").await?;
            Ok((pinned.as_deref() == Some("true")).then_some(()))
        })
        .await
}

// spec: TST-052, UI-190, UI-191
#[tokio::test(flavor = "multi_thread")]
async fn a_long_transcript_keeps_a_bounded_dom_and_a_delta_rerenders_only_the_streaming_row()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/long"),
        FakeRun::fresh().with_history(history(10_000)),
    );
    install_probe(&journey).await?;
    let thread = create_thread(&journey, "long").await?;
    click(&journey, &row(&thread)).await?;
    until_present(&journey, &items_of(&thread)).await?;

    // At the bottom, at the top: a bounded window of the 10,000 messages (UI-190).
    let bottom = dom(&journey).await?;
    scroll_to_top(&journey).await?;
    let top = dom(&journey).await?;
    for (place, dom) in [("bottom", &bottom), ("top", &top)] {
        assert!(
            dom.items <= MAX_ITEMS && dom.nodes <= MAX_NODES,
            "at the {place}: {dom:?}"
        );
    }
    scroll_to_bottom(&journey).await?;

    // Each delta commits and renders the streaming row only (UI-191).
    let process = journey.fake_process(&launcher, &thread).await?;
    open_stream(&journey, &process).await?;
    start_recording(&journey.driver, true).await?;
    let deltas = streamed_deltas(20);
    for count in 1..=deltas.len() {
        process.emit(vec![text_delta(&deltas[count - 1])]);
        until_streamed(&journey, &deltas[..count].concat()).await?;
    }
    let recording = stop_recording(&journey.driver).await?;
    let items: Vec<&String> = recording
        .renders
        .keys()
        .filter(|key| *key != "outside")
        .collect();
    assert_eq!(items, ["streaming"], "renders: {:?}", recording.renders);
    assert!(
        recording.commits >= 20,
        "one commit per delta at least: {recording:?}"
    );
    close_stream(&journey, &process, &deltas).await?;
    journey.finish("transcript performance").await
}

#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark counts stay far below 2^52"
)]
fn float(count: u64) -> f64 {
    count as f64
}

/// Count, mean, median, 95th percentile and maximum of `values`.
fn stats(values: &mut [f64]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values.sort_by(f64::total_cmp);
    let last = values.len() - 1;
    let at = |percent: usize| values[(last * percent).div_ceil(100)];
    let mean = values.iter().sum::<f64>() / float(values.len() as u64);
    format!(
        "n={} mean={mean:.2} p50={:.2} p95={:.2} max={:.2}",
        values.len(),
        at(50),
        at(95),
        at(100)
    )
}

/// Frames late by at least one vsync (60 Hz): the sum, over frame intervals, of the vsyncs skipped.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a rounded, positive number of vsyncs"
)]
fn dropped_frames(frames: &[f64]) -> u64 {
    const VSYNC_MS: f64 = 1000.0 / 60.0;
    frames
        .windows(2)
        .map(|pair| ((pair[1] - pair[0]) / VSYNC_MS).round().max(1.0) as u64 - 1)
        .sum()
}

fn report(name: &str, recording: &Recording, events: u64) {
    let mut intervals: Vec<f64> = recording
        .frames
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    let mut long = recording.long_tasks.clone();
    let long_total: f64 = long.iter().sum();
    eprintln!("{name}:");
    eprintln!("  frame interval ms: {}", stats(&mut intervals));
    eprintln!("  dropped frames: {}", dropped_frames(&recording.frames));
    eprintln!(
        "  long tasks: {} (total {long_total:.0} ms) {}",
        long.len(),
        stats(&mut long)
    );
    eprintln!("  commits: {}", recording.commits);
    if !recording.keys.is_empty() {
        let mut keys = recording.keys.clone();
        eprintln!("  key to next frame ms: {}", stats(&mut keys));
        eprintln!(
            "  keyboard/input events of 16 ms or more: {:?}",
            recording.slow_events
        );
    }
    if events > 0 {
        let (handled, total, max, _) = recording.messages;
        eprintln!(
            "  events: {events}, commits/event {:.2}; ws messages {handled} mean {:.3} ms max {max:.2} ms",
            float(recording.commits) / float(events),
            total / float(handled.max(1)),
        );
    }
    if !recording.commit_sizes.is_empty() {
        let mut sizes: Vec<f64> = recording.commit_sizes.iter().copied().map(float).collect();
        eprintln!("  components rendered per commit: {}", stats(&mut sizes));
        eprintln!("  renders by item: {:?}", recording.renders);
        let mut outside: Vec<(&String, &u64)> = recording.outside.iter().collect();
        outside.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!(
            "  most rendered outside the transcript: {:?}",
            &outside[..outside.len().min(15)]
        );
    }
}

/// Main-thread time of the page from Chromium's own counters (seconds): script, layout, style, all tasks.
async fn metrics(journey: &Journey) -> anyhow::Result<BTreeMap<String, f64>> {
    let value = journey
        .driver
        .cdp()
        .send_raw("Performance.getMetrics", json!({}))
        .await?;
    let mut metrics = BTreeMap::new();
    for metric in value["metrics"].as_array().into_iter().flatten() {
        if let (Some(name), Some(value)) = (metric["name"].as_str(), metric["value"].as_f64())
            && matches!(
                name,
                "ScriptDuration" | "LayoutDuration" | "RecalcStyleDuration" | "TaskDuration"
            )
        {
            metrics.insert(name.to_owned(), value);
        }
    }
    Ok(metrics)
}

fn report_metrics(before: &BTreeMap<String, f64>, after: &BTreeMap<String, f64>, events: u64) {
    let line: Vec<String> = after
        .iter()
        .map(|(name, value)| {
            let spent = (value - before.get(name).copied().unwrap_or_default()) * 1000.0;
            if events > 0 {
                format!(
                    "{name} {spent:.0} ms ({:.2} ms/event)",
                    spent / float(events)
                )
            } else {
                format!("{name} {spent:.0} ms")
            }
        })
        .collect();
    eprintln!("  main thread: {}", line.join(", "));
}

/// Starts Chromium's sampling profiler when `OMFP_PERF_PROFILE` is set.
async fn start_profile(journey: &Journey) -> anyhow::Result<()> {
    if std::env::var_os("OMFP_PERF_PROFILE").is_some() {
        let cdp = journey.driver.cdp();
        cdp.send_raw("Profiler.enable", json!({})).await?;
        cdp.send_raw("Profiler.setSamplingInterval", json!({ "interval": 100 }))
            .await?;
        cdp.send_raw("Profiler.start", json!({})).await?;
    }
    Ok(())
}

/// Stops the profiler started by [`start_profile`] and prints the functions with the most self time.
async fn report_profile(journey: &Journey) -> anyhow::Result<()> {
    if std::env::var_os("OMFP_PERF_PROFILE").is_none() {
        return Ok(());
    }
    let value = journey
        .driver
        .cdp()
        .send_raw("Profiler.stop", json!({}))
        .await?;
    let profile = &value["profile"];
    let mut frames = BTreeMap::new();
    for node in profile["nodes"].as_array().into_iter().flatten() {
        let frame = &node["callFrame"];
        let url = frame["url"].as_str().unwrap_or_default();
        let file = url.rsplit('/').next().unwrap_or_default();
        let name = frame["functionName"].as_str().unwrap_or_default();
        let label = format!(
            "{} {file}:{}:{}",
            if name.is_empty() { "(anonymous)" } else { name },
            frame["lineNumber"].as_i64().unwrap_or_default() + 1,
            frame["columnNumber"].as_i64().unwrap_or_default() + 1
        );
        frames.insert(node["id"].as_i64().unwrap_or_default(), label);
    }
    let mut self_time: BTreeMap<&str, f64> = BTreeMap::new();
    let samples = profile["samples"].as_array().cloned().unwrap_or_default();
    let deltas = profile["timeDeltas"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for (sample, delta) in samples.iter().zip(deltas.iter().skip(1)) {
        if let Some(label) = frames.get(&sample.as_i64().unwrap_or_default()) {
            *self_time.entry(label).or_default() += delta.as_f64().unwrap_or_default() / 1000.0;
        }
    }
    let mut top: Vec<(&str, f64)> = self_time.into_iter().collect();
    top.sort_by(|a, b| b.1.total_cmp(&a.1));
    eprintln!("  self time (ms):");
    for (label, ms) in top.into_iter().take(25) {
        eprintln!("    {ms:8.1}  {label}");
    }
    Ok(())
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Scrolls from the top to the bottom over at most `frames` frames (one step per frame, the remaining distance
/// spread over the frames left), recording; returns the recording and the frames used.
async fn scroll_down(journey: &Journey, frames: u64) -> anyhow::Result<(Recording, u64)> {
    let script = "const [css, frames, done] = arguments;
        const t = document.querySelector(css);
        const perf = window.__omfpPerf;
        perf.start(false);
        let frame = 0;
        const step = () => {
          const remaining = t.scrollHeight - t.clientHeight - t.scrollTop;
          if (remaining <= 1 || frame >= frames) {
            done([perf.stop(), frame]);
            return;
          }
          t.scrollTop += Math.max(t.clientHeight / 2, remaining / (frames - frame));
          frame += 1;
          requestAnimationFrame(step);
        };
        requestAnimationFrame(step);";
    let value = journey
        .driver
        .execute_async(script, vec![json!(TRANSCRIPT), json!(frames)])
        .await?
        .json()
        .clone();
    Ok((
        Recording::from_json(&value[0]),
        value[1].as_u64().unwrap_or_default(),
    ))
}

/// Streams `deltas` at `rate` events per second, recording; returns the recording.
async fn stream_timed(
    journey: &Journey,
    process: &FakeProcess,
    deltas: &[String],
    rate: u64,
    track_renders: bool,
) -> anyhow::Result<Recording> {
    open_stream(journey, process).await?;
    start_recording(&journey.driver, track_renders).await?;
    let mut ticks = tokio::time::interval(Duration::from_micros(1_000_000 / rate.max(1)));
    for delta in deltas {
        ticks.tick().await;
        process.emit(vec![text_delta(delta)]);
    }
    until_streamed(journey, &deltas.concat()).await?;
    let recording = stop_recording(&journey.driver).await?;
    close_stream(journey, process, deltas).await?;
    Ok(recording)
}

/// [`open_timed`] while recording: prints the time to the DOM and to paint, the WebSocket messages handled (the
/// thread's snapshot when it was not followed) and the long tasks.
async fn switch_timed(journey: &Journey, name: &str, id: &str, shown: &str) -> anyhow::Result<()> {
    start_recording(&journey.driver, false).await?;
    let (in_dom, painted) = open_timed(journey, id, shown).await?;
    let recording = stop_recording(&journey.driver).await?;
    let (handled, total, max, bytes) = recording.messages;
    let long: f64 = recording.long_tasks.iter().sum();
    eprintln!(
        "{name}: in the DOM after {in_dom:.0} ms, painted after {painted:.0} ms; ws messages {handled} \
         ({:.0} kB, handled in {total:.0} ms, longest {max:.0} ms); long tasks {} (total {long:.0} ms); commits {}",
        float(bytes) / 1000.0,
        recording.long_tasks.len(),
        recording.commits,
    );
    Ok(())
}

/// The transcript benchmark on a thread of `messages` messages, streaming at `rate` events per second for `seconds`.
async fn transcript_benchmark(messages: usize, rate: u64, seconds: u64) -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/long"),
        FakeRun::fresh().with_history(history(messages)),
    );
    journey
        .driver
        .set_script_timeout(Duration::from_secs(120))
        .await?;
    install_probe(&journey).await?;
    let long = create_thread(&journey, "long").await?;
    let other = create_thread(&journey, "other").await?;
    eprintln!("transcript benchmark: {messages} messages, {rate} events/s for {seconds} s");

    let (in_dom, painted) = open_timed(&journey, &long, &items_of(&long)).await?;
    eprintln!("first open: items in the DOM after {in_dom:.0} ms, painted after {painted:.0} ms");
    let bottom = dom(&journey).await?;
    eprintln!(
        "DOM at the bottom: {} items, {} transcript elements, {} page elements",
        bottom.items, bottom.nodes, bottom.page
    );

    scroll_to_top(&journey).await?;
    let top = dom(&journey).await?;
    eprintln!(
        "DOM at the top: {} items, {} transcript elements, {} page elements",
        top.items, top.nodes, top.page
    );
    start_profile(&journey).await?;
    let (recording, frames) = scroll_down(&journey, 600).await?;
    report(
        &format!("scroll top to bottom ({frames} frames)"),
        &recording,
        0,
    );
    report_profile(&journey).await?;
    scroll_to_bottom(&journey).await?;

    let empty = format!("{TRANSCRIPT}[data-thread-id=\"{other}\"]");
    for round in 1..=3 {
        switch_timed(&journey, &format!("switch away {round}"), &other, &empty).await?;
        switch_timed(
            &journey,
            &format!("switch back {round}"),
            &long,
            &items_of(&long),
        )
        .await?;
    }

    let process = journey.fake_process(&launcher, &long).await?;
    let count = usize::try_from(rate * seconds)?;
    let deltas = streamed_deltas(count);
    let before = metrics(&journey).await?;
    start_profile(&journey).await?;
    let recording = stream_timed(&journey, &process, &deltas, rate, false).await?;
    let after = metrics(&journey).await?;
    report(
        &format!("streaming {count} deltas at {rate}/s"),
        &recording,
        count as u64,
    );
    report_metrics(&before, &after, count as u64);
    report_profile(&journey).await?;
    let deltas = streamed_deltas(200);
    let recording = stream_timed(&journey, &process, &deltas, rate, true).await?;
    report("streaming 200 deltas, renders tracked", &recording, 200);

    journey.finish("transcript benchmark").await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: prints timings, asserts nothing (`just bench`)"]
async fn benchmark_transcript() -> anyhow::Result<()> {
    transcript_benchmark(
        usize::try_from(env_number("OMFP_PERF_MESSAGES", 10_000))?,
        env_number("OMFP_PERF_RATE", 100),
        env_number("OMFP_PERF_SECONDS", 10),
    )
    .await
}

// spec: TST-100
#[tokio::test(flavor = "multi_thread")]
async fn the_transcript_benchmark_runs_on_a_short_thread() -> anyhow::Result<()> {
    transcript_benchmark(200, 100, 1).await
}

/// The text typed into the composer, one key at a time.
const TYPED: &str = "the quick brown fox jumps over the lazy dog ";

async fn eval_on(driver: &WebDriver, script: &str, args: Vec<Value>) -> anyhow::Result<Value> {
    Ok(driver.execute(script, args).await?.json().clone())
}

/// Polls `script` (returning a boolean) in `driver`'s page until it is true; for the browsers without a [`Journey`].
async fn until_true(
    driver: &WebDriver,
    what: &str,
    script: &str,
    args: Vec<Value>,
) -> anyhow::Result<()> {
    for _ in 0..1200 {
        if eval_on(driver, script, args.clone()).await? == json!(true) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("timed out waiting for {what}")
}

async fn until_shown(driver: &WebDriver, css: &str) -> anyhow::Result<()> {
    until_true(
        driver,
        css,
        "return document.querySelector(arguments[0]) !== null;",
        vec![json!(css)],
    )
    .await
}

async fn composer_length(driver: &WebDriver) -> anyhow::Result<u64> {
    Ok(eval_on(
        driver,
        "const e = document.querySelector(arguments[0]); return e ? e.value.length : 0;",
        vec![json!(COMPOSER)],
    )
    .await?
    .as_u64()
    .unwrap_or_default())
}

/// Types `count` keys into the composer, one WebDriver call each: `pause` apart, or (`None`) each once the previous
/// one reached the input. Returns once every key is in the input.
async fn type_keys(
    driver: &WebDriver,
    count: usize,
    pause: Option<Duration>,
) -> anyhow::Result<()> {
    let start = composer_length(driver).await?;
    let chars: Vec<char> = TYPED.chars().collect();
    for (typed, index) in (1..).zip(0..count) {
        driver
            .find(By::Css(COMPOSER))
            .await?
            .send_keys(chars[index % chars.len()].to_string())
            .await?;
        match pause {
            Some(pause) => tokio::time::sleep(pause).await,
            None => {
                until_true(
                    driver,
                    "the key in the composer",
                    "return document.querySelector(arguments[0]).value.length >= arguments[1];",
                    vec![json!(COMPOSER), json!(start + typed)],
                )
                .await?;
            }
        }
    }
    until_true(
        driver,
        "the typed keys",
        "return document.querySelector(arguments[0]).value.length >= arguments[1];",
        vec![json!(COMPOSER), json!(start + count as u64)],
    )
    .await
}

/// Opens thread `id` in `driver`'s page and puts the focus in its composer with a WebDriver click: the page gets the
/// window's focus too (a script's `focus()` in an unfocused headless page fires no focus events, so the keymap would
/// not enter insert mode). Waits until it has (`data-mode` of the frame).
async fn open_composer(driver: &WebDriver, id: &str) -> anyhow::Result<()> {
    until_shown(driver, &row(id)).await?;
    eval_on(
        driver,
        "document.querySelector(arguments[0]).click();",
        vec![json!(row(id))],
    )
    .await?;
    until_shown(driver, &items_of(id)).await?;
    until_shown(driver, COMPOSER).await?;
    driver.find(By::Css(COMPOSER)).await?.click().await?;
    // The page has the window's focus now; MiniBrowser's click may land elsewhere, so focus the input itself too.
    eval_on(
        driver,
        "document.querySelector(arguments[0]).focus();",
        vec![json!(COMPOSER)],
    )
    .await?;
    until_shown(driver, "[data-testid=app][data-mode=insert]").await
}

// spec: TST-052, UI-197
#[tokio::test(flavor = "multi_thread")]
async fn a_key_press_in_the_composer_renders_the_input_only() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/typing"),
        FakeRun::fresh().with_history(history(300)),
    );
    install_probe(&journey).await?;
    let thread = create_thread(&journey, "typing").await?;
    open_composer(&journey.driver, &thread).await?;
    let keys = 10;
    start_recording(&journey.driver, true).await?;
    type_keys(&journey.driver, keys, None).await?;
    let recording = stop_recording(&journey.driver).await?;
    let items: Vec<&String> = recording
        .renders
        .keys()
        .filter(|key| *key != "outside")
        .collect();
    assert_eq!(
        items,
        Vec::<&String>::new(),
        "no transcript item renders: {recording:?}"
    );
    let rendered: u64 = recording.commit_sizes.iter().sum();
    assert!(
        rendered <= MAX_RENDERS_PER_KEY * keys as u64,
        "{rendered} components rendered for {keys} keys, per commit {:?}: {:?}",
        recording.commit_sizes,
        recording.outside
    );
    journey.finish("composer typing").await
}

fn report_typing(name: &str, recording: &Recording, keys: usize, script_ms: Option<f64>) {
    let keys = float(keys as u64);
    let mut latency = recording.keys.clone();
    let mut slow: Vec<f64> = recording
        .slow_events
        .iter()
        .map(|(_, duration)| *duration)
        .collect();
    eprintln!("{name}:");
    eprintln!("  key to next frame ms: {}", stats(&mut latency));
    eprintln!(
        "  keyboard/input events of 16 ms or more (event timing duration, input to paint): {}",
        stats(&mut slow)
    );
    eprintln!(
        "  commits/key {:.2}, long tasks {}{}",
        float(recording.commits) / keys,
        recording.long_tasks.len(),
        script_ms
            .map(|ms| format!(", script {:.2} ms/key", ms / keys))
            .unwrap_or_default()
    );
    if !recording.commit_sizes.is_empty() {
        let rendered: u64 = recording.commit_sizes.iter().sum();
        let items: u64 = recording
            .renders
            .iter()
            .filter(|(key, _)| *key != "outside")
            .map(|(_, renders)| renders)
            .sum();
        eprintln!(
            "  components rendered/key {:.1}, of them in transcript items {:.1}",
            float(rendered) / keys,
            float(items) / keys
        );
        let mut outside: Vec<(&String, &u64)> = recording.outside.iter().collect();
        outside.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = outside
            .iter()
            .take(20)
            .map(|(name, renders)| format!("{name} {:.1}", float(**renders) / keys))
            .collect();
        eprintln!("  per key outside the transcript: {}", top.join(", "));
    }
}

/// Moves the focus from the transcript into the composer `times` times; per move, the ms from `focus()` to the task
/// after the next frame (the keymap enters insert mode, UI-098), and the components rendered.
async fn focus_timed(driver: &WebDriver, times: usize) -> anyhow::Result<(Vec<f64>, u64)> {
    start_recording(driver, true).await?;
    let value = driver
        .execute_async(
            "const [transcript, composer, times, done] = arguments;
             const frame = () => new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
             const spent = [];
             for (let i = 0; i < times; i++) {
               document.querySelector(transcript).focus();
               await frame(); await frame();
               const started = performance.now();
               document.querySelector(composer).focus();
               await frame();
               spent.push(performance.now() - started);
             }
             done(spent);",
            vec![json!(TRANSCRIPT), json!(COMPOSER), json!(times)],
        )
        .await?
        .json()
        .clone();
    let recording = stop_recording(driver).await?;
    let spent = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    Ok((spent, recording.commit_sizes.iter().sum()))
}

/// Typing in thread `thread` of `driver`'s page, idle and while a reply streams at 100 events/s: 40 keys 120 ms
/// apart timed (with Chromium's script time when `journey` gives CDP), then 20 keys with renders counted; and the
/// focus moving into the composer.
async fn typing_runs(
    label: &str,
    driver: &WebDriver,
    journey: Option<&Journey>,
    thread: &str,
    launcher: &FakeLauncher,
) -> anyhow::Result<()> {
    open_composer(driver, thread).await?;
    let (mut spent, rendered) = focus_timed(driver, 10).await?;
    eprintln!(
        "{label}, focus into the composer: ms to the next frame {}, components rendered/focus {:.0}",
        stats(&mut spent),
        float(rendered) / 10.0
    );
    let process = e2e::fake_thread(launcher, thread).context("the thread's omp has started")?;
    let pause = Some(Duration::from_millis(120));
    for busy in [false, true] {
        let streaming = Arc::new(AtomicBool::new(busy));
        let emitter = if busy {
            process.emit(stream_start());
            until_shown(driver, STREAMING).await?;
            let streaming = Arc::clone(&streaming);
            let process = process.clone();
            Some(tokio::spawn(async move {
                let mut ticks = tokio::time::interval(Duration::from_millis(10));
                let mut sent = Vec::new();
                for delta in streamed_deltas(100_000) {
                    if !streaming.load(Ordering::Relaxed) {
                        break;
                    }
                    ticks.tick().await;
                    process.emit(vec![text_delta(&delta)]);
                    sent.push(delta);
                }
                sent
            }))
        } else {
            None
        };
        let state = if busy { "streaming" } else { "idle" };
        let before = match journey {
            Some(journey) => metrics(journey).await?.get("ScriptDuration").copied(),
            None => None,
        };
        start_recording(driver, false).await?;
        type_keys(driver, 40, pause).await?;
        let recording = stop_recording(driver).await?;
        let after = match journey {
            Some(journey) => metrics(journey).await?.get("ScriptDuration").copied(),
            None => None,
        };
        let script = before
            .zip(after)
            .map(|(before, after)| (after - before) * 1000.0);
        report_typing(
            &format!("{label}, {state} thread, 40 keys 120 ms apart"),
            &recording,
            40,
            script,
        );
        start_recording(driver, true).await?;
        type_keys(driver, 20, pause).await?;
        let recording = stop_recording(driver).await?;
        report_typing(
            &format!("{label}, {state} thread, 20 keys, renders counted"),
            &recording,
            20,
            None,
        );
        if let Some(emitter) = emitter {
            streaming.store(false, Ordering::Relaxed);
            let sent = emitter.await?;
            process.emit(stream_end(&sent.concat()));
            until_true(
                driver,
                "the end of the stream",
                "return document.querySelector(arguments[0]) === null;",
                vec![json!(STREAMING)],
            )
            .await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: prints timings, asserts nothing (`just bench`)"]
async fn benchmark_typing_chromium() -> anyhow::Result<()> {
    let messages = usize::try_from(env_number("OMFP_PERF_MESSAGES", 300))?;
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/typing"),
        FakeRun::fresh().with_history(history(messages)),
    );
    install_probe(&journey).await?;
    let thread = create_thread(&journey, "typing").await?;
    eprintln!("typing benchmark: {messages} messages");
    typing_runs(
        "chromium",
        &journey.driver,
        Some(&journey),
        &thread,
        &launcher,
    )
    .await?;
    journey.finish("typing benchmark (chromium)").await
}

/// A copy of the web bundle at `source` in `into` whose `index.html` runs the probe before anything else (for
/// browsers without CDP): the other entries are links to the original.
fn probed_bundle(source: &Path, into: &Path) -> anyhow::Result<PathBuf> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() != "index.html" {
            std::os::unix::fs::symlink(entry.path(), into.join(entry.file_name()))?;
        }
    }
    let html = std::fs::read_to_string(source.join("index.html"))?;
    std::fs::write(
        into.join("index.html"),
        html.replacen("<head>", &format!("<head>\n<script>{PROBE}</script>"), 1),
    )?;
    Ok(into.to_path_buf())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: prints timings, asserts nothing (`just bench`)"]
async fn benchmark_typing_webkit() -> anyhow::Result<()> {
    let messages = usize::try_from(env_number("OMFP_PERF_MESSAGES", 300))?;
    let launcher = FakeLauncher::new();
    let scratch = tempfile::tempdir()?;
    let bundle = probed_bundle(&e2e::web_bundle(), scratch.path())?;
    let journey = Journey::start_fake_with_bundle(launcher.clone(), bundle).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/typing"),
        FakeRun::fresh().with_history(history(messages)),
    );
    journey.connected().await?;
    let thread = create_thread(&journey, "typing").await?;
    let webkit = WebKitDriver::start().await?;
    let driver = webkit.session().await?;
    driver.goto(journey.daemon().url()).await?;
    until_true(
        &driver,
        "the page with the probe",
        "return document.querySelector('[data-testid=view]') !== null && window.__omfpPerf !== undefined;",
        Vec::new(),
    )
    .await?;
    let agent = eval_on(&driver, "return navigator.userAgent;", Vec::new()).await?;
    eprintln!("typing benchmark: {messages} messages, {agent}");
    typing_runs("webkitgtk", &driver, None, &thread, &launcher).await?;
    driver.quit().await?;
    drop(webkit);
    journey.finish("typing benchmark (webkitgtk)").await
}

/// Line `index` of a TypeScript module of functions of 12 lines; with `long`, every 5th function holds a line of
/// about 1,500 characters (an array literal). `variant` changes the line the way an edit would (0: as committed).
fn source_line(index: usize, long: bool, variant: usize) -> String {
    let function = index / 12;
    let suffix = if variant == 0 {
        String::new()
    } else {
        format!(" + {variant}")
    };
    match index % 12 {
        0 => format!(
            "export function handler{function}(cache: Cache, input: readonly number[]): Result<number> {{"
        ),
        1 => format!(
            "  // Takes the lock first: the early return only happens on a real error ({function})."
        ),
        2 => "  const guard = cache.lock();".to_owned(),
        3 => "  if (guard.isCold()) {".to_owned(),
        4 => format!("    guard.refill({{ size: input.length, label: \"handler {function}\" }});"),
        5 | 10 => "  }".to_owned(),
        6 if long && function.is_multiple_of(5) => format!(
            "  const table = [{}];",
            (0..300)
                .map(|n| (n * 7 + function).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        6 => "  const table = [1, 2, 3];".to_owned(),
        7 => "  let total = 0;".to_owned(),
        8 => "  for (const value of input) {".to_owned(),
        9 => format!("    total += value * {function} + guard.get(value % 64){suffix};"),
        _ => format!("  return {{ ok: true, value: total + table.length{suffix} }}; }}"),
    }
}

/// A module of `lines` lines (see [`source_line`]); `changed` picks the lines an edit changes (their variant).
fn source(lines: usize, long: bool, changed: impl Fn(usize) -> usize) -> String {
    let mut text = String::new();
    for index in 0..lines {
        text.push_str(&source_line(index, long, changed(index)));
        text.push('\n');
    }
    text
}

/// What a file's editor or diff shows.
#[derive(Debug)]
#[expect(
    dead_code,
    reason = "the page and the heights are printed by the benchmarks only"
)]
struct Editor {
    /// Elements under `.cm-content` (every editor of a split diff).
    nodes: u64,
    /// `.cm-line` elements.
    lines: u64,
    /// Elements of the whole page.
    page: u64,
    /// The class of the element that scrolls, its scroll height and client height.
    scroller: String,
    scroll_height: u64,
    client_height: u64,
    /// The computed `contain` of the kit's box around the scroller (UI-118).
    contain: String,
}

/// The script locating the element that scrolls `css`'s code: the nearest ancestor of its first `.cm-content` with
/// more content than room and a scrolling overflow.
const SCROLLER: &str = "const scrollerOf = (css) => {
      let t = document.querySelector(css + ' .cm-content');
      while (t && !(t.scrollHeight > t.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(t).overflowY))) {
        t = t.parentElement;
      }
      return t;
    };";

async fn editor_dom(driver: &WebDriver, css: &str) -> anyhow::Result<Editor> {
    let value = eval_on(
        driver,
        &format!(
            "{SCROLLER}
             const root = document.querySelector(arguments[0]);
             const t = scrollerOf(arguments[0]);
             const box = root.querySelector('.omfp-code-editor__view, .omfp-code-diff__view');
             return {{ nodes: root.querySelectorAll('.cm-content *').length,
                      lines: root.querySelectorAll('.cm-line').length,
                      page: document.getElementsByTagName('*').length,
                      scroller: t ? t.className : 'none',
                      scrollHeight: t ? t.scrollHeight : 0,
                      clientHeight: t ? t.clientHeight : 0,
                      contain: box ? getComputedStyle(box).contain : 'none' }};"
        ),
        vec![json!(css)],
    )
    .await?;
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
    Ok(Editor {
        nodes: count(&value, "nodes"),
        lines: count(&value, "lines"),
        page: count(&value, "page"),
        scroller: text("scroller"),
        scroll_height: count(&value, "scrollHeight"),
        client_height: count(&value, "clientHeight"),
        contain: text("contain"),
    })
}

/// Stops the probe's recording after `settle_ms` (the long tasks of the last frames, the end of a smooth scroll),
/// and lets its frame loop end.
async fn stop_after(driver: &WebDriver, settle_ms: u64) -> anyhow::Result<Recording> {
    let value = driver
        .execute_async(
            "const [ms, done] = arguments;
             setTimeout(() => {
               const recording = window.__omfpPerf.stop();
               requestAnimationFrame(() => requestAnimationFrame(() => done(recording)));
             }, ms);",
            vec![json!(settle_ms)],
        )
        .await?
        .json()
        .clone();
    Ok(Recording::from_json(&value))
}

/// Scrolls `css`'s code by `step` px per frame for `frames` frames (negative: up), recording.
async fn scroll_code(
    driver: &WebDriver,
    css: &str,
    frames: u64,
    step: i64,
) -> anyhow::Result<Recording> {
    let script = format!(
        "{SCROLLER}
        const [css, frames, step, done] = arguments;
        const t = scrollerOf(css);
        window.__omfpPerf.start(false);
        let frame = 0;
        const tick = () => {{
          if (frame >= frames) {{
            done();
            return;
          }}
          t.scrollTop += step;
          frame += 1;
          requestAnimationFrame(tick);
        }};
        requestAnimationFrame(tick);"
    );
    driver
        .execute_async(&script, vec![json!(css), json!(frames), json!(step)])
        .await?;
    stop_after(driver, 200).await
}

/// Where wheel turns come from.
enum Turns {
    /// CDP's `Input.dispatchMouseEvent`, 100 px a notch (Chromium).
    Cdp,
    /// Wheel buttons on the X server in this environment (`DISPLAY`, `XAUTHORITY`) by `xdotool`, 16 ms apart: the
    /// platform's own smooth wheel scrolling (`WebKitGTK`, whose WebDriver wheel actions scroll nothing).
    X(Vec<(String, String)>),
}

/// Turns the mouse wheel `notches` times (down when positive) over `css`'s code, recording.
async fn wheel_code(
    driver: &WebDriver,
    turns: &Turns,
    css: &str,
    notches: i64,
) -> anyhow::Result<Recording> {
    let at = eval_on(
        driver,
        &format!(
            "{SCROLLER}
             const r = scrollerOf(arguments[0]).getBoundingClientRect();
             const x = r.left + r.width / 2, y = r.top + r.height / 2;
             return [Math.round(x), Math.round(y), Math.round(window.screenX + x),
                     Math.round(window.screenY + window.outerHeight - window.innerHeight + y)];"
        ),
        vec![json!(css)],
    )
    .await?;
    let (x, y, screen_x, screen_y): (i64, i64, i64, i64) = serde_json::from_value(at)?;
    start_recording(driver, false).await?;
    match turns {
        Turns::Cdp => {
            let cdp = driver.cdp();
            for _ in 0..notches.unsigned_abs() {
                cdp.send_raw(
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": 100 * notches.signum() }),
                )
                .await?;
            }
        }
        Turns::X(environment) => {
            let status = std::process::Command::new("xdotool")
                .envs(environment.iter().map(|(name, value)| (name, value)))
                .args(["mousemove", &screen_x.to_string(), &screen_y.to_string()])
                .args(["click", "--repeat", &notches.unsigned_abs().to_string()])
                .args(["--delay", "16", if notches < 0 { "4" } else { "5" }])
                .status()?;
            anyhow::ensure!(status.success(), "xdotool failed: {status}");
        }
    }
    stop_after(driver, 1000).await
}

/// Puts the focus in `css`'s code with a click, then presses `keys` there in one WebDriver action (as fast as key
/// repeat or faster), recording until `settle_ms` after the last key.
async fn keys_code(
    driver: &WebDriver,
    css: &str,
    keys: impl Into<TypingData>,
    settle_ms: u64,
) -> anyhow::Result<Recording> {
    driver
        .find(By::Css(format!("{css} .cm-content")))
        .await?
        .click()
        .await?;
    start_recording(driver, false).await?;
    driver.action_chain().send_keys(keys).perform().await?;
    stop_after(driver, settle_ms).await
}

/// The scroll position of `css`'s code (px).
async fn scroll_top(driver: &WebDriver, css: &str) -> anyhow::Result<f64> {
    Ok(eval_on(
        driver,
        &format!("{SCROLLER} return scrollerOf(arguments[0]).scrollTop;"),
        vec![json!(css)],
    )
    .await?
    .as_f64()
    .unwrap_or_default())
}

/// Runs `text` on the `:` command line from normal mode and waits until the line closes.
async fn command_on(driver: &WebDriver, text: &str) -> anyhow::Result<()> {
    eval_on(
        driver,
        "if (document.activeElement) { document.activeElement.blur(); }",
        Vec::new(),
    )
    .await?;
    driver.action_chain().send_keys(":").perform().await?;
    until_true(
        driver,
        "the command line",
        "const e = document.activeElement; return e !== null && e.dataset.testid === 'command-line-input';",
        Vec::new(),
    )
    .await?;
    driver.action_chain().send_keys(text).perform().await?;
    driver
        .action_chain()
        .send_keys(Key::Enter)
        .perform()
        .await?;
    until_true(
        driver,
        "the command line to close",
        "return document.querySelector('[data-testid=command-line]') === null;",
        Vec::new(),
    )
    .await
}

async fn click_on(driver: &WebDriver, css: &str) -> anyhow::Result<()> {
    until_shown(driver, css).await?;
    eval_on(
        driver,
        "document.querySelector(arguments[0]).click();",
        vec![json!(css)],
    )
    .await?;
    Ok(())
}

/// A test file: its name, its committed text and its text in the working tree.
struct BigFile {
    name: &'static str,
    head: String,
    text: String,
}

/// The benchmark's files (line counts from `OMFP_PERF_*_LINES`), committed in a repository linked into thread
/// `thread`'s workspace as `app` and changed in the working tree; the repository lives as long as the result.
async fn link_big_files(
    journey: &Journey,
    thread: &str,
) -> anyhow::Result<(omfp_testkit::TempRepo, Vec<BigFile>)> {
    let lines = |name: &str, default: u64| usize::try_from(env_number(name, default));
    let hunks = lines("OMFP_PERF_HUNKS_LINES", 1000)?;
    let rewrite = lines("OMFP_PERF_REWRITE_LINES", 1000)?;
    let big = lines("OMFP_PERF_BIG_LINES", 20_000)?;
    let files = vec![
        // The reference: a small file, one screen and a half.
        BigFile {
            name: "small.ts",
            head: source(60, false, |_| 0),
            text: source(60, false, |i| usize::from(i % 20 == 9)),
        },
        // Many hunks: one changed line in every 20.
        BigFile {
            name: "hunks.ts",
            head: source(hunks, false, |_| 0),
            text: source(hunks, false, |i| usize::from(i % 20 == 9)),
        },
        // One big rewritten block in the middle.
        BigFile {
            name: "rewrite.ts",
            head: source(rewrite, false, |_| 0),
            text: source(rewrite, false, |i| {
                usize::from((rewrite * 3 / 10..rewrite * 7 / 10).contains(&i)) * 2
            }),
        },
        BigFile {
            name: "big.ts",
            head: source(big, true, |_| 0),
            text: source(big, true, |i| usize::from(i % 600 == 9)),
        },
    ];
    let repo = omfp_testkit::TempRepo::new()?;
    for file in &files {
        repo.commit_file(file.name, &file.head, file.name)?;
        std::fs::write(repo.path().join(file.name), &file.text)?;
    }
    let folder = journey.workspace_folder(thread).await?;
    std::os::unix::fs::symlink(repo.path(), folder.join("app"))?;
    // Rescans the workspace: `app` is one of its repositories now.
    journey.workspace_folder(thread).await?;
    Ok((repo, files))
}

/// Measures `css`'s code: its DOM, steady scrolling down and up, the wheel; with `profile`, Chromium's profile of
/// the scroll down.
async fn measure_code(
    driver: &WebDriver,
    profile: Option<&Journey>,
    turns: &Turns,
    label: &str,
    css: &str,
) -> anyhow::Result<()> {
    eprintln!("{label}: {:?}", editor_dom(driver, css).await?);
    let frames = env_number("OMFP_PERF_FRAMES", 240);
    if let Some(journey) = profile {
        start_profile(journey).await?;
    }
    let down = scroll_code(driver, css, frames, 120).await?;
    report(
        &format!("{label}: {frames} frames down at 120 px"),
        &down,
        0,
    );
    if let Some(journey) = profile {
        report_profile(journey).await?;
    }
    eprintln!("  after: {:?}", editor_dom(driver, css).await?);
    let up = scroll_code(driver, css, frames, -120).await?;
    report(&format!("{label}: {frames} frames up at 120 px"), &up, 0);
    let wheel = wheel_code(driver, turns, css, 40).await?;
    report(&format!("{label}: 40 wheel notches down"), &wheel, 0);
    eprintln!("  at {} px", scroll_top(driver, css).await?);
    let wheel = wheel_code(driver, turns, css, -40).await?;
    report(&format!("{label}: 40 wheel notches up"), &wheel, 0);
    eprintln!("  at {} px", scroll_top(driver, css).await?);
    for (name, keys) in [
        ("150 × j", "j".repeat(150)),
        ("150 × k", "k".repeat(150)),
        (
            "10 × PageDown",
            Key::PageDown.value().to_string().repeat(10),
        ),
        ("10 × PageUp", Key::PageUp.value().to_string().repeat(10)),
    ] {
        let profiled = profile.filter(|_| name.ends_with('j'));
        if let Some(journey) = profiled {
            start_profile(journey).await?;
        }
        let recording = keys_code(driver, css, keys, 500).await?;
        report(&format!("{label}: {name}"), &recording, 0);
        if let Some(journey) = profiled {
            report_profile(journey).await?;
        }
        eprintln!("  at {} px", scroll_top(driver, css).await?);
    }
    Ok(())
}

/// Opens each of `files` (`OMFP_PERF_FILES`: a comma-separated subset) in `driver`'s page and measures it in the
/// modes of `OMFP_PERF_MODES` (default `diff,view,edit`).
async fn file_runs(
    driver: &WebDriver,
    profile: Option<&Journey>,
    turns: &Turns,
    files: &[BigFile],
) -> anyhow::Result<()> {
    let only = std::env::var("OMFP_PERF_FILES").unwrap_or_default();
    let modes = std::env::var("OMFP_PERF_MODES").unwrap_or_else(|_| "diff,view,edit".to_owned());
    for file in files {
        if !only.is_empty() && !only.split(',').any(|name| name == file.name) {
            continue;
        }
        eprintln!(
            "{}: {} lines, {} bytes",
            file.name,
            file.text.lines().count(),
            file.text.len()
        );
        let root = format!("[data-testid=file][data-path$=\"{}\"]", file.name);
        let editor = format!("{root} [data-testid=file-editor]");
        let started = std::time::Instant::now();
        command_on(driver, &format!("open app/{}", file.name)).await?;
        until_shown(driver, &format!("{editor} .cm-line")).await?;
        eprintln!("  code shown after {} ms", started.elapsed().as_millis());
        for mode in modes.split(',') {
            let label = format!("{} {mode}", file.name);
            match mode {
                "diff" => {
                    let started = std::time::Instant::now();
                    click_on(driver, &format!("{root} [data-testid=file-diff]")).await?;
                    let diff = format!("{root} [data-testid=file-head-diff]");
                    until_shown(driver, &format!("{diff} .cm-line")).await?;
                    eprintln!("  diff shown after {} ms", started.elapsed().as_millis());
                    measure_code(driver, profile, turns, &label, &diff).await?;
                    click_on(driver, &format!("{root} [data-testid=file-diff]")).await?;
                    until_shown(driver, &format!("{editor} .cm-line")).await?;
                }
                "view" => measure_code(driver, profile, turns, &label, &editor).await?,
                "edit" => {
                    click_on(driver, &format!("{root} [data-testid=file-edit]")).await?;
                    let editing = format!("{editor}[data-mode=edit]");
                    until_shown(driver, &editing).await?;
                    // Editing starts in insert mode: vim's normal mode moves with `j`/`k`.
                    driver
                        .action_chain()
                        .send_keys(Key::Escape)
                        .perform()
                        .await?;
                    until_shown(driver, "[data-testid=mode-line][data-vim=normal]").await?;
                    measure_code(driver, profile, turns, &label, &editing).await?;
                    click_on(driver, &format!("{root} [data-testid=file-done]")).await?;
                    until_shown(driver, &format!("{editor}[data-mode=view]")).await?;
                }
                _ => anyhow::bail!("unknown mode {mode}"),
            }
        }
    }
    Ok(())
}

// spec: TST-107
#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: prints timings, asserts nothing (`just bench`)"]
async fn benchmark_file_chromium() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.push_run(FakeRun::fresh());
    let journey = Journey::start_fake(launcher).await?;
    journey
        .driver
        .set_script_timeout(Duration::from_secs(300))
        .await?;
    install_probe(&journey).await?;
    let thread = journey.create_thread("files").await?;
    let (_repo, files) = link_big_files(&journey, &thread).await?;
    let profile = std::env::var_os("OMFP_PERF_PROFILE").map(|_| &journey);
    file_runs(&journey.driver, profile, &Turns::Cdp, &files).await?;
    journey.finish("file benchmark (chromium)").await
}

// spec: TST-107
#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: prints timings, asserts nothing (`just bench`)"]
async fn benchmark_file_webkit() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.push_run(FakeRun::fresh());
    let scratch = tempfile::tempdir()?;
    let bundle = probed_bundle(&e2e::web_bundle(), scratch.path())?;
    let journey = Journey::start_fake_with_bundle(launcher, bundle).await?;
    journey.connected().await?;
    let thread = create_thread(&journey, "files").await?;
    let (_repo, files) = link_big_files(&journey, &thread).await?;
    let webkit = WebKitDriver::start().await?;
    let driver = webkit.session().await?;
    let result = async {
        driver.set_script_timeout(Duration::from_secs(300)).await?;
        driver.goto(journey.daemon().url()).await?;
        until_true(
            &driver,
            "the page with the probe",
            "return document.querySelector('[data-testid=view]') !== null && window.__omfpPerf !== undefined;",
            Vec::new(),
        )
        .await?;
        click_on(&driver, &row(&thread)).await?;
        until_shown(&driver, COMPOSER).await?;
        let agent = eval_on(&driver, "return navigator.userAgent;", Vec::new()).await?;
        eprintln!("file benchmark: {agent}");
        file_runs(&driver, None, &Turns::X(webkit.x_environment()?), &files).await
    }
    .await;
    driver.quit().await?;
    drop(webkit);
    result?;
    journey.finish("file benchmark (webkitgtk)").await
}

/// Elements under `.cm-content` of the 20,000-line file wherever it is scrolled (TST-107): the lines in view and
/// the editor's margin around them with their highlighting (up to about 1,600 measured), never the whole document.
const MAX_CODE_NODES: u64 = 3000;
/// `.cm-line`s of the 20,000-line file wherever it is scrolled (up to about 85 measured).
const MAX_CODE_LINES: u64 = 200;

/// `css`'s code at `place` scrolls in `.cm-scroller`, inside a layout root, with a bounded DOM (UI-118).
async fn assert_bounded(driver: &WebDriver, css: &str, place: &str) -> anyhow::Result<()> {
    let editor = editor_dom(driver, css).await?;
    assert!(
        editor
            .scroller
            .split(' ')
            .any(|class| class == "cm-scroller"),
        "{place}: CodeMirror's scroller scrolls: {editor:?}"
    );
    assert_eq!(editor.contain, "strict", "{place}: {editor:?}");
    assert!(
        editor.lines >= 20 && editor.lines <= MAX_CODE_LINES && editor.nodes <= MAX_CODE_NODES,
        "{place}: {editor:?}"
    );
    Ok(())
}

/// Scrolls `css`'s code to the middle of the file and waits two frames (the editor draws the lines there).
async fn scroll_to_middle(driver: &WebDriver, css: &str) -> anyhow::Result<()> {
    driver
        .execute_async(
            &format!(
                "{SCROLLER}
                 const [css, done] = arguments;
                 const t = scrollerOf(css);
                 t.scrollTop = t.scrollHeight / 2;
                 requestAnimationFrame(() => requestAnimationFrame(() => done()));"
            ),
            vec![json!(css)],
        )
        .await?;
    Ok(())
}

// spec: TST-107, UI-118
#[tokio::test(flavor = "multi_thread")]
async fn a_big_file_scrolls_in_a_layout_root_with_a_bounded_dom() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    launcher.push_run(FakeRun::fresh());
    let journey = Journey::start_fake(launcher).await?;
    install_probe(&journey).await?;
    let thread = journey.create_thread("files").await?;
    let (_repo, _files) = link_big_files(&journey, &thread).await?;
    let driver = &journey.driver;
    let root = "[data-testid=file][data-path$=\"big.ts\"]";
    command_on(driver, "open app/big.ts").await?;
    let editor = format!("{root} [data-testid=file-editor]");
    let diff = format!("{root} [data-testid=file-head-diff]");
    for (mode, css) in [("view", &editor), ("diff", &diff)] {
        if mode == "diff" {
            click_on(driver, &format!("{root} [data-testid=file-diff]")).await?;
        }
        until_shown(driver, &format!("{css} .cm-line")).await?;
        assert_bounded(driver, css, &format!("{mode} at the top")).await?;
        scroll_code(driver, css, 60, 120).await?;
        assert_bounded(driver, css, &format!("{mode} after a steady scroll")).await?;
        scroll_to_middle(driver, css).await?;
        assert_bounded(driver, css, &format!("{mode} in the middle")).await?;
    }
    journey.finish("big file").await
}
