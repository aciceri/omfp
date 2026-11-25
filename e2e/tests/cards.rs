//! Cards in the chat (#229, E2E-77): a scripted agent draws a widget in the card of its own `omfp_card` call and waits
//! for the user. The thread awaits the user while the card waits; the widget's `submit` and the card's Cancel answer
//! the calls, and the agent reads the answers. Once the turn ends the cards collapse; opened again after a reload,
//! the widget mounts from the call's arguments and draws its finished state. Takes `omfp-card-waiting.png`. A card of
//! `omfp:file` and a Markdown reply show an image of the workspace (#294, #295, E2E-81); a card of `omfp:terminal`
//! keeps one height (E2E-82). Run with `just test-e2e`.

use e2e::Journey;
use fake_llm::{Reply, Step, ToolCall};
use serde_json::{Value, json};

/// A choice among `options`: a button each while the card waits, then what was chosen.
const PICK: &str = r#"// A choice among options; props: options.
import { useAnswer } from "omfp/sdk";

export default function Pick({ options }: { options: string[] }) {
  const answer = useAnswer();
  if (answer.state !== "waiting") {
    return <p data-testid="pick-done">{answer.state === "submitted" ? `Picked ${String(answer.value)}` : answer.state}</p>;
  }
  return (
    <div data-testid="pick">
      {options.map((option) => (
        <button key={option} type="button" data-testid={`pick-${option}`} onClick={() => void answer.submit(option)}>
          {option}
        </button>
      ))}
    </div>
  );
}
"#;

fn card(call: &str) -> String {
    format!("[data-testid=tool-card][data-call={call}]")
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

// spec: E2E-77, AGT-096, AGT-097, DMN-911, UI-920, UI-921, UI-922
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_waits_for_the_answer_of_a_widget_card_in_the_chat() -> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_pick",
            "omfp_card",
            json!({ "code": PICK, "title": "Pick a colour", "props": { "options": ["red", "blue"] }, "wait": true }),
        )])),
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_again",
            "omfp_card",
            json!({ "code": PICK, "title": "Pick again", "props": { "options": ["green"] }, "wait": true }),
        )])),
        Step::reply(Reply::text("Picked blue, then you declined.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("cards journey").await?;
    journey.send("pick a colour").await?;

    // The card waits: its widget shows in the open card and the thread awaits the user.
    let pick = card("call_pick");
    journey
        .element(&format!(
            "{pick} [data-testid=card-widget][data-state=waiting] [data-testid=pick-blue]"
        ))
        .await?;
    journey.status(&thread, "awaiting").await?;
    journey.screenshot("card-waiting").await?;
    journey
        .click(&format!("{pick} [data-testid=pick-blue]"))
        .await?;

    // The second card is cancelled from the card itself, outside the widget.
    let again = card("call_again");
    journey
        .element(&format!(
            "{again} [data-testid=card-widget][data-state=waiting]"
        ))
        .await?;
    journey
        .click(&format!("{again} [data-testid=card-cancel]"))
        .await?;
    journey.reply("Picked blue, then you declined.").await?;

    let requests = journey.fake.requests();
    let picked: Value = serde_json::from_str(&tool_result(&requests, "call_pick"))?;
    assert_eq!(picked, json!({ "status": "submitted", "value": "blue" }));
    let declined: Value = serde_json::from_str(&tool_result(&requests, "call_again"))?;
    assert_eq!(declined, json!({ "status": "cancelled" }));

    // The turn ended: the cards make way.
    journey.status(&thread, "idle").await?;
    journey
        .gone(&format!("{pick} [data-testid=card-widget]"))
        .await?;
    journey
        .gone(&format!("{again} [data-testid=card-widget]"))
        .await?;

    // After a reload, opening the card mounts the widget again, which draws its finished state.
    journey.driver.refresh().await?;
    journey.connected().await?;
    journey.click(&e2e::row(&thread)).await?;
    journey
        .click(&format!("{pick} [data-testid=toggle]"))
        .await?;
    let done =
        format!("{pick} [data-testid=card-widget][data-state=submitted] [data-testid=pick-done]");
    journey.element(&done).await?;
    journey.screenshot("card-answered").await?;
    assert_eq!(journey.text(&done).await?, "Picked blue");
    journey.finish("cards").await
}

/// How tall the shown part of the image in `css` is, and its natural width (`0` until it loaded).
const IMAGE_SIZE: &str = "const image = document.querySelector(arguments[0]); \
    if (image === null) return null; \
    const viewport = image.closest('.omfp-image__viewport') ?? image; \
    return [Math.round(viewport.getBoundingClientRect().height), image.naturalWidth];";

// spec: E2E-81, UI-923, UI-572
#[tokio::test(flavor = "multi_thread")]
async fn a_card_and_a_reply_show_an_image_of_the_workspace() -> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_image",
            "omfp_card",
            json!({ "widget": "omfp:file", "title": "The icon", "props": { "path": "icon.png" } }),
        )])),
        Step::reply(Reply::text("Here it is again: ![the icon](icon.png)")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("image journey").await?;
    let folder = journey.workspace_folder(&thread).await?;
    std::fs::write(
        folder.join("icon.png"),
        include_bytes!("../../docs/brand/icon-32.png"),
    )?;
    journey.send("show me the icon").await?;

    // The card's image viewer has a height of its own: the card's body, which grows with its content, gives it none.
    let shown = format!("{} [data-testid=file-image] img", card("call_image"));
    let [height, width] = journey
        .until("the card's image", async |journey| {
            let size = journey.eval(IMAGE_SIZE, vec![json!(shown)]).await?;
            Ok(serde_json::from_value::<[u64; 2]>(size)
                .ok()
                .filter(|[_, width]| *width > 0))
        })
        .await?;
    assert!(height >= 200, "the card's image is {height} px tall");
    assert_eq!(width, 32);

    // The reply's Markdown image names the same file: it loads from the daemon's raw-file route.
    journey.reply("Here it is again").await?;
    let inline = "[data-testid=markdown] img[alt='the icon']";
    let [_, width] = journey
        .until("the reply's image", async |journey| {
            let size = journey.eval(IMAGE_SIZE, vec![json!(inline)]).await?;
            Ok(serde_json::from_value::<[u64; 2]>(size)
                .ok()
                .filter(|[_, width]| *width > 0))
        })
        .await?;
    assert_eq!(width, 32);
    journey.screenshot("card-image").await?;
    journey.finish("card-image").await
}

/// A card's terminal emulator after 30 more frames: `[height before, height after, min(70vh, 36rem)]` in px.
const TERMINAL_HEIGHTS: &str = "const host = document.querySelector(arguments[0]); \
    if (host === null) return null; \
    const before = host.getBoundingClientRect().height; \
    const rem = parseFloat(getComputedStyle(document.documentElement).fontSize); \
    const expected = Math.min(innerHeight * 0.7, 36 * rem); \
    return new Promise((resolve) => { \
      const step = (left) => left === 0 \
        ? resolve([before, host.getBoundingClientRect().height, expected].map(Math.round)) \
        : requestAnimationFrame(() => step(left - 1)); \
      step(30); \
    });";

// spec: E2E-82, UI-923
#[tokio::test(flavor = "multi_thread")]
async fn a_card_terminal_keeps_one_height() -> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_terminal",
            "omfp_card",
            json!({
                "widget": "omfp:terminal",
                "title": "A shell",
                "props": { "command": "printf 'ready\\r\\n'; exec cat" }
            }),
        )])),
        Step::reply(Reply::text("The terminal is open.")),
    ])
    .await?;
    journey.connected().await?;
    journey.create_thread("card terminal journey").await?;

    // The emulator fits its rows to its height: a height that came from those rows would grow with every fit once the
    // padding around them is taller than a row, as it is at a roomy density.
    journey
        .eval(
            "document.documentElement.style.setProperty('--omfp-density', '2');",
            Vec::new(),
        )
        .await?;
    journey.send("open a terminal").await?;
    journey.reply("The terminal is open.").await?;
    let emulator = format!(
        "{} [data-testid=card-widget] .omfp-terminal-emulator",
        card("call_terminal")
    );
    let xterm = format!("{emulator} .xterm");
    journey
        .until("the card's terminal output", async |page| {
            Ok(page.text(&xterm).await?.contains("ready").then_some(()))
        })
        .await?;
    let heights = journey
        .eval(TERMINAL_HEIGHTS, vec![json!(emulator)])
        .await?;
    let [before, after, expected] = serde_json::from_value::<[i64; 3]>(heights)?;
    assert_eq!(
        (before, after),
        (expected, expected),
        "the card's terminal height"
    );
    journey.screenshot("card-terminal").await?;
    journey.finish("card-terminal").await
}
