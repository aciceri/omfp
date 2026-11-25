//! A thread's omp session read from its file, without omp (DMN-337): when omp cannot start (its saved model cannot be
//! restored, say), the transcript of the session's current branch stays readable and the model it saved is known.
//!
//! omp 19.2.0 writes a session as JSONL (`session/session-manager.ts`): an optional `title` slot, the `session` header
//! (its `id` is the session id), then entries with `id` and `parentId` forming a tree. The current branch runs from the
//! leaf the last entry selects (its `activeLeafId` when it has one, else itself) up to the root, as omp's
//! `resolveSessionLeafId` and `buildSessionContext` read it; its messages are what `get_messages` answers after a resume.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

use omfp_core::SessionInfo;
use omp_rpc::AgentMessage;
use serde_json::{Map, Value, json};

/// What a session file holds for the transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRead {
    /// The session id (its header's `id`, else the one asked for), the file, `resumed`.
    pub session: SessionInfo,
    /// The messages of the current branch, oldest first.
    pub messages: Vec<AgentMessage>,
    /// `provider/id` the session restores (DMN-338): the last `model_change` of role `default` on the branch, else the
    /// last assistant message's `provider/model`; `None` when the branch names no model.
    pub model: Option<String>,
}

/// Custom message types omp keeps out of `get_messages` (`tui/src/chat/messages.ts`).
const HIDDEN_CUSTOM_TYPES: &[&str] = &["prewalk-plan", "vibe-mode-context"];
/// The custom message following the `branch_summary` of a rewind; omp sends the report once, as this message.
const REWIND_REPORT: &str = "rewind-report";

/// The session of a thread from its session directory: the file of `session_id` when given (DMN-213), else the newest
/// one, as `--continue` resumes. `Ok(None)` when there is no such file.
pub fn read(session_dir: &Path, session_id: Option<&str>) -> std::io::Result<Option<SessionRead>> {
    let file = match session_id {
        Some(id) => crate::threads::session_file(session_dir, id),
        None => newest(session_dir),
    };
    let Some(file) = file else {
        return Ok(None);
    };
    let reader = std::io::BufReader::new(std::fs::File::open(&file)?);
    let mut header = None;
    let mut entries = Vec::new();
    for line in reader.lines() {
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("session") => {
                header = entry.get("id").and_then(Value::as_str).map(str::to_owned);
            }
            Some("title") | None => {}
            Some(_) if entry.get("id").and_then(Value::as_str).is_some() => entries.push(entry),
            Some(_) => {}
        }
    }
    let branch = branch(&entries);
    Ok(Some(SessionRead {
        session: SessionInfo {
            session_id: header
                .or_else(|| session_id.map(str::to_owned))
                .unwrap_or_default(),
            session_file: Some(file.to_string_lossy().into_owned()),
            resumed: true,
        },
        messages: messages(&branch),
        model: model(&branch),
    }))
}

/// The newest session file of a directory (names start with their creation time).
fn newest(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl") && path.is_file())
        .max()
}

fn id(entry: &Map<String, Value>) -> Option<&str> {
    entry.get("id").and_then(Value::as_str)
}

/// The leaf replaying `entry` selects: its `activeLeafId` when present (`null`: none), else itself.
fn selected_leaf(entry: &Map<String, Value>) -> Option<&str> {
    match entry.get("activeLeafId") {
        Some(leaf) => leaf.as_str(),
        None => id(entry),
    }
}

/// The current branch, root first: from the leaf the last entry selects up its parents. A selected leaf that no longer
/// exists falls back to the newest entry selecting itself; a parent cycle ends the walk.
fn branch(entries: &[Map<String, Value>]) -> Vec<&Map<String, Value>> {
    let by_id: HashMap<&str, &Map<String, Value>> = entries
        .iter()
        .filter_map(|entry| Some((id(entry)?, entry)))
        .collect();
    let Some(last) = entries.last() else {
        return Vec::new();
    };
    let Some(leaf) = selected_leaf(last) else {
        return Vec::new();
    };
    let leaf = by_id.get(leaf).copied().or_else(|| {
        entries
            .iter()
            .rev()
            .find(|entry| selected_leaf(entry).is_some() && selected_leaf(entry) == id(entry))
    });
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut current = leaf;
    while let Some(entry) = current {
        if !seen.insert(id(entry)) {
            break;
        }
        path.push(entry);
        current = entry
            .get("parentId")
            .and_then(Value::as_str)
            .and_then(|parent| by_id.get(parent).copied());
    }
    path.reverse();
    path
}

fn kind(entry: &Map<String, Value>) -> &str {
    entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The branch's messages as `get_messages` gives them: after the latest `/clear` boundary when it follows the latest
/// compaction; else, with a compaction, its summary, the messages it kept and those after it; else every message.
fn messages(branch: &[&Map<String, Value>]) -> Vec<AgentMessage> {
    let last = |name: &str| branch.iter().rposition(|entry| kind(entry) == name);
    let reset = last("reset_boundary");
    let compaction = last("compaction");
    let range = |range: std::ops::Range<usize>| range.filter_map(|index| message(branch, index));
    match (reset, compaction) {
        (Some(reset), compaction) if compaction.is_none_or(|compaction| reset > compaction) => {
            range(reset + 1..branch.len()).collect()
        }
        (_, Some(compaction)) => {
            let entry = branch[compaction];
            let first_kept = entry.get("firstKeptEntryId").and_then(Value::as_str);
            let kept = branch[..compaction]
                .iter()
                .position(|kept| first_kept.is_some() && id(kept) == first_kept)
                .unwrap_or(compaction);
            AgentMessage::from_value(compaction_summary(entry))
                .ok()
                .into_iter()
                .chain(range(kept..compaction))
                .chain(range(compaction + 1..branch.len()))
                .collect()
        }
        _ => range(0..branch.len()).collect(),
    }
}

/// The message of branch entry `index`, if it shows one: a `message` (not an assistant's retry recovery nor an empty
/// error turn), a `custom_message` (not omp's hidden ones), a `branch_summary` (not a rewind's, whose report follows it).
fn message(branch: &[&Map<String, Value>], index: usize) -> Option<AgentMessage> {
    let entry = branch[index];
    let value = match kind(entry) {
        "message" => {
            let message = entry.get("message")?;
            if message["role"] == "assistant"
                && (message["retryRecovery"] == true || empty_error_turn(message))
            {
                return None;
            }
            message.clone()
        }
        "custom_message" => {
            let custom_type = entry.get("customType").and_then(Value::as_str)?;
            let content = entry
                .get("content")
                .filter(|content| content.is_string() || content.is_array())?;
            if HIDDEN_CUSTOM_TYPES.contains(&custom_type) {
                return None;
            }
            let mut message = json!({
                "role": "custom",
                "customType": custom_type,
                "content": content,
                "display": entry.get("display").and_then(Value::as_bool).unwrap_or(false),
                "timestamp": timestamp(entry),
            });
            for key in ["details", "attribution"] {
                if let Some(value) = entry.get(key) {
                    message[key] = value.clone();
                }
            }
            message
        }
        "branch_summary" => {
            let summary = entry
                .get("summary")
                .and_then(Value::as_str)
                .filter(|summary| !summary.is_empty())?;
            let rewind = branch.get(index + 1).is_some_and(|next| {
                kind(next) == "custom_message"
                    && next.get("customType").and_then(Value::as_str) == Some(REWIND_REPORT)
            });
            if rewind {
                return None;
            }
            json!({
                "role": "branchSummary",
                "summary": summary,
                "fromId": entry.get("fromId").cloned().unwrap_or(Value::Null),
                "timestamp": timestamp(entry),
            })
        }
        _ => return None,
    };
    AgentMessage::from_value(value).ok()
}

/// The summary message a `compaction` entry stands for in the transcript.
fn compaction_summary(entry: &Map<String, Value>) -> Value {
    let mut message = json!({
        "role": "compactionSummary",
        "summary": entry.get("summary").cloned().unwrap_or_else(|| Value::from("")),
        "tokensBefore": entry.get("tokensBefore").cloned().unwrap_or_else(|| Value::from(0)),
        "timestamp": timestamp(entry),
    });
    for key in ["shortSummary", "tokensAfter", "method", "warning"] {
        if let Some(value) = entry.get(key) {
            message[key] = value.clone();
        }
    }
    message
}

/// An entry's ISO time in milliseconds, as omp stamps the messages it rebuilds; `null` when unreadable.
fn timestamp(entry: &Map<String, Value>) -> Value {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .map_or(Value::Null, |time| Value::from(time.as_millisecond()))
}

/// omp's `isEmptyErrorTurn`: an assistant message that stopped on an error without any content worth showing.
fn empty_error_turn(message: &Value) -> bool {
    let blank = |value: &Value| value.as_str().is_none_or(|text| text.trim().is_empty());
    message["stopReason"] == "error"
        && !message["content"].as_array().is_some_and(|blocks| {
            blocks.iter().any(|block| match block["type"].as_str() {
                Some("text") => !blank(&block["text"]),
                Some("thinking") => {
                    !blank(&block["thinking"]) || !blank(&block["thinkingSignature"])
                }
                Some("redactedThinking") => !blank(&block["data"]),
                Some("fallback") => false,
                _ => true,
            })
        })
}

/// The model the branch restores (omp's `buildSessionContext`): the last `model_change` of role `default`; before the
/// first of them, the last assistant message's `provider/model` (sessions older than `model_change`).
fn model(branch: &[&Map<String, Value>]) -> Option<String> {
    let mut model = None;
    let mut explicit = false;
    for entry in branch {
        match kind(entry) {
            "model_change" => {
                let role = entry
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("default");
                if role == "default"
                    && let Some(selected) = entry.get("model").and_then(Value::as_str)
                {
                    model = Some(selected.to_owned());
                    explicit = true;
                }
            }
            "message" if !explicit => {
                let Some(message) = entry.get("message") else {
                    continue;
                };
                if message["role"] == "assistant"
                    && message["model"] != "<synthetic>"
                    && let (Some(provider), Some(id)) =
                        (message["provider"].as_str(), message["model"].as_str())
                {
                    model = Some(format!("{provider}/{id}"));
                }
            }
            _ => {}
        }
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "01a1ccee-5f2d-728a-a238-e1916209ba9e";

    fn user(id: &str, parent: Option<&str>, text: &str) -> Value {
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": "2026-11-11T23:25:36.751Z",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 1 }
        })
    }

    fn assistant(id: &str, parent: &str, text: &str, model: &str) -> Value {
        let (provider, model) = model.split_once('/').unwrap();
        json!({
            "type": "message", "id": id, "parentId": parent, "timestamp": "2026-11-11T23:25:37.717Z",
            "message": {
                "role": "assistant", "content": [{ "type": "text", "text": text }], "api": "openai-completions",
                "provider": provider, "model": model, "stopReason": "stop", "timestamp": 2,
                "usage": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                           "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 } }
            }
        })
    }

    /// A session file named as omp names it, with the title slot and the header first.
    fn session(dir: &Path, name: &str, entries: &[Value]) -> PathBuf {
        let mut lines = vec![
            json!({ "type": "title", "v": 1, "title": "t", "pad": "  " }),
            json!({ "type": "session", "version": 3, "id": SESSION, "timestamp": "2026-11-11T23:25:36.429Z", "cwd": "/ws" }),
        ];
        lines.extend_from_slice(entries);
        let text: Vec<String> = lines.iter().map(Value::to_string).collect();
        let path = dir.join(format!("{name}_{SESSION}.jsonl"));
        std::fs::write(&path, text.join("\n") + "\n").unwrap();
        path
    }

    /// Each message as `role: text` (its first text block, or its summary).
    fn shown(read: &SessionRead) -> Vec<String> {
        read.messages
            .iter()
            .map(|message| {
                let value = serde_json::to_value(message).unwrap();
                let text = value["content"][0]["text"]
                    .as_str()
                    .or_else(|| value["content"].as_str())
                    .or_else(|| value["summary"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                format!("{}: {text}", value["role"].as_str().unwrap())
            })
            .collect()
    }

    // spec: DMN-337, DMN-338
    #[test]
    fn the_session_file_gives_the_current_branch_and_the_model_it_restores() {
        let dir = tempfile::tempdir().unwrap();
        let file = session(
            dir.path(),
            "2026-11-11T23-25-36-429Z",
            &[
                json!({ "type": "model_change", "id": "m1", "parentId": null, "timestamp": "2026-11-11T23:25:36.489Z",
                        "model": "anthropic/claude-opus-5-5" }),
                user("u1", Some("m1"), "first question"),
                assistant("a1", "u1", "first answer", "anthropic/claude-opus-5-5"),
                user("u2", Some("a1"), "abandoned branch"),
                assistant("a2", "u2", "abandoned answer", "anthropic/claude-opus-5-5"),
                // `navigate_tree` back to `a1`: the next entries hang there; a usage marker keeps the leaf.
                json!({ "type": "branch_summary", "id": "b1", "parentId": "a1", "timestamp": "2026-11-11T23:26:00.000Z",
                        "summary": "Tried another idea", "fromId": "a2" }),
                user("u3", Some("b1"), "second question"),
                json!({ "type": "model_change", "id": "m2", "parentId": "u3", "timestamp": "2026-11-11T23:26:01.000Z",
                        "model": "openai/gpt-6", "role": "smol" }),
                assistant("a3", "m2", "second answer", "openai/gpt-6"),
                json!({ "type": "custom_message", "id": "c1", "parentId": "a3", "timestamp": "2026-11-11T23:26:02.000Z",
                        "customType": "note", "content": "a note", "display": true }),
                json!({ "type": "custom", "customType": "session_exit", "id": "x1", "parentId": "c1",
                        "timestamp": "2026-11-11T23:27:00.000Z", "data": {} }),
                json!({ "type": "model_usage", "id": "mu1", "parentId": "x1", "activeLeafId": "x1",
                        "timestamp": "2026-11-11T23:27:01.000Z" }),
            ],
        );

        let read = read(dir.path(), Some(SESSION)).unwrap().unwrap();

        assert_eq!(
            shown(&read),
            [
                "user: first question",
                "assistant: first answer",
                "branchSummary: Tried another idea",
                "user: second question",
                "assistant: second answer",
                "custom: a note",
            ]
        );
        assert_eq!(
            read.session,
            SessionInfo {
                session_id: SESSION.to_owned(),
                session_file: Some(file.to_string_lossy().into_owned()),
                resumed: true,
            }
        );
        // A `model_change` of another role and later assistant messages never replace the explicit default.
        assert_eq!(read.model.as_deref(), Some("anthropic/claude-opus-5-5"));
        let custom = serde_json::to_value(&read.messages[5]).unwrap();
        assert_eq!(custom["timestamp"].to_string(), "1794439562000.0");
    }

    // spec: DMN-337
    #[test]
    fn a_compaction_shows_its_summary_the_kept_messages_and_those_after_it() {
        let dir = tempfile::tempdir().unwrap();
        session(
            dir.path(),
            "2026-11-11T23-25-36-429Z",
            &[
                user("u1", None, "old question"),
                assistant("a1", "u1", "old answer", "fake/scripted"),
                user("u2", Some("a1"), "kept question"),
                assistant("a2", "u2", "kept answer", "fake/scripted"),
                json!({ "type": "compaction", "id": "k1", "parentId": "a2", "timestamp": "2026-11-11T23:30:00.000Z",
                        "summary": "We talked", "firstKeptEntryId": "u2", "tokensBefore": 1200 }),
                user("u3", Some("k1"), "new question"),
                json!({ "type": "message", "id": "a3", "parentId": "u3", "timestamp": "2026-11-11T23:31:00.000Z",
                        "message": { "role": "assistant", "content": [], "provider": "fake", "model": "scripted",
                                     "stopReason": "error", "errorMessage": "boom", "timestamp": 3 } }),
            ],
        );

        let read = read(dir.path(), None).unwrap().unwrap();

        assert_eq!(
            shown(&read),
            [
                "compactionSummary: We talked",
                "user: kept question",
                "assistant: kept answer",
                "user: new question",
            ],
            "an empty error turn is left out, as omp does"
        );
        // Without `model_change`, the last assistant message names the model.
        assert_eq!(read.model.as_deref(), Some("fake/scripted"));
    }

    // spec: DMN-337
    #[test]
    fn a_clear_boundary_starts_the_transcript_and_a_missing_file_reads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path(), None).unwrap(), None);
        assert_eq!(read(dir.path(), Some(SESSION)).unwrap(), None);
        session(
            dir.path(),
            "2026-11-11T23-25-36-429Z",
            &[
                user("u1", None, "before clear"),
                json!({ "type": "reset_boundary", "id": "r1", "parentId": "u1", "timestamp": "2026-11-11T23:30:00.000Z" }),
                user("u2", Some("r1"), "after clear"),
            ],
        );

        let read = read(dir.path(), Some(SESSION)).unwrap().unwrap();

        assert_eq!(shown(&read), ["user: after clear"]);
        assert_eq!(read.model, None);
    }
}
