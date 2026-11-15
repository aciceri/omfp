//! The composer on a phone (E2E-37, UI-421..UI-424) in headless Chromium emulating a 360 × 800 touch phone through
//! Chrome's debugging protocol (`Emulation.*`: a mobile viewport, touch, so `(pointer: coarse)` matches). The on-screen
//! keyboard is a viewport 480 px tall: with `interactive-widget=resizes-content` (UI-424) Chrome shrinks the layout
//! viewport by the keyboard's height, as here. Screenshots land in the run's screenshot directory as
//! `omfp-phone-composer-<state>.png`; nothing compares pixels.

use e2e::Journey;
use fake_llm::{Reply, Step};
use serde_json::{Value, json};
use thirtyfour::prelude::*;

/// The phone's width, and its height with the keyboard closed and open.
const WIDTH: u32 = 360;
const HEIGHT: u32 = 800;
const KEYBOARD_OPEN: u32 = 480;

/// Emulates a touch phone `height` px tall.
async fn phone(journey: &Journey, height: u32) -> anyhow::Result<()> {
    let cdp = journey.driver.cdp();
    cdp.send_raw(
        "Emulation.setDeviceMetricsOverride",
        json!({ "width": WIDTH, "height": height, "deviceScaleFactor": 2, "mobile": true }),
    )
    .await?;
    cdp.send_raw(
        "Emulation.setTouchEmulationEnabled",
        json!({ "enabled": true, "maxTouchPoints": 5 }),
    )
    .await?;
    Ok(())
}

/// The heights of the transcript and of the composer area, and the distinct tops of the composer bar's visible
/// buttons (one row: one top), in px.
async fn measure(journey: &Journey) -> anyhow::Result<(f64, f64, Vec<f64>)> {
    let value: Value = journey
        .eval(
            "const height = (css) => document.querySelector(css).getBoundingClientRect().height;
             const tops = [...document.querySelectorAll('[data-testid=composer] .omfp-compose__bar button')]
               .map((b) => b.getBoundingClientRect())
               .filter((r) => r.width > 0)
               .map((r) => Math.round(r.top));
             return [height('[data-testid=transcript]'), height('.omfp-composer-area'), [...new Set(tops)]];",
            Vec::new(),
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

// spec: E2E-37, UI-421, UI-422, UI-423, UI-424, UI-693
#[tokio::test(flavor = "multi_thread")]
async fn the_composer_fits_a_phone_with_its_keyboard_open() -> anyhow::Result<()> {
    let journey = Journey::start(vec![Step::reply(Reply::text("Held reply.")).gated()]).await?;
    journey.connected().await?;
    let thread = journey.create_thread("phone composer journey").await?;
    journey.send("start").await?;
    journey.fake.wait_for_gate(0).await;
    journey.status(&thread, "running").await?;
    journey.busy().await?;

    let viewport = journey
        .attr("meta[name=viewport]", "content")
        .await?
        .unwrap_or_default();
    anyhow::ensure!(
        viewport.contains("interactive-widget=resizes-content"),
        "viewport meta: {viewport}"
    );

    phone(&journey, HEIGHT).await?;
    journey
        .element("[data-testid=composer][data-touch=true]")
        .await?;
    let input = "[data-testid=composer-input]";
    let attributes = [
        "autocorrect",
        "autocapitalize",
        "spellcheck",
        "enterkeyhint",
    ];
    let mut values = Vec::new();
    for name in attributes {
        values.push(journey.attr(input, name).await?.unwrap_or_default());
    }
    assert_eq!(values, ["on", "sentences", "true", "enter"]);
    anyhow::ensure!(
        journey
            .count("[data-testid=composer] .omfp-kbd, [data-testid=composer] .omfp-compose__hint")
            .await?
            == 0,
        "key hints on a touch screen"
    );
    let (_, _, tops) = measure(&journey).await?;
    anyhow::ensure!(
        tops.len() == 1,
        "the composer's buttons on rows at {tops:?}"
    );
    journey.fits("[data-testid=app]").await?;
    journey.screenshot("phone-composer-busy").await?;

    // The keyboard opens: the composer's controls stay one row, its activity keeps a 44 px touch target, and the
    // transcript keeps most of what is left.
    phone(&journey, KEYBOARD_OPEN).await?;
    let field = journey.element(input).await?;
    field.send_keys("first").await?;
    field.send_keys(Key::Enter).await?;
    field.send_keys("second").await?;
    let draft = journey
        .eval(
            "return document.querySelector(arguments[0]).value;",
            vec![json!(input)],
        )
        .await?;
    assert_eq!(
        draft.as_str(),
        Some("first\nsecond"),
        "Enter made no new line"
    );
    let (transcript, composer, tops) = measure(&journey).await?;
    eprintln!("keyboard open: transcript {transcript} px, composer {composer} px");
    anyhow::ensure!(
        tops.len() == 1,
        "the composer's buttons on rows at {tops:?}"
    );
    anyhow::ensure!(
        composer <= 190.0,
        "the composer with activity is {composer} px tall"
    );
    anyhow::ensure!(
        transcript >= 150.0,
        "the transcript is {transcript} px tall"
    );
    journey.fits("[data-testid=app]").await?;
    journey.screenshot("phone-composer-keyboard").await?;

    journey.fake.release_gate(0);
    journey.status(&thread, "idle").await?;
    journey.finish("E2E-37").await
}
