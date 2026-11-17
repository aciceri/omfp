//! The views manifest, `<workspace>/.omfp/views.json` (AGT-010, AGT-011): checked with the rules and messages the app
//! shares (`fixtures/views/cases.json`), read with its own messages for an unreadable file or invalid JSON, and
//! written with sorted keys (DMN-083). A view is a layout or a TSX view file, `./views/<name>.tsx`
//! (AGT-074).

use std::path::Path;

use serde_json::{Map, Value};

use super::folders::is_widget_name;
use super::read_json;

/// Layout containers (AGT-010).
pub const CONTAINERS: [&str; 4] = ["row", "col", "tabs", "stack"];

/// The extensions of a TSX view file (AGT-074).
pub const VIEW_FILE_EXTENSIONS: [&str; 4] = ["tsx", "ts", "jsx", "js"];

/// The view file of the view object `view` (its `view` string, AGT-074); `None` for a layout view.
#[must_use]
pub fn view_file(view: &Value) -> Option<&str> {
    view.get("view")?.as_str()
}

/// Whether `file` names a TSX view file: `./views/<name>.<ext>`, `<name>` a widget name and `<ext>`
/// one of [`VIEW_FILE_EXTENSIONS`] (AGT-074).
#[must_use]
pub fn is_view_file(file: &str) -> bool {
    file.strip_prefix("./views/")
        .and_then(|rest| rest.rsplit_once('.'))
        .is_some_and(|(name, extension)| {
            is_widget_name(name) && VIEW_FILE_EXTENSIONS.contains(&extension)
        })
}

/// Checks `manifest` against AGT-010, AGT-060 and AGT-074; the error is the first rule it breaks, as
/// `<JSON path>: <reason>`.
/// Views are checked in id order, then panels in id order, `default` last.
pub fn check(manifest: &Value) -> Result<(), String> {
    let Some(views) = manifest.get("views").and_then(Value::as_object) else {
        return Err("views: expected an object of views".to_owned());
    };
    if views.is_empty() {
        return Err("views: no view".to_owned());
    }
    // Sorted here: serde_json keeps insertion order when `preserve_order` is enabled anywhere in the build.
    let mut ids: Vec<&String> = views.keys().collect();
    ids.sort();
    for id in ids {
        let path = format!("views.{id}");
        let Some(view) = views.get(id).and_then(Value::as_object) else {
            return Err(format!("{path}: expected {{ title, layout }}"));
        };
        if let Some(file) = view.get("view") {
            if view.contains_key("layout") {
                return Err(format!("{path}: a layout or a view file, not both"));
            }
            if !file.as_str().is_some_and(is_view_file) {
                return Err(format!("{path}.view: expected \"./views/<name>.tsx\""));
            }
            continue;
        }
        check_node(view.get("layout"), &format!("{path}.layout"))?;
    }
    if let Some(panels) = manifest.get("panels") {
        let Some(panels) = panels.as_object() else {
            return Err("panels: expected an object of panels".to_owned());
        };
        let mut ids: Vec<&String> = panels.keys().collect();
        ids.sort();
        for id in ids {
            check_node(panels.get(id), &format!("panels.{id}"))?;
        }
    }
    if let Some(default) = manifest.get("default")
        && !default.as_str().is_some_and(|id| views.contains_key(id))
    {
        let shown = default
            .as_str()
            .map_or_else(|| default.to_string(), str::to_owned);
        return Err(format!("default: no view {shown}"));
    }
    Ok(())
}

fn check_node(node: Option<&Value>, path: &str) -> Result<(), String> {
    let shape = || {
        format!(
            "{path}: expected a widget name, [widget, props] or [container, props, ...children]"
        )
    };
    match node {
        Some(Value::String(name)) => check_name(name, path),
        Some(Value::Array(items)) => {
            let Some(Value::String(head)) = items.first() else {
                return Err(shape());
            };
            check_name(head, path)?;
            let Some(props) = items.get(1).and_then(Value::as_object) else {
                return Err(format!("{path}: {head} needs a props object"));
            };
            let children = items.get(2..).unwrap_or_default();
            if !CONTAINERS.contains(&head.as_str()) {
                return if children.is_empty() {
                    Ok(())
                } else {
                    Err(format!("{path}: a widget reference is [name, props]"))
                };
            }
            if children.is_empty() {
                return Err(format!("{path}: {head} has no children"));
            }
            if let Some(sizes) = props.get("sizes") {
                check_sizes(sizes, children.len(), path)?;
            }
            for (index, child) in children.iter().enumerate() {
                check_node(Some(child), &format!("{path}[{}]", index + 2))?;
            }
            Ok(())
        }
        _ => Err(shape()),
    }
}

fn check_name(name: &str, path: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        Err(format!("{path}: a widget name is empty"))
    } else {
        Ok(())
    }
}

fn check_sizes(sizes: &Value, children: usize, path: &str) -> Result<(), String> {
    let valid = sizes.as_array().is_some_and(|sizes| {
        sizes.len() == children
            && sizes
                .iter()
                .all(|size| size.as_f64().is_some_and(|size| size > 0.0))
    });
    if valid {
        Ok(())
    } else {
        Err(format!("{path}: sizes must be {children} positive numbers"))
    }
}

/// The manifest at `path` and why it is refused (DMN-082): `(None, None)` without a file, `(Some, None)` when valid,
/// `(None, Some(error))` when it cannot be read, is not JSON or breaks AGT-010.
#[must_use]
pub fn read(path: &Path) -> (Option<Value>, Option<String>) {
    let checked = read_json(path, "views.json").and_then(|manifest| {
        manifest
            .map(|manifest| check(&manifest).map(|()| manifest))
            .transpose()
    });
    match checked {
        Ok(manifest) => (manifest, None),
        Err(error) => (None, Some(error)),
    }
}

/// `manifest` as written by `views.set` (DMN-083): two-space indentation, object keys sorted at every level, final
/// newline.
#[must_use]
pub fn to_text(manifest: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&sorted(manifest)).unwrap_or_default();
    text.push('\n');
    text
}

/// A copy of `value` whose objects hold their keys in sorted order, whatever map `serde_json` is built with.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                if let Some(item) = object.get(key) {
                    out.insert(key.clone(), sorted(item));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn cases() -> Vec<(String, Value, Option<String>)> {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/views/cases.json"),
        )
        .unwrap();
        let file: Value = serde_json::from_str(&text).unwrap();
        file["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| {
                (
                    case["name"].as_str().unwrap().to_owned(),
                    case["manifest"].clone(),
                    case["error"].as_str().map(str::to_owned),
                )
            })
            .collect()
    }

    // spec: AGT-010, AGT-011, AGT-060, AGT-074
    #[test]
    fn every_shared_case_gets_its_error() {
        let cases = cases();
        assert!(cases.len() >= 20);
        for (name, manifest, error) in cases {
            assert_eq!(check(&manifest).err(), error, "{name}");
        }
    }

    // spec: AGT-011, DMN-082
    #[test]
    fn an_absent_unreadable_or_invalid_file_has_its_own_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("views.json");
        assert_eq!(read(&path), (None, None), "no file: no error");

        std::fs::write(&path, "{ \"views\": ").unwrap();
        let (manifest, error) = read(&path);
        assert_eq!(manifest, None);
        let error = error.unwrap();
        assert!(
            error.starts_with(".omfp/views.json is not JSON: "),
            "{error}"
        );

        std::fs::write(&path, r#"{ "views": {} }"#).unwrap();
        assert_eq!(read(&path), (None, Some("views: no view".to_owned())));

        std::fs::write(&path, r#"{ "views": { "a": { "layout": "omfp:chat" } } }"#).unwrap();
        assert_eq!(
            read(&path),
            (
                Some(json!({ "views": { "a": { "layout": "omfp:chat" } } })),
                None
            )
        );

        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let (manifest, error) = read(&path);
        assert_eq!(manifest, None);
        let error = error.unwrap();
        assert!(
            error.starts_with("cannot read .omfp/views.json: "),
            "{error}"
        );
    }

    // spec: DMN-083
    #[test]
    fn written_manifests_sort_keys_at_every_level_with_two_spaces_and_a_final_newline() {
        let mut inner = Map::new();
        inner.insert("title".into(), json!("Work"));
        inner.insert(
            "layout".into(),
            json!(["row", { "z": 1, "a": 2 }, "omfp:chat"]),
        );
        let mut views = Map::new();
        views.insert("work".into(), Value::Object(inner));
        let mut manifest = Map::new();
        manifest.insert("views".into(), Value::Object(views));
        manifest.insert("default".into(), json!("work"));

        assert_eq!(
            to_text(&Value::Object(manifest)),
            "{\n  \"default\": \"work\",\n  \"views\": {\n    \"work\": {\n      \"layout\": [\n        \"row\",\n        \
             {\n          \"a\": 2,\n          \"z\": 1\n        },\n        \"omfp:chat\"\n      ],\n      \"title\": \
             \"Work\"\n    }\n  }\n}\n"
        );
    }
}
