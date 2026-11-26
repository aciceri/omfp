//! A workspace's key bindings, `<workspace>/.omfp/keymap.json` (DMN-120, UI-210): read and checked by the daemon with
//! the rules of [`SCHEMA`] (errors as `<JSON path>: <reason>`), never written: it belongs to the user and the agent.

use std::path::{Path, PathBuf};

use omfp_protocol::{KeymapMode, WorkspaceBinding};
use serde_json::Value;

use super::{OMFP_DIR, read_json};

/// The JSON Schema of the workspace keymap, served as [`SCHEMA_URL`] (DMN-122).
pub const SCHEMA: &str = include_str!("keymap.schema.json");
/// Host URI of [`SCHEMA`] (DMN-122).
pub const SCHEMA_URL: &str = "omfp://schemas/keymap.json";

/// `<workspace>/.omfp/keymap.json` (DMN-120).
#[must_use]
pub fn path(workspace: &Path) -> PathBuf {
    workspace.join(OMFP_DIR).join("keymap.json")
}

/// The bindings of `keymap` in file order, or the first rule of [`SCHEMA`] it breaks as `<JSON path>: <reason>`.
pub fn check(keymap: &Value) -> Result<Vec<WorkspaceBinding>, String> {
    let Some(entries) = keymap.get("bindings").and_then(Value::as_array) else {
        return Err("bindings: expected an array of bindings".to_owned());
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| binding(entry, &format!("bindings[{index}]")))
        .collect()
}

fn binding(entry: &Value, path: &str) -> Result<WorkspaceBinding, String> {
    if !entry.is_object() {
        return Err(format!("{path}: expected an object with keys and command"));
    }
    let mode = match entry.get("mode").map(|mode| mode.as_str()) {
        None | Some(Some("normal")) => KeymapMode::Normal,
        Some(Some("threads")) => KeymapMode::Threads,
        Some(Some("view")) => KeymapMode::View,
        Some(Some("insert")) => KeymapMode::Insert,
        Some(Some("cmdline")) => KeymapMode::Cmdline,
        Some(_) => {
            return Err(format!(
                "{path}.mode: expected normal, threads, view, insert or cmdline"
            ));
        }
    };
    let keys = entry
        .get("keys")
        .and_then(Value::as_str)
        .filter(|keys| !keys.trim().is_empty())
        .ok_or_else(|| format!("{path}.keys: expected a key sequence such as \"Space t\""))?;
    let command = entry
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!("{path}.command: expected a command id (\"\" removes the binding)")
        })?;
    let args = match entry.get("args") {
        None => Vec::new(),
        Some(args) => args
            .as_array()
            .and_then(|args| {
                args.iter()
                    .map(|arg| arg.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| format!("{path}.args: expected an array of strings"))?,
    };
    Ok(WorkspaceBinding {
        mode,
        keys: keys.to_owned(),
        command: command.to_owned(),
        args,
    })
}

/// The workspace keymap at `path` (DMN-120): `Ok(None)` without a file, the bindings when valid, else why it is
/// refused (unreadable, not JSON, or the first rule it breaks).
pub fn read(path: &Path) -> Result<Option<Vec<WorkspaceBinding>>, String> {
    read_json(path, "keymap.json")?
        .map(|keymap| check(&keymap))
        .transpose()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn bound(mode: KeymapMode, keys: &str, command: &str, args: &[&str]) -> WorkspaceBinding {
        WorkspaceBinding {
            mode,
            keys: keys.to_owned(),
            command: command.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
        }
    }

    // spec: DMN-120, AGT-030
    #[test]
    fn a_valid_keymap_gives_its_bindings_in_file_order_with_normal_mode_by_default() {
        let keymap = json!({
            "$schema": SCHEMA_URL,
            "bindings": [
                { "keys": "Space t", "command": "tests.run", "args": ["unit"] },
                { "mode": "insert", "keys": "Ctrl-t", "command": "" },
                { "mode": "threads", "keys": "Ctrl-j", "command": "thread.next" },
                { "mode": "view", "keys": "Ctrl-j", "command": "nav.next" },
                { "mode": "cmdline", "keys": "Ctrl-r", "command": "history.search", "note": "ignored" }
            ],
            "comment": "unknown keys are ignored"
        });
        assert_eq!(
            check(&keymap),
            Ok(vec![
                bound(KeymapMode::Normal, "Space t", "tests.run", &["unit"]),
                bound(KeymapMode::Insert, "Ctrl-t", "", &[]),
                bound(KeymapMode::Threads, "Ctrl-j", "thread.next", &[]),
                bound(KeymapMode::View, "Ctrl-j", "nav.next", &[]),
                bound(KeymapMode::Cmdline, "Ctrl-r", "history.search", &[]),
            ])
        );
        assert_eq!(check(&json!({ "bindings": [] })), Ok(Vec::new()));
    }

    // spec: DMN-120
    #[test]
    fn an_invalid_keymap_names_the_first_rule_it_breaks() {
        for (keymap, error) in [
            (json!([]), "bindings: expected an array of bindings"),
            (
                json!({ "bindings": {} }),
                "bindings: expected an array of bindings",
            ),
            (
                json!({ "bindings": ["Space t"] }),
                "bindings[0]: expected an object with keys and command",
            ),
            (
                json!({ "bindings": [{ "keys": "a", "command": "x" }, { "keys": " ", "command": "x" }] }),
                "bindings[1].keys: expected a key sequence such as \"Space t\"",
            ),
            (
                json!({ "bindings": [{ "keys": "a" }] }),
                "bindings[0].command: expected a command id (\"\" removes the binding)",
            ),
            (
                json!({ "bindings": [{ "keys": "a", "command": "x", "mode": "visual" }] }),
                "bindings[0].mode: expected normal, threads, view, insert or cmdline",
            ),
            (
                json!({ "bindings": [{ "keys": "a", "command": "x", "args": ["1", 2] }] }),
                "bindings[0].args: expected an array of strings",
            ),
        ] {
            assert_eq!(check(&keymap), Err(error.to_owned()), "{keymap}");
        }
    }

    // spec: DMN-120
    #[test]
    fn reading_tells_a_missing_file_from_an_unreadable_or_invalid_one_and_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path());
        assert_eq!(read(&file), Ok(None), "no file: no bindings, no error");
        assert!(!file.exists(), "never written");

        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ \"bindings\": ").unwrap();
        let error = read(&file).unwrap_err();
        assert!(
            error.starts_with(".omfp/keymap.json is not JSON: "),
            "{error}"
        );

        std::fs::write(
            &file,
            r#"{ "bindings": [{ "keys": "g t", "command": "tests.run" }] }"#,
        )
        .unwrap();
        assert_eq!(
            read(&file),
            Ok(Some(vec![bound(
                KeymapMode::Normal,
                "g t",
                "tests.run",
                &[]
            )]))
        );

        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let error = read(&file).unwrap_err();
        assert!(
            error.starts_with("cannot read .omfp/keymap.json: "),
            "{error}"
        );
    }

    // spec: DMN-122
    #[test]
    fn the_schema_names_its_url_and_the_rules_the_check_applies() {
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        assert_eq!(schema["$id"], SCHEMA_URL);
        assert_eq!(schema["required"], json!(["bindings"]));
        let binding = &schema["$defs"]["binding"];
        assert_eq!(binding["required"], json!(["keys", "command"]));
        assert_eq!(
            binding["properties"]["mode"]["enum"],
            json!(["normal", "threads", "view", "insert", "cmdline"])
        );
        assert_eq!(binding["properties"]["args"]["items"]["type"], "string");
    }
}
