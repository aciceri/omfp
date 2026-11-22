//! The thread reducer (ARC-006, D-04): a total function from (state, event) to state, shared by daemon and UI.

use serde_json::Value;

use crate::event::{ProcessState, ThreadEvent};
use crate::live::live_error_message;
use crate::state::{
    BackgroundJob, LiveLine, LiveSession, Notice, NoticeFrame, QueueState, StreamingMessage,
    ThreadState, ToolRun, ToolStatus, TranscriptEntry, Widget,
};
use crate::wire::{
    AgentMessage, AssistantMessageEvent, ExtensionUiRequest, LivePhase, LiveTranscriptEvent,
    MessageEndEvent, MessageUpdateEvent, RpcAgentEvent, RpcNotification, TodoPhase,
};

impl ThreadState {
    /// Applies one event. Never panics; frames that do not change the visible state are ignored.
    pub fn apply(&mut self, event: &ThreadEvent) {
        match event {
            ThreadEvent::Omp { frame } => self.apply_frame(frame),
            ThreadEvent::History { session, messages } => {
                self.session = Some(session.clone());
                self.transcript = messages
                    .iter()
                    .map(|message| TranscriptEntry {
                        message_id: None,
                        message: message.clone(),
                    })
                    .collect();
                // The restored branch has its own list (omp's `getLatestTodoSnapshotIdentity`).
                self.todo_phases = messages
                    .iter()
                    .rev()
                    .find_map(committed_todo_phases)
                    .unwrap_or_default();
                self.streaming = None;
                // Their positions referred to the replaced transcript; a command's output and the URLs omp opened
                // stay (a restart after `/mcp reauth` resumes the same transcript), at most after its last message.
                let len = self.transcript.len();
                self.notices.retain_mut(|notice| {
                    let keep = matches!(
                        notice.frame,
                        NoticeFrame::CommandOutput(_) | NoticeFrame::OpenUrl(_)
                    );
                    notice.position = notice.position.min(len);
                    keep
                });
                // A new process numbers its requests from scratch, so old prompt ids would collide.
                self.prompts.clear();
            }
            ThreadEvent::Process { state } => {
                self.process = state.clone();
                if !matches!(state, ProcessState::Running | ProcessState::Starting) {
                    // Nothing can answer, settle or finish streaming without a process (DMN-035); a live session
                    // ends with it (DMN-103).
                    self.settled = true;
                    self.interactions.clear();
                    self.streaming = None;
                    self.live = None;
                }
            }
            ThreadEvent::InteractionResolved { id, .. } => self.close_interaction(id),
            ThreadEvent::LiveSession { owner } => match owner {
                Some(owner) => {
                    self.live = Some(LiveSession {
                        owner: owner.clone(),
                        phase: None,
                        transcript: Vec::new(),
                        context_end: self.transcript.len(),
                    });
                    self.live_error = None;
                }
                None => self.live = None,
            },
            ThreadEvent::OpenFile(request) => self.open_file = Some(request.clone()),
            ThreadEvent::Notify(request) => self.notify = Some(request.clone()),
            ThreadEvent::Unknown { raw } => self.notice(NoticeFrame::Unknown(raw.clone())),
        }
    }

    fn apply_frame(&mut self, frame: &RpcNotification) {
        match frame {
            RpcNotification::RpcAgentEvent(event) => self.apply_agent_event(event),
            RpcNotification::SessionSettled(_) => self.settled = true,
            RpcNotification::PromptResult(result) => {
                if let Some(id) = &result.id {
                    self.prompts.insert(id.clone(), result.clone());
                }
            }
            RpcNotification::ExtensionUiRequest(request) => self.apply_ui_request(request),
            RpcNotification::AvailableCommandsUpdate(update) => {
                self.commands.clone_from(&update.commands);
            }
            RpcNotification::SubagentLifecycle(lifecycle) => {
                let subagent = self
                    .subagents
                    .entry(lifecycle.payload.id.clone())
                    .or_default();
                subagent.lifecycle = Some(lifecycle.payload.clone());
            }
            RpcNotification::SubagentProgress(progress) => {
                // The subagent id lives in the raw `AgentProgress` record.
                if let Some(id) = progress.payload.progress.get("id").and_then(Value::as_str) {
                    self.subagents.entry(id.to_owned()).or_default().progress =
                        Some(progress.payload.clone());
                }
            }
            RpcNotification::ExtensionError(error) => {
                self.notice(NoticeFrame::ExtensionError(error.clone()));
            }
            RpcNotification::RpcFrameError(error) => {
                self.notice(NoticeFrame::RpcFrameError(error.clone()));
            }
            RpcNotification::Unknown(raw) => self.notice(NoticeFrame::Unknown(raw.clone())),
            RpcNotification::CommandOutput(output) => {
                self.notice(NoticeFrame::CommandOutput(output.clone()));
            }
            RpcNotification::LivePhase(event) => self.live_phase(event.phase),
            RpcNotification::LiveTranscript(event) => self.live_transcript(event),
            RpcNotification::LiveEnd(end) => {
                self.live = None;
                self.live_error = end.error.as_deref().map(live_error_message);
            }
            RpcNotification::Ready(_)
            | RpcNotification::SubagentEvent(_)
            | RpcNotification::LiveLevels(_)
            | RpcNotification::DictationState(_)
            | RpcNotification::DictationTranscript(_)
            | RpcNotification::DictationEnd(_)
            | RpcNotification::BtwDelta(_)
            | RpcNotification::BtwRecord(_)
            | RpcNotification::SessionInfoUpdate(_)
            | RpcNotification::ConfigUpdate(_) => {}
        }
    }

    /// omp repeats the `error` phase after `live_end`: phases only apply to an open session (PRT-052).
    fn live_phase(&mut self, phase: LivePhase) {
        if let Some(live) = &mut self.live {
            live.phase = Some(phase);
        }
    }

    /// One line per role and turn; a later frame replaces the text until the final one (PRT-052).
    fn live_transcript(&mut self, event: &LiveTranscriptEvent) {
        let Some(live) = &mut self.live else {
            return;
        };
        let line = LiveLine {
            role: event.role,
            turn: event.turn,
            text: event.text.clone(),
            r#final: event.r#final,
        };
        match live
            .transcript
            .iter_mut()
            .find(|known| known.role == event.role && known.turn == event.turn)
        {
            Some(known) if !known.r#final => *known = line,
            Some(_) => {}
            None => live.transcript.push(line),
        }
    }

    fn apply_agent_event(&mut self, event: &RpcAgentEvent) {
        match event {
            RpcAgentEvent::AgentStart(_) => self.settled = false,
            RpcAgentEvent::MessageStart(start) => {
                if let AgentMessage::Assistant(message) = &start.message {
                    self.streaming = Some(StreamingMessage {
                        message_id: start.message_id.clone(),
                        message: message.clone(),
                    });
                }
            }
            RpcAgentEvent::MessageUpdate(update) => self.apply_message_update(update),
            RpcAgentEvent::MessageEnd(end) => self.end_message(end),
            RpcAgentEvent::ToolExecutionStart(start) => {
                let run = self.tool_run(&start.tool_call_id, &start.tool_name, ToolStatus::Running);
                run.status = ToolStatus::Running;
                run.args.clone_from(&start.args);
                run.intent.clone_from(&start.intent);
            }
            RpcAgentEvent::ToolExecutionUpdate(update) => {
                let run =
                    self.tool_run(&update.tool_call_id, &update.tool_name, ToolStatus::Running);
                if update.args.is_some() {
                    run.args.clone_from(&update.args);
                }
                run.partial_result.clone_from(&update.partial_result);
                if let Some(frame) = update.partial_result.as_ref().and_then(job_frame) {
                    run.background = Some(frame);
                }
            }
            RpcAgentEvent::ToolStreamUpdate(update) => {
                let run =
                    self.tool_run(&update.tool_call_id, &update.tool_name, ToolStatus::Pending);
                run.preview.clone_from(&update.update);
            }
            RpcAgentEvent::ToolExecutionEnd(end) => {
                let run = self.tool_run(&end.tool_call_id, &end.tool_name, ToolStatus::Running);
                run.result.clone_from(&end.result);
                // The start result of a background job: its text is omp's notice to the model, not the job's
                // output; later updates, which may have come first, are newer.
                if run.background.is_none() {
                    run.background =
                        end.result
                            .as_ref()
                            .and_then(job_frame)
                            .map(|job| BackgroundJob {
                                output: None,
                                ..job
                            });
                }
                run.status = if end.is_error == Some(true) {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Succeeded
                };
            }
            RpcAgentEvent::QueueUpdate(update) => {
                self.queue = QueueState {
                    steering: update.steering.clone(),
                    follow_up: update.follow_up.clone(),
                    live_steered: update.live_steered,
                };
            }
            RpcAgentEvent::AutoCompactionStart(e) => {
                self.notice(NoticeFrame::AutoCompactionStart(e.clone()));
            }
            RpcAgentEvent::AutoCompactionEnd(e) => {
                self.notice(NoticeFrame::AutoCompactionEnd(e.clone()));
            }
            RpcAgentEvent::ManualCompactionStart(e) => {
                self.notice(NoticeFrame::ManualCompactionStart(e.clone()));
            }
            RpcAgentEvent::ManualCompactionEnd(e) => {
                self.notice(NoticeFrame::ManualCompactionEnd(e.clone()));
            }
            RpcAgentEvent::AutoRetryStart(e) => self.notice(NoticeFrame::AutoRetryStart(e.clone())),
            RpcAgentEvent::AutoRetryEnd(e) => self.notice(NoticeFrame::AutoRetryEnd(e.clone())),
            RpcAgentEvent::Notice(e) => self.notice(NoticeFrame::Notice(e.clone())),
            // `agent_end.messages` can be compacted: the transcript comes from `message_end` only (DMN-043).
            RpcAgentEvent::AgentEnd(_)
            | RpcAgentEvent::TurnStart(_)
            | RpcAgentEvent::TurnEnd(_)
            | RpcAgentEvent::CacheWarmingStart(_)
            | RpcAgentEvent::CacheWarmingEnd(_)
            | RpcAgentEvent::RetryFallbackApplied(_)
            | RpcAgentEvent::RetryFallbackSucceeded(_)
            | RpcAgentEvent::ModelChanged(_)
            | RpcAgentEvent::ConfigWarningsChanged(_)
            | RpcAgentEvent::AdvisorCostChanged(_)
            | RpcAgentEvent::AdvisorYielded(_)
            | RpcAgentEvent::TtsrTriggered(_)
            | RpcAgentEvent::TodoReminder(_)
            | RpcAgentEvent::TodoAutoClear(_)
            | RpcAgentEvent::IrcMessage(_)
            | RpcAgentEvent::ThinkingLevelChanged(_) => {}
            RpcAgentEvent::GoalUpdated(update) => self.goal.clone_from(&update.goal),
        }
    }

    fn apply_message_update(&mut self, update: &MessageUpdateEvent) {
        let event = &update.assistant_message_event;
        if matches!(
            event,
            AssistantMessageEvent::Done(_) | AssistantMessageEvent::Error(_)
        ) {
            return;
        }
        if self
            .streaming
            .as_ref()
            .is_none_or(|streaming| streaming.message_id != update.message_id)
        {
            self.streaming = Some(StreamingMessage::new(update.message_id.clone()));
        }
        if let Some(streaming) = &mut self.streaming {
            streaming.apply(event);
        }
    }

    fn end_message(&mut self, end: &MessageEndEvent) {
        let entry = TranscriptEntry {
            message_id: end.message_id.clone(),
            message: end.message.clone(),
        };
        let existing = end.message_id.as_ref().and_then(|id| {
            self.transcript
                .iter()
                .position(|entry| entry.message_id.as_ref() == Some(id))
        });
        match existing {
            Some(index) => self.transcript[index] = entry,
            None => self.transcript.push(entry),
        }
        if let Some(phases) = committed_todo_phases(&end.message) {
            self.todo_phases = phases;
        }
        // Records injected mid-stream (IRC, advisor cards) end without ending the streaming reply.
        if matches!(end.message, AgentMessage::Assistant(_)) {
            self.streaming = None;
        }
    }

    /// The run with `tool_call_id`, created with `status` when it is new.
    fn tool_run(
        &mut self,
        tool_call_id: &str,
        tool_name: &str,
        status: ToolStatus,
    ) -> &mut ToolRun {
        let index = self
            .tools
            .iter()
            .position(|run| run.tool_call_id == tool_call_id)
            .unwrap_or_else(|| {
                self.tools.push(ToolRun {
                    tool_call_id: tool_call_id.to_owned(),
                    tool_name: tool_name.to_owned(),
                    args: None,
                    intent: None,
                    status,
                    partial_result: None,
                    preview: None,
                    result: None,
                    background: None,
                });
                self.tools.len() - 1
            });
        &mut self.tools[index]
    }

    fn apply_ui_request(&mut self, request: &ExtensionUiRequest) {
        let presentation = &mut self.presentation;
        match request {
            ExtensionUiRequest::Select(_)
            | ExtensionUiRequest::Confirm(_)
            | ExtensionUiRequest::Input(_)
            | ExtensionUiRequest::Editor(_)
            | ExtensionUiRequest::Ask(_)
            | ExtensionUiRequest::Form(_) => self.interactions.push(request.clone()),
            ExtensionUiRequest::Cancel(cancel) => self.close_interaction(&cancel.target_id),
            ExtensionUiRequest::Notify(notify) => presentation.notifications.push(notify.clone()),
            ExtensionUiRequest::SetStatus(status) => match &status.status_text {
                Some(text) => {
                    presentation
                        .statuses
                        .insert(status.status_key.clone(), text.clone());
                }
                None => {
                    presentation.statuses.remove(&status.status_key);
                }
            },
            ExtensionUiRequest::SetWidget(widget) => match &widget.widget_lines {
                Some(lines) => {
                    let value = Widget {
                        lines: lines.clone(),
                        placement: widget.widget_placement,
                    };
                    presentation
                        .widgets
                        .insert(widget.widget_key.clone(), value);
                }
                None => {
                    presentation.widgets.remove(&widget.widget_key);
                }
            },
            ExtensionUiRequest::SetTitle(title) => presentation.title = Some(title.title.clone()),
            ExtensionUiRequest::SetEditorText(text) => {
                presentation.editor_text = Some(text.text.clone());
            }
            ExtensionUiRequest::OpenUrl(open) => {
                presentation.open_url = Some(open.clone());
                self.notice(NoticeFrame::OpenUrl(open.clone()));
            }
        }
    }

    /// Removes the pending dialog(s) with omp id `id`.
    fn close_interaction(&mut self, id: &str) {
        self.interactions
            .retain(|request| dialog_id(request) != Some(id));
    }

    fn notice(&mut self, frame: NoticeFrame) {
        self.notices.push(Notice {
            position: self.transcript.len(),
            frame,
        });
    }
}

/// The id of a dialog request; `None` for other methods.
fn dialog_id(request: &ExtensionUiRequest) -> Option<&str> {
    match request {
        ExtensionUiRequest::Select(select) => Some(&select.id),
        ExtensionUiRequest::Confirm(confirm) => Some(&confirm.id),
        ExtensionUiRequest::Input(input) => Some(&input.id),
        ExtensionUiRequest::Editor(editor) => Some(&editor.id),
        ExtensionUiRequest::Ask(ask) => Some(&ask.id),
        ExtensionUiRequest::Form(form) => Some(&form.id),
        _ => None,
    }
}

/// The phases a successful, state-changing `todo` result committed (omp's `committedTodoPhases`): `None` for other
/// messages, failed calls, `view` reads and phases that do not decode. A task of an older omp's `blocked` status
/// (gone since 18.19) reads as `pending`, as omp restores it.
fn committed_todo_phases(message: &AgentMessage) -> Option<Vec<TodoPhase>> {
    let AgentMessage::ToolResult(result) = message else {
        return None;
    };
    if result.tool_name.as_deref() != Some("todo") || result.is_error == Some(true) {
        return None;
    }
    let details = result.details.as_ref()?;
    if details.get("op").and_then(Value::as_str) == Some("view") {
        return None;
    }
    let mut phases = details.get("phases")?.clone();
    for phase in phases.as_array_mut().into_iter().flatten() {
        let tasks = phase.get_mut("tasks").and_then(Value::as_array_mut);
        for status in tasks
            .into_iter()
            .flatten()
            .filter_map(|task| task.get_mut("status"))
        {
            if status == "blocked" {
                *status = Value::from("pending");
            }
        }
    }
    serde_json::from_value(phases).ok()
}

/// The background job of an omp tool result or partial result (`{ content, details: { async: { state, jobId },
/// exitCode? } }`, PRT-100); `None` without a job frame or with an unknown state.
fn job_frame(result: &Value) -> Option<BackgroundJob> {
    let details = result.get("details")?;
    let frame = details.get("async")?;
    let state = serde_json::from_value(frame.get("state")?.clone()).ok()?;
    let output: String = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect();
    Some(BackgroundJob {
        job_id: frame.get("jobId")?.as_str()?.to_owned(),
        state,
        output: (!output.is_empty()).then_some(output),
        exit_code: details.get("exitCode").and_then(Value::as_i64),
    })
}

/// Applies `events` in order to a fresh state.
pub fn replay<'a>(events: impl IntoIterator<Item = &'a ThreadEvent>) -> ThreadState {
    let mut state = ThreadState::default();
    for event in events {
        state.apply(event);
    }
    state
}
