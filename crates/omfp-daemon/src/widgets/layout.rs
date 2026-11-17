//! Adding a widget to a view of the manifest (`views.mount`, PRT-080, the placement `omfp_ui_mount` takes): a pure
//! function over the manifest's JSON; a cell added to a container with `sizes` weighs the mean of its siblings, as
//! `:widget add` does in the window (UI-300).

use serde_json::{Map, Value, json};

use super::default_manifest;
use super::manifest::{self, CONTAINERS};

/// Where the widget goes in an existing view (`views.mount`'s `place`, PRT-080).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// A tab of the `tabs` container whose `id` prop is `side`.
    SideTab,
    /// A tab of the `tabs` container whose `id` prop is `main`, after the conversation.
    MainTab,
    /// The end of the view's root container; a root widget becomes a `row` of both.
    End,
}

impl Place {
    fn slot(self) -> Option<&'static str> {
        match self {
            Self::SideTab => Some("side"),
            Self::MainTab => Some("main"),
            Self::End => None,
        }
    }
}

/// What to add, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Addition<'a> {
    /// The widget reference (`./widgets/<name>`, `user:<name>`, `omfp:<name>`).
    pub reference: &'a str,
    /// The node's props; an empty object adds the bare reference.
    pub props: Map<String, Value>,
    /// The view to change; `None` for the manifest's default view.
    pub view: Option<&'a str>,
    pub place: Place,
}

/// Whether `node` is a container (`[kind, props, ...children]`) rather than a widget.
pub(super) fn is_container(node: &Value) -> bool {
    node.as_array()
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .is_some_and(|kind| CONTAINERS.contains(&kind))
}

/// The children of `node` when it is a container; none for a widget.
fn children(node: &Value) -> impl Iterator<Item = &Value> {
    node.as_array()
        .filter(|_| is_container(node))
        .into_iter()
        .flatten()
        .skip(2)
}

/// Whether a widget node of `node` (itself when it is no container) passes `test`.
fn any_widget(node: &Value, test: &impl Fn(&Value) -> bool) -> bool {
    if is_container(node) {
        children(node).any(|child| any_widget(child, test))
    } else {
        test(node)
    }
}

/// The reference of a widget node: the node itself or its first item.
pub(super) fn reference_of(node: &Value) -> Option<&str> {
    match node {
        Value::String(reference) => Some(reference),
        Value::Array(items) => items.first()?.as_str(),
        _ => None,
    }
}

/// The instance of a widget node (AGT-010): its `instance` prop when it is a string, else its reference.
pub(super) fn instance_of(node: &Value) -> Option<String> {
    let reference = reference_of(node)?;
    Some(
        node.get(1)
            .and_then(|props| props.get("instance"))
            .and_then(Value::as_str)
            .unwrap_or(reference)
            .to_owned(),
    )
}

/// Whether `node` shows the widget instance `instance`.
pub(super) fn shows(node: &Value, instance: &str) -> bool {
    any_widget(node, &|widget| {
        instance_of(widget).as_deref() == Some(instance)
    })
}

/// The ids of the views of `manifest` whose layout holds the widget `reference` (DMN-209), in the manifest's order.
#[must_use]
pub fn views_using(manifest: &Value, reference: &str) -> Vec<String> {
    manifest
        .get("views")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, view)| {
            view.get("layout").is_some_and(|layout| {
                any_widget(layout, &|widget| reference_of(widget) == Some(reference))
            })
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// Whether `node` is the `tabs` container whose `id` prop is `id`.
fn is_slot(node: &Value, id: &str) -> bool {
    is_container(node) && node[0] == "tabs" && node[1].get("id").and_then(Value::as_str) == Some(id)
}

/// The `tabs` container of `node` whose `id` prop is `id`.
fn find_slot<'a>(node: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if is_slot(node, id) {
        return Some(node);
    }
    if !is_container(node) {
        return None;
    }
    node.as_array_mut()?
        .iter_mut()
        .skip(2)
        .find_map(|child| find_slot(child, id))
}

/// Appends `child` to the container `node`; with `sizes`, it weighs the mean of its siblings.
fn append(node: &mut Value, child: Value) {
    if let Some(sizes) = node[1].get_mut("sizes").and_then(Value::as_array_mut) {
        let weights: Vec<f64> = sizes.iter().filter_map(Value::as_f64).collect();
        #[allow(clippy::cast_precision_loss)] // A handful of cells.
        let mean = if weights.is_empty() {
            1.0
        } else {
            // Rounded to four decimals.
            (weights.iter().sum::<f64>() / weights.len() as f64 * 10_000.0).round() / 10_000.0
        };
        // Whole weights stay integers, as the window writes them (`2`, not `2.0`).
        #[allow(clippy::cast_possible_truncation)]
        let weight = if mean.fract() == 0.0 {
            json!(mean as i64)
        } else {
            json!(mean)
        };
        sizes.push(weight);
    }
    if let Some(items) = node.as_array_mut() {
        items.push(child);
    }
}

/// The view `view` of `manifest` (else its default view) with `addition` added, and a sentence saying where it went.
/// `manifest` is the valid manifest of the workspace, `None` without a file (the default manifest is used, DMN-088).
pub fn add(manifest: Option<&Value>, addition: &Addition<'_>) -> Result<(Value, String), String> {
    let mut manifest = manifest.cloned().unwrap_or_else(default_manifest);
    let node = if addition.props.is_empty() {
        json!(addition.reference)
    } else {
        json!([addition.reference, addition.props])
    };
    let instance = instance_of(&node).unwrap_or_default();
    let view_id = addition
        .view
        .map(str::to_owned)
        .or_else(|| default_view(&manifest))
        .ok_or("views.json has no views")?;
    let views = manifest
        .get_mut("views")
        .and_then(Value::as_object_mut)
        .ok_or("views.json has no views")?;
    let said = if let Some(view) = views.get_mut(&view_id) {
        // A TSX view has no layout to add to (AGT-074).
        if let Some(file) = manifest::view_file(view) {
            return Err(format!(
                "view {view_id:?} is a TSX view ({file}): edit that file, or mount into another view"
            ));
        }
        let layout = view
            .get_mut("layout")
            .ok_or_else(|| format!("view {view_id:?} has no layout"))?;
        if shows(layout, &instance) {
            return Err(format!(
                "view {view_id:?} already shows {instance}; give the new one another `instance` prop to add a second \
                 one"
            ));
        }
        match addition.place.slot() {
            Some(slot) => {
                let container = find_slot(layout, slot).ok_or_else(|| {
                    format!(
                        "view {view_id:?} has no `tabs` container with \"id\": \"{slot}\"; use place: \"end\", or \
                         another view"
                    )
                })?;
                append(container, node);
                format!("as a tab of the {slot} tabs of view {view_id:?}")
            }
            None if is_container(layout) => {
                append(layout, node);
                format!("at the end of view {view_id:?}")
            }
            None => {
                *layout = json!(["row", {}, layout.take(), node]);
                format!("at the end of view {view_id:?}")
            }
        }
    } else {
        views.insert(view_id.clone(), json!({ "title": view_id, "layout": node }));
        format!("as the new view {view_id:?}")
    };
    manifest::check(&manifest).map_err(|error| format!("the result would be invalid: {error}"))?;
    Ok((manifest, said))
}

/// The view of `manifest` a widget goes to without a `view`: its `default` view when it names one of its views, else
/// the first view id.
pub(super) fn default_view(manifest: &Value) -> Option<String> {
    let views = manifest.get("views")?.as_object()?;
    manifest
        .get("default")
        .and_then(Value::as_str)
        .filter(|default| views.contains_key(*default))
        .or_else(|| views.keys().map(String::as_str).min())
        .map(str::to_owned)
}

/// Whether `node` holds the `tabs` container whose `id` prop is `id`.
fn has_slot(node: &Value, id: &str) -> bool {
    is_slot(node, id) || children(node).any(|child| has_slot(child, id))
}

/// Whether a view of `manifest` has the `main` slot, the `tabs` container with `"id": "main"` where files open
/// (UI-115); without one they open in a floating panel.
#[must_use]
pub fn has_main_slot(manifest: &Value) -> bool {
    manifest
        .get("views")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|views| views.values())
        .filter_map(|view| view.get("layout"))
        .any(|layout| has_slot(layout, "main"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addition(reference: &str, place: Place) -> Addition<'_> {
        Addition {
            reference,
            props: Map::new(),
            view: None,
            place,
        }
    }

    fn layout(manifest: &Value) -> &Value {
        &manifest["views"]["default"]["layout"]
    }

    // spec: PRT-080, DMN-088
    #[test]
    fn without_a_file_the_default_view_gets_the_widget_as_a_side_tab() {
        let (manifest, said) = add(None, &addition("./widgets/tests", Place::SideTab)).unwrap();
        assert_eq!(
            layout(&manifest)[3],
            json!(["tabs", { "id": "side" }, "omfp:workspace", "omfp:changes", "omfp:background", "omfp:mcp", "./widgets/tests"])
        );
        assert_eq!(said, "as a tab of the side tabs of view \"default\"");
        assert_eq!(manifest["$schema"], "omfp://schemas/views.json");
    }

    // spec: PRT-080
    #[test]
    fn main_tabs_and_the_end_of_the_root_take_the_widget_with_its_props() {
        let mut main = addition("user:clock", Place::MainTab);
        main.props.insert("zone".to_owned(), json!("UTC"));
        let (manifest, _) = add(None, &main).unwrap();
        assert_eq!(
            layout(&manifest)[2][3],
            json!(["tabs", { "id": "main" }, "omfp:chat", ["user:clock", { "zone": "UTC" }]])
        );

        let (manifest, said) =
            add(Some(&manifest), &addition("./widgets/tests", Place::End)).unwrap();
        assert_eq!(
            layout(&manifest)[1],
            json!({ "sizes": [3, 1, 2] }),
            "the mean weight"
        );
        assert_eq!(layout(&manifest)[4], json!("./widgets/tests"));
        assert_eq!(said, "at the end of view \"default\"");
    }

    // spec: PRT-080
    #[test]
    fn a_root_widget_becomes_a_row_and_an_unknown_view_is_created() {
        let manifest = json!({ "views": { "solo": { "layout": "omfp:chat" } } });
        let (manifest, _) = add(Some(&manifest), &addition("./widgets/tests", Place::End)).unwrap();
        assert_eq!(
            manifest["views"]["solo"]["layout"],
            json!(["row", {}, "omfp:chat", "./widgets/tests"])
        );

        let mut dashboard = addition("./widgets/tests", Place::SideTab);
        dashboard.view = Some("dash");
        let (manifest, said) = add(Some(&manifest), &dashboard).unwrap();
        assert_eq!(
            manifest["views"]["dash"],
            json!({ "title": "dash", "layout": "./widgets/tests" })
        );
        assert_eq!(said, "as the new view \"dash\"");
    }

    // spec: PRT-080
    #[test]
    fn duplicates_and_missing_slots_are_refused_with_the_fix() {
        let (manifest, _) = add(None, &addition("./widgets/tests", Place::SideTab)).unwrap();
        let error = add(Some(&manifest), &addition("./widgets/tests", Place::End)).unwrap_err();
        assert!(error.contains("already shows ./widgets/tests"), "{error}");
        assert!(error.contains("`instance` prop"), "{error}");

        let mut second = addition("./widgets/tests", Place::End);
        second.props.insert("instance".to_owned(), json!("tests-2"));
        assert!(add(Some(&manifest), &second).is_ok());

        let plain =
            json!({ "default": "a", "views": { "a": { "layout": ["row", {}, "omfp:chat"] } } });
        let error = add(Some(&plain), &addition("./widgets/tests", Place::SideTab)).unwrap_err();
        assert!(error.contains("\"id\": \"side\""), "{error}");
        assert!(error.contains("use place: \"end\""), "{error}");
    }

    // spec: AGT-074
    #[test]
    fn a_tsx_view_has_no_layout_to_add_to() {
        let manifest = json!({ "default": "ops", "views": {
            "ops": { "view": "./views/ops.tsx" },
            "work": { "layout": "omfp:chat" },
        } });
        assert_eq!(manifest::check(&manifest), Ok(()));
        let error = add(Some(&manifest), &addition("./widgets/tests", Place::End)).unwrap_err();
        assert_eq!(
            error,
            "view \"ops\" is a TSX view (./views/ops.tsx): edit that file, or mount into another view"
        );
        let mut elsewhere = addition("./widgets/tests", Place::End);
        elsewhere.view = Some("work");
        let (added, _) = add(Some(&manifest), &elsewhere).unwrap();
        assert_eq!(added["views"]["ops"], json!({ "view": "./views/ops.tsx" }));
        assert!(!has_main_slot(&manifest));
    }

    // spec: AGT-028
    #[test]
    fn a_manifest_keeps_the_main_slot_when_a_view_has_it() {
        assert!(has_main_slot(&default_manifest()));
        let (with_dashboard, _) = add(None, &{
            let mut dashboard = addition("./widgets/tests", Place::End);
            dashboard.view = Some("dash");
            dashboard
        })
        .unwrap();
        assert!(has_main_slot(&with_dashboard));
        let without = json!({ "views": {
            "a": { "layout": ["row", {}, "omfp:chat", ["tabs", { "id": "side" }, "omfp:changes"]] },
            "b": { "layout": "./widgets/tests" },
        } });
        assert!(!has_main_slot(&without));
    }
}
