//! The `/kit` gallery (UI-173, UI-174, UI-514, UI-797) at motion `none`, 17 px, compact, in light and dark for every
//! built-in theme (UI-510: Synth, the six palettes, and `accent` from macOS's blue, yellow and graphite accents,
//! UI-512), `matugen` with the matugen fixture theme as the user theme, and the Dux Solarized preset (whose tertiary and
//! secondary sit on the surface at 1.14:1 until the contrast guard, UI-196, moves them), in both skins (UI-794: Crush
//! and boring): every text role, status color and gradient stop the page measures reaches WCAG AA, the guard
//! overrides nothing but in the Dux preset, and elsewhere the item focus (UI-132) stays visible: its bar at 3:1, its
//! tint apart from the other states. Full-page shots land in the run's screenshot directory (`e2e::screenshot_dir`)
//! as `omfp-kit-<theme>-<scheme>.png` (Crush) and `omfp-kit-boring-<theme>-<scheme>.png` for a look by eye; nothing
//! compares them.

use anyhow::{Context, bail};
use e2e::Journey;
use serde_json::json;

/// The page width of every shot (CSS px, device scale 1).
const WIDTH: u32 = 1400;

/// The gallery's themes: the shot's name and the query that picks it (`accent` from four of macOS's accents).
const THEMES: [(&str, &str); 13] = [
    ("synth", "theme=synth"),
    ("catppuccin", "theme=catppuccin"),
    ("gruvbox", "theme=gruvbox"),
    ("tokyo-night", "theme=tokyo-night"),
    ("nord", "theme=nord"),
    ("solarized", "theme=solarized"),
    ("rose-pine", "theme=rose-pine"),
    ("accent", "theme=accent"),
    ("accent-yellow", "theme=accent&accent=%23ffc600"),
    ("accent-graphite", "theme=accent&accent=%238c8c8c"),
    ("accent-green", "theme=accent&accent=%2362ba46"),
    ("matugen", "theme=matugen"),
    ("dux-solarized", "theme=dux-solarized"),
];
/// The one theme the contrast guard must correct; it leaves every other as it is.
const GUARDED_THEME: &str = "dux-solarized";
const SCHEMES: [&str; 2] = ["light", "dark"];
/// The skins (UI-794): each one's shot prefix after `omfp-kit-`.
const SKINS: [(&str, &str); 2] = [("crush", ""), ("boring", "boring-")];

/// Opens the gallery with `theme` (its query), `scheme` and `skin` and waits until it says it is ready (fonts loaded,
/// contrast measured).
async fn open(journey: &Journey, theme: &str, scheme: &str, skin: &str) -> anyhow::Result<()> {
    let base = format!("http://{}", journey.daemon().addr());
    journey
        .driver
        .goto(format!(
            "{base}/kit?scheme={scheme}&{theme}&skin={skin}&motion=none&density=compact&fs=17"
        ))
        .await?;
    journey
        .until("the gallery to be ready", async |page| {
            Ok((page
                .count("[data-testid=kit-gallery][data-ready=true]")
                .await?
                == 1)
                .then_some(()))
        })
        .await
}

/// A screenshot of the whole page: the viewport is made as tall as the page first.
async fn full_page(journey: &Journey) -> anyhow::Result<Vec<u8>> {
    let cdp = journey.driver.cdp();
    cdp.send_raw(
        "Emulation.setDeviceMetricsOverride",
        json!({ "width": WIDTH, "height": 1000, "deviceScaleFactor": 1, "mobile": false }),
    )
    .await?;
    let height = journey
        .eval("return document.documentElement.scrollHeight;", Vec::new())
        .await?
        .as_u64()
        .context("the page height")?;
    cdp.send_raw(
        "Emulation.setDeviceMetricsOverride",
        json!({ "width": WIDTH, "height": height, "deviceScaleFactor": 1, "mobile": false }),
    )
    .await?;
    let png = journey.driver.screenshot_as_png().await?;
    cdp.send_raw("Emulation.clearDeviceMetricsOverride", json!({}))
        .await?;
    Ok(png)
}

/// The contrast rows (text roles, status colors, gradient stops) under AA: `text on background ratio`.
async fn contrast_failures(journey: &Journey) -> anyhow::Result<Vec<String>> {
    let measured = journey.count("[data-testid=contrast]").await?;
    anyhow::ensure!(measured > 0, "the gallery measured no contrast");
    let value = journey
        .eval(
            "return [...document.querySelectorAll('[data-testid=contrast][data-pass=false]')]
               .map((row) => `${row.dataset.text} on ${row.dataset.background} ${row.dataset.ratio}`);",
            Vec::new(),
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

/// The focus measures (UI-132) as `(passes, what on surface: value / threshold)`, printed for the record.
async fn focus_measures(journey: &Journey) -> anyhow::Result<Vec<(bool, String)>> {
    let measured = journey.count("[data-testid=focus-contrast]").await?;
    anyhow::ensure!(measured > 0, "the gallery measured no focus");
    let value = journey
        .eval(
            "return [...document.querySelectorAll('[data-testid=focus-contrast]')].map(({ dataset: row }) =>
               [row.pass === 'true', `${row.what} on ${row.surface}: ${row.value} / ${row.threshold}`]);",
            Vec::new(),
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

// spec: UI-173, UI-174, UI-132, UI-135, UI-140, UI-196, UI-337, UI-338, UI-510, UI-512, UI-514, UI-736, UI-794, UI-797
#[tokio::test(flavor = "multi_thread")]
async fn the_kit_gallery_text_reaches_aa_in_every_theme_and_skin() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    let mut problems = Vec::new();
    for (skin, prefix) in SKINS {
        for (theme, query) in THEMES {
            for scheme in SCHEMES {
                open(&journey, query, scheme, skin).await?;
                let shown = journey.attr("html", "data-skin").await?;
                if shown.as_deref() != Some(skin) {
                    problems.push(format!("{skin} {theme} {scheme}: data-skin {shown:?}"));
                }
                let shown = journey.attr("html", "data-theme").await?;
                let expected = if theme.starts_with("accent") {
                    "accent"
                } else if theme == GUARDED_THEME || theme == "matugen" {
                    "synth"
                } else {
                    theme
                };
                if shown.as_deref() != Some(expected) {
                    problems.push(format!("{skin} {theme} {scheme}: data-theme {shown:?}"));
                }
                assert_eq!(
                    journey.attr("html", "data-motion").await?.as_deref(),
                    Some("none"),
                    "the gallery runs at motion none"
                );
                for failure in contrast_failures(&journey).await? {
                    problems.push(format!("{skin} {theme} {scheme}: under AA: {failure}"));
                }
                let guarded = journey.attr("html", "data-contrast-guard").await?;
                if (theme == GUARDED_THEME) != guarded.is_some() {
                    problems.push(format!(
                        "{skin} {theme} {scheme}: contrast guard overrides {guarded:?}"
                    ));
                }
                for (passes, measure) in focus_measures(&journey).await? {
                    eprintln!("focus {skin} {theme} {scheme}: {measure}");
                    if !passes && theme != GUARDED_THEME {
                        problems.push(format!(
                            "{skin} {theme} {scheme}: focus not visible: {measure}"
                        ));
                    }
                }
                // The gallery's four composers start their text where their bars' chips start.
                anyhow::ensure!(
                    journey
                        .composer_aligned("[data-testid=kit-gallery]")
                        .await?
                        == 4
                );
                if let Some(dir) = e2e::screenshot_dir() {
                    let path = dir.join(format!("omfp-kit-{prefix}{theme}-{scheme}.png"));
                    std::fs::write(&path, full_page(&journey).await?)?;
                    eprintln!("screenshot: {}", path.display());
                }
            }
        }
    }
    journey.finish("kit gallery").await?;
    if !problems.is_empty() {
        bail!("the gallery's contrast:\n{}", problems.join("\n"));
    }
    Ok(())
}

// spec: UI-166
#[tokio::test(flavor = "multi_thread")]
async fn code_scrolls_without_moving_its_label_or_covering_its_last_line() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    open(&journey, "theme=synth", "dark", "crush").await?;
    let failures = journey
        .eval(
            r"
            const blocks = [...document.querySelectorAll('.omfp-code--block[data-lang]')];
            const failures = [];
            for (const block of blocks) {
                // Constrain the real Code and Markdown specimens, including those inside tool cards.
                block.style.width = '120px';
                block.style.maxHeight = '72px';
                const code = block.querySelector('code');
                const before = code.getBoundingClientRect();
                const range = document.createRange();
                range.selectNodeContents(code);
                const textBefore = range.getBoundingClientRect();
                code.scrollLeft = code.scrollWidth;
                code.scrollTop = code.scrollHeight;
                const textAfter = range.getBoundingClientRect();
                if (code.scrollLeft <= 0 || textAfter.left >= textBefore.left) {
                    failures.push('Long code did not scroll horizontally: ' + block.dataset.lang);
                }
                if (block.scrollLeft !== 0 || block.scrollTop !== 0 ||
                    code.getBoundingClientRect().top !== before.top) {
                    failures.push('The code frame and language label moved with the text');
                }
                const bottom = code.getBoundingClientRect().top + code.clientTop + code.clientHeight;
                if (bottom - textAfter.bottom < 19) {
                    failures.push('The last line has no clearance for an expanded overlay scrollbar');
                }
                if (block.getBoundingClientRect().height > 73) {
                    failures.push('The code block no longer respects its height limit');
                }
            }
            if (!blocks.some(block => block.closest('.omfp-markdown'))) {
                failures.push('No Markdown fence exercised');
            }
            if (!blocks.some(block => block.closest('.omfp-tool'))) {
                failures.push('No tool code exercised');
            }
            return failures;
            ",
            Vec::new(),
        )
        .await?;
    assert_eq!(failures, json!([]));
    journey.finish("code scrolling").await
}
