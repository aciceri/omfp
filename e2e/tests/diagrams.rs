//! Mermaid diagrams in the transcript (06-testing.md, E2E-76): Mermaid itself draws a reply's ```` ```mermaid ````
//! fence in headless Chromium, in the theme's colors, again when the scheme changes; the source is one toggle away and
//! a source Mermaid cannot draw stays code with Mermaid's reason. Shots `omfp-diagram-light.png` and
//! `omfp-diagram-dark.png` land in the run's screenshot directory.

use e2e::Journey;
use fake_llm::{Reply, Step};
use serde_json::json;

const REPLY: &str = "The release flow:

```mermaid
flowchart LR
  plan[Plan] --> build[Build]
  build --> ship{Ship?}
  ship -->|yes| done[Done]
  ship -->|no| plan
```

And a sketch that is not finished:

```mermaid
flowchart LR
  plan -->
```
";

const DRAWN: &str = "[data-testid=mermaid-diagram][data-state=drawn]";
const FAILED: &str = "[data-testid=mermaid-diagram][data-state=failed]";

/// The fill of the drawing's first node and the token it takes (`--omfp-primary-container`), both as `[r, g, b]`
/// drawn on a canvas, so the notations the browser serializes them in do not matter.
async fn node_fill(journey: &Journey) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let value = journey
        .eval(
            "const context = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
             const rgb = (color) => {
               context.clearRect(0, 0, 1, 1);
               context.fillStyle = color;
               context.fillRect(0, 0, 1, 1);
               return [...context.getImageData(0, 0, 1, 1).data.slice(0, 3)];
             };
             const node = document.querySelector(arguments[0] + ' .node :is(rect, polygon, path)');
             const probe = document.createElement('span');
             probe.style.color = 'var(--omfp-primary-container)';
             document.body.append(probe);
             const token = getComputedStyle(probe).color;
             probe.remove();
             return [rgb(getComputedStyle(node).fill), rgb(token)];",
            vec![json!(DRAWN)],
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

/// Waits until the drawing's nodes are filled with the theme's `--omfp-primary-container`.
async fn until_themed(journey: &Journey) -> anyhow::Result<()> {
    journey
        .until("the nodes in --omfp-primary-container", async |page| {
            let (fill, token) = node_fill(page).await?;
            Ok((fill == token).then_some(()))
        })
        .await
}

// spec: E2E-76, UI-981, UI-982, UI-983
#[tokio::test(flavor = "multi_thread")]
async fn a_reply_s_mermaid_diagram_is_drawn_in_the_theme_s_colors() -> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::text(REPLY)),
        Step::reply(Reply::text(REPLY)),
    ])
    .await?;
    journey.connected().await?;
    journey.create_thread("diagrams").await?;
    journey.scheme("light").await?;
    journey.send("draw the release flow").await?;
    journey.reply("The release flow").await?;

    journey.element(&format!("{DRAWN} svg")).await?;
    let labels: Vec<String> = serde_json::from_value(
        journey
            .eval(
                "return [...document.querySelectorAll(arguments[0] + ' .node')].map((node) => node.textContent.trim());",
                vec![json!(DRAWN)],
            )
            .await?,
    )?;
    assert_eq!(labels, ["Plan", "Build", "Ship?", "Done"]);
    until_themed(&journey).await?;
    let light = node_fill(&journey).await?.0;
    journey.screenshot("diagram-light").await?;

    // Another scheme: drawn again in its colors.
    journey.scheme("dark").await?;
    until_themed(&journey).await?;
    assert_ne!(node_fill(&journey).await?.0, light);
    journey.screenshot("diagram-dark").await?;

    // The source, and back.
    journey
        .click(&format!("{DRAWN} button[aria-pressed=false]"))
        .await?;
    journey
        .element(&format!("{DRAWN} pre[data-lang=mermaid]"))
        .await?;
    assert_eq!(journey.count(&format!("{DRAWN} svg")).await?, 0);
    assert!(
        journey
            .text(&format!("{DRAWN} pre"))
            .await?
            .starts_with("flowchart LR\n  plan[Plan] --> build[Build]")
    );
    journey
        .click(&format!("{DRAWN} button[aria-pressed=true]"))
        .await?;
    journey.element(&format!("{DRAWN} svg")).await?;

    // The unfinished sketch: its code and Mermaid's reason.
    journey
        .element(&format!("{FAILED} pre[data-lang=mermaid]"))
        .await?;
    let reason = journey
        .text(&format!("{FAILED} .omfp-diagram__error"))
        .await?;
    assert!(reason.starts_with("Diagram not drawn: "), "{reason}");
    // Mermaid draws in elements of its own under `<body>`: none is left behind, failed drawings included.
    assert_eq!(journey.count("body > [id^='domfp-mermaid-']").await?, 0);

    // Repeating the same diagram in another reply must not share SVG IDs (markers and filters use them).
    journey.send("repeat the flow").await?;
    journey
        .until("both diagram instances", async |page| {
            Ok((page.count(&format!("{DRAWN} svg")).await? == 2).then_some(()))
        })
        .await?;
    let duplicates: Vec<String> = serde_json::from_value(
        journey
            .eval(
                "const ids = [...document.querySelectorAll('[data-testid=mermaid-diagram] [id]')].map(e => e.id);
                 return ids.filter((id, index) => ids.indexOf(id) !== index);",
                Vec::new(),
            )
            .await?,
    )?;
    assert_eq!(duplicates, Vec::<String>::new());

    journey.finish("mermaid diagrams").await
}
