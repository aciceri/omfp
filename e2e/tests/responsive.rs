//! The responsive frame (UI-200..UI-209) in headless Chromium at 1400, 900 and 390 px: wide keeps the left column and
//! the splitters; narrow collapses the column to a strip that opens over the content and keeps the chat header on one
//! row; a phone shows one area at a time behind a bottom bar, without splitters. Screenshots land in the run's
//! screenshot directory as `omfp-responsive-<width>-<scheme>.png`; nothing compares pixels.
//!
//! A touch phone (E2E-36) emulated over CDP (touch, `mobile` metrics: `pointer: coarse`, `hover: none`) at 412 × 915
//! and sideways at 915 × 412, shots as `omfp-touch-*.png`.

use e2e::Journey;
use fake_llm::{Reply, Step};
use omfp_daemon::agent::FakeLauncher;
use omfp_protocol::method::{ThreadsCreate, WorkspacesCreate};
use omfp_protocol::{CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use serde_json::json;
use thirtyfour::prelude::*;

/// The first message, whose placeholder title (DMN-073) is too long for the full header's top row at 1400 px.
const LONG_PROMPT: &str =
    "Summarize every responsive breakpoint thoroughly, including narrow phones";

/// The `data-layout` the frame settles on.
async fn layout(journey: &Journey, expected: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("data-layout {expected}"), async |page| {
            let value = page.attr("[data-testid=app]", "data-layout").await?;
            Ok((value.as_deref() == Some(expected)).then_some(()))
        })
        .await
}

/// The rendered height of the first element matching `css`, in px.
async fn height(journey: &Journey, css: &str) -> anyhow::Result<f64> {
    let value = journey
        .eval(
            "return document.querySelector(arguments[0]).getBoundingClientRect().height;",
            vec![serde_json::json!(css)],
        )
        .await?;
    Ok(value.as_f64().unwrap_or_default())
}

async fn shots(journey: &Journey, width: u32) -> anyhow::Result<()> {
    for scheme in ["light", "dark"] {
        journey.scheme(scheme).await?;
        journey.until_scheme(scheme).await?;
        journey
            .screenshot(&format!("responsive-{width}-{scheme}"))
            .await?;
    }
    journey.scheme("light").await?;
    Ok(())
}

/// Wide: the left column, its splitter and the view's splitters; no strip; the full header on two rows.
async fn wide(journey: &Journey) -> anyhow::Result<()> {
    layout(journey, "wide").await?;
    // The desktop journeys have a mouse (TST-116): hover rules apply, the touch rules do not.
    let mouse = journey
        .eval(
            "return matchMedia('(hover: hover) and (pointer: fine)').matches;",
            Vec::new(),
        )
        .await?;
    anyhow::ensure!(mouse == json!(true), "the desktop browser has no mouse");
    anyhow::ensure!(journey.count("[data-testid=left-column]").await? == 1);
    anyhow::ensure!(journey.count("[data-testid=left-splitter]").await? == 1);
    anyhow::ensure!(journey.count("[data-testid=left-strip]").await? == 0);
    anyhow::ensure!(journey.count("[data-testid=splitter]").await? > 0);
    // The full header keeps its chips and the session controls on one bar (two rows, not three).
    let bar = height(
        journey,
        "[data-testid=thread-header] .omfp-chat-header__bar",
    )
    .await?;
    anyhow::ensure!(bar < 40.0, "the header's bar wraps: {bar} px tall");
    // Its top row stays one line with a title too long for it: the title gives way, the controls do not wrap.
    let top = height(
        journey,
        "[data-testid=thread-header] .omfp-chat-header__top",
    )
    .await?;
    anyhow::ensure!(top < 40.0, "the header's top row wraps: {top} px tall");
    journey.fits("[data-testid=thread-header]").await?;
    // The composer's text and caret start at its bar's first chip (UI-337).
    anyhow::ensure!(journey.composer_aligned("[data-testid=composer]").await? == 1);
    shots(journey, 1400).await
}

/// Narrow (half a screen): the strip, a one-row header with its ⋯ menu, nothing scrolling sideways, side cells at
/// a readable width; the toggle opens the column over the content and Escape closes it.
async fn narrow(journey: &Journey) -> anyhow::Result<()> {
    layout(journey, "narrow").await?;
    journey.element("[data-testid=left-strip]").await?;
    journey.element("[data-testid=header-more]").await?;
    let header = height(journey, "[data-testid=thread-header]").await?;
    anyhow::ensure!(header < 64.0, "the header wraps: {header} px tall");
    journey.fits("[data-testid=view]").await?;
    journey.fits("[data-testid=thread-header]").await?;
    // The ⋯ is on the header's row, inside it, and nothing covers it.
    let more = journey
        .eval(
            "const more = document.querySelector('[data-testid=header-more]').getBoundingClientRect();
             const header = document.querySelector('[data-testid=thread-header]').getBoundingClientRect();
             const hit = document.elementFromPoint(more.left + more.width / 2, more.top + more.height / 2);
             return more.width > 0 && more.right <= header.right + 0.5
               && hit !== null && hit.closest('[data-testid=header-more]') !== null;",
            Vec::new(),
        )
        .await?;
    anyhow::ensure!(
        more.as_bool() == Some(true),
        "the header's ⋯ is cut or covered"
    );
    // Side by side, no cell is squashed under 16rem (UI-203).
    let narrowest = journey
        .eval(
            "const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
             const cells = [...document.querySelectorAll('.omfp-view__row:not([data-tabbed]) > .omfp-view__cell')];
             return Math.min(...cells.map(c => c.getBoundingClientRect().width / rem));",
            Vec::new(),
        )
        .await?;
    let narrowest = narrowest.as_f64().unwrap_or_default();
    anyhow::ensure!(narrowest >= 15.9, "a cell is {narrowest} rem wide");
    shots(journey, 900).await?;
    journey.click("[data-testid=left-toggle]").await?;
    journey
        .element("[data-testid=left-column][data-open]")
        .await?;
    journey.screenshot("responsive-900-open").await?;
    journey.keys(Key::Escape).await?;
    journey.gone("[data-testid=left-column][data-open]").await
}

/// Phone: the bottom bar, no visible splitters, 44 px touch targets, the threads as a full-screen panel.
async fn phone(journey: &Journey) -> anyhow::Result<()> {
    layout(journey, "mobile").await?;
    journey.element("[data-testid=area-bar]").await?;
    anyhow::ensure!(journey.count("[data-testid=left-strip]").await? == 0);
    let splitters = journey
        .eval(
            "return [...document.querySelectorAll('[data-testid=splitter], [data-testid=left-splitter]')]
               .filter(e => e.getBoundingClientRect().width > 0).length;",
            Vec::new(),
        )
        .await?;
    anyhow::ensure!(
        splitters.as_u64() == Some(0),
        "visible splitters: {splitters}"
    );
    journey.fits("[data-testid=app]").await?;
    // Touch targets of at least 44 px; icon buttons at least 44 × 44 (UI-205).
    let lowest = journey
        .eval(
            "return Math.min(...[...document.querySelectorAll('[data-testid=area-button], [data-testid=send], [data-testid=header-more]')]
               .map(e => e.getBoundingClientRect().height));",
            Vec::new(),
        )
        .await?;
    let lowest = lowest.as_f64().unwrap_or_default();
    anyhow::ensure!(lowest >= 44.0, "a touch target is {lowest} px tall");
    let narrowest = journey
        .eval(
            "return Math.min(...[...document.querySelectorAll('[data-testid=send], [data-testid=header-more]')]
               .map(e => e.getBoundingClientRect().width));",
            Vec::new(),
        )
        .await?;
    let narrowest = narrowest.as_f64().unwrap_or_default();
    anyhow::ensure!(narrowest >= 44.0, "an icon button is {narrowest} px wide");
    // The chat's column has the same narrow margin on both sides: no scrollbar's gutter (this browser's are
    // classic) on either.
    let margins = journey
        .eval(
            "const box = document.querySelector('[data-testid=composer]').getBoundingClientRect();
             return [box.left, innerWidth - box.right];",
            Vec::new(),
        )
        .await?;
    let margins: Vec<f64> = serde_json::from_value(margins)?;
    anyhow::ensure!(
        matches!(margins[..], [left, right] if (left - right).abs() <= 1.0 && left <= 16.0),
        "the composer's margins are {margins:?}"
    );
    // The actions sit beside the text, at its bottom, not on a row of their own under it; with one line of text the
    // two share a centre (UI-423).
    let beside = journey
        .eval(
            "const input = document.querySelector('[data-testid=composer-input]').getBoundingClientRect();
             const send = document.querySelector('[data-testid=send]').getBoundingClientRect();
             const centre = (r) => r.top + r.height / 2;
             return send.left >= input.right - 0.5 && Math.abs(centre(send) - centre(input)) <= 2;",
            Vec::new(),
        )
        .await?;
    anyhow::ensure!(
        beside.as_bool() == Some(true),
        "the composer's actions are not beside its text"
    );
    shots(journey, 390).await?;
    journey
        .click("[data-testid=area-button][data-area=threads]")
        .await?;
    journey
        .element("[data-testid=left-column][data-open]")
        .await?;
    journey.screenshot("responsive-390-threads").await?;
    journey
        .click("[data-testid=area-button][data-area=chat]")
        .await?;
    journey.gone("[data-testid=left-column][data-open]").await
}

/// Measuring helpers of the touch checks: `shown(e)` (laid out and not visually hidden, as React Aria's dismiss
/// buttons are inside a clipped box); `seen(e)`, its opacity with its ancestors'; `small(css, scope, square)`, the
/// shown targets under 44 px tall (or wide too when `square`) as `name w×h` strings.
const TOUCH_JS: &str = "
const clipped = (e) => { for (let n = e; n; n = n.parentElement) if (getComputedStyle(n).clipPath !== 'none') return true; return false; };
const shown = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && !clipped(e); };
const seen = (e) => { let o = 1; for (let n = e; n; n = n.parentElement) o *= Number(getComputedStyle(n).opacity); return o; };
const name = (e) => e.dataset.testid ?? e.getAttribute('aria-label') ?? e.textContent.trim().slice(0, 24);
const small = (css, scope, square) => [...scope.querySelectorAll(css)].filter(shown)
  .map((e) => [e, e.getBoundingClientRect()])
  .filter(([, r]) => r.height < 43.5 || (square && r.width < 43.5))
  .map(([e, r]) => `${name(e)} ${Math.round(r.width)}x${Math.round(r.height)}`);
";

/// Runs `body` after [`TOUCH_JS`] and returns the list of offenders it returns, failing with `what` unless empty.
async fn none(journey: &Journey, what: &str, body: &str) -> anyhow::Result<()> {
    let value = journey
        .eval(&format!("{TOUCH_JS}\n{body}"), Vec::new())
        .await?;
    let offenders = value.as_array().cloned().unwrap_or_default();
    anyhow::ensure!(offenders.is_empty(), "{what}: {offenders:?}");
    Ok(())
}

/// Emulates a touch phone of `width` × `height` px: touch events, a coarse primary pointer, no hover.
async fn touch_screen(journey: &Journey, width: u32, height: u32) -> anyhow::Result<()> {
    let cdp = journey.driver.cdp();
    cdp.send_raw(
        "Emulation.setTouchEmulationEnabled",
        json!({ "enabled": true, "maxTouchPoints": 5 }),
    )
    .await?;
    cdp.send_raw(
        "Emulation.setDeviceMetricsOverride",
        json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": true }),
    )
    .await?;
    journey
        .until("a touch-only screen", async |page| {
            let value = page
                .eval(
                    "return matchMedia('(hover: none) and (pointer: coarse)').matches;",
                    Vec::new(),
                )
                .await?;
            Ok((value.as_bool() == Some(true)).then_some(()))
        })
        .await
}

/// The chat on a touch phone (UI-419): no chat header, the mode line at the top in its place with the title keeping
/// room and the thread's `⋯`, whose popover, an overlay portalled out of the app, holds the model and has touch-sized
/// controls (UI-413). Boxes are measured at rest (TST-110).
async fn touch_chat(journey: &Journey) -> anyhow::Result<()> {
    journey
        .settled("[data-testid=mode-bar] [data-testid=header-more]")
        .await?;
    anyhow::ensure!(journey.count("[data-testid=thread-header]").await? == 0);
    // Above the view, the state's mark on the same line as the title (not wrapped away under it).
    let placed = journey
        .eval(
            "const bar = document.querySelector('[data-testid=mode-bar]').getBoundingClientRect();
             const view = document.querySelector('[data-testid=view]').getBoundingClientRect();
             const mark = document.querySelector('[data-testid=mode-thread-state] [role=img]').getBoundingClientRect();
             return bar.bottom <= view.top + 0.5 && mark.width > 0
               && mark.top >= bar.top - 0.5 && mark.bottom <= bar.bottom + 0.5 && mark.right <= bar.right;",
            Vec::new(),
        )
        .await?;
    anyhow::ensure!(
        placed.as_bool() == Some(true),
        "the mode line is not above the view, or its state mark is cut"
    );
    let title = journey
        .eval(
            "return document.querySelector('[data-testid=mode-thread-title]').getBoundingClientRect().width;",
            Vec::new(),
        )
        .await?;
    let title = title.as_f64().unwrap_or_default();
    anyhow::ensure!(title >= 120.0, "the thread title is {title} px wide");
    // Always shown on touch, "Fork from here" sits on the name line, above the message's text, not over it (UI-660).
    let clear = journey
        .eval(
            "const fork = document.querySelector('[data-testid=message-fork]');
             const body = fork.closest('.omfp-msg').querySelector('.omfp-msg__body').getBoundingClientRect();
             return fork.getBoundingClientRect().bottom <= body.top + 0.5;",
            Vec::new(),
        )
        .await?;
    anyhow::ensure!(
        clear.as_bool() == Some(true),
        "Fork from here lies over the message's text"
    );
    journey.screenshot("touch-412-chat").await?;
    journey.click("[data-testid=header-more]").await?;
    journey
        .element(".omfp-popdialog [data-testid=model-select]")
        .await?;
    journey.settled(".omfp-popdialog").await?;
    none(
        journey,
        "small controls in the header's popover",
        "return small('button, [role=button]', document.querySelector('.omfp-popdialog'), false);",
    )
    .await?;
    none(
        journey,
        "a popover out of the window",
        "const width = visualViewport.width;
         return [...document.querySelectorAll('.omfp-popover')].filter(shown)
           .map((e) => e.getBoundingClientRect()).filter((r) => r.left < 0 || r.right > width + 0.5)
           .map((r) => `${Math.round(r.left)}..${Math.round(r.right)} of ${width}`);",
    )
    .await?;
    journey.screenshot("touch-412-more").await?;
    journey.keys(Key::Escape).await?;
    journey.gone(".omfp-popdialog").await
}

/// The threads on a touch phone: no control a tap can hit is invisible (UI-415), the small icons are touch targets
/// (UI-416), inputs are 16 px or more (UI-417); the workspace's delete dialog has touch-sized buttons (UI-413) and no
/// key chips (UI-418).
async fn touch_threads(journey: &Journey) -> anyhow::Result<()> {
    journey
        .click("[data-testid=area-button][data-area=threads]")
        .await?;
    let column = "document.querySelector('[data-testid=left-column]')";
    journey
        .settled("[data-testid=left-column][data-open]")
        .await?;
    none(
        journey,
        "invisible controls",
        &format!(
            "return [...{column}.querySelectorAll('button, [role=button], input')].filter(shown)
               .filter((e) => seen(e) < 0.5).map(name);"
        ),
    )
    .await?;
    none(
        journey,
        "small icon targets",
        "return small(`[data-testid=workspace-menu], [data-testid=delete-workspace], [data-testid=row-delete],
           [data-testid=workspace-collapse]`, document, true);",
    )
    .await?;
    none(
        journey,
        "inputs under 16 px",
        &format!(
            "return [...{column}.querySelectorAll('input')].filter(shown)
               .filter((e) => parseFloat(getComputedStyle(e).fontSize) < 16).map(name);"
        ),
    )
    .await?;
    journey.screenshot("touch-412-threads").await?;
    journey.click("[data-testid=delete-workspace]").await?;
    let dialog = "document.querySelector('[data-testid=delete-workspace-dialog]')";
    journey
        .settled("[data-testid=delete-workspace-dialog]")
        .await?;
    none(
        journey,
        "small dialog buttons",
        &format!("return small('button', {dialog}, false);"),
    )
    .await?;
    none(
        journey,
        "key chips on the dialog's buttons",
        &format!("return [...{dialog}.querySelectorAll('.omfp-kbd')].filter(shown).map(name);"),
    )
    .await?;
    journey.screenshot("touch-412-dialog").await?;
    journey
        .click("[data-testid=cancel-delete-workspace]")
        .await?;
    journey.gone("[data-testid=delete-workspace-dialog]").await
}

// spec: E2E-36, UI-413, UI-414, UI-415, UI-416, UI-417, UI-418, UI-419, TST-110
#[tokio::test(flavor = "multi_thread")]
async fn a_touch_phone_gets_touch_sizes_in_both_orientations() -> anyhow::Result<()> {
    let journey = Journey::start(vec![Step::reply(Reply::text("hello from a phone"))]).await?;
    journey.connected().await?;
    let thread = journey
        .create_thread("a touch journey with a rather long workspace name")
        .await?;
    journey.send(LONG_PROMPT).await?;
    journey.reply("hello from a phone").await?;
    journey.status(&thread, "idle").await?;
    journey.driver.set_window_rect(0, 0, 412, 915).await?;
    touch_screen(&journey, 412, 915).await?;
    layout(&journey, "mobile").await?;
    touch_chat(&journey).await?;
    touch_threads(&journey).await?;
    // Sideways, a phone keeps the phone layout (UI-414).
    journey.driver.set_window_rect(0, 0, 915, 412).await?;
    touch_screen(&journey, 915, 412).await?;
    journey
        .click("[data-testid=area-button][data-area=chat]")
        .await?;
    layout(&journey, "mobile").await?;
    journey.element("[data-testid=area-bar]").await?;
    journey
        .element("[data-testid=mode-bar] [data-testid=header-more]")
        .await?;
    journey.fits("[data-testid=app]").await?;
    journey.screenshot("touch-915-landscape").await?;
    journey.finish("touch").await
}

// spec: UI-200, UI-201, UI-202, UI-203, UI-204, UI-205, UI-206, UI-207, UI-209, UI-337, TST-116
#[tokio::test(flavor = "multi_thread")]
async fn the_frame_adapts_to_its_width() -> anyhow::Result<()> {
    let journey = Journey::start(vec![Step::reply(Reply::text(
        "hello from the responsive journey",
    ))])
    .await?;
    journey.connected().await?;
    journey.driver.set_window_rect(0, 0, 1400, 1000).await?;
    let thread = journey
        .create_thread("a responsive journey with a rather long workspace name")
        .await?;
    journey.send(LONG_PROMPT).await?;
    journey.reply("hello from the responsive journey").await?;
    journey.status(&thread, "idle").await?;
    wide(&journey).await?;
    journey.driver.set_window_rect(0, 0, 900, 1000).await?;
    narrow(&journey).await?;
    journey.driver.set_window_rect(0, 0, 390, 844).await?;
    phone(&journey).await?;
    journey.finish("responsive").await
}

/// Measures the actual scroll ancestor, not an implementation-specific wrapper.
const SIDEBAR_SCROLL: &str = "
const column = document.querySelector('[data-testid=left-column]');
const row = column.querySelector('[data-testid=thread-row]');
let scroller = row.parentElement;
while (scroller && !(scroller.scrollHeight > scroller.clientHeight &&
  ['auto', 'scroll'].includes(getComputedStyle(scroller).overflowY))) scroller = scroller.parentElement;
if (!scroller) throw new Error('no overflowing thread list');
const box = (e) => { const r = e.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
return {
  header: ['.omfp-brand', '[data-testid=thread-filter]']
    .map((css) => box(column.querySelector(css))),
  row: box(row), scroll: scroller.scrollTop, list: box(scroller),
  outer: column.scrollTop, page: document.scrollingElement.scrollTop,
};";

// spec: UI-201, UI-204, UI-209
#[tokio::test(flavor = "multi_thread")]
async fn detached_sidebar_keeps_its_controls_while_only_threads_scroll() -> anyhow::Result<()> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    journey.connected().await?;
    let last = seed_sidebar_threads(&journey).await?;

    for (width, expected) in [(1400, "wide"), (900, "narrow"), (390, "mobile")] {
        journey.driver.set_window_rect(0, 0, width, 844).await?;
        if expected == "mobile" {
            touch_screen(&journey, width, 844).await?;
        }
        layout(&journey, expected).await?;
        if expected == "narrow" {
            journey.click("[data-testid=left-toggle]").await?;
        } else if expected == "mobile" {
            journey
                .click("[data-testid=area-button][data-area=threads]")
                .await?;
        }
        journey.settled("[data-testid=left-column]").await?;
        // Focus can reveal a row when the drawer opens; start the wheel measurement at the top.
        journey
            .eval(
                "document.querySelector('[data-testid=thread-row]').scrollIntoView({block: 'start'});",
                Vec::new(),
            )
            .await?;
        let before = journey.eval(SIDEBAR_SCROLL, Vec::new()).await?;
        if expected != "wide" {
            anyhow::ensure!(
                before["header"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|rect| rect[1].as_f64().unwrap() >= 0.0),
                "{expected}: header is outside the viewport: {before}"
            );
        }
        let list = &before["list"];
        journey
            .driver
            .cdp()
            .send_raw(
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseWheel",
                    "x": list[0].as_f64().unwrap() + list[2].as_f64().unwrap() / 2.0,
                    "y": list[1].as_f64().unwrap() + list[3].as_f64().unwrap() - 50.0,
                    "deltaX": 0, "deltaY": 400,
                }),
            )
            .await?;
        let after =
            journey
                .until("the thread list moving after a wheel", async |page| {
                    let state = page.eval(SIDEBAR_SCROLL, Vec::new()).await?;
                    Ok((state["scroll"].as_f64().unwrap()
                        > before["scroll"].as_f64().unwrap() + 200.0)
                        .then_some(state))
                })
                .await?;
        anyhow::ensure!(
            after["row"][1].as_f64().unwrap() < before["row"][1].as_f64().unwrap() - 200.0,
            "{expected}: rows did not move: {before} -> {after}"
        );
        if expected == "wide" {
            // Preserve the wide column's existing whole-column scrolling.
            anyhow::ensure!(
                after["header"][0][1].as_f64().unwrap()
                    < before["header"][0][1].as_f64().unwrap() - 200.0,
                "wide scrolling changed: {before} -> {after}"
            );
            continue;
        }
        assert_eq!(
            after["header"], before["header"],
            "{expected}: header moved"
        );
        assert_eq!(after["outer"], json!(0), "{expected}: drawer scrolled");
        assert_eq!(after["page"], json!(0), "{expected}: page scrolled");
        journey
            .screenshot(&format!("sidebar-{width}-scrolled"))
            .await?;
        check_sidebar_controls(&journey, &last, expected, &before["header"]).await?;
    }
    journey.finish("sidebar-scroll").await
}

async fn seed_sidebar_threads(journey: &Journey) -> anyhow::Result<String> {
    let mut connection = journey.connect().await?;
    let workspace = connection
        .call::<WorkspacesCreate>(CreateWorkspaceParams {
            name: Some("Sidebar workspace".to_owned()),
            link: None,
        })
        .await?;
    for index in 0..40 {
        connection
            .call::<ThreadsCreate>(CreateThreadParams {
                title: Some(format!("Sidebar {index:02}")),
                workspace: ThreadWorkspace::Id(workspace.id.clone()),
                model: None,
                thinking_level: None,
                approval_mode: None,
                prompt: None,
                images: Vec::new(),
            })
            .await?;
    }
    connection.close().await?;
    journey
        .until("40 sidebar threads", async |page| {
            Ok((page.count("[data-testid=thread-row]").await? == 40).then_some(()))
        })
        .await?;
    Ok(e2e::row(
        journey
            .thread_ids()
            .await?
            .last()
            .ok_or_else(|| anyhow::anyhow!("no sidebar threads"))?,
    ))
}

async fn check_sidebar_controls(
    journey: &Journey,
    last: &str,
    expected: &str,
    header: &serde_json::Value,
) -> anyhow::Result<()> {
    // The filter and "+ workspace" (at the list's end) remain reachable, focusable and editable after scrolling.
    journey.click("[data-testid=add-workspace]").await?;
    journey
        .type_into("[data-testid=new-workspace-input]", "Unsubmitted workspace")
        .await?;
    let workspace_input = journey.element("[data-testid=new-workspace-input]").await?;
    workspace_input.send_keys(Key::Control + "a").await?;
    workspace_input.send_keys(Key::Backspace).await?;
    journey.click("[data-testid=thread-filter]").await?;
    journey
        .type_into("[data-testid=thread-filter]", "Sidebar 39")
        .await?;
    journey
        .until("the filtered last thread", async |page| {
            let titles = page.texts("[data-testid=thread-row]").await?;
            Ok((titles.len() == 1 && titles[0].contains("Sidebar 39")).then_some(()))
        })
        .await?;
    journey.click("[data-testid=clear-thread-filter]").await?;
    journey
        .until("all sidebar threads restored", async |page| {
            Ok((page.count("[data-testid=thread-row]").await? == 40).then_some(()))
        })
        .await?;
    let focused_row = journey
        .eval(
            "const row = document.querySelector(arguments[0]); row.focus();
             return document.activeElement === row;",
            vec![json!(last)],
        )
        .await?;
    assert_eq!(
        focused_row,
        json!(true),
        "{expected}: row did not take focus"
    );
    let focused = journey.eval(SIDEBAR_SCROLL, Vec::new()).await?;
    assert_eq!(
        &focused["header"], header,
        "{expected}: focusing a row moved the header"
    );
    // Select a different thread at each breakpoint; reselecting the current thread is not this regression.
    if expected == "narrow" {
        journey.click(last).await?;
    } else {
        journey.click("[data-testid=thread-row]").await?;
    }
    journey.gone("[data-testid=left-column][data-open]").await
}
