//! Transcript pinning (UI-042) against a daemon on the fake launcher. While a tool runs, in headless Chromium: a bash
//! call streams its output into an expanded card, which grows the transcript, while the user scrolls up with the
//! wheel (a real wheel event through CDP's `Input.dispatchMouseEvent`) and with `PageUp`. While a reply streams, in
//! headless Chromium and in `WebKitGTK`, the engine of `omfp desktop`: text deltas at a model's rate
//! grow a long last message while the user scrolls up with the wheel, `PageUp`, and a scrollbar drag. The view must
//! stay where the user scrolled to and unpinned; `End` pins it again and the growth then keeps the bottom in view.
//!
//! Chromium gets a touchpad's many small wheel deltas and a mouse wheel's few notches from WebDriver's wheel input
//! source. `WebKitGTK`'s `WebKitWebDriver` dispatches those wheel events without scrolling, so the `WebKitGTK` case
//! clicks the wheel buttons on its X server with `xdotool`: `WebKitGTK` then scrolls smoothly, a few pixels per frame,
//! as for the user. It drives nixpkgs' `MiniBrowser` through `WebKitWebDriver` (`OMFP_WEBKIT_WEBDRIVER` of the dev
//! shell) under `xvfb-run`, never the user's display.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Context;
use e2e::{Journey, WebKitDriver, row};
use omfp_core::wire::{AgentMessage, RpcNotification};
use omfp_daemon::agent::{FakeLauncher, FakeProcess, FakeRun};
use omfp_protocol::method::ThreadsCreate;
use omfp_protocol::{CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use serde_json::{Value, json};
use thirtyfour::common::command::{Actions, Command};
use thirtyfour::prelude::*;

const TRANSCRIPT: &str = "[data-testid=transcript]";
const CALL: &str = "call_running_bash";
const CARD: &str = "[data-testid=tool-card][data-call=call_running_bash]";

fn message(value: &Value) -> AgentMessage {
    serde_json::from_value(value.clone()).expect("a valid agent message")
}

fn frame(value: &Value) -> RpcNotification {
    serde_json::from_value(value.clone()).expect("a valid omp frame")
}

fn assistant(content: &Value) -> Value {
    json!({
        "role": "assistant", "api": "openai-completions", "provider": "fake", "model": "scripted",
        "content": content, "stopReason": "toolUse", "timestamp": 0,
        "usage": {
            "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
            "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 },
        },
    })
}

/// Enough questions and answers to scroll.
fn history() -> Vec<AgentMessage> {
    (0..40)
        .flat_map(|index| {
            [
                json!({ "role": "user", "timestamp": 0, "content": [{ "type": "text", "text": format!("Question {index}?") }] }),
                assistant(&json!([{ "type": "text", "text": format!(
                    "Answer {index}.\n\nA paragraph long enough to take a few lines of the transcript, so that forty \
                     of them make a list much taller than the window."
                ) }])),
            ]
        })
        .map(|value| message(&value))
        .collect()
}

/// The turn so far: the assistant calls bash, and omp starts running it.
fn start_running() -> Vec<RpcNotification> {
    let call = assistant(&json!([{
        "type": "toolCall", "id": CALL, "name": "bash", "arguments": { "command": "make test" },
    }]));
    vec![
        frame(&json!({ "type": "agent_start" })),
        frame(&json!({ "type": "message_start", "messageId": "run", "message": call })),
        frame(&json!({ "type": "message_end", "messageId": "run", "message": call })),
        frame(&json!({
            "type": "tool_execution_start", "toolCallId": CALL, "toolName": "bash",
            "args": { "command": "make test" },
        })),
    ]
}

/// The output after `lines` lines, as omp streams it.
fn output(lines: usize) -> RpcNotification {
    let text: String = (1..=lines)
        .map(|line| format!("line {line} of the test run\n"))
        .collect::<Vec<_>>()
        .concat();
    frame(&json!({
        "type": "tool_execution_update", "toolCallId": CALL, "toolName": "bash",
        "args": { "command": "make test" },
        "partialResult": { "content": [{ "type": "text", "text": text }], "details": {} },
    }))
}

/// `(scrollTop, scrollHeight, pinned)` of the transcript.
async fn scroll_state(journey: &Journey) -> anyhow::Result<(f64, f64, bool)> {
    let value = journey
        .eval(
            "const t = document.querySelector(arguments[0]);
             return [t.scrollTop, t.scrollHeight, t.dataset.pinned === 'true'];",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

/// Streams `count` more output lines, one update each, waiting until the card shows each one.
async fn stream_lines(
    journey: &Journey,
    process: &FakeProcess,
    shown: &mut usize,
    count: usize,
) -> anyhow::Result<()> {
    for _ in 0..count {
        *shown += 1;
        process.emit(vec![output(*shown)]);
        let needle = format!("line {shown} of the test run");
        journey
            .until(&needle, async |page| {
                Ok(page.text(CARD).await?.contains(&needle).then_some(()))
            })
            .await?;
    }
    Ok(())
}

/// A real wheel turn over the transcript (`delta_y` < 0 scrolls up), through the compositor like a mouse.
async fn wheel(journey: &Journey, delta_y: f64) -> anyhow::Result<()> {
    let value = journey
        .eval(
            "const r = document.querySelector(arguments[0]).getBoundingClientRect();
             return [r.left + r.width / 2, r.top + r.height / 2];",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    let (x, y): (f64, f64) = serde_json::from_value(value)?;
    journey
        .driver
        .cdp()
        .send_raw(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": delta_y }),
        )
        .await?;
    Ok(())
}

/// Waits until the transcript scrolled up from `bottom` and reports it unpinned; returns where it stopped.
async fn until_scrolled_up(journey: &Journey, bottom: f64, how: &str) -> anyhow::Result<f64> {
    journey
        .until(
            &format!("the transcript scrolled up with {how}"),
            async |page| {
                let (top, _, pinned) = scroll_state(page).await?;
                Ok((top < bottom - 100.0 && !pinned).then_some(top))
            },
        )
        .await
}

/// Streams ten more lines and checks the view stayed at `top`, unpinned.
async fn stays_at(
    journey: &Journey,
    process: &FakeProcess,
    shown: &mut usize,
    top: f64,
    how: &str,
) -> anyhow::Result<()> {
    stream_lines(journey, process, shown, 10).await?;
    let (after, _, pinned) = scroll_state(journey).await?;
    assert_eq!(
        (after, pinned),
        (top, false),
        "after scrolling up with {how}, the running tool's output must not pull the view back"
    );
    Ok(())
}

/// Creates a thread in a new workspace through the protocol (the launcher's next run plays it); returns its id.
async fn create_thread(journey: &Journey) -> anyhow::Result<String> {
    let mut connection = journey.connect().await?;
    let thread = connection
        .call::<ThreadsCreate>(CreateThreadParams {
            title: None,
            workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                name: Some("scroll".to_owned()),
                link: None,
            }),
            model: None,
            thinking_level: None,
            approval_mode: None,
            prompt: None,
            images: Vec::new(),
        })
        .await?
        .id;
    connection.close().await?;
    Ok(thread)
}

// spec: UI-042
#[tokio::test(flavor = "multi_thread")]
async fn scrolling_up_while_a_tool_streams_output_stays_where_the_user_scrolled()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/scroll"),
        FakeRun::fresh().with_history(history()),
    );
    let thread = create_thread(&journey).await?;
    journey.click(&row(&thread)).await?;
    journey.opened(&thread).await?;

    // A running bash call whose card the user expanded: its output grows the transcript.
    let process = journey.fake_process(&launcher, &thread).await?;
    process.emit(start_running());
    journey
        .until("the running card", async |page| {
            Ok((page.count(CARD).await? == 1).then_some(()))
        })
        .await?;
    journey
        .eval(
            "document.querySelector(arguments[0] + ' [data-testid=toggle]').click();",
            vec![json!(CARD)],
        )
        .await?;
    let mut shown = 0;
    stream_lines(&journey, &process, &mut shown, 5).await?;
    let (bottom, _, pinned) = scroll_state(&journey).await?;
    assert!(pinned, "the view follows the bottom at first");

    // The wheel: the user scrolled up, the output keeps coming.
    wheel(&journey, -400.0).await?;
    let top = until_scrolled_up(&journey, bottom, "the wheel").await?;
    stays_at(&journey, &process, &mut shown, top, "the wheel").await?;

    // End pins the view again, and the output keeps the bottom in view.
    let bottom = back_to_bottom(&journey, &process, &mut shown).await?;

    // PageUp: the same.
    journey.keys(Key::PageUp).await?;
    let top = until_scrolled_up(&journey, bottom, "PageUp").await?;
    stays_at(&journey, &process, &mut shown, top, "PageUp").await?;

    // A scroll that comes with no wheel, key or pointer event — dragging the scrollbar in WebKitGTK, whose overlay
    // scrollbars send no pointer events to the page: the same.
    let bottom = back_to_bottom(&journey, &process, &mut shown).await?;
    journey
        .eval(
            "const t = document.querySelector(arguments[0]); t.scrollTop = t.scrollTop - 600;",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    let top = until_scrolled_up(&journey, bottom, "the scrollbar").await?;
    stays_at(&journey, &process, &mut shown, top, "the scrollbar").await?;

    journey.finish("scrolling while a tool runs").await
}

/// `End` on the focused transcript pins the view again; the output then keeps the bottom in view. Returns the
/// bottom's `scrollTop`.
async fn back_to_bottom(
    journey: &Journey,
    process: &FakeProcess,
    shown: &mut usize,
) -> anyhow::Result<f64> {
    journey
        .eval(
            "document.querySelector(arguments[0]).focus();",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    journey.keys(Key::End).await?;
    journey
        .until("the pinned transcript", async |page| {
            Ok(scroll_state(page).await?.2.then_some(()))
        })
        .await?;
    stream_lines(journey, process, shown, 5).await?;
    let (bottom, height, pinned) = scroll_state(journey).await?;
    let client: f64 = serde_json::from_value(
        journey
            .eval(
                "return document.querySelector(arguments[0]).clientHeight;",
                vec![json!(TRANSCRIPT)],
            )
            .await?,
    )?;
    assert!(
        pinned && height - bottom - client <= 32.0,
        "pinned at the bottom while the output grows"
    );
    Ok(bottom)
}

// A streaming reply: text deltas at a model's rate grow the last message, rendered as Markdown, while the user
// scrolls up — with a touchpad's many small wheel deltas, a mouse wheel's few notches, `PageUp`, and a scrollbar drag
// (`scrollTop` set frame by frame, as WebKitGTK's scrollbars do without any pointer event in the page).

const STREAMING: &str = "[data-testid=transcript-item][data-key=streaming]";
const STREAM_ID: &str = "scroll-stream";
/// Text deltas per second, a fast model's rate.
const DELTAS_PER_SECOND: u64 = 60;
/// Longest wait of the streaming reply's case for one condition.
const WAIT: Duration = Duration::from_secs(20);

/// Delta `index` of the streamed reply: a numbered word, with paragraph ends and a code block now and then, so the
/// Markdown re-renders and changes its height as it streams.
fn delta(index: usize) -> String {
    match index % 60 {
        0 => "\n\n```text\n".to_owned(),
        8 => "\n```\n\n".to_owned(),
        step if step % 12 == 0 => format!("n{index}n.\n\n"),
        _ => format!("n{index}n "),
    }
}

fn text_delta(delta: &str) -> RpcNotification {
    frame(&json!({
        "type": "message_update", "messageId": STREAM_ID, "message": { "role": "assistant" },
        "assistantMessageEvent": { "type": "text_delta", "contentIndex": 0, "delta": delta },
    }))
}

/// omp's frames that start the reply, with a first delta long enough to make it taller than the window.
fn reply_start() -> Vec<RpcNotification> {
    let opening: String = (0..600).map(|index| delta(index + 1_000_000)).collect();
    vec![
        frame(&json!({ "type": "agent_start" })),
        frame(
            &json!({ "type": "message_start", "messageId": STREAM_ID, "message": assistant(&json!([])) }),
        ),
        frame(&json!({
            "type": "message_update", "messageId": STREAM_ID, "message": { "role": "assistant" },
            "assistantMessageEvent": { "type": "text_start", "contentIndex": 0, "block": { "type": "text", "text": "" } },
        })),
        text_delta(&opening),
    ]
}

/// The streaming reply: its deltas go out at [`DELTAS_PER_SECOND`] from a task of their own until it is dropped.
struct Reply {
    sent: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Reply {
    fn start(process: FakeProcess) -> Self {
        process.emit(reply_start());
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&sent);
        let task = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_millis(1000 / DELTAS_PER_SECOND));
            let mut index = 0;
            loop {
                index += 1;
                ticks.tick().await;
                process.emit(vec![text_delta(&delta(index))]);
                counter.store(index, Ordering::SeqCst);
            }
        });
        Self { sent, task }
    }

    fn sent(&self) -> usize {
        self.sent.load(Ordering::SeqCst)
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn eval_on(driver: &WebDriver, script: &str, args: Vec<Value>) -> anyhow::Result<Value> {
    Ok(driver.execute(script, args).await?.json().clone())
}

/// The transcript's scroll state.
#[derive(Debug, Clone, Copy)]
struct View {
    top: f64,
    /// Pixels between the bottom of the view and the bottom of the content.
    below: f64,
    height: f64,
    pinned: bool,
}

async fn view(driver: &WebDriver) -> anyhow::Result<View> {
    let value = eval_on(
        driver,
        "const t = document.querySelector(arguments[0]);
         return [t.scrollTop, t.scrollHeight - t.scrollTop - t.clientHeight, t.scrollHeight, t.dataset.pinned === 'true'];",
        vec![json!(TRANSCRIPT)],
    )
    .await?;
    let (top, below, height, pinned) = serde_json::from_value(value)?;
    Ok(View {
        top,
        below,
        height,
        pinned,
    })
}

/// Polls `probe` until it returns `Some`, failing after [`WAIT`] with `what` and the transcript's scroll state (for a
/// page without a [`Journey`]).
async fn until_on<T>(
    driver: &WebDriver,
    what: &str,
    mut probe: impl AsyncFnMut(&WebDriver) -> anyhow::Result<Option<T>>,
) -> anyhow::Result<T> {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Some(value) = probe(driver).await? {
            return Ok(value);
        }
        if std::time::Instant::now() > deadline {
            let state = view(driver).await;
            anyhow::bail!("timed out waiting for {what}; the transcript: {state:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Where the page's wheel input comes from.
enum Wheel {
    /// WebDriver's wheel input source, `delta_y` pixels a turn (Chromium; `WebKitGTK`'s dispatches wheel events but
    /// scrolls nothing).
    WebDriver,
    /// Wheel buttons pressed on the X server in this environment (`DISPLAY`, `XAUTHORITY`) by `xdotool`, one notch a
    /// turn: the platform's own wheel scrolling, smooth scrolling included (`WebKitGTK`).
    X(Vec<(String, String)>),
}

/// A wheel gesture over the transcript: `steps` turns of `delta_y` pixels (< 0 scrolls up) a frame apart.
async fn wheel_gesture(
    driver: &WebDriver,
    wheel: &Wheel,
    steps: usize,
    delta_y: i64,
) -> anyhow::Result<()> {
    let value = eval_on(
        driver,
        "const r = document.querySelector(arguments[0]).getBoundingClientRect();
         const x = r.left + r.width / 2, y = r.top + r.height / 2;
         return [Math.round(x), Math.round(y), Math.round(window.screenX + x),
                 Math.round(window.screenY + window.outerHeight - window.innerHeight + y)];",
        vec![json!(TRANSCRIPT)],
    )
    .await?;
    let (x, y, screen_x, screen_y): (i64, i64, i64, i64) = serde_json::from_value(value)?;
    match wheel {
        Wheel::WebDriver => {
            let actions: Vec<Value> = (0..steps)
                .flat_map(|_| {
                    [
                        json!({ "type": "scroll", "x": x, "y": y, "deltaX": 0, "deltaY": delta_y, "origin": "viewport", "duration": 0 }),
                        json!({ "type": "pause", "duration": 16 }),
                    ]
                })
                .collect();
            driver
                .cmd(Command::PerformActions(Actions::from(json!([
                    { "type": "wheel", "id": "wheel", "actions": actions },
                ]))))
                .await?;
        }
        Wheel::X(environment) => {
            let button = if delta_y < 0 { "4" } else { "5" };
            let status = std::process::Command::new("xdotool")
                .envs(environment.iter().map(|(name, value)| (name, value)))
                .args(["mousemove", &screen_x.to_string(), &screen_y.to_string()])
                .args([
                    "click",
                    "--repeat",
                    &steps.to_string(),
                    "--delay",
                    "16",
                    button,
                ])
                .status()
                .context("cannot run xdotool (run inside `nix develop`)")?;
            anyhow::ensure!(status.success(), "xdotool failed: {status}");
        }
    }
    Ok(())
}

/// Drags the scrollbar up `steps` × `step` pixels: `scrollTop` set from where the drag started, one step per frame.
async fn drag_scrollbar(driver: &WebDriver, steps: u32, step: u32) -> anyhow::Result<()> {
    driver
        .execute_async(
            "const [css, steps, step, done] = arguments;
             const t = document.querySelector(css);
             const start = t.scrollTop;
             let moved = 0;
             const next = () => {
               if (moved === steps) { done(); return; }
               moved += 1;
               t.scrollTop = start - moved * step;
               requestAnimationFrame(next);
             };
             requestAnimationFrame(next);",
            vec![json!(TRANSCRIPT), json!(steps), json!(step)],
        )
        .await?;
    Ok(())
}

/// Presses `key` on the focused transcript.
async fn press(driver: &WebDriver, key: Key) -> anyhow::Result<()> {
    eval_on(
        driver,
        "document.querySelector(arguments[0]).focus();",
        vec![json!(TRANSCRIPT)],
    )
    .await?;
    driver.action_chain().send_keys(key).perform().await?;
    Ok(())
}

/// Waits until the view rests unpinned at least 100 px above the bottom (the same position for a few polls: smooth
/// scrolling has ended); returns its state.
async fn until_rests_up(driver: &WebDriver, how: &str) -> anyhow::Result<View> {
    let mut last = None;
    let mut same = 0;
    until_on(
        driver,
        &format!("the view to rest above the bottom after {how}"),
        async |page| {
            let now = view(page).await?;
            if now.pinned || now.below < 100.0 {
                (last, same) = (None, 0);
                return Ok(None);
            }
            if last == Some(now.top) {
                same += 1;
            } else {
                (last, same) = (Some(now.top), 0);
            }
            Ok((same >= 4).then_some(now))
        },
    )
    .await
}

/// Lets the reply stream on for a second and checks that the view stayed at `at`, unpinned, while the reply grew.
async fn rests(driver: &WebDriver, reply: &Reply, at: View, how: &str) -> anyhow::Result<()> {
    let until = reply.sent() + usize::try_from(DELTAS_PER_SECOND)?;
    until_on(driver, "a second more of the reply", async |page| {
        Ok((reply.sent() >= until && view(page).await?.height > at.height).then_some(()))
    })
    .await?;
    let after = view(driver).await?;
    assert!(
        (after.top - at.top).abs() <= 1.0 && !after.pinned,
        "after scrolling up with {how}, the streaming reply must not pull the view back: {at:?} -> {after:?}"
    );
    Ok(())
}

/// `End` pins the view again; it then follows the reply at the bottom.
async fn to_bottom(driver: &WebDriver, reply: &Reply) -> anyhow::Result<()> {
    press(driver, Key::End).await?;
    let until = reply.sent() + 30;
    until_on(driver, "the pinned bottom", async |page| {
        let now = view(page).await?;
        Ok((reply.sent() >= until && now.pinned && now.below <= 32.0).then_some(()))
    })
    .await
}

/// The streaming reply's case of UI-042 in `driver`'s page, showing `thread`'s omp.
/// `gestures` are the wheel gestures to try: what they stand for, turns, pixels a turn.
async fn scroll_up_while_a_reply_streams(
    driver: &WebDriver,
    launcher: &FakeLauncher,
    thread: &str,
    wheel: &Wheel,
    gestures: &[(&str, usize, i64)],
) -> anyhow::Result<()> {
    let process = until_on(driver, "the thread's omp to start", async |_| {
        Ok(e2e::fake_thread(launcher, thread))
    })
    .await?;
    let reply = Reply::start(process);
    until_on(driver, "the streaming reply at the bottom", async |page| {
        let shown = eval_on(
            page,
            "const r = document.querySelector(arguments[0]);
             return r !== null && r.offsetHeight > 2 * document.querySelector(arguments[1]).clientHeight;",
            vec![json!(STREAMING), json!(TRANSCRIPT)],
        )
        .await?
            == json!(true);
        let now = view(page).await?;
        Ok((shown && reply.sent() >= 30 && now.pinned && now.below <= 32.0).then_some(()))
    })
    .await?;

    for &(how, steps, delta_y) in gestures {
        wheel_gesture(driver, wheel, steps, delta_y).await?;
        let at = until_rests_up(driver, how).await?;
        rests(driver, &reply, at, how).await?;
        to_bottom(driver, &reply).await?;
    }

    let how = "PageUp";
    press(driver, Key::PageUp).await?;
    let at = until_rests_up(driver, how).await?;
    rests(driver, &reply, at, how).await?;

    to_bottom(driver, &reply).await?;
    let how = "the scrollbar";
    drag_scrollbar(driver, 20, 12).await?;
    let at = until_rests_up(driver, how).await?;
    rests(driver, &reply, at, how).await
}

// spec: UI-042
#[tokio::test(flavor = "multi_thread")]
async fn scrolling_up_while_a_reply_streams_stays_where_the_user_scrolled() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/scroll"),
        FakeRun::fresh().with_history(history()),
    );
    let thread = create_thread(&journey).await?;
    journey.click(&row(&thread)).await?;
    journey.opened(&thread).await?;
    scroll_up_while_a_reply_streams(
        &journey.driver,
        &launcher,
        &thread,
        &Wheel::WebDriver,
        &[
            ("a touchpad (many small wheel deltas)", 15, -16),
            ("a mouse wheel", 3, -120),
        ],
    )
    .await?;
    journey.finish("scrolling while a reply streams").await
}

// spec: UI-042
#[tokio::test(flavor = "multi_thread")]
async fn scrolling_up_while_a_reply_streams_stays_where_the_user_scrolled_in_webkitgtk()
-> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/scroll"),
        FakeRun::fresh().with_history(history()),
    );
    journey.connected().await?;
    let thread = create_thread(&journey).await?;
    let webkit = WebKitDriver::start().await?;
    let driver = webkit.session().await?;
    driver.goto(journey.daemon().url()).await?;
    let result = async {
        let thread_row = row(&thread);
        until_on(&driver, "the thread's row", async |page| {
            Ok((eval_on(
                page,
                "return document.querySelector(arguments[0]) !== null;",
                vec![json!(thread_row)],
            )
            .await?
                == json!(true))
            .then_some(()))
        })
        .await?;
        eval_on(
            &driver,
            "document.querySelector(arguments[0]).click();",
            vec![json!(thread_row)],
        )
        .await?;
        let agent = eval_on(&driver, "return navigator.userAgent;", Vec::new()).await?;
        eprintln!("streaming reply in {agent}");
        let wheel = Wheel::X(webkit.x_environment()?);
        scroll_up_while_a_reply_streams(
            &driver,
            &launcher,
            &thread,
            &wheel,
            &[("a wheel notch", 1, -1), ("three wheel notches", 3, -1)],
        )
        .await
    }
    .await;
    driver.quit().await?;
    drop(webkit);
    result?;
    journey
        .finish("scrolling while a reply streams (webkitgtk)")
        .await
}

/// Short questions and answers around one reply several windows tall.
fn long_reply_history() -> Vec<AgentMessage> {
    let long: String = (0..80)
        .map(|index| {
            format!("Long reply paragraph {index}: a line of text to read through by keyboard.")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    [
        json!({ "role": "user", "timestamp": 0, "content": [{ "type": "text", "text": "Short question?" }] }),
        assistant(&json!([{ "type": "text", "text": "Short answer." }])),
        json!({ "role": "user", "timestamp": 0, "content": [{ "type": "text", "text": "Long question?" }] }),
        assistant(&json!([{ "type": "text", "text": format!("Long reply start.\n\n{long}\n\nLong reply end.") }])),
        json!({ "role": "user", "timestamp": 0, "content": [{ "type": "text", "text": "After question?" }] }),
        assistant(&json!([{ "type": "text", "text": "After answer." }])),
    ]
    .iter()
    .map(message)
    .collect()
}

/// The focused transcript item and the view, edges relative to the viewport's top.
#[derive(Debug)]
struct Focused {
    /// The start of the item's text.
    text: String,
    scroll_top: f64,
    view: f64,
    top: f64,
    bottom: f64,
}

async fn focused_item(journey: &Journey) -> anyhow::Result<Focused> {
    let value = journey
        .eval(
            "const t = document.querySelector(arguments[0]);
             const item = document.activeElement?.closest('[data-testid=transcript-item]');
             const view = t.getBoundingClientRect().top;
             const r = item ? item.getBoundingClientRect() : { top: view, bottom: view };
             return [item ? item.textContent.slice(0, 200) : '', t.scrollTop, t.clientHeight, r.top - view,
                     r.bottom - view];",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    let (text, scroll_top, view, top, bottom) = serde_json::from_value(value)?;
    Ok(Focused {
        text,
        scroll_top,
        view,
        top,
        bottom,
    })
}

/// Presses `key` and returns the focused item once the press took effect (its scroll or focus change).
async fn press_nav(journey: &Journey, key: &str) -> anyhow::Result<Focused> {
    let before = focused_item(journey).await?;
    journey.keys(key).await?;
    journey
        .until(&format!("`{key}` to scroll or move"), async |page| {
            let after = focused_item(page).await?;
            Ok(
                (after.text != before.text || (after.scroll_top - before.scroll_top).abs() > 0.5)
                    .then_some(after),
            )
        })
        .await
}

/// Pages with `key` (`j` down, `k` up) through the focused long reply until the press that moves on, checking each
/// step keeps the focus and scrolls at most 40 % of the viewport; returns the steps and the item it moved to.
async fn page_through(
    journey: &Journey,
    key: &str,
    mut item: Focused,
) -> anyhow::Result<(usize, Focused)> {
    let down = key == "j";
    let mut steps = 0;
    loop {
        let edge_shows = if down {
            item.bottom <= item.view + 1.0
        } else {
            item.top >= -1.0
        };
        let next = press_nav(journey, key).await?;
        if edge_shows {
            assert!(
                !next.text.contains("Long reply start."),
                "once its edge shows, {key} moves on"
            );
            return Ok((steps, next));
        }
        assert!(
            next.text.contains("Long reply start."),
            "the focus stays on the long reply while {key} pages: {}",
            next.text
        );
        let step = (next.scroll_top - item.scroll_top) * if down { 1.0 } else { -1.0 };
        assert!(
            step > 0.0 && step <= 0.4 * item.view + 1.0,
            "a step of at most 40 % of the viewport: {step}"
        );
        steps += 1;
        assert!(steps < 100, "the long reply's edge never showed");
        item = next;
    }
}

// spec: E2E-31, UI-370, UI-371
#[tokio::test(flavor = "multi_thread")]
async fn a_long_reply_is_read_by_keyboard_before_the_focus_moves_on() -> anyhow::Result<()> {
    let launcher = FakeLauncher::new();
    let journey = Journey::start_fake(launcher.clone()).await?;
    launcher.push_run_for(
        journey.data_dir().join("workspaces/scroll"),
        FakeRun::fresh().with_history(long_reply_history()),
    );
    let thread = create_thread(&journey).await?;
    journey.click(&row(&thread)).await?;
    journey.opened(&thread).await?;
    journey
        .until("the long reply", async |page| {
            Ok(page
                .text(TRANSCRIPT)
                .await?
                .contains("Long reply end.")
                .then_some(()))
        })
        .await?;

    // From the first item, `j` down to the long reply: its top shows, the focus on it.
    journey
        .eval(
            "document.querySelector(arguments[0]).focus();",
            vec![json!(TRANSCRIPT)],
        )
        .await?;
    journey.keys("gg").await?;
    let mut item = journey
        .until("the first item focused", async |page| {
            let item = focused_item(page).await?;
            Ok((!item.text.is_empty() && item.scroll_top < 1.0).then_some(item))
        })
        .await?;
    for _ in 0..10 {
        if item.text.contains("Long reply start.") {
            break;
        }
        item = press_nav(&journey, "j").await?;
    }
    assert!(
        item.text.contains("Long reply start."),
        "j reaches the long reply: {item:?}"
    );
    assert!(
        item.top >= -1.0 && item.top < item.view,
        "its top shows when it gets the focus: {item:?}"
    );
    assert!(
        item.bottom - item.top > 2.0 * item.view,
        "the reply is several windows tall: {item:?}"
    );

    // `j` pages through it until its end shows, then moves on.
    let (steps, after) = page_through(&journey, "j", item).await?;
    assert!(
        steps >= 3,
        "several steps through a reply several windows tall: {steps}"
    );
    assert!(
        after.text.contains("After question?"),
        "j moves on to the next message: {after:?}"
    );

    // `k`: back to the long reply, its end showing, then up through it to its top, then on to the question.
    let back = press_nav(&journey, "k").await?;
    assert!(
        back.text.contains("Long reply start."),
        "k focuses the long reply: {back:?}"
    );
    assert!(
        back.bottom > 0.0 && back.bottom <= back.view + 1.0,
        "its end shows: {back:?}"
    );
    let (_, before) = page_through(&journey, "k", back).await?;
    assert!(
        before.text.contains("Long question?"),
        "k moves on to the question: {before:?}"
    );

    // `j` to the long reply again, then `Ctrl-d`: half a window down, the focus kept while the reply shows.
    let long = press_nav(&journey, "j").await?;
    assert!(
        long.text.contains("Long reply start."),
        "j focuses the long reply again: {long:?}"
    );
    journey
        .driver
        .action_chain()
        .key_down(Key::Control)
        .send_keys("d")
        .key_up(Key::Control)
        .perform()
        .await?;
    let half = journey
        .until("Ctrl-d to scroll", async |page| {
            let after = focused_item(page).await?;
            Ok((after.scroll_top > long.scroll_top + 1.0).then_some(after))
        })
        .await?;
    assert!(
        (half.scroll_top - long.scroll_top - long.view / 2.0).abs() <= 1.0,
        "half a window: {long:?} → {half:?}"
    );
    assert!(
        half.text.contains("Long reply start."),
        "the focus stays on the reply it shows: {half:?}"
    );

    journey.finish("reading a long reply by keyboard").await
}
