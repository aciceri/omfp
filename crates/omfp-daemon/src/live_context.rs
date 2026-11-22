//! The instructions of a live voice session (DMN-105): omp gives its realtime model only the live prompt, so the daemon
//! sends that prompt, a note telling the voice to ask the agent what it does not know (DMN-115), and the thread's
//! conversation.

use omfp_core::ThreadState;
use omp_rpc::{AgentMessage, AssistantContent, AssistantMessage, MessageContent, UserContent};

use crate::voice_log::Spoken;

/// omp's bundled live prompt, copied verbatim from omp 19.1.0
/// `packages/coding-agent/src/live/prompts/live-instructions.md`; re-synced on every omp upgrade (ENV-020). omp
/// renders the instructions it gets with the same Handlebars variables (`{{firstName}}`, `{{username}}`).
const BUNDLED_PROMPT: &str = include_str!("live_instructions.txt");

/// About how many characters of conversation the section keeps.
pub const CONVERSATION_CAP: usize = 16_000;

/// How many characters one turn keeps, its label included: a longer turn keeps its start and its end around `…`.
const TURN_CAP: usize = 2_000;

/// How many characters of a compaction summary the section keeps, from its start.
const SUMMARY_CAP: usize = 4_000;

/// How many distinct tool uses an assistant turn names.
const TOOL_NOTES: usize = 8;

/// omp's `customType` of the requests the voice delegated (`LIVE_DELEGATION_MESSAGE_TYPE`, 19.1.0).
const LIVE_DELEGATION: &str = "live-delegation";

/// Between two turns.
const TURN_SEPARATOR: &str = "\n\n";

/// How the model reads the section.
const INSTRUCTION: &str = "The conversation of this thread so far, oldest first (earlier turns may be left out, long \
                           turns are shortened around …). `User (spoken):` turns are requests you delegated by voice; \
                           `Voice conversation:` turns are what you and the user said in an earlier live session; \
                           `[used: …]` names the tools the agent ran. Use it to answer questions about the thread; \
                           delegate when you need more than it holds.\n\n";

/// What `{{` becomes: omp's Handlebars (`packages/utils/src/prompt.ts`) has no escape syntax, but its `default`
/// helper returns the string literal, so this renders as `{{`.
const ESCAPED_BRACES: &str = r#"{{default "" "{{"}}"#;

/// omfp's note after omp's prompt (DMN-115): that prompt leaves the voice to answer "ordinary conversation" itself, so
/// it says it does not know what the agent could look up. Deliberately generic: the agent's reach varies with each
/// workspace.
const ASK_THE_AGENT: &str = "<ask-the-agent>\n\
                             The client backend can look things up and act well beyond code. Something you do not \
                             know or cannot be sure of (about the user, their notes and data, what happened before, \
                             anything outside this conversation): MUST delegate it at once, the whole question in the \
                             request. NEVER answer that you do not know, say no, or ask the user before the backend \
                             tried.\n\
                             </ask-the-agent>\n";

/// The `instructions` of `live_connect`: omp's bundled prompt, omfp's note (DMN-115), then the thread context when the
/// thread has a conversation; `spoken` are the conversations of ended sessions not in the thread yet (DMN-113).
#[must_use]
pub fn live_instructions(title: &str, state: &ThreadState, spoken: &[Spoken]) -> String {
    let prompt = format!("{}\n\n{ASK_THE_AGENT}", BUNDLED_PROMPT.trim_end());
    match thread_context(title, state, spoken, CONVERSATION_CAP) {
        Some(context) => format!("{prompt}{context}"),
        None => prompt,
    }
}

/// The thread context section: the title if any, how to use the conversation, the summary of a compaction, and the
/// most recent turns within `cap` characters, oldest first, `spoken` last. `None` without a conversation. Every `{{`
/// is escaped for omp's renderer.
#[must_use]
pub fn thread_context(
    title: &str,
    state: &ThreadState,
    spoken: &[Spoken],
    cap: usize,
) -> Option<String> {
    let messages: Vec<&AgentMessage> = state
        .transcript
        .iter()
        .map(|entry| &entry.message)
        .collect();
    // omp's history after a restart starts with the summary of what its last compaction removed.
    let (summary, messages) = match messages
        .iter()
        .rposition(|message| matches!(message, AgentMessage::CompactionSummary(_)))
    {
        Some(at) => (compaction_summary(messages[at]), &messages[at + 1..]),
        None => (None, &messages[..]),
    };
    let mut parts = Vec::new();
    if let Some(summary) = summary {
        parts.push(format!("Summary of the earlier conversation:\n{summary}"));
    }
    let mut turns = turns(messages.iter().copied());
    turns.extend(
        spoken
            .iter()
            .map(|spoken| clip(Speaker::Voice.label(), &spoken.lines(), TURN_CAP)),
    );
    parts.extend(recent(&turns, cap));
    if parts.is_empty() {
        return None;
    }
    let mut section = String::from("\n<thread-context>\n");
    let title = title.trim();
    if !title.is_empty() {
        section.push_str("Thread title: ");
        section.push_str(title);
        section.push_str("\n\n");
    }
    section.push_str(INSTRUCTION);
    section.push_str(&parts.join(TURN_SEPARATOR));
    section.push_str("\n</thread-context>\n");
    Some(escape(&section))
}

/// The start of a compaction summary, within [`SUMMARY_CAP`].
fn compaction_summary(message: &AgentMessage) -> Option<String> {
    let AgentMessage::CompactionSummary(compaction) = message else {
        return None;
    };
    let summary = compaction.summary.as_deref()?.trim();
    if summary.is_empty() {
        return None;
    }
    if summary.chars().count() <= SUMMARY_CAP {
        return Some(summary.to_owned());
    }
    let mut kept: String = summary.chars().take(SUMMARY_CAP - 1).collect();
    kept.push('…');
    Some(kept)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Assistant,
}

impl Role {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::User => "User: ",
            Self::Assistant => "Assistant: ",
        }
    }
}

/// Who said a turn of the section.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Speaker {
    User,
    /// A request the voice delegated.
    Spoken,
    /// What was said in an earlier live session (DMN-113).
    Voice,
    Assistant,
}

impl Speaker {
    fn label(self) -> &'static str {
        match self {
            Self::User => "User: ",
            Self::Spoken => "User (spoken): ",
            Self::Voice => "Voice conversation:\n",
            Self::Assistant => "Assistant: ",
        }
    }
}

/// Consecutive messages of one speaker.
struct Turn {
    speaker: Speaker,
    text: String,
    /// Distinct tool uses, in order.
    tools: Vec<String>,
}

impl Turn {
    fn render(&self) -> String {
        let mut body = String::new();
        if !self.tools.is_empty() {
            body.push_str("[used: ");
            let named: Vec<&str> = self
                .tools
                .iter()
                .take(TOOL_NOTES)
                .map(String::as_str)
                .collect();
            body.push_str(&named.join(", "));
            let more = self.tools.len().saturating_sub(TOOL_NOTES);
            if more > 0 {
                body.push_str(" and ");
                body.push_str(&more.to_string());
                body.push_str(" more");
            }
            body.push(']');
            if !self.text.is_empty() {
                body.push('\n');
            }
        }
        body.push_str(&self.text);
        clip(self.speaker.label(), &body, TURN_CAP)
    }
}

/// The conversation as `Speaker: text` turns, consecutive messages of one speaker merged.
fn turns<'a>(messages: impl IntoIterator<Item = &'a AgentMessage>) -> Vec<String> {
    let mut turns: Vec<Turn> = Vec::new();
    for message in messages {
        let Some((speaker, text, tools)) = contribution(message) else {
            continue;
        };
        if !matches!(turns.last(), Some(last) if last.speaker == speaker) {
            turns.push(Turn {
                speaker,
                text: String::new(),
                tools: Vec::new(),
            });
        }
        let Some(turn) = turns.last_mut() else {
            continue;
        };
        if !text.is_empty() {
            if !turn.text.is_empty() {
                turn.text.push_str(TURN_SEPARATOR);
            }
            turn.text.push_str(&text);
        }
        for tool in tools {
            if !turn.tools.contains(&tool) {
                turn.tools.push(tool);
            }
        }
    }
    turns.iter().map(Turn::render).collect()
}

/// What a message adds to the section: user prompts, the requests the voice delegated, assistant text and the tools
/// the assistant ran (name and path), never their output.
fn contribution(message: &AgentMessage) -> Option<(Speaker, String, Vec<String>)> {
    match message {
        AgentMessage::Custom(custom) if custom.custom_type.as_deref() == Some(LIVE_DELEGATION) => {
            let text = joined(
                custom
                    .content
                    .as_ref()
                    .map(content_parts)
                    .unwrap_or_default(),
            );
            (!text.is_empty()).then_some((Speaker::Spoken, text, Vec::new()))
        }
        AgentMessage::Custom(custom)
            if custom.custom_type.as_deref() == Some(crate::voice_log::CUSTOM_TYPE) =>
        {
            let text = voice_turns(custom.details.as_ref());
            (!text.is_empty()).then_some((Speaker::Voice, text, Vec::new()))
        }
        AgentMessage::Assistant(assistant) => {
            let text = message_text(message)
                .map(|(_, text)| text)
                .unwrap_or_default();
            let tools = tool_uses(assistant);
            (!text.is_empty() || !tools.is_empty()).then_some((Speaker::Assistant, text, tools))
        }
        _ => message_text(message).map(|(_, text)| (Speaker::User, text, Vec::new())),
    }
}

/// `name path` for each tool call, `name` alone without a `path` argument.
fn tool_uses(message: &AssistantMessage) -> Vec<String> {
    message
        .content
        .iter()
        .flatten()
        .filter_map(|part| match part {
            AssistantContent::ToolCall(call) => {
                let name = call.name.as_deref()?;
                let path = call
                    .arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get("path"))
                    .and_then(serde_json::Value::as_str);
                Some(path.map_or_else(|| name.to_owned(), |path| format!("{name} {path}")))
            }
            _ => None,
        })
        .collect()
}

/// The turns of a voice conversation's `details` (`{ turns: [{ role, text }] }`, DMN-113), one `User:` or
/// `Assistant:` line each.
fn voice_turns(details: Option<&serde_json::Value>) -> String {
    details
        .and_then(|details| details.get("turns"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|turn| {
            let role = turn.get("role")?.as_str()?;
            let text = turn.get("text")?.as_str()?.trim();
            (!text.is_empty()).then(|| format!("{}{text}", crate::voice_log::label(role)))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text parts of a user or custom message's content, images as `[image]`.
fn content_parts(content: &MessageContent) -> Vec<&str> {
    match content {
        MessageContent::String(text) => vec![text.as_str()],
        MessageContent::UserContentList(list) => list
            .iter()
            .map(|part| match part {
                UserContent::Text(text) => text.text.as_deref().unwrap_or_default(),
                UserContent::Image(_) => "[image]",
            })
            .collect(),
    }
}

/// The non-blank parts, trimmed, one per line.
fn joined(parts: Vec<&str>) -> String {
    parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text a message contributes: user prompts (not omp's synthetic ones) and assistant text, images as `[image]`.
pub(crate) fn message_text(message: &AgentMessage) -> Option<(Role, String)> {
    let parts: Vec<&str> = match message {
        AgentMessage::User(user) if user.synthetic != Some(true) => {
            user.content.as_ref().map(content_parts).unwrap_or_default()
        }
        AgentMessage::Assistant(assistant) => assistant
            .content
            .iter()
            .flatten()
            .filter_map(|part| match part {
                AssistantContent::Text(text) => text.text.as_deref(),
                AssistantContent::Image(_) => Some("[image]"),
                _ => None,
            })
            .collect(),
        _ => return None,
    };
    let role = match message {
        AgentMessage::User(_) => Role::User,
        _ => Role::Assistant,
    };
    let text = joined(parts);
    (!text.is_empty()).then_some((role, text))
}

/// The most recent turns within `cap` characters (separators included), oldest first, cut at turn boundaries.
fn recent(turns: &[String], cap: usize) -> Vec<String> {
    let mut kept = Vec::new();
    let mut used = 0;
    for turn in turns.iter().rev() {
        let size = turn.chars().count()
            + if kept.is_empty() {
                0
            } else {
                TURN_SEPARATOR.len()
            };
        if used + size > cap {
            break;
        }
        used += size;
        kept.push(turn.clone());
    }
    kept.reverse();
    kept
}

/// `label` and `body` within `cap` characters: a longer body keeps its start and its end around `…`.
fn clip(label: &str, body: &str, cap: usize) -> String {
    let length = body.chars().count();
    let room = cap.saturating_sub(label.chars().count());
    if length <= room {
        return format!("{label}{body}");
    }
    let room = room.saturating_sub(1);
    let head = room / 2;
    let start: String = body.chars().take(head).collect();
    let end: String = body.chars().skip(length - (room - head)).collect();
    format!("{label}{start}…{end}")
}

/// Makes every `{{` render literally in omp's Handlebars.
fn escape(text: &str) -> String {
    text.replace("{{", ESCAPED_BRACES)
}

#[cfg(test)]
mod tests {
    use omfp_core::TranscriptEntry;
    use serde_json::{Value, json};

    use super::*;

    fn state(messages: &[Value]) -> ThreadState {
        ThreadState {
            transcript: messages
                .iter()
                .map(|message| TranscriptEntry {
                    message_id: None,
                    message: AgentMessage::from_value(message.clone()).unwrap(),
                })
                .collect(),
            ..ThreadState::default()
        }
    }

    fn user(text: &str) -> Value {
        json!({ "role": "user", "content": text })
    }

    fn assistant(text: &str) -> Value {
        json!({ "role": "assistant", "content": [{ "type": "text", "text": text }] })
    }

    /// The turns of a section, without its title and instruction lines.
    fn conversation(section: &str) -> Vec<String> {
        let body = section
            .split_once(INSTRUCTION)
            .unwrap()
            .1
            .strip_suffix("\n</thread-context>\n")
            .unwrap();
        body.split(TURN_SEPARATOR).map(str::to_owned).collect()
    }

    // spec: DMN-105
    #[test]
    fn the_section_has_the_title_the_instruction_and_the_turns_oldest_first() {
        let state = state(&[
            user("fix the login"),
            assistant("Done, the token was stale."),
        ]);

        let section = thread_context("Login bug", &state, &[], CONVERSATION_CAP).unwrap();

        assert_eq!(
            section,
            "\n<thread-context>\nThread title: Login bug\n\nThe conversation of this thread so far, oldest first \
             (earlier turns may be left out, long turns are shortened around …). `User (spoken):` turns are requests \
             you delegated by voice; `Voice conversation:` turns are what you and the user said in an earlier live \
             session; `[used: …]` names the tools the agent ran. Use it to answer questions about the \
             thread; delegate when you need more than it holds.\n\nUser: fix the login\n\nAssistant: Done, the token \
             was stale.\n</thread-context>\n"
        );
    }

    // spec: DMN-105
    #[test]
    fn an_untitled_thread_has_no_title_line() {
        let section = thread_context(" ", &state(&[user("hi")]), &[], CONVERSATION_CAP).unwrap();

        assert!(!section.contains("Thread title"));
        assert_eq!(conversation(&section), vec!["User: hi"]);
    }

    // spec: DMN-105
    #[test]
    fn thinking_tool_output_and_synthetic_prompts_are_left_out_tools_are_named() {
        let state = state(&[
            json!({ "role": "user", "content": [
                { "type": "text", "text": "what is in this screenshot?" },
                { "type": "image", "data": "AAAA", "mimeType": "image/png" },
            ] }),
            json!({ "role": "user", "content": "a reminder omp injected", "synthetic": true }),
            json!({ "role": "developer", "content": "developer note" }),
            json!({ "role": "assistant", "content": [
                { "type": "thinking", "thinking": "pondering the image" },
                { "type": "text", "text": "Let me look." },
                { "type": "toolCall", "id": "t1", "name": "read", "arguments": { "path": "a.png" } },
            ] }),
            json!({ "role": "toolResult", "toolCallId": "t1", "toolName": "read",
                    "content": [{ "type": "text", "text": "tool output" }], "isError": false }),
            json!({ "role": "assistant", "content": [
                { "type": "toolCall", "id": "t2", "name": "bash", "arguments": {} },
            ] }),
            assistant("A login form."),
        ]);

        let section = thread_context("", &state, &[], CONVERSATION_CAP).unwrap();

        assert_eq!(
            conversation(&section).join(TURN_SEPARATOR),
            "User: what is in this screenshot?\n[image]\n\nAssistant: [used: read a.png, bash]\nLet me look.\n\nA \
             login form."
        );
        for left_out in [
            "pondering",
            "tool output",
            "a reminder",
            "developer note",
            "AAAA",
        ] {
            assert!(!section.contains(left_out), "{left_out} leaked");
        }
    }

    // spec: DMN-105
    #[test]
    fn consecutive_messages_of_one_role_are_one_turn() {
        let turns = turns(
            &[user("a"), user("b"), assistant("c"), assistant("d")]
                .map(|message| AgentMessage::from_value(message).unwrap()),
        );

        assert_eq!(turns, vec!["User: a\n\nb", "Assistant: c\n\nd"]);
    }

    // spec: DMN-105
    #[test]
    fn requests_delegated_by_voice_are_spoken_user_turns() {
        let state = state(&[
            user("fix the login"),
            assistant("Done."),
            json!({ "role": "custom", "customType": "live-delegation", "content": "run the tests",
                    "display": true, "attribution": "agent" }),
            json!({ "role": "custom", "customType": "other", "content": "an extension's note", "display": true }),
            assistant("All green."),
        ]);

        let section = thread_context("", &state, &[], CONVERSATION_CAP).unwrap();

        assert_eq!(
            conversation(&section),
            vec![
                "User: fix the login",
                "Assistant: Done.",
                "User (spoken): run the tests",
                "Assistant: All green.",
            ]
        );
    }

    // spec: DMN-105, DMN-113
    #[test]
    fn an_earlier_voice_conversation_is_one_turn() {
        let state = state(&[
            user("fix the login"),
            json!({ "role": "custom", "customType": "omfp-voice", "display": true,
                    "content": "Voice conversation · 2 turns\n(…)\nUser: is it done?\nAssistant: Yes.",
                    "details": { "turns": [
                        { "role": "user", "text": "is it done?" },
                        { "role": "assistant", "text": "Yes." },
                    ] } }),
        ]);

        let section = thread_context("", &state, &[], CONVERSATION_CAP).unwrap();

        assert_eq!(
            conversation(&section),
            vec![
                "User: fix the login",
                "Voice conversation:\nUser: is it done?\nAssistant: Yes.",
            ]
        );
    }

    // spec: DMN-105, DMN-113
    #[test]
    fn a_conversation_not_in_the_thread_yet_comes_last() {
        let spoken = Spoken::from_lines(&[omfp_core::LiveLine {
            role: omp_rpc::LiveRole::User,
            turn: 1,
            text: "and the logout?".into(),
            r#final: true,
        }])
        .unwrap();

        let section = thread_context(
            "",
            &state(&[user("fix the login")]),
            &[spoken],
            CONVERSATION_CAP,
        )
        .unwrap();

        assert_eq!(
            conversation(&section),
            vec![
                "User: fix the login",
                "Voice conversation:\nUser: and the logout?",
            ]
        );
    }

    // spec: DMN-105
    #[test]
    fn a_compaction_summary_comes_first_within_its_budget() {
        let summary = format!("We fixed the login. {}", "y".repeat(SUMMARY_CAP));
        let state = state(&[
            user("before the compaction"),
            json!({ "role": "compactionSummary", "summary": summary, "tokensBefore": 1000 }),
            user("after it"),
        ]);

        let section = thread_context("", &state, &[], CONVERSATION_CAP).unwrap();
        let parts = conversation(&section);

        assert_eq!(parts.len(), 2);
        assert!(
            parts[0].starts_with("Summary of the earlier conversation:\nWe fixed the login. yyy")
        );
        assert!(parts[0].ends_with("y…"));
        assert_eq!(
            parts[0].chars().count(),
            "Summary of the earlier conversation:\n".len() + SUMMARY_CAP
        );
        assert_eq!(parts[1], "User: after it");
    }

    // spec: DMN-105
    #[test]
    fn the_cap_keeps_the_most_recent_whole_turns() {
        let turns: Vec<String> = ["User: 0123456789", "Assistant: abcdef", "User: xyz"]
            .map(str::to_owned)
            .into();
        // "Assistant: abcdef" (17) + separator (2) + "User: xyz" (9) = 28.
        assert_eq!(recent(&turns, 28), vec!["Assistant: abcdef", "User: xyz"]);
        assert_eq!(recent(&turns, 27), vec!["User: xyz"]);
        assert_eq!(recent(&turns, 1_000), turns);
    }

    // spec: DMN-105
    #[test]
    fn a_long_turn_keeps_its_start_and_its_end() {
        assert_eq!(clip("User: ", "0123456789ééé", 13), "User: 012…ééé");
        assert_eq!(clip("User: ", "short", 13), "User: short");

        let long = format!("start {} end", "x".repeat(TURN_CAP));
        let turns =
            turns(&[assistant(&long)].map(|message| AgentMessage::from_value(message).unwrap()));
        assert_eq!(turns[0].chars().count(), TURN_CAP);
        assert!(turns[0].starts_with("Assistant: start xxx"));
        assert!(turns[0].contains('…'));
        assert!(turns[0].ends_with("xxx end"));
    }

    // spec: DMN-105
    #[test]
    fn one_long_answer_no_longer_pushes_the_conversation_out() {
        let mut messages = vec![user("first")];
        messages.push(assistant(&"x".repeat(CONVERSATION_CAP)));
        messages.push(user("last"));

        let parts =
            conversation(&thread_context("", &state(&messages), &[], CONVERSATION_CAP).unwrap());

        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "User: first");
        assert_eq!(parts[2], "User: last");
    }

    // spec: DMN-105
    #[test]
    fn the_cap_applies_to_the_section() {
        let messages: Vec<Value> = (0..40)
            .map(|turn| {
                if turn % 2 == 0 {
                    user(&format!("question {turn}"))
                } else {
                    assistant(&format!("answer {turn} {}", "x".repeat(TURN_CAP)))
                }
            })
            .collect();

        let parts =
            conversation(&thread_context("", &state(&messages), &[], CONVERSATION_CAP).unwrap());

        let [.., question, answer] = parts.as_slice() else {
            panic!("too few turns: {parts:?}");
        };
        assert_eq!(question, "User: question 38");
        assert!(answer.starts_with("Assistant: answer 39 "));
        assert!(parts.join(TURN_SEPARATOR).chars().count() <= CONVERSATION_CAP);
        assert!(!parts.iter().any(|part| part.contains("question 0")));
    }

    // spec: DMN-105
    #[test]
    fn handlebars_in_the_title_and_the_turns_is_escaped() {
        let state = state(&[user("render {{firstName}} and {{{raw}}} or {{#if x}}")]);

        let section = thread_context("{{title}}", &state, &[], CONVERSATION_CAP).unwrap();

        assert!(section.contains(r#"Thread title: {{default "" "{{"}}title}}"#));
        assert!(section.contains(
            r#"User: render {{default "" "{{"}}firstName}} and {{default "" "{{"}}{raw}}} or {{default "" "{{"}}#if x}}"#
        ));
        // Outside the escapes no `{{` is left.
        assert_eq!(section.replace(ESCAPED_BRACES, "").matches("{{").count(), 0);
    }

    // spec: DMN-105, DMN-115
    #[test]
    fn without_a_conversation_there_is_no_section() {
        assert_eq!(
            thread_context("Title", &ThreadState::default(), &[], CONVERSATION_CAP),
            None
        );
        let thinking_only = state(&[json!({ "role": "assistant", "content": [
            { "type": "thinking", "thinking": "hmm" },
        ] })]);
        assert_eq!(
            thread_context("Title", &thinking_only, &[], CONVERSATION_CAP),
            None
        );
        assert_eq!(
            live_instructions("Title", &thinking_only, &[]),
            format!("{}\n\n{ASK_THE_AGENT}", BUNDLED_PROMPT.trim_end())
        );
    }

    // spec: DMN-105, DMN-115
    #[test]
    fn the_instructions_are_the_bundled_prompt_the_note_then_the_section() {
        let state = state(&[user("hi")]);

        let instructions = live_instructions("", &state, &[]);

        assert!(instructions.starts_with("You: omp Live, realtime voice surface"));
        assert!(instructions.contains("{{firstName}} (OS account: {{username}})"));
        assert_eq!(
            instructions,
            format!(
                "{}\n\n{ASK_THE_AGENT}{}",
                BUNDLED_PROMPT.trim_end(),
                thread_context("", &state, &[], CONVERSATION_CAP).unwrap()
            )
        );
    }
}
