//! The instructions of a live voice session (DMN-105): omp gives its realtime model only the live prompt, so the daemon
//! sends that prompt followed by the thread's conversation.

use omfp_core::ThreadState;
use omp_rpc::{AgentMessage, AssistantContent, MessageContent, UserContent};

/// omp's bundled live prompt, copied verbatim from omp 18.18.6
/// `packages/coding-agent/src/live/prompts/live-instructions.md`; re-synced on every omp upgrade (ENV-020). omp
/// renders the instructions it gets with the same Handlebars variables (`{{firstName}}`, `{{username}}`).
const BUNDLED_PROMPT: &str = include_str!("live_instructions.txt");

/// About how many characters of conversation the section keeps.
pub const CONVERSATION_CAP: usize = 8_000;

/// Between two turns.
const TURN_SEPARATOR: &str = "\n\n";

/// What `{{` becomes: omp's Handlebars (`packages/utils/src/prompt.ts`) has no escape syntax, but its `default`
/// helper returns the string literal, so this renders as `{{`.
const ESCAPED_BRACES: &str = r#"{{default "" "{{"}}"#;

/// The `instructions` of `live_connect`: omp's bundled prompt, then the thread context when the thread has a
/// conversation.
#[must_use]
pub fn live_instructions(title: &str, state: &ThreadState) -> String {
    match thread_context(title, state, CONVERSATION_CAP) {
        Some(context) => format!("{}\n{context}", BUNDLED_PROMPT.trim_end()),
        None => BUNDLED_PROMPT.to_owned(),
    }
}

/// The thread context section: the title if any, how to use the conversation, and the most recent turns within
/// `cap` characters, oldest first. `None` without a conversation. Every `{{` is escaped for omp's renderer.
#[must_use]
pub fn thread_context(title: &str, state: &ThreadState, cap: usize) -> Option<String> {
    let turns = turns(state.transcript.iter().map(|entry| &entry.message));
    let kept = recent(&turns, cap);
    if kept.is_empty() {
        return None;
    }
    let mut section = String::from("\n<thread-context>\n");
    let title = title.trim();
    if !title.is_empty() {
        section.push_str("Thread title: ");
        section.push_str(title);
        section.push_str("\n\n");
    }
    section.push_str(
        "The conversation of this thread so far, oldest first (earlier turns may be left out). Use it to answer \
         questions about the thread; delegate when you need more than it holds.\n\n",
    );
    section.push_str(&kept.join(TURN_SEPARATOR));
    section.push_str("\n</thread-context>\n");
    Some(escape(&section))
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

/// The conversation as `Role: text` turns, consecutive messages of one role merged.
fn turns<'a>(messages: impl IntoIterator<Item = &'a AgentMessage>) -> Vec<String> {
    let mut turns: Vec<(Role, String)> = Vec::new();
    for message in messages {
        let Some((role, text)) = message_text(message) else {
            continue;
        };
        match turns.last_mut() {
            Some((last, merged)) if *last == role => {
                merged.push_str(TURN_SEPARATOR);
                merged.push_str(&text);
            }
            _ => turns.push((role, text)),
        }
    }
    turns
        .into_iter()
        .map(|(role, text)| format!("{}{text}", role.label()))
        .collect()
}

/// The text a message contributes: user prompts (not omp's synthetic ones) and assistant text, images as `[image]`.
pub(crate) fn message_text(message: &AgentMessage) -> Option<(Role, String)> {
    let parts: Vec<&str> = match message {
        AgentMessage::User(user) if user.synthetic != Some(true) => match &user.content {
            Some(MessageContent::String(text)) => vec![text.as_str()],
            Some(MessageContent::UserContentList(list)) => list
                .iter()
                .map(|part| match part {
                    UserContent::Text(text) => text.text.as_deref().unwrap_or_default(),
                    UserContent::Image(_) => "[image]",
                })
                .collect(),
            None => Vec::new(),
        },
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
    let text = parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some((role, text))
}

/// The most recent turns within `cap` characters (separators included), oldest first, cut at turn boundaries. A
/// newest turn longer than `cap` keeps its label and its tail after `…`.
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
        if used + size <= cap {
            used += size;
            kept.push(turn.clone());
        } else {
            if kept.is_empty() {
                kept.push(tail(turn, cap));
            }
            break;
        }
    }
    kept.reverse();
    kept
}

/// The label of `turn` and the end of its text, `cap` characters in all.
fn tail(turn: &str, cap: usize) -> String {
    let label = [Role::User, Role::Assistant]
        .into_iter()
        .map(Role::label)
        .find(|label| turn.starts_with(label))
        .unwrap_or_default();
    let text = &turn[label.len()..];
    let room = cap.saturating_sub(label.chars().count() + 1);
    let skip = text.chars().count().saturating_sub(room);
    let rest: String = text.chars().skip(skip).collect();
    format!("{label}…{rest}")
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
            .split_once("than it holds.\n\n")
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

        let section = thread_context("Login bug", &state, CONVERSATION_CAP).unwrap();

        assert_eq!(
            section,
            "\n<thread-context>\nThread title: Login bug\n\nThe conversation of this thread so far, oldest first \
             (earlier turns may be left out). Use it to answer questions about the thread; delegate when you need \
             more than it holds.\n\nUser: fix the login\n\nAssistant: Done, the token was stale.\n</thread-context>\n"
        );
    }

    // spec: DMN-105
    #[test]
    fn an_untitled_thread_has_no_title_line() {
        let section = thread_context(" ", &state(&[user("hi")]), CONVERSATION_CAP).unwrap();

        assert!(!section.contains("Thread title"));
        assert_eq!(conversation(&section), vec!["User: hi"]);
    }

    // spec: DMN-105
    #[test]
    fn only_user_prompts_and_assistant_text_count() {
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

        let section = thread_context("", &state, CONVERSATION_CAP).unwrap();

        assert!(section.ends_with(
            "than it holds.\n\nUser: what is in this screenshot?\n[image]\n\nAssistant: Let me look.\n\nA login \
             form.\n</thread-context>\n"
        ));
        for left_out in [
            "pondering",
            "tool output",
            "a reminder",
            "developer note",
            "a.png",
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
    fn a_newest_turn_longer_than_the_cap_keeps_its_tail() {
        let turns: Vec<String> = ["User: older", "Assistant: 0123456789ééé"]
            .map(str::to_owned)
            .into();

        let kept = recent(&turns, 20);

        assert_eq!(kept, vec!["Assistant: …56789ééé"]);
        assert_eq!(kept[0].chars().count(), 20);
    }

    // spec: DMN-105
    #[test]
    fn the_cap_applies_to_the_section() {
        let long = "x".repeat(CONVERSATION_CAP);
        let state = state(&[user("first"), assistant(&long), user("last")]);

        let section = thread_context("", &state, CONVERSATION_CAP).unwrap();

        assert_eq!(conversation(&section), vec!["User: last"]);
    }

    // spec: DMN-105
    #[test]
    fn handlebars_in_the_title_and_the_turns_is_escaped() {
        let state = state(&[user("render {{firstName}} and {{{raw}}} or {{#if x}}")]);

        let section = thread_context("{{title}}", &state, CONVERSATION_CAP).unwrap();

        assert!(section.contains(r#"Thread title: {{default "" "{{"}}title}}"#));
        assert!(section.contains(
            r#"User: render {{default "" "{{"}}firstName}} and {{default "" "{{"}}{raw}}} or {{default "" "{{"}}#if x}}"#
        ));
        // Outside the escapes no `{{` is left.
        assert_eq!(section.replace(ESCAPED_BRACES, "").matches("{{").count(), 0);
    }

    // spec: DMN-105
    #[test]
    fn without_a_conversation_there_is_no_section() {
        assert_eq!(
            thread_context("Title", &ThreadState::default(), CONVERSATION_CAP),
            None
        );
        let tools_only = state(&[json!({ "role": "assistant", "content": [
            { "type": "toolCall", "id": "t", "name": "bash", "arguments": {} },
        ] })]);
        assert_eq!(thread_context("Title", &tools_only, CONVERSATION_CAP), None);
        assert_eq!(live_instructions("Title", &tools_only), BUNDLED_PROMPT);
    }

    // spec: DMN-105
    #[test]
    fn the_instructions_are_the_bundled_prompt_then_the_section() {
        let state = state(&[user("hi")]);

        let instructions = live_instructions("", &state);

        assert!(instructions.starts_with("You: omp Live, realtime voice surface"));
        assert!(instructions.contains("{{firstName}} (OS account: {{username}})"));
        assert_eq!(
            instructions,
            format!(
                "{}\n{}",
                BUNDLED_PROMPT.trim_end(),
                thread_context("", &state, CONVERSATION_CAP).unwrap()
            )
        );
    }
}
