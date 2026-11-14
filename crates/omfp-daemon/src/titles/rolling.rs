//! Conversation-driven cadence, bounded context and conservative publication (DMN-073).

use std::collections::VecDeque;
use std::time::Duration;

use omfp_core::ThreadState;
use tokio::time::Instant;

use crate::live_context::{Role, message_text};

const MESSAGES: usize = 4;
const CHARACTERS: usize = 400;
const COOLDOWN: Duration = Duration::from_secs(300);
const RECENT: usize = 8;
const EXCERPT: usize = 600;
const ORIGIN: usize = 1000;

#[derive(Default)]
pub(crate) struct RollingTitle {
    messages: usize,
    characters: usize,
    last_attempt: Option<Instant>,
    origin: String,
    admitted: VecDeque<String>,
    candidate: Option<String>,
}

impl RollingTitle {
    pub(crate) fn observe(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        if self.origin.is_empty() {
            self.origin = message.chars().take(ORIGIN).collect();
        }
        self.messages = (self.messages + 1).min(MESSAGES);
        self.characters =
            (self.characters + message.chars().take(CHARACTERS).count()).min(CHARACTERS);
        if self.admitted.len() == RECENT {
            self.admitted.pop_front();
        }
        self.admitted
            .push_back(message.chars().take(EXCERPT).collect());
    }

    pub(crate) fn ready(&self, initial: bool, now: Instant) -> bool {
        ((initial && self.last_attempt.is_none())
            || (self.messages >= MESSAGES && self.characters >= CHARACTERS))
            && self
                .last_attempt
                .is_none_or(|last| now.duration_since(last) >= COOLDOWN)
    }

    pub(crate) fn begin(&mut self, now: Instant) {
        self.messages = 0;
        self.characters = 0;
        self.last_attempt = Some(now);
    }

    pub(crate) fn failed(&mut self) {
        // Confirmation must come from consecutive successful reconsiderations.
        self.candidate = None;
    }

    pub(crate) fn publish(&mut self, current: &str, title: &str, initial: bool) -> bool {
        if initial {
            self.candidate = None;
            return true;
        }
        if similar(current, title) {
            self.candidate = None;
            return false;
        }
        if self
            .candidate
            .as_ref()
            .is_some_and(|last| similar(last, title))
        {
            self.candidate = None;
            return true;
        }
        self.candidate = Some(title.to_owned());
        false
    }

    pub(crate) fn context(&self, current: &str, state: &ThreadState) -> String {
        // History is authoritative after a restart or navigation; admitted text also covers queued prompts
        // whose message_end has not arrived yet. Never include tools, thinking or synthetic user messages.
        let origin = state
            .transcript
            .iter()
            .find_map(|entry| {
                let (role, text) = message_text(&entry.message)?;
                (role == Role::User).then_some(text)
            })
            .unwrap_or_else(|| self.origin.clone());
        let mut context = format!(
            "Current title: {current}\nEarlier project context: {}\n\nRecent conversation (oldest first):\n",
            origin.chars().take(ORIGIN).collect::<String>()
        );
        let mut recent = Vec::with_capacity(RECENT);
        for entry in state.transcript.iter().rev() {
            if let Some((role, text)) = message_text(&entry.message) {
                recent.push((role, text.chars().take(EXCERPT).collect::<String>()));
                if recent.len() == RECENT {
                    break;
                }
            }
        }
        for (role, text) in recent.into_iter().rev() {
            context.push_str(role.label());
            context.push_str(&text);
            context.push('\n');
        }
        context.push_str(
            "\nRecent admitted user messages (may overlap the conversation or still be queued):\n",
        );
        for text in &self.admitted {
            context.push_str("User: ");
            context.push_str(text);
            context.push('\n');
        }
        context
    }
}

/// Ignore casing, punctuation, word order and small wording edits, not just byte-identical output.
fn similar(a: &str, b: &str) -> bool {
    fn normalized(text: &str) -> Vec<char> {
        let lowered = text.to_lowercase();
        let mut words: Vec<_> = lowered
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        words.sort_unstable();
        words.join(" ").chars().collect()
    }
    let (a, b) = (normalized(a), normalized(b));
    let length = a.len().max(b.len());
    let allowance = length / 5;
    if a.len().abs_diff(b.len()) > allowance {
        return false;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let previous = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(previous + 1)
                .min(diagonal + usize::from(ca != cb));
            diagonal = previous;
        }
    }
    row[b.len()] <= allowance
}

#[cfg(test)]
mod tests {
    use super::*;
    use omfp_core::TranscriptEntry;
    use serde_json::json;

    // spec: DMN-073
    #[test]
    fn rolling_cadence_requires_both_conversation_thresholds_and_the_exact_cooldown() {
        let mut policy = RollingTitle::default();
        let now = Instant::now();
        policy.begin(now);
        for _ in 0..3 {
            policy.observe(&"x".repeat(200));
        }
        assert!(!policy.ready(false, now + COOLDOWN));
        policy.observe("fourth message");
        assert!(!policy.ready(false, now + COOLDOWN - Duration::from_nanos(1)));
        assert!(policy.ready(false, now + COOLDOWN));
        policy.begin(now + COOLDOWN);
        for _ in 0..4 {
            policy.observe(&"x".repeat(99));
        }
        assert!(!policy.ready(false, now + COOLDOWN * 2));
        policy.observe("four");
        assert!(policy.ready(false, now + COOLDOWN * 2));
        policy.begin(now + COOLDOWN * 2);
        policy.failed();
        assert!(
            !policy.ready(true, now + COOLDOWN * 3),
            "failed placeholders also need new conversation"
        );
    }

    // spec: DMN-073
    #[test]
    fn rolling_context_keeps_origin_and_recent_both_roles_without_tools_or_thinking() {
        let mut policy = RollingTitle::default();
        policy.observe("Newly admitted deployment task");
        let mut state = ThreadState::default();
        let messages = [
            json!({"role":"user", "content":"Original checkout project"}),
            json!({"role":"user", "synthetic":true, "content":"SYNTHETIC SECRET"}),
            json!({"role":"assistant", "content":[{"type":"thinking","thinking":"THINKING SECRET"},{"type":"text","text":"The checkout API is finished"}]}),
            json!({"role":"toolResult", "toolCallId":"x", "toolName":"read", "content":[{"type":"text","text":"TOOL SECRET"}], "isError":false}),
            json!({"role":"user", "content":"Now configure staging"}),
            json!({"role":"assistant", "content":[{"type":"text","text":"Container deployment is underway"}]}),
        ];
        for message in messages {
            state.transcript.push(TranscriptEntry {
                message_id: None,
                message: serde_json::from_value(message).unwrap(),
            });
        }
        let context = policy.context("Checkout payments", &state);
        for expected in [
            "Original checkout project",
            "The checkout API is finished",
            "Now configure staging",
            "Container deployment is underway",
            "Newly admitted deployment task",
        ] {
            assert!(context.contains(expected), "missing {expected}: {context}");
        }
        for excluded in ["SYNTHETIC SECRET", "THINKING SECRET", "TOOL SECRET"] {
            assert!(!context.contains(excluded));
        }
        // A long history rolls out, but the earlier project anchor survives.
        for index in 0..20 {
            state.transcript.push(TranscriptEntry {message_id:None, message:serde_json::from_value(json!({"role":"user", "content":format!("Recent deployment turn {index}: {}", "界".repeat(2000))})).unwrap()});
            policy.observe(&format!("Admitted turn {index}: {}", "界".repeat(2000)));
        }
        let context = policy.context("Checkout payments", &state);
        assert!(context.contains("Original checkout project"));
        assert!(context.contains("Recent deployment turn 19:"));
        assert!(!context.contains("Recent deployment turn 0:"));
        assert!(
            context.chars().count() < super::super::MESSAGE_CHARS,
            "the command must not truncate recent context"
        );
    }

    // spec: DMN-073
    #[test]
    fn rolling_titles_ignore_cosmetic_variants_but_require_consecutive_topic_confirmation() {
        let mut policy = RollingTitle::default();
        for variant in [
            "CHECKOUT payments!",
            "Payments checkout",
            "Checkout payment",
        ] {
            assert!(!policy.publish("Checkout payments", variant, false));
        }
        assert!(!policy.publish("Checkout payments", "Deployment pipeline", false));
        policy.failed();
        assert!(!policy.publish("Checkout payments", "Deployment pipeline", false));
        assert!(policy.publish("Checkout payments", "Deployment pipelines", false));
        assert!(!policy.publish("Deployment pipelines", "Deployment pipeline", false));
    }
}
