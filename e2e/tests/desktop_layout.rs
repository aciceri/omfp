//! The desktop layout at 1100, 1440 and 1920 px (E2E-38), headless Chromium (omp and git for real: the rows need git statuses): the workspace
//! widget's repository rows with a long name, a dirty status, an origin and an "also in" warning never draw one cell
//! over another nor past the row (UI-453); the side tab bar stays one row (UI-452); floating panels open below the
//! view's top bars and do not hide each other's title bars (UI-454). The composer and the transcript (E2E-41) at 1100,
//! 1280, 1440 and 1920 px: the composer's buttons on one row, the primary last at its bottom-right, idle and busy
//! alike (UI-480, UI-481), and the transcript's column where the composer's is (UI-485). Screenshots land in the run's
//! screenshot directory (`e2e::screenshot_dir`) as `omfp-desktop-<width>.png`, `omfp-desktop-<width>-panels.png` and
//! `omfp-composer-<width>-<state>.png`; nothing compares pixels.

use e2e::{Journey, row};
use fake_llm::{Reply, Step};
use omfp_protocol::method::{ThreadsCreate, ViewsMount, WorkspacesAddRepo};
use omfp_protocol::{
    AddRepoParams, CreateThreadParams, CreateWorkspaceParams, MountParams, RepoSource,
    ThreadWorkspace,
};
use omfp_testkit::TempRepo;
use serde_json::Value;
use thirtyfour::prelude::Key;

/// The repository name of the QA report that broke the rows.
const LONG: &str = "infra-terraform-modules-shared-configuration";
/// The other workspace linking the same checkout: the row's "also in" warning names it.
const OTHER: &str = "web-client-redesign";

/// A thread in a new workspace `name`, linking `link` when given.
fn new_thread(name: &str, link: Option<String>) -> CreateThreadParams {
    CreateThreadParams {
        title: None,
        workspace: ThreadWorkspace::New(CreateWorkspaceParams {
            name: Some(name.to_owned()),
            link,
        }),
        model: None,
        thinking_level: None,
        approval_mode: None,
        prompt: None,
        images: Vec::new(),
    }
}

/// Two workspaces sharing the dirty checkout `shared`: the selected thread's links it as [`LONG`] next to `other`;
/// returns the selected thread's workspace id.
async fn workspaces(
    journey: &Journey,
    shared: &TempRepo,
    other: &TempRepo,
) -> anyhow::Result<String> {
    let path = |repo: &TempRepo| repo.path().to_string_lossy().into_owned();
    let mut connection = journey.connect().await?;
    connection
        .call::<ThreadsCreate>(new_thread(OTHER, Some(path(shared))))
        .await?;
    let thread = connection
        .call::<ThreadsCreate>(new_thread("desktop-layout", None))
        .await?;
    for (name, repo) in [(LONG, shared), ("api-server", other)] {
        connection
            .call::<WorkspacesAddRepo>(AddRepoParams {
                workspace_id: thread.workspace_id.clone(),
                name: Some(name.to_owned()),
                source: RepoSource::Link { path: path(repo) },
            })
            .await?;
    }
    connection.close().await?;
    journey.click(&row(&thread.id)).await?;
    journey.opened(&thread.id).await?;
    let long = format!("[data-testid=workspace-repo][data-name=\"{LONG}\"]");
    journey
        .until("the long row with its status and warning", async |page| {
            let ready = page
                .count(&format!(
                    "{long} [data-testid=git-status], {long} [data-testid=shared-repo]"
                ))
                .await?
                == 2;
            Ok(ready.then_some(()))
        })
        .await?;
    Ok(thread.workspace_id)
}

/// The offenders of `script`'s returned list, failing with `what` unless it is empty.
async fn none(journey: &Journey, what: &str, script: &str) -> anyhow::Result<()> {
    let value = journey.eval(script, Vec::new()).await?;
    let offenders = value.as_array().cloned().unwrap_or_default();
    anyhow::ensure!(offenders.is_empty(), "{what}: {offenders:?}");
    Ok(())
}

/// Every repository row: no two of its parts overlap, none runs past the row, none is cut (UI-453).
const ROWS: &str = "
const parts = ['.omfp-workspace__repo-name', '[data-testid=repo-kind]', '[data-testid=git-status]',
  '[data-testid=remove-repo]', '[data-testid=repo-branch]', '[data-testid=repo-origin]', '[data-testid=shared-repo]'];
const out = [];
for (const row of document.querySelectorAll('[data-widget=\"omfp:workspace\"] [data-testid=workspace-repo]')) {
  const box = row.getBoundingClientRect();
  const shown = parts.map((css) => [css, row.querySelector(css)]).filter(([, e]) => e !== null)
    .map(([css, e]) => [css, e, e.getBoundingClientRect()]);
  for (const [css, e, r] of shown) {
    if (r.left < box.left - 0.5 || r.right > box.right + 0.5) out.push(`${row.dataset.name} ${css} out of the row`);
    if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'visible') out.push(`${row.dataset.name} ${css} cut`);
  }
  for (let i = 0; i < shown.length; i++) for (let j = i + 1; j < shown.length; j++) {
    const [a, , p] = shown[i]; const [b, , q] = shown[j];
    const x = Math.min(p.right, q.right) - Math.max(p.left, q.left);
    const y = Math.min(p.bottom, q.bottom) - Math.max(p.top, q.top);
    if (x > 0.5 && y > 0.5) out.push(`${row.dataset.name} ${a} over ${b}`);
  }
}
return out;";

/// The side slot's tab bar: its tabs on one row, the bar one `bar-h` tall (UI-452).
const SIDE_TABS: &str = "
const bar = [...document.querySelectorAll('[data-testid=view] .omfp-view__tabs:not([data-slot]) > .omfp-view__tabbar')][0];
if (!bar) return ['no side tab bar'];
const tops = new Set([...bar.querySelectorAll('[role=tab]')].map((t) => Math.round(t.getBoundingClientRect().top)));
const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
const height = bar.getBoundingClientRect().height;
return [...(tops.size === 1 ? [] : [`tabs on ${tops.size} rows`]), ...(height <= 2.6 * rem ? [] : [`bar ${height} px`])];";

/// Sizes the window `width` px wide and waits for the wide frame.
async fn window(journey: &Journey, width: u32) -> anyhow::Result<()> {
    journey.driver.set_window_rect(0, 0, width, 900).await?;
    journey
        .until(&format!("a {width} px window"), async |page| {
            let value = page.eval("return innerWidth;", Vec::new()).await?;
            Ok((value.as_u64() == Some(u64::from(width))).then_some(()))
        })
        .await?;
    journey
        .until("the wide frame", async |page| {
            let layout = page.attr("[data-testid=app]", "data-layout").await?;
            Ok((layout.as_deref() == Some("wide")).then_some(()))
        })
        .await
}

/// The rows and the side tabs at `width`, then a screenshot.
async fn layout_at(journey: &Journey, width: u32) -> anyhow::Result<()> {
    window(journey, width).await?;
    none(journey, &format!("repository rows at {width} px"), ROWS).await?;
    journey.fits("[data-widget=\"omfp:workspace\"]").await?;
    none(journey, &format!("side tabs at {width} px"), SIDE_TABS).await?;
    journey.screenshot(&format!("desktop-{width}")).await?;
    Ok(())
}

/// Two floating panels: below the header and the side tab bar, inside the window, each title bar in sight (UI-454).
const PANELS: &str = "
const view = document.querySelector('[data-testid=view]').getBoundingClientRect();
const bars = [...document.querySelectorAll('[data-testid=view] :is([data-widget=\"omfp:header\"], .omfp-view__tabbar)')]
  .map((e) => e.getBoundingClientRect()).filter((r) => r.height > 0 && r.top <= view.top + 1);
const below = Math.max(...bars.map((r) => r.bottom));
const out = [];
for (const panel of document.querySelectorAll('[data-testid=floating-widget]')) {
  const r = panel.getBoundingClientRect();
  if (r.top < below - 0.5) out.push(`${panel.dataset.panel} at ${r.top} above ${below}`);
  if (r.left < 0 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5) out.push(`${panel.dataset.panel} out of the window`);
  const bar = panel.querySelector('.omfp-floating-file__bar').getBoundingClientRect();
  const hit = document.elementFromPoint(bar.left + bar.width / 2, bar.top + bar.height / 2);
  if (hit === null || hit.closest('[data-testid=floating-widget]') !== panel) out.push(`${panel.dataset.panel}'s bar covered`);
}
return out;";

/// Waits until `script` finds no offender (the panels follow a resize a frame later), else fails naming them.
async fn settled(journey: &Journey, what: &str, script: &str) -> anyhow::Result<()> {
    let clear = journey
        .until(what, async |page| {
            let value = page.eval(script, Vec::new()).await?;
            Ok((value.as_array().is_some_and(Vec::is_empty)).then_some(()))
        })
        .await;
    if clear.is_err() {
        none(journey, what, script).await?;
    }
    Ok(())
}

/// Mounts two panels and checks where they open at `width`.
async fn panels_at(journey: &Journey, workspace_id: &str, width: u32) -> anyhow::Result<()> {
    let mut connection = journey.connect().await?;
    for id in ["todos-panel", "deploy-status"] {
        connection
            .call::<ViewsMount>(MountParams {
                workspace_id: workspace_id.to_owned(),
                id: id.to_owned(),
                widget: Some("omfp:todos".to_owned()),
                code: None,
                props: None,
                view: None,
                place: None,
                panel: true,
            })
            .await?;
    }
    connection.close().await?;
    journey
        .until("two panels", async |page| {
            Ok((page.count("[data-testid=floating-widget]").await? == 2).then_some(()))
        })
        .await?;
    settled(journey, &format!("floating panels at {width} px"), PANELS).await?;
    journey
        .screenshot(&format!("desktop-{width}-panels"))
        .await?;
    Ok(())
}

// spec: E2E-38, UI-452, UI-453, UI-454, TST-111
#[tokio::test(flavor = "multi_thread")]
async fn repository_rows_side_tabs_and_floating_panels_at_desktop_widths() -> anyhow::Result<()> {
    let shared = TempRepo::new()?;
    std::fs::write(shared.path().join("README.md"), "changed\n")?;
    let other = TempRepo::new()?;
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    journey.scheme("light").await?;
    let workspace_id = workspaces(&journey, &shared, &other).await?;
    for width in [1920, 1440, 1100] {
        layout_at(&journey, width).await?;
    }
    panels_at(&journey, &workspace_id, 1100).await?;
    journey.driver.set_window_rect(0, 0, 1920, 900).await?;
    settled(&journey, "floating panels at 1920 px", PANELS).await?;
    journey.screenshot("desktop-1920-panels").await?;
    journey
        .finish("desktop layout: repository rows, side tabs, floating panels")
        .await
}

/// The composer's box, its bar's buttons and the transcript's column: the visible buttons' tops (one row: one top),
/// their test ids in order, the gap between the last one's right edge and the bar's content edge, the box's bottom and
/// the last button's (both from the box's bottom), the box's height, the transcript content's left and right edges
/// less the box's, the key hints cut or drawn over a button, and the transcript's scrollbar width.
const COMPOSER: &str = "
const box = document.querySelector('[data-testid=composer] .omfp-compose');
const b = box.getBoundingClientRect();
const bar = box.querySelector('.omfp-compose__bar');
const style = getComputedStyle(bar);
const end = bar.getBoundingClientRect().right - parseFloat(style.paddingRight) - parseFloat(style.borderRightWidth);
const buttons = [...bar.querySelectorAll('button')].map((e) => [e.dataset.testid ?? '', e.getBoundingClientRect()])
  .filter(([, r]) => r.width > 0);
const last = buttons.at(-1)[1];
const transcript = document.querySelector('[data-testid=transcript]');
const column = transcript.querySelector('.omfp-transcript__content').getBoundingClientRect();
const hints = [...bar.querySelectorAll('.omfp-compose__hint')].map((e) => [e.textContent, e.getBoundingClientRect()]);
const shown = hints.filter(([, r]) => r.width > 0 && r.top < b.bottom && r.bottom > b.top);
const cut = shown.filter(([, r]) => r.left < b.left || r.right > b.right || r.bottom > b.bottom
  || buttons.some(([, q]) => Math.min(r.right, q.right) > Math.max(r.left, q.left)
    && Math.min(r.bottom, q.bottom) > Math.max(r.top, q.top))).map(([text]) => text);
return {
  tops: [...new Set(buttons.map(([, r]) => Math.round(r.top)))],
  order: buttons.map(([id]) => id),
  end: end - last.right,
  right: b.right - last.right,
  bottom: b.bottom - last.bottom,
  height: b.height,
  left: column.left - b.left,
  width: column.right - b.right,
  cut,
  scrollbar: transcript.offsetWidth - transcript.clientWidth,
};";

/// The composer's width in `rem` and whether it is tight (`data-tight`).
const TIGHT: &str = "
const form = document.querySelector('[data-testid=composer]');
const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
return [form.getBoundingClientRect().width / rem, form.dataset.tight === 'true'];";

/// Waits until the composer follows the window's width: tight exactly below 34rem (UI-481). Its `ResizeObserver` sets
/// `data-tight` a frame or more after a resize, so a measure before that has the long labels of the width before:
/// coming from 1280 px, the busy actions wrapped at 1100 px under a loaded machine (TST-111).
async fn composer_follows(journey: &Journey) -> anyhow::Result<()> {
    journey
        .until("the composer to follow the width (UI-481)", async |page| {
            let (width, tight): (f64, bool) =
                serde_json::from_value(page.eval(TIGHT, Vec::new()).await?)?;
            Ok(((width < 34.0) == tight).then_some(()))
        })
        .await
}

/// The composer at `width` in `state` (`idle`, `busy`): its measures, a screenshot, and what is wrong with them.
async fn composer_at(
    journey: &Journey,
    width: u32,
    state: &str,
) -> anyhow::Result<(Value, Vec<String>)> {
    window(journey, width).await?;
    composer_follows(journey).await?;
    let value = journey.eval(COMPOSER, Vec::new()).await?;
    eprintln!("composer at {width} px, {state}: {value}");
    journey
        .screenshot(&format!("composer-{width}-{state}"))
        .await?;
    let number = |key: &str| value[key].as_f64().unwrap_or(f64::NAN);
    let mut wrong = Vec::new();
    let at = format!("at {width} px, {state}");
    if value["tops"].as_array().map(Vec::len) != Some(1) {
        wrong.push(format!("buttons on several rows {at}: {}", value["tops"]));
    }
    if value["order"].as_array().and_then(|order| order.last()) != Some(&Value::from("send")) {
        wrong.push(format!("the primary is not last {at}: {}", value["order"]));
    }
    if number("end").abs() > 0.5 {
        wrong.push(format!(
            "the primary ends {} px before the bar {at}",
            number("end")
        ));
    }
    if number("left").abs() > 0.5 || number("width").abs() > 0.5 {
        wrong.push(format!(
            "the transcript's column is off the composer's {at}: left {}, right {}",
            number("left"),
            number("width")
        ));
    }
    if value["cut"].as_array().is_some_and(|cut| !cut.is_empty()) {
        wrong.push(format!("key hints cut or covered {at}: {}", value["cut"]));
    }
    Ok((value, wrong))
}

/// The window widths of E2E-41: the wide frame's narrowest, a 13" laptop's, and two larger.
const COMPOSER_WIDTHS: [u32; 4] = [1920, 1440, 1280, 1100];

/// The selected tab of the side slot.
async fn side_tab(journey: &Journey) -> anyhow::Result<Option<String>> {
    let value = journey
        .eval(
            "const tab = document.querySelector(
               '[data-testid=view] .omfp-view__tabs:not([data-slot]) [data-testid=view-tab][aria-selected=true]');
             return tab === null ? null : tab.textContent;",
            Vec::new(),
        )
        .await?;
    Ok(value.as_str().map(str::to_owned))
}

/// The header's workspace chip is a button that shows `omfp:workspace` (UI-484): at 1920 px (the full header), with
/// Changes selected in the side slot, pressing it selects the Workspace tab again.
async fn chip_reveals_workspace(journey: &Journey) -> anyhow::Result<()> {
    window(journey, 1920).await?;
    journey
        .eval(
            "[...document.querySelectorAll('[data-testid=view] .omfp-view__tabs:not([data-slot]) [data-testid=view-tab]')]
               .find((t) => t.textContent === 'Changes').click();",
            Vec::new(),
        )
        .await?;
    journey
        .until("the Changes tab", async |page| {
            Ok((side_tab(page).await?.as_deref() == Some("Changes")).then_some(()))
        })
        .await?;
    journey
        .click("[data-testid=thread-header] [data-testid=workspace-chip]")
        .await?;
    journey
        .until("the Workspace tab", async |page| {
            Ok((side_tab(page).await?.as_deref() == Some("Workspace")).then_some(()))
        })
        .await
}

/// The descriptions matching `css` that are cut (wider or taller than their box), by text.
async fn cut(journey: &Journey, css: &str) -> anyhow::Result<Vec<Value>> {
    let value = journey
        .eval(
            "return [...document.querySelectorAll(arguments[0])]
               .filter((e) => e.scrollWidth > e.clientWidth + 1 || e.scrollHeight > e.clientHeight + 1)
               .map((e) => e.textContent);",
            vec![Value::from(css)],
        )
        .await?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

/// At 1920 px nothing is cut where there is room (UI-486): the `:` menu's descriptions and the which-key panel's.
async fn menus_not_cut(journey: &Journey) -> anyhow::Result<()> {
    let blur = "if (document.activeElement) { document.activeElement.blur(); }";
    journey.eval(blur, Vec::new()).await?;
    journey.keys(":").await?;
    journey.until_focused("command-line-input").await?;
    let menu = cut(journey, ".omfp-cmdmenu__description").await?;
    journey.screenshot("command-menu-1920").await?;
    journey.keys(Key::Escape).await?;
    journey.gone("[data-testid=command-line]").await?;
    journey.eval(blur, Vec::new()).await?;
    journey.keys(" ").await?;
    journey
        .element("[data-testid=which-key][data-keys=Space]")
        .await?;
    let which = cut(journey, ".omfp-whichkey__description").await?;
    journey.screenshot("which-key-1920").await?;
    journey.keys(Key::Escape).await?;
    anyhow::ensure!(
        menu.is_empty() && which.is_empty(),
        "cut in the : menu {menu:?}, in which-key {which:?}"
    );
    Ok(())
}

// spec: E2E-41, UI-480, UI-481, UI-484, UI-485, UI-486, TST-111
#[tokio::test(flavor = "multi_thread")]
async fn the_composer_keeps_its_primary_bottom_right_and_the_transcript_its_column()
-> anyhow::Result<()> {
    let long = (1..=40)
        .map(|line| format!("Line {line} of a reply long enough to scroll the transcript."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let journey = Journey::start(vec![
        Step::reply(Reply::text(long)),
        Step::reply(Reply::text("Held reply.")).gated(),
    ])
    .await?;
    journey.connected().await?;
    journey.scheme("light").await?;
    let thread = journey.create_thread("composer layout journey").await?;
    journey.send("first").await?;
    journey.reply("Line 40").await?;
    journey.status(&thread, "idle").await?;
    let mut wrong = Vec::new();
    let mut idle = Vec::new();
    for width in COMPOSER_WIDTHS {
        let (value, found) = composer_at(&journey, width, "idle").await?;
        idle.push(value);
        wrong.extend(found);
    }
    journey.send("second").await?;
    journey.fake.wait_for_gate(1).await;
    journey.busy().await?;
    for (width, idle) in COMPOSER_WIDTHS.into_iter().zip(idle) {
        let (busy, found) = composer_at(&journey, width, "busy").await?;
        wrong.extend(found);
        // The primary stays where it was and the composer keeps its height when a turn starts.
        for key in ["right", "bottom", "height"] {
            let (before, after) = (idle[key].as_f64(), busy[key].as_f64());
            if before.zip(after).is_none_or(|(a, b)| (a - b).abs() > 0.5) {
                wrong.push(format!(
                    "{key} at {width} px: {before:?} idle, {after:?} busy"
                ));
            }
        }
    }
    anyhow::ensure!(wrong.is_empty(), "the composer's layout: {wrong:#?}");
    journey.fake.release_gate(1);
    journey.status(&thread, "idle").await?;
    chip_reveals_workspace(&journey).await?;
    menus_not_cut(&journey).await?;
    journey.finish("E2E-41").await
}
