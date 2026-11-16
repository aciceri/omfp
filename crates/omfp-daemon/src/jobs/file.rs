//! A workspace's declared jobs, `<workspace>/.omfp/jobs.json` (DMN-320, DMN-321): read and checked with the rules of
//! [`SCHEMA`] (errors as `<JSON path>: <reason>`), and changed one job at a time by `jobs.define`, `jobs.remove` and
//! `jobs.enable` (DMN-332), written like `views.json` (DMN-083).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use omfp_core::wire::ThinkingLevel;
use omfp_protocol::{
    ApprovalMode, JobDefinition, JobPrompt, JobTrigger, MissedPolicy, PromptThread, RestartPolicy,
};
use serde::Deserialize as _;
use serde_json::{Map, Value};

use super::cron::Schedule;
use crate::widgets::manifest;

/// The JSON Schema of the jobs file, served as [`SCHEMA_URL`] (DMN-331).
pub const SCHEMA: &str = include_str!("../widgets/jobs.schema.json");
/// Host URI of [`SCHEMA`] (DMN-331).
pub const SCHEMA_URL: &str = "omfp://schemas/jobs.json";
/// The longest job name, in characters (DMN-320).
pub const NAME_MAX: usize = 64;
/// The longest description, in characters (DMN-320).
pub const DESCRIPTION_MAX: usize = 200;

/// The keys of a job (DMN-320).
const JOB_KEYS: [&str; 9] = [
    "description",
    "trigger",
    "schedule",
    "missed",
    "command",
    "prompt",
    "cwd",
    "restart",
    "enabled",
];
/// The keys of a job's prompt (DMN-320).
const PROMPT_KEYS: [&str; 5] = ["text", "thread", "model", "thinkingLevel", "approvalMode"];

/// `<workspace>/.omfp/jobs.json` (DMN-320).
#[must_use]
pub fn path(workspace: &Path) -> PathBuf {
    workspace.join(".omfp").join("jobs.json")
}

/// The jobs file at `path` (DMN-321): `Ok(None)` without a file, its jobs by name when valid, else why it is refused
/// (unreadable, not JSON, or the first rule it breaks).
pub fn read(path: &Path) -> Result<Option<BTreeMap<String, JobDefinition>>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read .omfp/jobs.json: {error}")),
    };
    parse_jobs_file(&bytes).map(Some)
}

/// The jobs of a jobs file's bytes by name, defaults filled in, or why it is refused: not JSON, or the first rule of
/// DMN-320 it breaks as `<JSON path>: <reason>` (DMN-321).
pub fn parse_jobs_file(bytes: &[u8]) -> Result<BTreeMap<String, JobDefinition>, String> {
    let file: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!(".omfp/jobs.json is not JSON: {error}"))?;
    check(&file)
}

/// The jobs of a parsed jobs file by name, or the first rule of DMN-320 it breaks as `<JSON path>: <reason>`.
pub fn check(file: &Value) -> Result<BTreeMap<String, JobDefinition>, String> {
    let not_jobs = || "jobs: expected an object of jobs by name".to_owned();
    let file = file.as_object().ok_or_else(not_jobs)?;
    for (key, value) in file {
        match key.as_str() {
            "jobs" => {}
            "$schema" if value.is_string() => {}
            "$schema" => return Err("$schema: expected a string".to_owned()),
            _ => return Err(format!("{key}: unknown key")),
        }
    }
    let jobs = file
        .get("jobs")
        .and_then(Value::as_object)
        .ok_or_else(not_jobs)?;
    jobs.iter()
        .map(|(name, value)| {
            let path = format!("jobs.{name}");
            check_name(name).map_err(|reason| format!("{path}: {reason}"))?;
            Ok((name.clone(), job(value, &path)?))
        })
        .collect()
}

/// The job `name` would declare with the definition `job` (`jobs.define`, PRT-205), defaults filled in, or the first
/// rule of DMN-320 it breaks: `name: <reason>` for the name, `job.<path>: <reason>` for the definition.
pub fn validate_job(name: &str, job_value: &Value) -> Result<JobDefinition, String> {
    check_name(name).map_err(|reason| format!("name: {reason}"))?;
    job(job_value, "job")
}

/// Why `name` is no job name (DMN-320): 1–64 characters of `a-z`, `0-9`, `.`, `_`, `-`, starting with a letter or
/// digit.
pub fn check_name(name: &str) -> Result<(), String> {
    let first = name.chars().next();
    let valid = first.is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && name.len() <= NAME_MAX
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "expected 1-{NAME_MAX} characters of a-z, 0-9, \".\", \"_\" and \"-\", starting with a letter or digit"
        ))
    }
}

/// The folder a command run of the workspace `folder` runs in (DMN-320, DMN-327): `folder` joined with `cwd`, `..`
/// resolved without following links; `None` when `cwd` is absolute, empty or leaves the folder (a checked job's never
/// does).
#[must_use]
pub fn cwd_folder(folder: &Path, cwd: Option<&str>) -> Option<PathBuf> {
    match cwd {
        None => Some(folder.to_owned()),
        Some(cwd) => relative_folder(cwd)
            .ok()
            .map(|relative| folder.join(relative)),
    }
}

/// Why a `cwd` is refused, `<reason>` of `<path>.cwd: <reason>`.
enum CwdError {
    NotRelative,
    Leaves,
}

/// `cwd` with `.` dropped and `..` applied to the components before it, links not followed (DMN-089).
fn relative_folder(cwd: &str) -> Result<PathBuf, CwdError> {
    let requested = Path::new(cwd);
    if cwd.is_empty() || requested.is_absolute() {
        return Err(CwdError::NotRelative);
    }
    let mut relative = PathBuf::new();
    for component in requested.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !relative.pop() {
                    return Err(CwdError::Leaves);
                }
            }
            Component::Normal(name) => relative.push(name),
            Component::Prefix(_) | Component::RootDir => return Err(CwdError::NotRelative),
        }
    }
    Ok(relative)
}

fn unknown_keys(object: &Map<String, Value>, known: &[&str], path: &str) -> Result<(), String> {
    match object.keys().find(|key| !known.contains(&key.as_str())) {
        Some(key) => Err(format!("{path}.{key}: unknown key")),
        None => Ok(()),
    }
}

/// The job at `path` with its defaults, or the first rule of DMN-320 it breaks.
#[allow(clippy::too_many_lines)] // One rule after the other, in DMN-320's order.
fn job(value: &Value, path: &str) -> Result<JobDefinition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path}: expected an object with command or prompt"))?;
    unknown_keys(object, &JOB_KEYS, path)?;
    let description = match object.get("description") {
        None => None,
        Some(Value::String(text)) if text.chars().count() <= DESCRIPTION_MAX => Some(text.clone()),
        Some(_) => {
            return Err(format!(
                "{path}.description: expected a string of at most {DESCRIPTION_MAX} characters"
            ));
        }
    };
    let trigger = match object.get("trigger").map(Value::as_str) {
        None | Some(Some("manual")) => JobTrigger::Manual,
        Some(Some("schedule")) => JobTrigger::Schedule,
        Some(Some("service")) => JobTrigger::Service,
        Some(_) => {
            return Err(format!(
                "{path}.trigger: expected manual, schedule or service"
            ));
        }
    };
    let only_schedule = |key: &str| format!("{path}.{key}: only with trigger \"schedule\"");
    let schedule = match (trigger, object.get("schedule")) {
        (JobTrigger::Schedule, None) => {
            return Err(format!(
                "{path}.schedule: required with trigger \"schedule\""
            ));
        }
        (JobTrigger::Schedule, Some(Value::String(expression))) => {
            Schedule::parse(expression).map_err(|error| format!("{path}.schedule: {error}"))?;
            Some(expression.clone())
        }
        (JobTrigger::Schedule, Some(_)) => {
            return Err(format!(
                "{path}.schedule: expected a cron expression such as \"0 8 * * 1-5\""
            ));
        }
        (_, Some(_)) => return Err(only_schedule("schedule")),
        (_, None) => None,
    };
    let missed = match object.get("missed") {
        None => MissedPolicy::Skip,
        Some(_) if trigger != JobTrigger::Schedule => return Err(only_schedule("missed")),
        Some(value) => match value.as_str() {
            Some("skip") => MissedPolicy::Skip,
            Some("run") => MissedPolicy::Run,
            _ => return Err(format!("{path}.missed: expected skip or run")),
        },
    };
    let command = match object.get("command") {
        None => None,
        Some(Value::String(command)) if !command.trim().is_empty() => Some(command.clone()),
        Some(_) => {
            return Err(format!(
                "{path}.command: expected a non-blank shell command"
            ));
        }
    };
    let prompt = object
        .get("prompt")
        .map(|value| prompt(value, &format!("{path}.prompt")))
        .transpose()?;
    if command.is_some() == prompt.is_some() {
        return Err(format!(
            "{path}: expected exactly one of command and prompt"
        ));
    }
    if trigger == JobTrigger::Service && command.is_none() {
        return Err(format!("{path}: a service needs command"));
    }
    let cwd = match object.get("cwd") {
        None => None,
        Some(_) if command.is_none() => return Err(format!("{path}.cwd: only with command")),
        Some(value) => {
            let cwd = value.as_str().unwrap_or_default();
            match relative_folder(cwd) {
                Ok(_) => Some(cwd.to_owned()),
                Err(CwdError::NotRelative) => {
                    return Err(format!(
                        "{path}.cwd: expected a folder relative to the workspace folder"
                    ));
                }
                Err(CwdError::Leaves) => {
                    return Err(format!("{path}.cwd: leaves the workspace folder"));
                }
            }
        }
    };
    let restart = match object.get("restart") {
        None => RestartPolicy::Always,
        Some(_) if trigger != JobTrigger::Service => {
            return Err(format!("{path}.restart: only with trigger \"service\""));
        }
        Some(value) => match value.as_str() {
            Some("always") => RestartPolicy::Always,
            Some("on-failure") => RestartPolicy::OnFailure,
            _ => return Err(format!("{path}.restart: expected always or on-failure")),
        },
    };
    let enabled = match object.get("enabled") {
        None => true,
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => return Err(format!("{path}.enabled: expected true or false")),
    };
    Ok(JobDefinition {
        description,
        trigger,
        schedule,
        missed,
        command,
        prompt,
        cwd,
        restart,
        enabled,
    })
}

/// The prompt at `path` with its defaults, or the first rule of DMN-320 it breaks.
fn prompt(value: &Value, path: &str) -> Result<JobPrompt, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path}: expected an object with text"))?;
    unknown_keys(object, &PROMPT_KEYS, path)?;
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("{path}.text: expected non-blank text"))?;
    let thread = match object.get("thread").map(Value::as_str) {
        None | Some(Some("new")) => PromptThread::New,
        Some(Some("reuse")) => PromptThread::Reuse,
        Some(_) => return Err(format!("{path}.thread: expected new or reuse")),
    };
    let model = match object.get("model") {
        None => None,
        Some(Value::String(model))
            if model
                .split_once('/')
                .is_some_and(|(provider, id)| !provider.is_empty() && !id.is_empty()) =>
        {
            Some(model.clone())
        }
        Some(_) => return Err(format!("{path}.model: expected provider/id")),
    };
    let thinking_level = match object.get("thinkingLevel") {
        None => None,
        Some(value) => match (value.as_str(), ThinkingLevel::deserialize(value)) {
            (Some(level), Ok(_)) => Some(level.to_owned()),
            _ => {
                return Err(format!(
                    "{path}.thinkingLevel: expected inherit, off, minimal, low, medium, high, xhigh or max"
                ));
            }
        },
    };
    let approval_mode = match object.get("approvalMode") {
        None => None,
        Some(value) => match ApprovalMode::deserialize(value) {
            Ok(mode) => Some(mode.as_str().to_owned()),
            Err(_) => {
                return Err(format!(
                    "{path}.approvalMode: expected always-ask, always-ask-cached, write or yolo"
                ));
            }
        },
    };
    Ok(JobPrompt {
        text: text.to_owned(),
        thread,
        model,
        thinking_level,
        approval_mode,
    })
}

/// Why the jobs file cannot be changed (DMN-332).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The current file is refused (DMN-321): it is never overwritten.
    #[error("cannot change .omfp/jobs.json while it is invalid: {0}")]
    InvalidFile(String),
    /// The new job breaks a rule of DMN-320 (`name: …` or `job.<path>: …`).
    #[error("{0}")]
    InvalidJob(String),
    /// The file declares no job of that name.
    #[error("no job {0} in .omfp/jobs.json")]
    NotFound(String),
}

/// The jobs file `current` (`None`: no file yet, `{ "jobs": {} }`) as a JSON object, refused when invalid.
fn current_file(current: Option<&[u8]>) -> Result<Map<String, Value>, WriteError> {
    let Some(bytes) = current else {
        let mut file = Map::new();
        file.insert("jobs".to_owned(), Value::Object(Map::new()));
        return Ok(file);
    };
    parse_jobs_file(bytes).map_err(WriteError::InvalidFile)?;
    match serde_json::from_slice(bytes) {
        Ok(Value::Object(file)) => Ok(file),
        _ => Err(WriteError::InvalidFile(
            "jobs: expected an object of jobs by name".to_owned(),
        )),
    }
}

/// The `jobs` object of a checked file.
fn jobs_of(file: &mut Map<String, Value>) -> Result<&mut Map<String, Value>, WriteError> {
    match file.get_mut("jobs") {
        Some(Value::Object(jobs)) => Ok(jobs),
        _ => Err(WriteError::InvalidFile(
            "jobs: expected an object of jobs by name".to_owned(),
        )),
    }
}

/// The changed file checked again (DMN-321) and written as `views.set` writes `views.json` (DMN-083): two-space
/// indentation, keys sorted at every level, a final newline.
fn finish(file: Map<String, Value>) -> Result<Vec<u8>, WriteError> {
    let file = Value::Object(file);
    check(&file).map_err(WriteError::InvalidJob)?;
    Ok(manifest::to_text(&file).into_bytes())
}

/// The jobs file `current` (`None`: none yet) with job `name` set to the object `job` as given, `enabled: true`
/// dropped (the default), or removed when `job` is `None` (a last job leaves `{ "jobs": {} }`), as the bytes to
/// write atomically (DMN-332). An invalid current file or job is refused; removing a job the file does not declare
/// is [`WriteError::NotFound`]. Whether an existing job may be replaced is the caller's call (PRT-205).
pub fn write_job(
    current: Option<&[u8]>,
    name: &str,
    job: Option<&Value>,
) -> Result<Vec<u8>, WriteError> {
    let mut file = current_file(current)?;
    let jobs = jobs_of(&mut file)?;
    match job {
        Some(job) => {
            validate_job(name, job).map_err(WriteError::InvalidJob)?;
            let mut job = job.clone();
            if let Some(object) = job.as_object_mut()
                && object.get("enabled") == Some(&Value::Bool(true))
            {
                object.remove("enabled");
            }
            jobs.insert(name.to_owned(), job);
        }
        None => {
            if jobs.remove(name).is_none() {
                return Err(WriteError::NotFound(name.to_owned()));
            }
        }
    }
    finish(file)
}

/// The jobs file `current` with job `name`'s `enabled` set (DMN-332, PRT-207): `true` removes the key (the default),
/// `false` writes it.
pub fn set_enabled(
    current: Option<&[u8]>,
    name: &str,
    enabled: bool,
) -> Result<Vec<u8>, WriteError> {
    let mut file = current_file(current)?;
    let Some(Value::Object(job)) = jobs_of(&mut file)?.get_mut(name) else {
        return Err(WriteError::NotFound(name.to_owned()));
    };
    if enabled {
        job.remove("enabled");
    } else {
        job.insert("enabled".to_owned(), Value::Bool(false));
    }
    finish(file)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The example of 02-daemon "Declared jobs (M18)".
    const SPEC_EXAMPLE: &str = r#"{
  "$schema": "omfp://schemas/jobs.json",
  "jobs": {
    "test": { "description": "Fast tests", "command": "nix develop -c just test", "cwd": "omfp" },
    "mail": {
      "trigger": "schedule",
      "schedule": "0 8 * * 1-5",
      "prompt": { "text": "Read my unread mail and list what needs an answer.", "thread": "reuse" }
    },
    "inbox-watch": { "trigger": "service", "command": "./scripts/watch-inbox.sh", "restart": "on-failure" }
  }
}"#;

    fn command_job(command: &str) -> JobDefinition {
        JobDefinition {
            description: None,
            trigger: JobTrigger::Manual,
            schedule: None,
            missed: MissedPolicy::Skip,
            command: Some(command.to_owned()),
            prompt: None,
            cwd: None,
            restart: RestartPolicy::Always,
            enabled: true,
        }
    }

    fn file_error(file: &Value) -> String {
        check(file).unwrap_err()
    }

    // spec: DMN-320, DMN-331
    #[test]
    fn the_spec_example_is_valid_with_its_defaults_filled_in() {
        let jobs = parse_jobs_file(SPEC_EXAMPLE.as_bytes()).unwrap();
        assert_eq!(
            jobs.keys().collect::<Vec<_>>(),
            ["inbox-watch", "mail", "test"]
        );
        assert_eq!(
            jobs["test"],
            JobDefinition {
                description: Some("Fast tests".to_owned()),
                cwd: Some("omfp".to_owned()),
                ..command_job("nix develop -c just test")
            }
        );
        assert_eq!(
            jobs["mail"],
            JobDefinition {
                trigger: JobTrigger::Schedule,
                schedule: Some("0 8 * * 1-5".to_owned()),
                command: None,
                prompt: Some(JobPrompt {
                    text: "Read my unread mail and list what needs an answer.".to_owned(),
                    thread: PromptThread::Reuse,
                    model: None,
                    thinking_level: None,
                    approval_mode: None,
                }),
                ..command_job("")
            }
        );
        assert_eq!(
            jobs["inbox-watch"],
            JobDefinition {
                trigger: JobTrigger::Service,
                restart: RestartPolicy::OnFailure,
                ..command_job("./scripts/watch-inbox.sh")
            }
        );
    }

    // spec: DMN-320
    #[test]
    fn every_key_is_read_and_absent_ones_take_their_defaults() {
        let jobs = check(&json!({
            "jobs": {
                "a": { "command": "true" },
                "0.b_c-d": {
                    "description": "x".repeat(DESCRIPTION_MAX),
                    "trigger": "schedule",
                    "schedule": "@daily",
                    "missed": "run",
                    "prompt": {
                        "text": "hi",
                        "thread": "new",
                        "model": "anthropic/claude-sonnet-4",
                        "thinkingLevel": "high",
                        "approvalMode": "always-ask-cached"
                    },
                    "enabled": false
                },
                "svc": { "trigger": "service", "command": "serve", "cwd": "./a/../b", "restart": "always" },
                "m": { "trigger": "manual", "prompt": { "text": "go" }, "enabled": true }
            }
        }))
        .unwrap();
        assert_eq!(jobs["a"], command_job("true"));
        assert_eq!(
            jobs["0.b_c-d"],
            JobDefinition {
                description: Some("x".repeat(DESCRIPTION_MAX)),
                trigger: JobTrigger::Schedule,
                schedule: Some("@daily".to_owned()),
                missed: MissedPolicy::Run,
                command: None,
                prompt: Some(JobPrompt {
                    text: "hi".to_owned(),
                    thread: PromptThread::New,
                    model: Some("anthropic/claude-sonnet-4".to_owned()),
                    thinking_level: Some("high".to_owned()),
                    approval_mode: Some("always-ask-cached".to_owned()),
                }),
                enabled: false,
                ..command_job("")
            }
        );
        assert_eq!(
            jobs["svc"],
            JobDefinition {
                trigger: JobTrigger::Service,
                cwd: Some("./a/../b".to_owned()),
                ..command_job("serve")
            }
        );
        assert_eq!(jobs["m"].prompt.as_ref().unwrap().thread, PromptThread::New);
        assert_eq!(check(&json!({ "jobs": {} })), Ok(BTreeMap::new()));
    }

    // spec: DMN-320, DMN-321
    #[test]
    #[allow(clippy::too_many_lines)] // One row per rule.
    fn an_invalid_file_names_the_first_rule_it_breaks() {
        let long_name = "a".repeat(NAME_MAX + 1);
        let name_rule = "expected 1-64 characters of a-z, 0-9, \".\", \"_\" and \"-\", starting with a letter or \
                         digit";
        let cases = [
            (json!([]), "jobs: expected an object of jobs by name".to_owned()),
            (json!({}), "jobs: expected an object of jobs by name".to_owned()),
            (json!({ "jobs": [] }), "jobs: expected an object of jobs by name".to_owned()),
            (json!({ "jobs": {}, "job": {} }), "job: unknown key".to_owned()),
            (json!({ "$schema": 1, "jobs": {} }), "$schema: expected a string".to_owned()),
            (json!({ "jobs": { "Mail": { "command": "x" } } }), format!("jobs.Mail: {name_rule}")),
            (json!({ "jobs": { "-x": { "command": "x" } } }), format!("jobs.-x: {name_rule}")),
            (json!({ "jobs": { "": { "command": "x" } } }), format!("jobs.: {name_rule}")),
            (json!({ "jobs": { "a b": { "command": "x" } } }), format!("jobs.a b: {name_rule}")),
            (
                json!({ "jobs": Map::from_iter([(long_name.clone(), json!({ "command": "x" }))]) }),
                format!("jobs.{long_name}: {name_rule}"),
            ),
            (json!({ "jobs": { "x": "true" } }), "jobs.x: expected an object with command or prompt".to_owned()),
            (json!({ "jobs": { "x": { "command": "x", "foo": 1 } } }), "jobs.x.foo: unknown key".to_owned()),
            (
                json!({ "jobs": { "x": { "command": "x", "description": "d".repeat(DESCRIPTION_MAX + 1) } } }),
                "jobs.x.description: expected a string of at most 200 characters".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "trigger": "cron" } } }),
                "jobs.x.trigger: expected manual, schedule or service".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "trigger": "schedule" } } }),
                "jobs.x.schedule: required with trigger \"schedule\"".to_owned(),
            ),
            (
                json!({ "jobs": { "mail": { "command": "x", "trigger": "schedule", "schedule": "0 25 * * *" } } }),
                "jobs.mail.schedule: hour: 25 is out of range 0-23".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "trigger": "schedule", "schedule": 5 } } }),
                "jobs.x.schedule: expected a cron expression such as \"0 8 * * 1-5\"".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "schedule": "@daily" } } }),
                "jobs.x.schedule: only with trigger \"schedule\"".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "trigger": "service", "missed": "run" } } }),
                "jobs.x.missed: only with trigger \"schedule\"".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "trigger": "schedule", "schedule": "@daily", "missed": "all" } } }),
                "jobs.x.missed: expected skip or run".to_owned(),
            ),
            (json!({ "jobs": { "x": { "command": " " } } }), "jobs.x.command: expected a non-blank shell command".to_owned()),
            (json!({ "jobs": { "x": {} } }), "jobs.x: expected exactly one of command and prompt".to_owned()),
            (
                json!({ "jobs": { "x": { "command": "x", "prompt": { "text": "y" } } } }),
                "jobs.x: expected exactly one of command and prompt".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "trigger": "service", "prompt": { "text": "y" } } } }),
                "jobs.x: a service needs command".to_owned(),
            ),
            (json!({ "jobs": { "x": { "prompt": "y" } } }), "jobs.x.prompt: expected an object with text".to_owned()),
            (
                json!({ "jobs": { "x": { "prompt": { "text": "y", "threadId": "z" } } } }),
                "jobs.x.prompt.threadId: unknown key".to_owned(),
            ),
            (json!({ "jobs": { "x": { "prompt": { "text": "\n" } } } }), "jobs.x.prompt.text: expected non-blank text".to_owned()),
            (
                json!({ "jobs": { "x": { "prompt": { "text": "y", "thread": "same" } } } }),
                "jobs.x.prompt.thread: expected new or reuse".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "prompt": { "text": "y", "model": "sonnet" } } } }),
                "jobs.x.prompt.model: expected provider/id".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "prompt": { "text": "y", "thinkingLevel": "huge" } } } }),
                "jobs.x.prompt.thinkingLevel: expected inherit, off, minimal, low, medium, high, xhigh or max".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "prompt": { "text": "y", "approvalMode": "ask" } } } }),
                "jobs.x.prompt.approvalMode: expected always-ask, always-ask-cached, write or yolo".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "prompt": { "text": "y" }, "cwd": "a" } } }),
                "jobs.x.cwd: only with command".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "cwd": "/tmp" } } }),
                "jobs.x.cwd: expected a folder relative to the workspace folder".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "cwd": "" } } }),
                "jobs.x.cwd: expected a folder relative to the workspace folder".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "cwd": 3 } } }),
                "jobs.x.cwd: expected a folder relative to the workspace folder".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "cwd": ".." } } }),
                "jobs.x.cwd: leaves the workspace folder".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "cwd": "a/../../b" } } }),
                "jobs.x.cwd: leaves the workspace folder".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "restart": "always" } } }),
                "jobs.x.restart: only with trigger \"service\"".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "trigger": "service", "restart": "never" } } }),
                "jobs.x.restart: expected always or on-failure".to_owned(),
            ),
            (
                json!({ "jobs": { "x": { "command": "x", "enabled": "yes" } } }),
                "jobs.x.enabled: expected true or false".to_owned(),
            ),
        ];
        for (file, message) in cases {
            assert_eq!(file_error(&file), message, "{file}");
        }
    }

    // spec: DMN-321
    #[test]
    fn a_file_that_cannot_be_read_or_is_not_json_says_so() {
        let error = parse_jobs_file(b"{ \"jobs\": ").unwrap_err();
        assert!(
            error.starts_with(".omfp/jobs.json is not JSON: "),
            "{error}"
        );
        let error = parse_jobs_file(b"\xff").unwrap_err();
        assert!(
            error.starts_with(".omfp/jobs.json is not JSON: "),
            "{error}"
        );

        let workspace = tempfile::tempdir().unwrap();
        let file = path(workspace.path());
        assert_eq!(file, workspace.path().join(".omfp/jobs.json"));
        assert_eq!(read(&file), Ok(None), "no file, no jobs");
        std::fs::create_dir_all(&file).unwrap();
        let error = read(&file).unwrap_err();
        assert!(
            error.starts_with("cannot read .omfp/jobs.json: "),
            "{error}"
        );
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, SPEC_EXAMPLE).unwrap();
        assert_eq!(read(&file).unwrap().map(|jobs| jobs.len()), Some(3));
    }

    // spec: DMN-320
    #[test]
    fn a_defined_job_is_checked_with_paths_relative_to_it() {
        assert_eq!(
            validate_job("mail", &json!({ "command": "x" })),
            Ok(command_job("x"))
        );
        assert_eq!(
            validate_job(
                "mail",
                &json!({ "trigger": "schedule", "schedule": "* * *", "command": "x" })
            ),
            Err(
                "job.schedule: expected 5 fields (minute hour day-of-month month day-of-week) or a macro such as \
                 @daily, found 3"
                    .to_owned()
            )
        );
        assert_eq!(
            validate_job("mail", &json!({ "command": "x", "foo": true })),
            Err("job.foo: unknown key".to_owned())
        );
        assert_eq!(
            validate_job("mail", &json!(null)),
            Err("job: expected an object with command or prompt".to_owned())
        );
        assert_eq!(
            validate_job("Mail!", &json!({ "command": "x" })),
            Err(
                "name: expected 1-64 characters of a-z, 0-9, \".\", \"_\" and \"-\", starting with a letter or digit"
                    .to_owned()
            )
        );
    }

    // spec: DMN-320
    #[test]
    fn a_cwd_resolves_inside_the_workspace_folder_without_following_links() {
        let folder = Path::new("/ws/checkout");
        assert_eq!(cwd_folder(folder, None), Some(folder.to_owned()));
        assert_eq!(cwd_folder(folder, Some("omfp")), Some(folder.join("omfp")));
        assert_eq!(
            cwd_folder(folder, Some("./a/../b/")),
            Some(folder.join("b"))
        );
        assert_eq!(cwd_folder(folder, Some(".")), Some(folder.to_owned()));
        assert_eq!(cwd_folder(folder, Some("a/../..")), None);
        assert_eq!(cwd_folder(folder, Some("/etc")), None);
    }

    /// The text `write_job` and `set_enabled` give for `file`.
    fn text(file: &Value) -> String {
        manifest::to_text(file)
    }

    fn written(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    // spec: DMN-332
    #[test]
    fn writing_a_job_changes_only_that_job_with_sorted_keys_and_a_final_newline() {
        let bytes = write_job(
            None,
            "test",
            Some(&json!({ "command": "just test", "enabled": true, "cwd": "omfp" })),
        )
        .unwrap();
        assert_eq!(
            written(&bytes),
            "{\n  \"jobs\": {\n    \"test\": {\n      \"command\": \"just test\",\n      \"cwd\": \"omfp\"\n    }\n  }\n}\n",
            "no file: {{ \"jobs\": {{}} }} plus the job, `enabled: true` dropped"
        );

        let bytes = write_job(
            Some(SPEC_EXAMPLE.as_bytes()),
            "test",
            Some(&json!({ "command": "just test", "enabled": false })),
        )
        .unwrap();
        let mut expected: Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        expected["jobs"]["test"] = json!({ "command": "just test", "enabled": false });
        assert_eq!(
            written(&bytes),
            text(&expected),
            "the other jobs and $schema stay"
        );

        let bytes = write_job(Some(SPEC_EXAMPLE.as_bytes()), "mail", None).unwrap();
        let mut expected: Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        expected["jobs"].as_object_mut().unwrap().remove("mail");
        assert_eq!(written(&bytes), text(&expected));

        let only = write_job(None, "a", Some(&json!({ "command": "x" }))).unwrap();
        assert_eq!(
            written(&write_job(Some(&only), "a", None).unwrap()),
            "{\n  \"jobs\": {}\n}\n",
            "the last job leaves an empty file, never none"
        );
        assert_eq!(
            write_job(Some(&only), "b", None),
            Err(WriteError::NotFound("b".to_owned()))
        );
    }

    // spec: DMN-332
    #[test]
    fn an_invalid_file_or_job_is_never_written() {
        let invalid = br#"{ "jobs": { "x": { "command": "x", "foo": 1 } } }"#;
        let error = write_job(Some(invalid), "y", Some(&json!({ "command": "y" }))).unwrap_err();
        assert_eq!(
            error,
            WriteError::InvalidFile("jobs.x.foo: unknown key".to_owned())
        );
        assert_eq!(
            error.to_string(),
            "cannot change .omfp/jobs.json while it is invalid: jobs.x.foo: unknown key"
        );
        assert!(matches!(
            set_enabled(Some(b"not json"), "x", false),
            Err(WriteError::InvalidFile(error)) if error.starts_with(".omfp/jobs.json is not JSON: ")
        ));
        assert_eq!(
            write_job(
                None,
                "x",
                Some(&json!({ "command": "x", "trigger": "service", "missed": "run" }))
            ),
            Err(WriteError::InvalidJob(
                "job.missed: only with trigger \"schedule\"".to_owned()
            ))
        );
        assert_eq!(
            write_job(None, "X", Some(&json!({ "command": "x" }))).map_err(|error| error.to_string()),
            Err(
                "name: expected 1-64 characters of a-z, 0-9, \".\", \"_\" and \"-\", starting with a letter or digit"
                    .to_owned()
            )
        );
    }

    // spec: DMN-332
    #[test]
    fn enabling_removes_the_key_and_disabling_writes_it() {
        let disabled = set_enabled(Some(SPEC_EXAMPLE.as_bytes()), "test", false).unwrap();
        let mut expected: Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        expected["jobs"]["test"]["enabled"] = json!(false);
        assert_eq!(written(&disabled), text(&expected));
        assert!(!parse_jobs_file(&disabled).unwrap()["test"].enabled);

        let enabled = set_enabled(Some(&disabled), "test", true).unwrap();
        let expected: Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(written(&enabled), text(&expected));
        assert_eq!(
            set_enabled(Some(&enabled), "nope", true),
            Err(WriteError::NotFound("nope".to_owned()))
        );
        assert_eq!(
            set_enabled(None, "test", true),
            Err(WriteError::NotFound("test".to_owned()))
        );
    }

    // spec: DMN-331
    #[test]
    fn the_schema_is_served_listed_in_the_docs_index_and_has_the_keys_of_the_rules() {
        let read = crate::widgets::read_uri(SCHEMA_URL, None).unwrap();
        assert_eq!(read.content, SCHEMA);
        assert_eq!(
            read.content_type,
            Some(omp_rpc::HostUriResultContentType::ApplicationJson)
        );
        let index = crate::widgets::shell::index();
        assert_eq!(
            index
                .lines()
                .filter(|line| line.starts_with(&format!("- {SCHEMA_URL} — ")))
                .count(),
            1,
            "{index}"
        );

        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        assert_eq!(schema["$id"], SCHEMA_URL);
        assert_eq!(schema["additionalProperties"], json!(false));
        let keys = |object: &Value| {
            let mut keys: Vec<String> = object.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        let sorted = |known: &[&str]| {
            let mut known: Vec<String> = known.iter().map(|&key| key.to_owned()).collect();
            known.sort();
            known
        };
        assert_eq!(keys(&schema["properties"]), sorted(&["$schema", "jobs"]));
        let job = &schema["$defs"]["job"];
        assert_eq!(job["additionalProperties"], json!(false));
        assert_eq!(keys(&job["properties"]), sorted(&JOB_KEYS));
        let prompt = &schema["$defs"]["prompt"];
        assert_eq!(prompt["additionalProperties"], json!(false));
        assert_eq!(keys(&prompt["properties"]), sorted(&PROMPT_KEYS));
        assert_eq!(
            schema["properties"]["jobs"]["propertyNames"]["pattern"],
            "^[a-z0-9][a-z0-9._-]{0,63}$"
        );
    }
}
