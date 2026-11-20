//! Cards in the chat (#229, E2E-77): a scripted agent draws a widget in the card of its own `omfp_card` call and waits
//! for the user. The thread awaits the user while the card waits; the widget's `submit` and the card's Cancel answer
//! the calls, and the agent reads the answers. Once the turn ends the cards collapse; opened again after a reload,
//! the widget mounts from the call's arguments and draws its finished state. Takes `omfp-card-waiting.png`. Run with
//! `just test-e2e`.

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
