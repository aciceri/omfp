//! Reordering the left column (E2E-60), against a daemon on the fake launcher in headless Chromium: WebDriver pointer
//! actions drag a thread within its workspace, a thread onto another workspace and a workspace section (UI-621,
//! UI-622); `Alt-ArrowDown` moves the focused thread and the live region says where it went (UI-624, UI-625); the
//! order is the daemon's, so it survives a reload, and a moved thread is not unread (UI-620, DMN-280, DMN-284).

use e2e::Journey;
use omfp_daemon::agent::FakeLauncher;
use omfp_protocol::method::{ThreadsCreate, WorkspacesCreate};
use omfp_protocol::{CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use serde_json::{Value, json};
use thirtyfour::prelude::*;

/// The column as `[workspace id, [thread id…]]`, top to bottom.
type Column = Vec<(String, Vec<String>)>;

const COLUMN: &str =
    "return [...document.querySelectorAll('[data-testid=workspace-section]')].map((section) => [
    section.dataset.workspaceId,
    [...section.querySelectorAll('[data-testid=thread-row]')].map((row) => row.dataset.threadId),
  ]);";

async fn column(journey: &Journey) -> anyhow::Result<Column> {
    Ok(serde_json::from_value(
        journey.eval(COLUMN, Vec::new()).await?,
    )?)
}

/// Waits until the column shows `expected`.
async fn until_column(journey: &Journey, what: &str, expected: &Column) -> anyhow::Result<()> {
    journey
        .until(what, async |page| {
            Ok((column(page).await? == *expected).then_some(()))
        })
        .await
}

fn row(id: &str) -> String {
    format!("[data-testid=thread-row][data-thread-id=\"{id}\"]")
}

/// The unread mark of thread `id`'s row.
fn unread(id: &str) -> String {
    format!("{} [data-testid=unread]", row(id))
}

fn section_name(id: &str) -> String {
    format!(
        "[data-testid=workspace-section][data-workspace-id=\"{id}\"] [data-testid=workspace-section-name]"
    )
}

/// Drags `from` with the mouse onto the upper part of `onto` (just above its middle: "before it"), through a first
/// move of 8 px that starts the drag (UI-621).
async fn drag(journey: &Journey, from: &str, onto: &str) -> anyhow::Result<()> {
    let source = journey.element(from).await?;
    let target = journey.element(onto).await?;
    let height = target.rect().await?.height;
    // Offsets are from the element's centre: a quarter of its height above it.
    let above = -(height / 4.0).round().max(2.0);
    #[allow(clippy::cast_possible_truncation)] // A row is a few dozen pixels tall.
    let above = above as i64;
    journey
        .driver
        .action_chain()
        .move_to_element_center(&source)
        .click_and_hold()
        .move_by_offset(0, 8)
        .move_to_element_with_offset(&target, 0, above)
        .move_by_offset(0, 1)
        .release()
        .perform()
        .await?;
    Ok(())
}

/// The column `sections` describe, by reference.
fn order(sections: &[(&String, &[&String])]) -> Column {
    sections
        .iter()
        .map(|(id, rows)| {
            (
                (*id).clone(),
                rows.iter().map(|row| (*row).clone()).collect(),
            )
        })
        .collect()
}

/// Workspaces `alpha` and `beta`, then threads `a1` and `a2` in alpha and `b1` in beta, over the protocol; returns
/// `[alpha, beta, a1, a2, b1]`.
async fn seed(journey: &Journey) -> anyhow::Result<[String; 5]> {
    let mut connection = journey.connect().await?;
    let mut ids = Vec::new();
    for name in ["alpha", "beta"] {
        let workspace = connection
            .call::<WorkspacesCreate>(CreateWorkspaceParams {
                name: Some(name.to_owned()),
                link: None,
            })
            .await?;
        ids.push(workspace.id);
    }
    for (title, workspace) in [("a1", 0), ("a2", 0), ("b1", 1)] {
        let thread = connection
            .call::<ThreadsCreate>(CreateThreadParams {
                title: Some(title.to_owned()),
                workspace: ThreadWorkspace::Id(ids[workspace].clone()),
                model: None,
                thinking_level: None,
                approval_mode: None,
                prompt: None,
                images: Vec::new(),
            })
            .await?;
        ids.push(thread.id);
    }
    connection.close().await?;
    ids.try_into()
        .map_err(|ids| anyhow::anyhow!("expected 5 ids, got {ids:?}"))
}

// spec: E2E-60, UI-620, UI-621, UI-622, UI-624, UI-625, DMN-280, DMN-284
#[tokio::test(flavor = "multi_thread")]
async fn threads_and_workspaces_are_dragged_into_an_order_that_survives_a_reload()
-> anyhow::Result<()> {
    let journey = Journey::start_fake(FakeLauncher::new()).await?;
    journey.connected().await?;
    let [alpha, beta, a1, a2, b1] = seed(&journey).await?;
    until_column(
        &journey,
        "the new workspaces and threads, last",
        &order(&[(&alpha, &[&a1, &a2]), (&beta, &[&b1])]),
    )
    .await?;

    // A thread within its workspace, then one onto another workspace (UI-621).
    drag(&journey, &row(&a2), &row(&a1)).await?;
    until_column(
        &journey,
        "a2 dragged before a1",
        &order(&[(&alpha, &[&a2, &a1]), (&beta, &[&b1])]),
    )
    .await?;
    drag(&journey, &row(&a1), &row(&b1)).await?;
    until_column(
        &journey,
        "a1 dragged into beta",
        &order(&[(&alpha, &[&a2]), (&beta, &[&a1, &b1])]),
    )
    .await?;
    // The user moved it: not news (DMN-284), so it is not unread.
    assert_eq!(journey.count(&unread(&a1)).await?, 0, "a1 is not unread");

    // A section by its name (UI-622).
    drag(&journey, &section_name(&beta), &section_name(&alpha)).await?;
    until_column(
        &journey,
        "beta dragged above alpha",
        &order(&[(&beta, &[&a1, &b1]), (&alpha, &[&a2])]),
    )
    .await?;

    // The keys: Alt-ArrowDown on the focused row, announced (UI-624, UI-625).
    journey
        .eval(
            "document.querySelector(arguments[0]).focus();",
            vec![json!(row(&a1))],
        )
        .await?;
    journey
        .driver
        .action_chain()
        .key_down(Key::Alt)
        .send_keys(Key::Down)
        .key_up(Key::Alt)
        .perform()
        .await?;
    until_column(
        &journey,
        "a1 moved down by Alt-ArrowDown",
        &order(&[(&beta, &[&b1, &a1]), (&alpha, &[&a2])]),
    )
    .await?;
    journey
        .until("the move's announcement", async |page| {
            let said = page.text("[data-testid=live-region]").await?;
            Ok((said == "Moved a1 to position 2 of 2 in beta").then_some(()))
        })
        .await?;
    let focused: Value = journey
        .eval(
            "return document.activeElement?.dataset.threadId ?? null;",
            Vec::new(),
        )
        .await?;
    assert_eq!(focused, json!(a1), "the focus stays on the moved row");
    assert_eq!(journey.count(&unread(&a1)).await?, 0, "a1 is not unread");
    journey.screenshot("reorder").await?;

    // The order is the daemon's: a reload shows it again.
    journey.driver.refresh().await?;
    journey.connected().await?;
    until_column(
        &journey,
        "the order after a reload",
        &order(&[(&beta, &[&b1, &a1]), (&alpha, &[&a2])]),
    )
    .await?;
    journey.screenshot("reorder-reloaded").await?;

    journey.finish("reorder").await
}
