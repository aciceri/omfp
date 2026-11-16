//! Screenshots of the main screens for a visual check (06-testing.md, L5): a busy thread with a Markdown reply, a tool
//! card and a queued follow-up in the light and dark schemes and in a custom matugen theme, the `:` command line, the
//! left column's "+ workspace" field in path mode completing a folder, the Workspace widget (Link / Clone switch) with
//! two linked repositories and its path field completing a folder, a pending approval and an `ask` question. The
//! screenshots land in the run's screenshot directory; nothing compares pixels. Run with `just test-e2e`.

use std::path::Path;

use e2e::Journey;
use fake_llm::{Reply, Step, ToolCall};
use omfp_protocol::Theme;
use omfp_protocol::method::ThemeSet;
use omfp_testkit::TempRepo;
use serde_json::json;
use thirtyfour::prelude::*;

const MARKDOWN: &str = "# Release plan

Here is the **plan**, starting with `cargo xtask spec-trace`:

- build the web bundle
- run the journeys with `just test-e2e`
  - nested: look at every screenshot

## Code

```rust
fn main() {
    println!(\"hello from the release plan\");
}
```

| Step | Status | Owner |
|------|--------|-------|
| build | done | ci |
| test | running | `nextest` |
";

/// A Material 3 scheme as matugen exports it (the baseline purple), with the `success`/`warning` custom colors of
/// `contrib/matugen/config.toml`: `(role, light, dark)`.
const PALETTE: &[(&str, &str, &str)] = &[
    ("primary", "#6750a4", "#d0bcff"),
    ("on_primary", "#ffffff", "#381e72"),
    ("primary_container", "#eaddff", "#4f378b"),
    ("on_primary_container", "#21005d", "#eaddff"),
    ("secondary", "#625b71", "#ccc2dc"),
    ("on_secondary", "#ffffff", "#332d41"),
    ("secondary_container", "#e8def8", "#4a4458"),
    ("on_secondary_container", "#1d192b", "#e8def8"),
    ("tertiary", "#7d5260", "#efb8c8"),
    ("on_tertiary", "#ffffff", "#492532"),
    ("tertiary_container", "#ffd8e4", "#633b48"),
    ("on_tertiary_container", "#31111d", "#ffd8e4"),
    ("error", "#b3261e", "#f2b8b5"),
    ("on_error", "#ffffff", "#601410"),
    ("error_container", "#f9dedc", "#8c1d18"),
    ("on_error_container", "#410e0b", "#f9dedc"),
    ("surface", "#fef7ff", "#141218"),
    ("on_surface", "#1d1b20", "#e6e0e9"),
    ("surface_variant", "#e7e0ec", "#49454f"),
    ("on_surface_variant", "#49454f", "#cac4d0"),
    ("surface_container_lowest", "#ffffff", "#0f0d13"),
    ("surface_container_low", "#f7f2fa", "#1d1b20"),
    ("surface_container", "#f3edf7", "#211f26"),
    ("surface_container_high", "#ece6f0", "#2b2930"),
    ("surface_container_highest", "#e6e0e9", "#36343b"),
    ("surface_dim", "#ded8e1", "#141218"),
    ("surface_bright", "#fef7ff", "#3b383e"),
    ("inverse_surface", "#322f35", "#e6e0e9"),
    ("inverse_on_surface", "#f5eff7", "#322f35"),
    ("inverse_primary", "#d0bcff", "#6750a4"),
    ("outline", "#79747e", "#938f99"),
    ("outline_variant", "#cac4d0", "#49454f"),
    ("shadow", "#000000", "#000000"),
    ("scrim", "#000000", "#000000"),
    ("success", "#386a20", "#9cd67d"),
    ("on_success", "#ffffff", "#0c3900"),
    ("warning", "#7c5800", "#f9bd22"),
    ("on_warning", "#ffffff", "#422c00"),
];

/// `contrib/matugen/omfp.css` rendered with [`PALETTE`], as `matugen` would write it.
fn matugen_theme() -> anyhow::Result<String> {
    let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("../contrib/matugen/omfp.css");
    let mut css = std::fs::read_to_string(template)?;
    for (role, light, dark) in PALETTE {
        css = css
            .replace(&format!("{{{{colors.{role}.light.hex}}}}"), light)
            .replace(&format!("{{{{colors.{role}.dark.hex}}}}"), dark);
    }
    anyhow::ensure!(!css.contains("{{"), "a matugen placeholder is left: {css}");
    Ok(css)
}

/// Sets the daemon's theme (UI-091) and waits until the page's `--omfp-primary` is `primary`.
async fn push_theme(journey: &Journey, css: Option<String>, primary: &str) -> anyhow::Result<()> {
    let mut connection = journey.connect().await?;
    connection.call::<ThemeSet>(Theme { css }).await?;
    connection.close().await?;
    until_primary(journey, primary).await
}

async fn until_primary(journey: &Journey, primary: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("--omfp-primary {primary}"), async |page| {
            Ok((page.token("--omfp-primary").await? == primary).then_some(()))
        })
        .await
}

/// A thread with a tool card and a Markdown reply, busy on a held turn with a follow-up queued.
async fn busy_thread(journey: &Journey) -> anyhow::Result<String> {
    let thread = journey.create_thread("screenshot journey").await?;
    journey.send("show me the plan").await?;
    journey.reply("hello from the release plan").await?;
    journey.status(&thread, "idle").await?;

    journey.send("next step").await?;
    journey.fake.wait_for_gate(2).await;
    journey.status(&thread, "running").await?;
    journey.busy().await?;
    journey.queue("and a follow-up after that").await?;
    journey
        .until("the queued follow-up", async |page| {
            let chips = page
                .count("[data-testid=queue-chip]:not([aria-busy=true])")
                .await?;
            Ok((chips == 1).then_some(()))
        })
        .await?;
    Ok(thread)
}

/// The `:` line open on a partial command, then closed again.
async fn command_line(journey: &Journey) -> anyhow::Result<()> {
    journey
        .eval(
            "if (document.activeElement) { document.activeElement.blur(); }",
            Vec::new(),
        )
        .await?;
    journey.keys(":").await?;
    journey.until_focused("command-line-input").await?;
    journey.keys("th").await?;
    journey.element("[data-testid=command-option]").await?;
    journey.screenshot("e-command-line").await?;
    journey.keys(Key::Escape).await?;
    journey.gone("[data-testid=command-line]").await
}

/// A folder with a few directories to complete in, `apps` a git repository: `(guard, typed partial path)`.
fn completion_folder() -> anyhow::Result<(tempfile::TempDir, String)> {
    let dir = tempfile::Builder::new().prefix("omfp-shot-").tempdir()?;
    for name in ["api-gateway", "apps/.git", "apps-legacy", "archive"] {
        std::fs::create_dir_all(dir.path().join(name))?;
    }
    let typed = format!("{}/ap", dir.path().display());
    Ok((dir, typed))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_thread_in_every_theme_the_command_line_and_the_new_workspace_path_completion()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "echo screenshot-run && ls -la" }),
        )])),
        Step::reply(Reply::text(MARKDOWN)),
        Step::reply(Reply::text("Working on the next step.")).gated(),
        Step::reply(Reply::text("The follow-up is done.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = busy_thread(&journey).await?;

    journey.scheme("light").await?;
    journey.screenshot("a-thread-light").await?;
    journey.scheme("dark").await?;
    journey.screenshot("b-thread-dark").await?;

    // A custom theme from the matugen template, in both schemes.
    let built_in = journey.token("--omfp-primary").await?;
    push_theme(&journey, Some(matugen_theme()?), "#d0bcff").await?;
    journey.screenshot("f-matugen-dark").await?;
    journey.scheme("light").await?;
    until_primary(&journey, "#6750a4").await?;
    journey.screenshot("f-matugen-light").await?;
    journey.scheme("dark").await?;
    push_theme(&journey, None, &built_in).await?;
    journey.scheme("light").await?;

    command_line(&journey).await?;

    journey.fake.release_gate(2);
    journey.reply("The follow-up is done.").await?;
    journey.status(&thread, "idle").await?;

    // "+ workspace" with a path: the field switches to path mode and completes on the daemon host (UI-023, DMN-021).
    let (_folder, typed) = completion_folder()?;
    journey
        .type_into("[data-testid=new-workspace-input]", &typed)
        .await?;
    journey
        .element("[data-testid=new-workspace][data-mode=path]")
        .await?;
    journey
        .until("three completions", async |page| {
            Ok((page
                .count("[data-testid=path-option][data-kind=directory]")
                .await?
                == 3)
                .then_some(()))
        })
        .await?;
    journey.screenshot("paths-new-workspace").await?;
    journey.screenshot("fix1-left-column").await?;
    journey
        .finish("screenshots: themes, command line and path completion")
        .await
}

/// The heading of the pending (unanswered) request, once one shows.
async fn pending(journey: &Journey) -> anyhow::Result<Option<String>> {
    journey
        .attr(
            "[data-testid=interaction]:not([aria-busy=true])",
            "aria-label",
        )
        .await
}

/// Clicks the option containing `needle` (omp's label) of the pending request, then waits until it closes.
async fn answer(journey: &Journey, needle: &str) -> anyhow::Result<()> {
    let clicked = journey
        .eval(
            "const option = [...document.querySelectorAll(
                 '[data-testid=interaction]:not([aria-busy=true]) [data-testid=interaction-option]')]
               .find(e => e.innerText.includes(arguments[0]));
             if (option) { option.click(); }
             return !!option;",
            vec![json!(needle)],
        )
        .await?;
    anyhow::ensure!(clicked == json!(true), "no option containing {needle:?}");
    journey
        .gone("[data-testid=interaction][aria-busy=true]")
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_panel_path_completion_a_pending_approval_and_an_ask_question()
-> anyhow::Result<()> {
    let form = r#"<form>
  <fieldset><legend>Which database should the service use?</legend>
    <label><input type="radio" name="db" value="postgres"> Postgres</label>
    <label><input type="radio" name="db" value="sqlite" checked> SQLite</label>
  </fieldset>
</form>"#;
    let linked = TempRepo::new()?;
    linked.commit_file("README.md", "Linked for the screenshot.\n", "readme")?;
    let other = TempRepo::new()?;
    other.commit_file("README.md", "Linked too.\n", "readme")?;
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "rm -rf build && cargo build --release" }),
        )])),
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_ask_1",
            "ask",
            json!({ "html": form }),
        )])),
        Step::reply(Reply::text("Using SQLite.")),
    ])
    .await?;
    journey.connected().await?;
    journey.scheme("light").await?;
    let thread = journey.create_asking_thread("approval screenshot").await?;

    for repo in [&linked, &other] {
        let before = journey
            .count("[data-testid=workspace-repo][data-kind=link]")
            .await?;
        journey
            .type_into(
                "[data-testid=add-repo-path]",
                &repo.path().to_string_lossy(),
            )
            .await?;
        journey.click("[data-testid=add-repo-submit]").await?;
        journey
            .until("the new link", async |page| {
                Ok((page
                    .count("[data-testid=workspace-repo][data-kind=link]")
                    .await?
                    > before)
                    .then_some(()))
            })
            .await?;
        journey.add_repo_answered().await?;
    }
    journey.fits("[data-widget=\"omfp:workspace\"]").await?;
    journey.screenshot("g-workspace-linked").await?;
    journey.screenshot("fix1-workspace-widget").await?;
    journey.screenshot("paths-workspace").await?;
    // The add control's path field, completing a folder to link.
    let (_folder, typed) = completion_folder()?;
    journey
        .type_into("[data-testid=add-repo-path]", &typed)
        .await?;
    journey
        .until("three completions", async |page| {
            Ok((page
                .count("[data-testid=path-option][data-kind=directory]")
                .await?
                == 3)
                .then_some(()))
        })
        .await?;
    journey.screenshot("paths-workspace-completion").await?;
    journey.click("[data-testid=composer-input]").await?;
    journey.gone("[data-testid=path-option]").await?;

    journey.send("build it").await?;
    // Approvals (bash, and the ask tool if it needs one) until the question shows; the first one is captured.
    let mut approvals = 0;
    loop {
        let heading = journey
            .until("a pending request", async |page| pending(page).await)
            .await?;
        if heading.contains("database") {
            break;
        }
        if approvals == 0 {
            journey.screenshot("c-approval").await?;
        }
        approvals += 1;
        answer(&journey, "Approve").await?;
    }
    anyhow::ensure!(approvals > 0, "no approval was asked");
    journey.screenshot("c-ask").await?;
    answer(&journey, "SQLite").await?;
    journey.reply("Using SQLite.").await?;
    journey.status(&thread, "idle").await?;
    journey
        .finish("screenshots: workspace, approval and ask")
        .await
}
