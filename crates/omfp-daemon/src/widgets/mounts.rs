//! Mounts (AGT-060..063): widget nodes of the views manifest whose `instance` prop is the mount id, in a view or as a
//! floating panel (`panels.<id>`). Pure functions over the manifest's JSON, like [`layout::add`]; the server writes
//! the result as `views.set` does.

use serde_json::{Map, Value, json};

use super::layout::{self, Addition, Place};
use super::{default_manifest, manifest};

/// Why a mount edit failed: `Invalid` is a `bad_request`, `NotFound` a `not_found` (PRT-080..082).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MountError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
}

/// Where a mount goes (AGT-061).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    /// The floating panel `panels.<id>`.
    Panel,
    /// The view `view` (`None`: the default view; a new id creates a view) at `place`.
    View { view: Option<&'a str>, place: Place },
}

/// A widget to mount (AGT-061).
#[derive(Debug, Clone, PartialEq)]
pub struct Mount<'a> {
    /// The mount id: the node's `instance` prop and the panel id.
    pub id: &'a str,
    /// The widget reference (`./widgets/<name>`, `omfp:<name>`).
    pub reference: &'a str,
    /// The node's props; `instance` is set to `id`.
    pub props: Map<String, Value>,
    pub target: Target<'a>,
}

/// A manifest with a mount placed or updated (PRT-080, PRT-081).
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub manifest: Value,
    /// The reference the mount renders.
    pub reference: String,
    /// The view showing it; `None` for a panel.
    pub view: Option<String>,
    /// Whether the id was mounted before.
    pub replaced: bool,
}

/// A manifest with a mount removed (PRT-082).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Removed {
    pub manifest: Value,
    /// Views that lost nodes and stay, in id order.
    pub views: Vec<String>,
    /// Views removed because they were left empty, in id order.
    pub removed_views: Vec<String>,
    /// Whether the panel `id` was removed.
    pub panel: bool,
}

/// Whether `node` is a widget node of mount `id`.
fn is_mount(node: &Value, id: &str) -> bool {
    !layout::is_container(node) && layout::instance_of(node).as_deref() == Some(id)
}

/// Calls `visit` on every widget node of mount `id` in `node`.
fn each_mount(node: &mut Value, id: &str, visit: &mut impl FnMut(&mut Value)) {
    if layout::is_container(node) {
        for child in node.as_array_mut().into_iter().flatten().skip(2) {
            each_mount(child, id, visit);
        }
    } else if is_mount(node, id) {
        visit(node);
    }
}

/// `node` without the nodes of mount `id` (setting `removed` when it drops one): `None` when nothing is left of it. A
/// container keeps the weights of the children it keeps.
fn prune(node: &Value, id: &str, removed: &mut bool) -> Option<Value> {
    if is_mount(node, id) {
        *removed = true;
        return None;
    }
    let Some(items) = node.as_array().filter(|_| layout::is_container(node)) else {
        return Some(node.clone());
    };
    let mut props = items.get(1).cloned().unwrap_or_else(|| json!({}));
    let sizes = props.get("sizes").and_then(Value::as_array).cloned();
    let mut children = Vec::new();
    let mut kept = Vec::new();
    for (index, child) in items.iter().skip(2).enumerate() {
        if let Some(child) = prune(child, id, removed) {
            children.push(child);
            if let Some(size) = sizes.as_ref().and_then(|sizes| sizes.get(index)) {
                kept.push(size.clone());
            }
        }
    }
    if children.is_empty() {
        return None;
    }
    if sizes.is_some() {
        props["sizes"] = Value::Array(kept);
    }
    let mut out = vec![items[0].clone(), props];
    out.extend(children);
    Some(Value::Array(out))
}

/// The view ids of `manifest`, in id order.
fn view_ids(manifest: &Value) -> Vec<String> {
    let mut ids: Vec<String> = manifest
        .get("views")
        .and_then(Value::as_object)
        .map(|views| views.keys().cloned().collect())
        .unwrap_or_default();
    ids.sort();
    ids
}

/// Removes the panel `id` and the nodes of mount `id` from every view but `except`.
fn remove(manifest: &mut Value, id: &str, except: Option<&str>) -> Result<Removed, MountError> {
    let mut removal = Removed::default();
    if let Some(object) = manifest.as_object_mut()
        && let Some(panels) = object.get_mut("panels").and_then(Value::as_object_mut)
    {
        removal.panel = panels.remove(id).is_some();
        if panels.is_empty() {
            object.remove("panels");
        }
    }
    for view_id in view_ids(manifest) {
        if except == Some(view_id.as_str()) {
            continue;
        }
        let Some(layout) = manifest["views"][&view_id].get("layout") else {
            continue;
        };
        let mut removed = false;
        let pruned = prune(layout, id, &mut removed);
        if !removed {
            continue;
        }
        if let Some(layout) = pruned {
            manifest["views"][&view_id]["layout"] = layout;
            removal.views.push(view_id);
        } else {
            if let Some(views) = manifest["views"].as_object_mut() {
                views.remove(&view_id);
            }
            removal.removed_views.push(view_id);
        }
    }
    if view_ids(manifest).is_empty() {
        return Err(MountError::Invalid(format!(
            "removing {id} would leave no view: it is all the workspace's only view shows; mount something else \
             there first"
        )));
    }
    let default_removed = manifest
        .get("default")
        .and_then(Value::as_str)
        .is_some_and(|default| removal.removed_views.iter().any(|view| view == default));
    if default_removed && let Some(object) = manifest.as_object_mut() {
        object.remove("default");
    }
    Ok(removal)
}

/// Whether `manifest` has the panel `id` or a node of mount `id`.
fn is_mounted(manifest: &Value, id: &str) -> bool {
    manifest
        .get("panels")
        .and_then(|panels| panels.get(id))
        .is_some()
        || manifest
            .get("views")
            .and_then(Value::as_object)
            .is_some_and(|views| {
                views
                    .values()
                    .filter_map(|view| view.get("layout"))
                    .any(|layout| layout::shows(layout, id))
            })
}

fn checked(manifest: &Value) -> Result<(), MountError> {
    manifest::check(manifest)
        .map_err(|error| MountError::Invalid(format!("the result would be invalid: {error}")))
}

fn not_mounted(id: &str) -> MountError {
    MountError::NotFound(format!("nothing is mounted with the id {id}"))
}

/// `manifest` (the workspace's valid manifest, `None` without a file: the default one) with `mount` placed (AGT-061):
/// a node of the mount already in the target view is replaced in place, any other one (another view, the panel)
/// removed.
pub fn mount(manifest: Option<&Value>, mount: &Mount<'_>) -> Result<Placed, MountError> {
    let mut manifest = manifest.cloned().unwrap_or_else(default_manifest);
    let mut props = mount.props.clone();
    props.insert("instance".to_owned(), json!(mount.id));
    let node = json!([mount.reference, props]);
    let replaced = is_mounted(&manifest, mount.id);
    let view = match mount.target {
        Target::Panel => {
            remove(&mut manifest, mount.id, None)?;
            let object = manifest
                .as_object_mut()
                .ok_or_else(|| MountError::Invalid("views.json is not an object".to_owned()))?;
            let panels = object.entry("panels").or_insert_with(|| json!({}));
            if !panels.is_object() {
                *panels = json!({});
            }
            panels[mount.id] = node;
            None
        }
        Target::View { view, place } => {
            let view_id = view
                .map(str::to_owned)
                .or_else(|| layout::default_view(&manifest))
                .ok_or_else(|| MountError::Invalid("views.json has no views".to_owned()))?;
            let mut in_place = false;
            if let Some(layout) = manifest
                .get_mut("views")
                .and_then(|views| views.get_mut(&view_id))
                .and_then(|view| view.get_mut("layout"))
            {
                each_mount(layout, mount.id, &mut |found| {
                    if !in_place {
                        found.clone_from(&node);
                        in_place = true;
                    }
                });
            }
            if in_place {
                remove(&mut manifest, mount.id, Some(&view_id))?;
            } else {
                remove(&mut manifest, mount.id, None)?;
                let addition = Addition {
                    reference: mount.reference,
                    props,
                    view: Some(&view_id),
                    place,
                };
                manifest = layout::add(Some(&manifest), &addition)
                    .map_err(MountError::Invalid)?
                    .0;
            }
            Some(view_id)
        }
    };
    checked(&manifest)?;
    Ok(Placed {
        manifest,
        reference: mount.reference.to_owned(),
        view,
        replaced,
    })
}

/// Merges `props` into the widget node `node` (a `null` value deletes the prop); `instance` never changes.
fn merge(node: &mut Value, props: &Map<String, Value>) {
    if let Value::String(reference) = node {
        *node = json!([reference.clone(), {}]);
    }
    let Some(own) = node.get_mut(1).and_then(Value::as_object_mut) else {
        return;
    };
    for (key, value) in props {
        if key == "instance" {
            continue;
        }
        if value.is_null() {
            own.remove(key);
        } else {
            own.insert(key.clone(), value.clone());
        }
    }
}

/// The reference of the widget node `node`.
fn reference_of(node: &Value) -> String {
    layout::reference_of(node).unwrap_or_default().to_owned()
}

/// `manifest` with `props` merged into the panel `id` and every node of mount `id` (AGT-062).
pub fn update(
    manifest: Option<&Value>,
    id: &str,
    props: &Map<String, Value>,
) -> Result<Placed, MountError> {
    let mut manifest = manifest.cloned().ok_or_else(|| not_mounted(id))?;
    let mut found: Option<(String, Option<String>)> = None;
    if let Some(panel) = manifest
        .get_mut("panels")
        .and_then(|panels| panels.get_mut(id))
    {
        if layout::is_container(panel) {
            return Err(MountError::Invalid(format!(
                "panel {id} is a layout of several widgets: edit .omfp/views.json instead"
            )));
        }
        merge(panel, props);
        found = Some((reference_of(panel), None));
    }
    for view_id in view_ids(&manifest) {
        if let Some(layout) = manifest["views"][&view_id].get_mut("layout") {
            each_mount(layout, id, &mut |node| {
                merge(node, props);
                if found.is_none() {
                    found = Some((reference_of(node), Some(view_id.clone())));
                }
            });
        }
    }
    let (reference, view) = found.ok_or_else(|| not_mounted(id))?;
    checked(&manifest)?;
    Ok(Placed {
        manifest,
        reference,
        view,
        replaced: true,
    })
}

/// `manifest` without the panel `id` and the nodes of mount `id` (AGT-063).
pub fn unmount(manifest: Option<&Value>, id: &str) -> Result<Removed, MountError> {
    let mut manifest = manifest.cloned().ok_or_else(|| not_mounted(id))?;
    let mut removal = remove(&mut manifest, id, None)?;
    if !removal.panel && removal.views.is_empty() && removal.removed_views.is_empty() {
        return Err(not_mounted(id));
    }
    checked(&manifest)?;
    removal.manifest = manifest;
    Ok(removal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(props) => props,
            _ => Map::new(),
        }
    }

    fn in_view<'a>(
        id: &'a str,
        reference: &'a str,
        view: Option<&'a str>,
        place: Place,
    ) -> Mount<'a> {
        Mount {
            id,
            reference,
            props: Map::new(),
            target: Target::View { view, place },
        }
    }

    fn panel<'a>(id: &'a str, reference: &'a str) -> Mount<'a> {
        Mount {
            id,
            reference,
            props: props(json!({ "title": "Clock" })),
            target: Target::Panel,
        }
    }

    // spec: AGT-061
    #[test]
    fn a_mount_without_a_file_goes_into_the_default_view_with_its_id_as_instance() {
        let mut dashboard = in_view("deploy", "./widgets/deploy", None, Place::SideTab);
        dashboard.props = props(json!({ "env": "prod", "instance": "ignored" }));
        let placed = mount(None, &dashboard).unwrap();
        assert_eq!(placed.view.as_deref(), Some("default"));
        assert!(!placed.replaced);
        assert_eq!(placed.reference, "./widgets/deploy");
        let side = &placed.manifest["views"]["default"]["layout"][3];
        assert_eq!(
            side[3],
            json!(["./widgets/deploy", { "env": "prod", "instance": "deploy" }])
        );
    }

    // spec: AGT-061
    #[test]
    fn a_new_view_id_makes_a_dashboard_and_mounting_again_replaces_in_place() {
        let first = mount(
            None,
            &in_view("board", "omfp:board", Some("dash"), Place::End),
        )
        .unwrap();
        assert_eq!(
            first.manifest["views"]["dash"],
            json!({ "title": "dash", "layout": ["omfp:board", { "instance": "board" }] })
        );
        let second = mount(
            Some(&first.manifest),
            &in_view("feed", "./widgets/feed", Some("dash"), Place::End),
        )
        .unwrap();
        let mut again = in_view("board", "./widgets/board", Some("dash"), Place::End);
        again.props = props(json!({ "rows": 3 }));
        let third = mount(Some(&second.manifest), &again).unwrap();
        assert!(third.replaced);
        assert_eq!(
            third.manifest["views"]["dash"]["layout"],
            json!([
                "row",
                {},
                ["./widgets/board", { "instance": "board", "rows": 3 }],
                ["./widgets/feed", { "instance": "feed" }]
            ]),
            "replaced where it was"
        );
    }

    // spec: AGT-060, AGT-061
    #[test]
    fn a_panel_mount_moves_the_id_out_of_the_views_and_back() {
        let in_side = mount(None, &in_view("clock", "omfp:clock", None, Place::SideTab)).unwrap();
        let floating = mount(Some(&in_side.manifest), &panel("clock", "omfp:clock")).unwrap();
        assert!(floating.replaced);
        assert_eq!(floating.view, None);
        assert_eq!(
            floating.manifest["panels"],
            json!({ "clock": ["omfp:clock", { "instance": "clock", "title": "Clock" }] })
        );
        assert_eq!(
            floating.manifest["views"],
            default_manifest()["views"],
            "the view lost the mount"
        );

        let back = mount(
            Some(&floating.manifest),
            &in_view("clock", "omfp:clock", None, Place::MainTab),
        )
        .unwrap();
        assert_eq!(back.manifest.get("panels"), None, "no empty panels object");
        assert_eq!(
            back.manifest["views"]["default"]["layout"][2][3]
                .as_array()
                .map(Vec::len),
            Some(4),
            "a main tab"
        );
    }

    // spec: AGT-062
    #[test]
    fn an_update_merges_props_deletes_nulls_and_keeps_the_instance() {
        let floating = mount(None, &panel("clock", "omfp:clock")).unwrap();
        let updated = update(
            Some(&floating.manifest),
            "clock",
            &props(json!({ "zone": "UTC", "title": null, "instance": "other" })),
        )
        .unwrap();
        assert_eq!(
            updated.manifest["panels"]["clock"],
            json!(["omfp:clock", { "instance": "clock", "zone": "UTC" }])
        );
        assert_eq!(
            (updated.reference.as_str(), updated.view),
            ("omfp:clock", None)
        );
        assert!(updated.replaced);

        let error = update(Some(&floating.manifest), "nope", &Map::new()).unwrap_err();
        assert_eq!(
            error,
            MountError::NotFound("nothing is mounted with the id nope".to_owned())
        );
        assert!(matches!(
            update(None, "clock", &Map::new()),
            Err(MountError::NotFound(_))
        ));
    }

    // spec: AGT-063
    #[test]
    fn an_unmount_prunes_empty_containers_and_views_and_their_weights() {
        let manifest = json!({
            "default": "dash",
            "panels": { "clock": "omfp:clock" },
            "views": {
                "dash": { "layout": ["omfp:board", { "instance": "board" }] },
                "work": { "layout": ["row", { "sizes": [3, 1, 2] },
                    "omfp:chat",
                    ["tabs", {}, ["./widgets/board", { "instance": "board" }]],
                    "omfp:changes"] }
            }
        });
        let removed = unmount(Some(&manifest), "board").unwrap();
        assert_eq!(removed.views, vec!["work".to_owned()]);
        assert_eq!(removed.removed_views, vec!["dash".to_owned()]);
        assert!(!removed.panel);
        assert_eq!(
            removed.manifest,
            json!({
                "panels": { "clock": "omfp:clock" },
                "views": { "work": { "layout": ["row", { "sizes": [3, 2] }, "omfp:chat", "omfp:changes"] } }
            }),
            "the default named the removed view"
        );

        let closed = unmount(Some(&removed.manifest), "clock").unwrap();
        assert!(closed.panel);
        assert_eq!(closed.manifest.get("panels"), None);
        assert!(matches!(
            unmount(Some(&closed.manifest), "clock"),
            Err(MountError::NotFound(_))
        ));
    }

    // spec: AGT-063
    #[test]
    fn the_last_view_is_never_emptied() {
        let manifest =
            json!({ "views": { "dash": { "layout": ["omfp:board", { "instance": "board" }] } } });
        let error = unmount(Some(&manifest), "board").unwrap_err();
        assert!(
            matches!(&error, MountError::Invalid(text) if text.contains("would leave no view")),
            "{error}"
        );
        let error = mount(Some(&manifest), &panel("board", "omfp:board")).unwrap_err();
        assert!(matches!(error, MountError::Invalid(_)), "{error}");
    }
}
