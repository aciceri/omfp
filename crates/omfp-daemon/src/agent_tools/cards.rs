//! `omfp_card { widget? | code?, props?, title?, wait?, timeoutSeconds? }` (AGT-096, AGT-097): a widget in the card
//! of the agent's own tool call in the chat, for one moment of the conversation, optionally waiting for the user's
//! answer. It writes no file and no view: the window draws the card from the call's arguments.

use std::time::Duration;

use omfp_protocol::CardAnswer;
use serde_json::{Map, Value, json};

use super::{AgentTool, ToolContext, ToolFuture, error_text};
use crate::server::convert::supervisor_error;

/// Name of the card tool (AGT-096).
pub const CARD: &str = "omfp_card";

pub(super) fn tool() -> AgentTool {
    AgentTool {
        name: CARD,
        label: "Card",
        description: "Draws a widget (a React component, as in views) in this tool call's card in the chat, for one \
                      moment of the conversation: a terminal showing something (useTerminal), a chart of a result, a \
                      question with its own controls. Nothing is written; content that should stay goes in a mount \
                      (omfp_ui_mount). Give exactly one of widget, a reference (a bare name means ./widgets/<name>; \
                      user:<name>, omfp:<name>), and code, the source of a one-file component (default export) \
                      compiled now as card.tsx: it imports only react, react/jsx-runtime, react-dom, omfp/sdk and \
                      omfp/kit, no relative file, and a compile error is this tool's error \
                      (card.tsx:line:column: message). props are the component's props, title the card's title. \
                      Answers JSON: {\"status\":\"shown\"} at once without wait. With wait: true it waits for the \
                      user, who meanwhile is asked for attention: the widget calls submit(value) (any JSON) or \
                      cancel() of useAnswer() from omfp/sdk, and the tool answers \
                      {\"status\":\"submitted\",\"value\":…} or {\"status\":\"cancelled\"} (the card also offers \
                      Cancel itself); with timeoutSeconds, {\"status\":\"timeout\"} once they passed. The widget \
                      mounts again whenever the card is shown (a reload, another window), so a waiting widget draws \
                      its finished state from useAnswer().state instead of asking again. Read omfp://docs/widgets, \
                      section Cards in the chat, before writing one.",
        parameters: json!({
            "type": "object",
            "properties": {
                "widget": {
                    "type": "string",
                    "minLength": 1,
                    "description": "A widget reference: a bare name means ./widgets/<name>; user:<name>, omfp:<name>."
                },
                "code": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The source of a one-file React component (default export), compiled as card.tsx."
                },
                "props": { "type": "object", "description": "The component's props." },
                "title": { "type": "string", "description": "The card's title." },
                "wait": {
                    "type": "boolean",
                    "description": "Wait until the user answers through useAnswer() (or cancels) and answer with it."
                },
                "timeoutSeconds": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "With wait: stop waiting after this long; the answer's status is then timeout."
                }
            },
            "additionalProperties": false
        }),
        run,
    }
}

/// A checked `omfp_card` call: the source to compile, if any, and how long it waits (`None`: it does not).
struct Card {
    code: Option<String>,
    wait: Option<Wait>,
}

/// How a waiting card ends besides an answer: after `timeout`, or never.
struct Wait {
    timeout: Option<Duration>,
}

impl Card {
    fn parse(arguments: &Map<String, Value>) -> Result<Self, String> {
        let text = |key: &str| match arguments.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.clone())),
            Some(_) => Err(format!("`{key}` must be a non-empty string")),
        };
        let widget = text("widget")?;
        let code = text("code")?;
        if widget.is_some() == code.is_some() {
            return Err("give exactly one of `widget` (a widget reference) and `code` (the source of a one-file \
                        component)"
                .to_owned());
        }
        if !matches!(
            arguments.get("props"),
            None | Some(Value::Null | Value::Object(_))
        ) {
            return Err("`props` must be an object: the component's props".to_owned());
        }
        if !matches!(
            arguments.get("title"),
            None | Some(Value::Null | Value::String(_))
        ) {
            return Err("`title` must be a string".to_owned());
        }
        let wait = match arguments.get("wait") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(wait)) => *wait,
            Some(_) => return Err("`wait` must be true or false".to_owned()),
        };
        let timeout = match arguments.get("timeoutSeconds") {
            None | Some(Value::Null) => None,
            Some(_) if !wait => {
                return Err(
                    "`timeoutSeconds` needs `wait: true`: a card without wait answers at once"
                        .to_owned(),
                );
            }
            Some(value) => Some(Duration::from_secs(
                value
                    .as_u64()
                    .filter(|seconds| *seconds >= 1)
                    .ok_or("`timeoutSeconds` must be an integer of at least 1")?,
            )),
        };
        Ok(Self {
            code,
            wait: wait.then_some(Wait { timeout }),
        })
    }
}

fn run(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let card = Card::parse(&arguments)?;
        let (Some(thread_id), Some(tool_call_id)) =
            (context.thread_id(), context.tool_call_id.as_deref())
        else {
            return Err(format!(
                "{CARD} needs a thread's agent: it draws the card in the chat of the thread whose tool call it is"
            ));
        };
        if let Some(code) = card.code {
            let widgets = std::sync::Arc::clone(&context.services.widgets);
            tokio::task::spawn_blocking(move || widgets.compile_card(&code))
                .await
                .map_err(|error| format!("internal: {error}"))?
                .map_err(|error| error.to_string())?;
        }
        let Some(wait) = card.wait else {
            return Ok(status("shown"));
        };
        let mut card_wait = context
            .services
            .supervisor
            .wait_card(thread_id, tool_call_id)
            .await
            .map_err(|error| error_text(&supervisor_error(error)))?;
        let answer = match wait.timeout {
            Some(timeout) => tokio::time::timeout(timeout, card_wait.answer()).await,
            None => Ok(card_wait.answer().await),
        };
        let Ok(answer) = answer else {
            return Ok(status("timeout"));
        };
        Ok(
            match answer.map_err(|error| error_text(&supervisor_error(error)))? {
                CardAnswer::Submit { value } => {
                    json!({ "status": "submitted", "value": value }).to_string()
                }
                CardAnswer::Cancel => status("cancelled"),
            },
        )
    })
}

/// The tool's JSON answer `{"status":…}` (AGT-096).
fn status(status: &str) -> String {
    json!({ "status": status }).to_string()
}

#[cfg(test)]
mod tests {
    use omfp_protocol::{Call, CardAnswerParams, ErrorCode};

    use super::super::tests::{Harness, TOOL_CALL_ID, args};
    use super::*;
    use crate::server::Principal;

    fn card(context: ToolContext, arguments: Value) -> ToolFuture {
        run(context, args(arguments))
    }

    fn answer(harness: &Harness, answer: CardAnswer) -> Call {
        Call::CardAnswer(CardAnswerParams {
            thread_id: harness.thread.clone(),
            tool_call_id: TOOL_CALL_ID.to_owned(),
            answer,
        })
    }

    /// The caller's status changed and it needs the user (or not, `attention`) now.
    async fn until_attention(
        harness: &Harness,
        changes: &mut tokio::sync::broadcast::Receiver<String>,
        attention: bool,
    ) {
        let supervisor = &harness.services.supervisor;
        while supervisor.status(&harness.thread).needs_attention != attention {
            assert_eq!(changes.recv().await.unwrap(), harness.thread);
        }
    }

    // spec: AGT-096
    #[tokio::test]
    async fn bad_arguments_are_errors_naming_the_rule() {
        let harness = Harness::new().await;
        for (arguments, rule) in [
            (json!({}), "exactly one of `widget`"),
            (
                json!({ "widget": "w", "code": "x" }),
                "exactly one of `widget`",
            ),
            (
                json!({ "widget": "  " }),
                "`widget` must be a non-empty string",
            ),
            (json!({ "code": 1 }), "`code` must be a non-empty string"),
            (
                json!({ "widget": "w", "props": [1] }),
                "`props` must be an object",
            ),
            (
                json!({ "widget": "w", "title": 2 }),
                "`title` must be a string",
            ),
            (
                json!({ "widget": "w", "wait": "yes" }),
                "`wait` must be true or false",
            ),
            (
                json!({ "widget": "w", "timeoutSeconds": 5 }),
                "`timeoutSeconds` needs `wait: true`",
            ),
            (
                json!({ "widget": "w", "wait": false, "timeoutSeconds": 5 }),
                "`timeoutSeconds` needs `wait: true`",
            ),
            (
                json!({ "widget": "w", "wait": true, "timeoutSeconds": 0 }),
                "`timeoutSeconds` must be an integer of at least 1",
            ),
            (
                json!({ "widget": "w", "wait": true, "timeoutSeconds": 1.5 }),
                "`timeoutSeconds` must be an integer of at least 1",
            ),
        ] {
            let error = card(harness.context(), arguments.clone())
                .await
                .unwrap_err();
            assert!(error.contains(rule), "{arguments}: {error}");
        }
    }

    // spec: AGT-096
    #[tokio::test]
    async fn a_card_needs_a_threads_agent_and_its_tool_call() {
        let harness = Harness::new().await;
        let mcp = ToolContext {
            principal: Principal::Mcp {
                client: "claude".to_owned(),
            },
            workspace_id: None,
            tool_call_id: None,
            ..harness.context()
        };
        let without_call = ToolContext {
            tool_call_id: None,
            ..harness.context()
        };
        for context in [mcp, without_call] {
            let error = card(context, json!({ "widget": "w" })).await.unwrap_err();
            assert!(
                error.contains("omfp_card needs a thread's agent"),
                "{error}"
            );
        }
    }

    // spec: AGT-096
    #[tokio::test]
    async fn a_card_without_wait_answers_shown_at_once_and_writes_nothing() {
        let harness = Harness::new().await;
        let views = harness
            .services
            .workspaces
            .get(&harness.workspace)
            .unwrap()
            .path
            .join(".omfp");
        let before = std::fs::read_dir(&views).map(Iterator::count).ok();
        for arguments in [
            json!({ "widget": "tests", "props": { "n": 1 }, "title": "Tests" }),
            json!({ "widget": "user:chart" }),
            json!({
                "code": "import { Card } from \"omfp/kit\";\nexport default () => <Card title=\"x\" />;\n",
                "wait": false
            }),
        ] {
            assert_eq!(
                card(harness.context(), arguments).await,
                Ok(r#"{"status":"shown"}"#.to_owned())
            );
        }
        assert_eq!(std::fs::read_dir(&views).map(Iterator::count).ok(), before);
        assert!(
            !harness
                .services
                .supervisor
                .status(&harness.thread)
                .needs_attention
        );
    }

    // spec: AGT-096, PRT-225
    #[tokio::test]
    async fn code_that_does_not_compile_or_imports_a_file_is_the_tools_error() {
        let harness = Harness::new().await;
        let error = card(
            harness.context(),
            json!({ "code": "export const a = 1;\nconst b = ;\n" }),
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("card.tsx:2:11: "), "{error}");

        let error = card(
            harness.context(),
            json!({ "code": "import { x } from \"./x\";\nexport default () => x;\n", "wait": true }),
        )
        .await
        .unwrap_err();
        assert!(
            error.starts_with("card.tsx:1:19: cannot import \"./x\": a card is one file"),
            "{error}"
        );
        assert!(
            !harness
                .services
                .supervisor
                .status(&harness.thread)
                .needs_attention
        );
    }

    // spec: AGT-097, DMN-911, PRT-224
    #[tokio::test]
    async fn a_waiting_card_needs_the_user_until_the_first_answer_which_the_tool_answers() {
        let harness = Harness::new().await;
        let mut changes = harness.services.supervisor.subscribe_changes();
        let waiting = tokio::spawn(card(
            harness.context(),
            json!({ "widget": "pick", "wait": true }),
        ));
        until_attention(&harness, &mut changes, true).await;
        assert!(!waiting.is_finished());

        let value = json!({ "files": ["a.rs", "b.rs"] });
        let answered = super::super::call(
            &harness.context(),
            answer(
                &harness,
                CardAnswer::Submit {
                    value: value.clone(),
                },
            ),
        )
        .await;
        assert_eq!(answered, Ok(json!({})));
        let text = waiting.await.unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({ "status": "submitted", "value": value })
        );
        until_attention(&harness, &mut changes, false).await;

        let late = super::super::call(&harness.context(), answer(&harness, CardAnswer::Cancel))
            .await
            .unwrap_err();
        assert_eq!(late.code, ErrorCode::AlreadyResolved, "{late:?}");
    }

    // spec: AGT-097, DMN-911
    #[tokio::test]
    async fn a_cancelled_card_answers_cancelled() {
        let harness = Harness::new().await;
        let mut changes = harness.services.supervisor.subscribe_changes();
        let waiting = tokio::spawn(card(
            harness.context(),
            json!({ "code": "export default () => null;", "wait": true, "timeoutSeconds": 3600 }),
        ));
        until_attention(&harness, &mut changes, true).await;
        harness
            .services
            .supervisor
            .answer_card(&harness.thread, TOOL_CALL_ID, CardAnswer::Cancel)
            .await
            .unwrap();
        assert_eq!(
            waiting.await.unwrap(),
            Ok(r#"{"status":"cancelled"}"#.to_owned())
        );
        assert!(
            !harness
                .services
                .supervisor
                .status(&harness.thread)
                .needs_attention
        );
    }

    // spec: AGT-097, DMN-911
    #[tokio::test(start_paused = true)]
    async fn a_waiting_card_times_out_and_a_later_answer_is_already_resolved() {
        let harness = Harness::new().await;
        let supervisor = harness.services.supervisor.clone();
        let mut changes = supervisor.subscribe_changes();
        let answer = card(
            harness.context(),
            json!({ "widget": "w", "wait": true, "timeoutSeconds": 30 }),
        )
        .await;
        assert_eq!(answer, Ok(r#"{"status":"timeout"}"#.to_owned()));
        // The wait started and ended: two changes, and the thread no longer needs the user.
        assert_eq!(changes.recv().await.unwrap(), harness.thread);
        assert_eq!(changes.recv().await.unwrap(), harness.thread);
        assert!(!supervisor.status(&harness.thread).needs_attention);
        let late = supervisor
            .answer_card(&harness.thread, TOOL_CALL_ID, CardAnswer::Cancel)
            .await
            .unwrap_err();
        assert!(
            matches!(late, crate::supervisor::SupervisorError::CardNotWaiting(_)),
            "{late:?}"
        );
    }

    // spec: AGT-097, DMN-911, DMN-151
    #[tokio::test]
    async fn a_dropped_call_stops_waiting_and_no_longer_needs_the_user() {
        let harness = Harness::new().await;
        let supervisor = harness.services.supervisor.clone();
        let mut changes = supervisor.subscribe_changes();
        let waiting = tokio::spawn(card(
            harness.context(),
            json!({ "widget": "w", "wait": true }),
        ));
        until_attention(&harness, &mut changes, true).await;

        waiting.abort();
        assert!(waiting.await.unwrap_err().is_cancelled());
        until_attention(&harness, &mut changes, false).await;
        let late = supervisor
            .answer_card(&harness.thread, TOOL_CALL_ID, CardAnswer::Cancel)
            .await
            .unwrap_err();
        assert!(
            matches!(late, crate::supervisor::SupervisorError::CardNotWaiting(_)),
            "{late:?}"
        );
    }
}
