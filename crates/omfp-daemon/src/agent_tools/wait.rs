//! `omfp_thread_wait { threadId, promptId?, timeoutSeconds? }` (AGT-043, AGT-044): waits for a thread's prompt or
//! settled session instead of polling it.

use std::time::Duration;

use omfp_core::{ProcessState, ThreadState};
use omp_rpc::{AgentMessage, AssistantContent};
use serde_json::{Map, Value, json};
use tokio::sync::broadcast::error::RecvError;

use super::{AgentTool, THREAD_WAIT, ToolContext, ToolFuture, error_text};
use crate::server::convert::supervisor_error;

/// How long a wait lasts without `timeoutSeconds`.
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
/// The longest `timeoutSeconds`.
const MAX_TIMEOUT_SECONDS: u64 = 3600;

pub(super) fn tool() -> AgentTool {
    AgentTool {
        name: THREAD_WAIT,
        label: "Wait for thread",
        description: "Waits until an omfp thread is done, instead of polling it: with promptId (from thread.prompt) \
                      until that prompt's result arrived, else until the thread's last prompt, one still being sent \
                      included, finished and its session settled (call it once thread.prompt answered, not in the \
                      same batch of parallel tool calls). Answers JSON { threadId, status, prompt?, \
                      lastAssistantText }; status is done, needs_attention (it waits for an answer: see \
                      interaction.respond, or a card of omfp_card: see card.answer), stopped, failed or timeout.",
        parameters: json!({
            "type": "object",
            "properties": {
                "threadId": { "type": "string", "minLength": 1, "description": "The thread to wait for (not your own)." },
                "promptId": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Wait for this prompt (the promptId thread.prompt answered) rather than for the thread to settle."
                },
                "timeoutSeconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECONDS,
                    "description": "Give up after this long (default 600); the answer's status is then timeout."
                }
            },
            "required": ["threadId"],
            "additionalProperties": false
        }),
        run,
    }
}

/// A checked `omfp_thread_wait` call.
struct Wait {
    thread_id: String,
    prompt_id: Option<String>,
    timeout: Duration,
}

impl Wait {
    fn parse(arguments: &Map<String, Value>) -> Result<Self, String> {
        let thread_id = match arguments.get("threadId") {
            Some(Value::String(id)) if !id.trim().is_empty() => id.trim().to_owned(),
            _ => return Err("`threadId` must be a non-empty string".to_owned()),
        };
        let prompt_id = match arguments.get("promptId") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) if !id.trim().is_empty() => Some(id.trim().to_owned()),
            Some(_) => return Err("`promptId` must be a non-empty string".to_owned()),
        };
        let seconds = match arguments.get("timeoutSeconds") {
            None | Some(Value::Null) => DEFAULT_TIMEOUT_SECONDS,
            Some(value) => value
                .as_u64()
                .filter(|seconds| (1..=MAX_TIMEOUT_SECONDS).contains(seconds))
                .ok_or_else(|| {
                    format!("`timeoutSeconds` must be an integer from 1 to {MAX_TIMEOUT_SECONDS}")
                })?,
        };
        Ok(Self {
            thread_id,
            prompt_id,
            timeout: Duration::from_secs(seconds),
        })
    }
}

fn run(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let wait = Wait::parse(&arguments)?;
        if context.thread_id() == Some(wait.thread_id.as_str()) {
            return Err(
                "a thread cannot wait for itself: its session never settles while this tool runs"
                    .to_owned(),
            );
        }
        let supervisor = &context.services.supervisor;
        let log = supervisor
            .thread_log(&wait.thread_id)
            .await
            .map_err(|error| error_text(&supervisor_error(error)))?;
        let mut prompts = supervisor
            .last_prompt(&wait.thread_id)
            .await
            .map_err(|error| error_text(&supervisor_error(error)))?;
        let explicit = wait.prompt_id.is_some();
        let mut events = log.listen();
        // A waiting card needs the user without an event in the log: its start is a status change (DMN-911).
        let mut changes = supervisor.subscribe_changes();
        let deadline = tokio::time::Instant::now() + wait.timeout;
        loop {
            let last = prompts.borrow_and_update().clone();
            let target = wait.prompt_id.clone().or(last.id);
            let answer = |status: &str| {
                log.with_state(|state| answer(&wait.thread_id, status, target.as_deref(), state))
            };
            // Without promptId, a prompt still on its way to omp is about to be the thread's last one (DMN-152).
            let sending = !explicit && last.sending > 0;
            let attention = supervisor.status(&wait.thread_id).needs_attention;
            if !sending
                && let Some(status) =
                    log.with_state(|state| status(state, target.as_deref(), explicit, attention))
            {
                return Ok(answer(status));
            }
            tokio::select! {
                received = events.recv() => {
                    if let Err(RecvError::Closed) = received {
                        return Ok(answer("stopped"));
                    }
                }
                changed = prompts.changed() => {
                    // The thread was forgotten: no prompt reaches it anymore.
                    if changed.is_err() {
                        return Ok(answer("stopped"));
                    }
                }
                // Any thread's status change, a card starting to wait among them (AGT-097); missed ones are re-read.
                _ = changes.recv() => {}
                () = tokio::time::sleep_until(deadline) => return Ok(answer("timeout")),
            }
        }
    })
}

/// Whether the wait is over, and why (AGT-043): `None` while the thread works on. `attention`: the thread needs the
/// user, for a pending interaction or a waiting card (DMN-911).
fn status(
    state: &ThreadState,
    target: Option<&str>,
    explicit: bool,
    attention: bool,
) -> Option<&'static str> {
    let done = match target {
        Some(id) => state.prompts.contains_key(id) && (explicit || state.settled),
        None => state.settled,
    };
    if done {
        return Some("done");
    }
    if attention {
        return Some("needs_attention");
    }
    match state.process {
        ProcessState::Failed { .. } => Some("failed"),
        ProcessState::Stopped => Some("stopped"),
        ProcessState::Starting | ProcessState::Running => None,
    }
}

/// The tool's answer: `{ threadId, status, prompt?, lastAssistantText }`.
fn answer(thread_id: &str, status: &str, target: Option<&str>, state: &ThreadState) -> String {
    let mut answer = json!({
        "threadId": thread_id,
        "status": status,
        "lastAssistantText": last_assistant_text(state),
    });
    if let Some(result) = target.and_then(|id| state.prompts.get(id)) {
        let result = serde_json::to_value(result).unwrap_or_default();
        let mut prompt = Map::new();
        for key in ["id", "status", "error"] {
            if let Some(value) = result.get(key) {
                prompt.insert(key.to_owned(), value.clone());
            }
        }
        answer["prompt"] = Value::Object(prompt);
    }
    serde_json::to_string_pretty(&answer).unwrap_or_default()
}

/// The text blocks of the transcript's last assistant message, `""` without one.
fn last_assistant_text(state: &ThreadState) -> String {
    state
        .transcript
        .iter()
        .rev()
        .find_map(|entry| match &entry.message {
            AgentMessage::Assistant(assistant) => Some(
                assistant
                    .content
                    .iter()
                    .flatten()
                    .filter_map(|part| match part {
                        AssistantContent::Text(text) => text.text.as_deref(),
                        _ => None,
                    })
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use omp_rpc::RpcNotification;

    use super::super::tests::{Harness, args};
    use super::*;
    use crate::agent::FakeRun;
    use crate::threads::NewThread;

    fn frames(values: Vec<Value>) -> Vec<RpcNotification> {
        values
            .into_iter()
            .map(|value| serde_json::from_value(value).expect("an omp frame"))
            .collect()
    }

    /// An assistant message of the fake omp answering `text`.
    fn answered(text: &str) -> Value {
        json!({
            "type": "message_end",
            "messageId": text,
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": text }],
                "stopReason": "stop",
                "timestamp": 0
            }
        })
    }

    fn prompt_result(id: &str, settled: bool) -> Value {
        json!({ "type": "prompt_result", "id": id, "agentInvoked": true, "status": "completed", "sessionSettled": settled })
    }

    async fn wait(context: ToolContext, arguments: Value) -> Result<Value, String> {
        let text = run(context, args(arguments)).await?;
        Ok(serde_json::from_str(&text).expect("a JSON answer"))
    }

    impl Harness {
        /// Another thread of the workspace, the one waited for.
        fn worker(&self) -> String {
            self.services
                .threads
                .create(NewThread::new("worker", self.workspace.clone()))
                .unwrap()
                .id
        }

        /// Starts the thread's fake omp with a prompt; answers the prompt's id.
        async fn prompt(&self, thread: &str) -> String {
            self.launcher.push_run(FakeRun::fresh());
            self.services
                .supervisor
                .prompt(thread, command("work"))
                .await
                .unwrap()
                .prompt_id
        }
    }

    fn command(text: &str) -> omp_rpc::PromptCommand {
        omp_rpc::PromptCommand {
            message: text.to_owned(),
            images: None,
            streaming_behavior: None,
        }
    }

    /// omp's frames of a whole turn answering prompt `id` with `text`.
    fn turn(id: &str, text: &str) -> Vec<RpcNotification> {
        frames(vec![
            json!({ "type": "agent_start" }),
            answered(text),
            prompt_result(id, true),
            json!({ "type": "agent_end", "messages": [] }),
            json!({ "type": "session_settled" }),
        ])
    }

    // spec: AGT-043, DMN-152
    #[tokio::test]
    async fn without_a_prompt_id_the_wait_follows_a_prompt_still_on_its_way_to_the_thread() {
        let harness = Harness::new().await;
        let worker = harness.worker();
        let first = harness.prompt(&worker).await;
        let process = harness.launcher.process(0);
        process.emit(turn(&first, "First."));
        let settled = wait(harness.context(), json!({ "threadId": worker }))
            .await
            .unwrap();
        assert_eq!(settled["prompt"]["id"], first.as_str());

        // `thread.prompt` and the wait side by side, as an agent's parallel tool calls run: omp has not admitted the
        // second prompt yet when the wait starts.
        let gate = harness.launcher.gate("prompt");
        let sending = tokio::spawn({
            let supervisor = harness.services.supervisor.clone();
            let worker = worker.clone();
            async move { supervisor.prompt(&worker, command("more work")).await }
        });
        gate.arrived(1).await;
        let waiting = tokio::spawn(wait(harness.context(), json!({ "threadId": worker })));
        // The wait checks the thread while omp holds the second prompt (the gate opens before any assertion, so a
        // failure does not leave omp's call blocked).
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        let answered_early = waiting.is_finished();
        gate.open();
        let second = sending.await.unwrap().unwrap().prompt_id;
        assert!(
            !answered_early,
            "the prompt on its way is the thread's last"
        );
        process.emit(turn(&second, "Second."));

        let answer = waiting.await.unwrap().unwrap();
        assert_eq!(
            answer,
            json!({
                "threadId": worker,
                "status": "done",
                "prompt": { "id": second, "status": "completed" },
                "lastAssistantText": "Second."
            })
        );
    }

    // spec: AGT-043, DMN-152
    #[tokio::test(start_paused = true)]
    async fn a_prompt_that_never_reached_omp_does_not_hold_the_wait() {
        let harness = Harness::new().await;
        let worker = harness.worker();
        harness.launcher.fail_next_launch("no model");
        let refused = harness
            .services
            .supervisor
            .prompt(&worker, command("work"))
            .await;
        assert!(refused.is_err(), "{refused:?}");

        let answer = wait(
            harness.context(),
            json!({ "threadId": worker, "timeoutSeconds": 5 }),
        )
        .await
        .unwrap();
        // Not `timeout`: the refused prompt is no longer on its way, and the thread has no last prompt.
        assert_eq!(answer["status"], "done", "{answer}");
    }

    // spec: AGT-043, DMN-152
    #[tokio::test]
    async fn without_a_prompt_id_the_wait_ends_once_the_last_prompt_finished_and_the_session_settled()
     {
        let harness = Harness::new().await;
        let worker = harness.worker();
        let prompt_id = harness.prompt(&worker).await;
        let process = harness.launcher.process(0);

        let waiting = tokio::spawn(wait(harness.context(), json!({ "threadId": worker })));
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "the prompt has no result yet");
        // The prompt yields, but background work wakes the agent again before the session settles.
        process.emit(frames(vec![
            json!({ "type": "agent_start" }),
            answered("Half way."),
            prompt_result(&prompt_id, false),
        ]));
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "the session has not settled");
        process.emit(frames(vec![
            answered("All done."),
            json!({ "type": "agent_end", "messages": [] }),
            json!({ "type": "session_settled" }),
        ]));

        let answer = waiting.await.unwrap().unwrap();
        assert_eq!(
            answer,
            json!({
                "threadId": worker,
                "status": "done",
                "prompt": { "id": prompt_id, "status": "completed" },
                "lastAssistantText": "All done."
            })
        );
    }

    // spec: AGT-043
    #[tokio::test]
    async fn a_prompt_id_ends_the_wait_at_its_result_and_an_idle_thread_answers_at_once() {
        let harness = Harness::new().await;
        let worker = harness.worker();

        let idle = wait(harness.context(), json!({ "threadId": worker }))
            .await
            .unwrap();
        assert_eq!(
            idle,
            json!({ "threadId": worker, "status": "done", "lastAssistantText": "" })
        );

        let prompt_id = harness.prompt(&worker).await;
        let process = harness.launcher.process(0);
        process.emit(frames(vec![
            json!({ "type": "agent_start" }),
            answered("Yielded."),
        ]));
        let waiting = tokio::spawn(wait(
            harness.context(),
            json!({ "threadId": worker, "promptId": prompt_id }),
        ));
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "no prompt_result yet");
        process.emit(frames(vec![prompt_result(&prompt_id, false)]));

        let answer = waiting.await.unwrap().unwrap();
        assert_eq!(answer["status"], "done");
        assert_eq!(answer["prompt"]["id"], prompt_id.as_str());
        assert_eq!(answer["lastAssistantText"], "Yielded.");
    }

    // spec: AGT-043
    #[tokio::test]
    async fn a_pending_interaction_ends_the_wait() {
        let harness = Harness::new().await;
        let worker = harness.worker();
        harness.prompt(&worker).await;
        let process = harness.launcher.process(0);
        process.emit(frames(vec![json!({ "type": "agent_start" })]));

        let waiting = tokio::spawn(wait(harness.context(), json!({ "threadId": worker })));
        tokio::task::yield_now().await;
        process.emit(frames(vec![json!({
            "type": "extension_ui_request",
            "id": "ask-1",
            "method": "select",
            "title": "Delete the branch?",
            "options": ["Yes", "No"]
        })]));

        assert_eq!(waiting.await.unwrap().unwrap()["status"], "needs_attention");
    }

    // spec: AGT-097, AGT-043
    #[tokio::test]
    async fn a_card_that_starts_waiting_ends_the_wait_with_needs_attention() {
        let harness = Harness::new().await;
        let worker = harness.worker();
        harness.prompt(&worker).await;
        let process = harness.launcher.process(0);
        process.emit(frames(vec![json!({ "type": "agent_start" })]));

        let waiting = tokio::spawn(wait(harness.context(), json!({ "threadId": worker })));
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "the worker works on");
        // No event in the worker's log: only its status changes.
        let _card = harness
            .services
            .supervisor
            .wait_card(&worker, "toolu_card")
            .await
            .unwrap();

        assert_eq!(waiting.await.unwrap().unwrap()["status"], "needs_attention");
    }

    // spec: AGT-043
    #[tokio::test]
    async fn a_process_that_ends_before_the_thread_is_done_ends_the_wait() {
        let harness = Harness::new().await;
        let worker = harness.worker();
        harness.prompt(&worker).await;
        let process = harness.launcher.process(0);
        process.emit(frames(vec![json!({ "type": "agent_start" })]));

        let waiting = tokio::spawn(wait(harness.context(), json!({ "threadId": worker })));
        tokio::task::yield_now().await;
        process.exit();

        let answer = waiting.await.unwrap().unwrap();
        assert_eq!(answer["status"], "failed", "{answer}");
    }

    // spec: AGT-043
    #[tokio::test(start_paused = true)]
    async fn the_wait_times_out_while_the_thread_works() {
        let harness = Harness::new().await;
        let worker = harness.worker();
        harness.prompt(&worker).await;
        harness
            .launcher
            .process(0)
            .emit(frames(vec![json!({ "type": "agent_start" })]));

        let answer = wait(
            harness.context(),
            json!({ "threadId": worker, "timeoutSeconds": 2 }),
        )
        .await
        .unwrap();
        assert_eq!(answer["status"], "timeout");
    }

    // spec: AGT-044
    #[tokio::test]
    async fn bad_waits_are_tool_errors() {
        let harness = Harness::new().await;
        let worker = harness.worker();
        let range = "`timeoutSeconds` must be an integer from 1 to 3600";
        for (arguments, reason) in [
            (json!({}), "`threadId` must be a non-empty string"),
            (
                json!({ "threadId": "  " }),
                "`threadId` must be a non-empty string",
            ),
            (
                json!({ "threadId": worker, "promptId": 3 }),
                "`promptId` must be a non-empty string",
            ),
            (json!({ "threadId": worker, "timeoutSeconds": 0 }), range),
            (json!({ "threadId": worker, "timeoutSeconds": 3601 }), range),
            (json!({ "threadId": worker, "timeoutSeconds": "5" }), range),
            (
                json!({ "threadId": harness.thread }),
                "cannot wait for itself",
            ),
            (json!({ "threadId": "01NOPE" }), "not_found: "),
        ] {
            let error = wait(harness.context(), arguments.clone())
                .await
                .unwrap_err();
            assert!(error.contains(reason), "{arguments}: {error}");
        }
    }
}
