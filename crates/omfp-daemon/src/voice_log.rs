//! The spoken conversation of a live voice session, kept in the thread (DMN-113) and handed to the agent with each
//! request the voice delegates (DMN-116). omp records no transcript of what was said, only the requests the voice
//! delegated, so while a session runs the daemon keeps its transcript in a file of the thread (named to omp by
//! [`TRANSCRIPT_ENV`]) that omfp's omp extension (`omp_extension.ts`) attaches to those requests, and when the session
//! ends it hands its turns to the extension, whose `/omfp-voice-log` command appends them to the session as one
//! `custom` message without a turn: the thread's agent, the chat and the next live session (DMN-105) then know it.

use std::path::Path;

use omfp_core::LiveLine;
use omp_rpc::LiveRole;
use serde_json::json;

use crate::paths::{DataDir, replace_file};

/// The extension's command.
pub const COMMAND: &str = "omfp-voice-log";

/// The `customType` of the message the command appends.
pub const CUSTOM_TYPE: &str = "omfp-voice";

/// The extension module.
const EXTENSION: &str = include_str!("omp_extension.ts");

/// The environment variable naming the thread's live transcript file to omfp's omp extension (DMN-116).
pub const TRANSCRIPT_ENV: &str = "OMFP_LIVE_TRANSCRIPT";

/// About how many characters of a running session's transcript the agent gets with a request (DMN-116).
const TRANSCRIPT_CAP: usize = crate::live_context::CONVERSATION_CAP;

/// Tells the thread's agent what the message is.
const NOTE: &str =
    "(Spoken with the user in live voice and already answered there: context, not a request.)";

/// What was said in one live session: its turns with text, in order, as `(role, text)` with omp's role names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spoken {
    turns: Vec<(&'static str, String)>,
}

impl Spoken {
    /// The turns of a session's `lines` (`live_transcript`, PRT-052), `None` when nobody said anything.
    #[must_use]
    pub fn from_lines(lines: &[LiveLine]) -> Option<Self> {
        let turns: Vec<(&'static str, String)> = lines
            .iter()
            .filter_map(|line| {
                let text = line.text.trim();
                let role = match line.role {
                    LiveRole::User => "user",
                    LiveRole::Assistant => "assistant",
                };
                (!text.is_empty()).then(|| (role, text.to_owned()))
            })
            .collect();
        (!turns.is_empty()).then_some(Self { turns })
    }

    /// One `User:` or `Assistant:` line per turn.
    #[must_use]
    pub fn lines(&self) -> String {
        self.turns
            .iter()
            .map(|(role, text)| format!("{}{text}", label(role)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The `prompt` that records the conversation in the thread through the extension.
    #[must_use]
    pub fn command(&self) -> String {
        let count = self.turns.len();
        let content = format!(
            "Voice conversation · {count} {}\n{NOTE}\n{}",
            if count == 1 { "turn" } else { "turns" },
            self.lines()
        );
        let turns: Vec<_> = self
            .turns
            .iter()
            .map(|(role, text)| json!({ "role": role, "text": text }))
            .collect();
        format!(
            "/{COMMAND} {}",
            json!({ "content": content, "turns": turns })
        )
    }

    /// What the agent gets with each delegated request (DMN-116): one `User:` or `Voice:` line per turn, each within
    /// 2,000 characters, the latest kept within `cap` characters.
    #[must_use]
    pub fn transcript(&self, cap: usize) -> String {
        let turns: Vec<String> = self
            .turns
            .iter()
            .map(|(role, text)| {
                let label = if *role == "user" { "User: " } else { "Voice: " };
                crate::live_context::clip(label, text, crate::live_context::TURN_CAP)
            })
            .collect();
        crate::live_context::recent(&turns, cap).join("\n")
    }
}

/// `User: ` or `Assistant: `.
#[must_use]
pub fn label(role: &str) -> &'static str {
    if role == "user" {
        "User: "
    } else {
        "Assistant: "
    }
}

/// Writes the extension module to `data.omp_extension_module()` unless it already holds this content: returns whether
/// it wrote (DMN-113).
pub fn write_extension(data: &DataDir) -> std::io::Result<bool> {
    let path = data.omp_extension_module();
    if std::fs::read_to_string(&path).is_ok_and(|current| current == EXTENSION) {
        return Ok(false);
    }
    replace_file(&path, EXTENSION.as_bytes(), None)?;
    Ok(true)
}

/// Keeps the file at `path` holding the transcript of a running session's `lines` (DMN-116); without a word said
/// (no session, a new one) it removes the file, so a request never gets an earlier session's words.
pub fn write_transcript(path: &Path, lines: &[LiveLine]) -> std::io::Result<()> {
    match Spoken::from_lines(lines) {
        Some(spoken) => replace_file(path, spoken.transcript(TRANSCRIPT_CAP).as_bytes(), None),
        None => match std::fs::remove_file(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn line(role: LiveRole, text: &str) -> LiveLine {
        LiveLine {
            role,
            turn: 1,
            text: text.into(),
            r#final: true,
        }
    }

    // spec: DMN-113
    #[test]
    fn the_command_carries_the_turns_and_the_message_content() {
        let command = Spoken::from_lines(&[
            line(LiveRole::User, " Ciao, mi senti? "),
            line(LiveRole::Assistant, "Sì."),
            line(LiveRole::User, "  "),
        ])
        .unwrap()
        .command();

        let json = command.strip_prefix("/omfp-voice-log ").unwrap();
        assert!(
            !json.contains('\n'),
            "one line: omp reads the arguments to the end"
        );
        let payload: Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            payload["turns"],
            json!([
                { "role": "user", "text": "Ciao, mi senti?" },
                { "role": "assistant", "text": "Sì." },
            ])
        );
        assert_eq!(
            payload["content"],
            format!("Voice conversation · 2 turns\n{NOTE}\nUser: Ciao, mi senti?\nAssistant: Sì.")
        );
    }

    // spec: DMN-113
    #[test]
    fn a_silent_session_records_nothing() {
        assert_eq!(Spoken::from_lines(&[]), None);
        assert_eq!(Spoken::from_lines(&[line(LiveRole::User, " ")]), None);
        assert!(
            Spoken::from_lines(&[line(LiveRole::Assistant, "Ciao!")])
                .unwrap()
                .command()
                .contains("Voice conversation · 1 turn\\n")
        );
    }

    // spec: DMN-113
    #[test]
    fn the_extension_is_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());

        assert!(write_extension(&data).unwrap());
        assert!(!write_extension(&data).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("omp/index.ts")).unwrap(),
            EXTENSION
        );
    }

    // spec: DMN-116
    #[test]
    fn the_transcript_file_follows_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thread/live-transcript.txt");
        let partial = LiveLine {
            r#final: false,
            turn: 2,
            ..line(LiveRole::User, "e i test")
        };

        write_transcript(
            &path,
            &[
                line(LiveRole::User, "Ciao"),
                line(LiveRole::Assistant, "Dimmi."),
                partial,
            ],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "User: Ciao\nVoice: Dimmi.\nUser: e i test",
            "partial lines too: the last words may still be transcribed"
        );

        write_transcript(&path, &[line(LiveRole::User, " ")]).unwrap();
        assert!(!path.exists());
        write_transcript(&path, &[]).unwrap();
        assert!(!path.exists());
    }

    // spec: DMN-116
    #[test]
    fn the_transcript_keeps_the_latest_turns() {
        let long = "x".repeat(5_000);
        let spoken = Spoken::from_lines(&[
            line(LiveRole::User, "first"),
            line(LiveRole::Assistant, &long),
            line(LiveRole::User, "last"),
        ])
        .unwrap();

        // Room for the last two turns (2,000 + 2 + 10 characters), not for the first.
        let transcript = spoken.transcript(2_020);

        assert!(transcript.ends_with("\nUser: last"), "{transcript}");
        assert!(
            transcript.starts_with("Voice: xxx"),
            "the first turn is left out"
        );
        assert_eq!(transcript.lines().next().unwrap().chars().count(), 2_000);
        assert!(transcript.contains('…'));
    }
}
