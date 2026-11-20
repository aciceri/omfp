//! The mount and library tools (AGT-061..066): `omfp_ui_mount`, `omfp_ui_update`, `omfp_ui_remove`,
//! `omfp_component_save` and `omfp_component_list`, adapters over `views.mount`, `views.mount_update`,
//! `views.unmount`, `widgets.save` and `widgets.library` (PRT-080..084) that answer in sentences.

use std::fmt::Write as _;

use omfp_protocol::{
    Call, Empty, MountParams, MountPlace, MountUpdateParams, Mounted, SaveWidgetParams,
    UnmountParams, Unmounted, WidgetLibrary, WidgetSaved,
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::{AgentTool, ToolContext, ToolFuture, call, error_text};

/// Name of the tool that mounts a widget (AGT-061).
pub const UI_MOUNT: &str = "omfp_ui_mount";
/// Name of the tool that updates a mounted widget (AGT-062).
pub const UI_UPDATE: &str = "omfp_ui_update";
/// Name of the tool that removes a mounted widget (AGT-063).
pub const UI_REMOVE: &str = "omfp_ui_remove";
/// Name of the tool that saves a component to the user library (AGT-064).
pub const COMPONENT_SAVE: &str = "omfp_component_save";
/// Name of the tool that lists the user library (AGT-065).
pub const COMPONENT_LIST: &str = "omfp_component_list";

/// The tools of this module, for the registry (AGT-066).
pub(crate) fn tools() -> Vec<AgentTool> {
    vec![mount(), update(), remove(), save(), list()]
}

const WORKSPACE_ID: &str = "The workspace (default: the calling thread's; required without one, e.g. over MCP with a user token).";
const ID: &str = "The mount id (letters, digits, `_`, `-`, `.` not first): the widget's `instance` (its useStored state), the panel's id, and the folder inline `code` goes to.";

fn mount() -> AgentTool {
    AgentTool {
        name: UI_MOUNT,
        label: "Mount UI",
        description: "Shows a widget to the user, live in every open omfp window: in a view of the workspace at \
                      `place` (`view` names the view; a new name makes a dashboard view showing only it) or as a \
                      floating panel over every view (`panel: true`). The widget is `code` (a React component in \
                      TSX, default export, imports only from react, omfp/sdk and omfp/kit; written to \
                      .omfp/widgets/<id>/index.tsx) or `widget`, a reference: `user:<name>` (a saved component, see \
                      omfp_component_list), `./widgets/<name>` or `omfp:<name>`. Mounting an id again replaces it \
                      and omfp_ui_remove { id } removes it. Pass data as `props` and change them later with \
                      omfp_ui_update. Keep the main slot, `[\"tabs\", {\"id\": \"main\"}, …\"omfp:chat\"]`, in every \
                      view: the files the user opens become its tabs (without it they float in a panel). Check new \
                      code with omfp_widget_check { name: \"<id>\" }; omfp://docs/widgets documents the SDK and kit.",
        parameters: json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "minLength": 1, "description": ID },
                "code": { "type": "string", "description": "The component's source (TSX); exclusive with `widget`." },
                "widget": { "type": "string", "minLength": 1, "description": "A widget reference: `user:<name>`, `./widgets/<name>` (a bare name), `omfp:<name>`; exclusive with `code`." },
                "props": { "type": "object", "description": "The widget's props; `title` labels its tab or panel." },
                "view": { "type": "string", "minLength": 1, "description": "The view (default: the default view); a new id creates a view showing only this widget." },
                "place": {
                    "type": "string",
                    "enum": ["side_tab", "main_tab", "end"],
                    "description": "Where in the view, as views.mount takes it: side_tab (default: the last tab of the view's `tabs` with \"id\": \"side\"), main_tab (the last tab of its main slot, next to the conversation) or end (the end of the view's root container)."
                },
                "panel": { "type": "boolean", "description": "Float it over every view instead of placing it in a view." },
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
        run: run_mount,
    }
}

fn update() -> AgentTool {
    AgentTool {
        name: UI_UPDATE,
        label: "Update UI",
        description: "Changes a widget mounted with omfp_ui_mount, live in every open window without losing its \
                      state: `props` are merged into its props (null deletes one), `code` replaces its inline \
                      source. Use it to drive a mounted widget (a progress value, a status, a list).",
        parameters: json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "minLength": 1, "description": "The mount id." },
                "props": { "type": "object", "description": "Props to merge; a null value deletes the prop." },
                "code": { "type": "string", "description": "New source of the mount's inline code." },
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
        run: run_update,
    }
}

fn remove() -> AgentTool {
    AgentTool {
        name: UI_REMOVE,
        label: "Remove UI",
        description: "Removes a widget mounted with omfp_ui_mount (or a floating panel) from the workspace's views; \
                      a view left empty goes too. Its folder stays: save it with omfp_component_save first if it \
                      should be reused.",
        parameters: json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "minLength": 1, "description": "The mount or panel id." },
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
        run: run_remove,
    }
}

fn save() -> AgentTool {
    AgentTool {
        name: COMPONENT_SAVE,
        label: "Save component",
        description: "Saves a widget to the user's component library (<data-dir>/widgets/<name>/), so any \
                      workspace, thread or later session can mount it as `user:<name>`: a workspace widget \
                      (`widget`, e.g. the folder of an inline mount) or `code`. Start the entry with a one-line \
                      comment saying what it shows and which props it takes: omfp_component_list shows it. An \
                      existing component is replaced only with `replace: true`.",
        parameters: json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "minLength": 1, "description": "The component's name (letters, digits, `_`, `-`, `.` not first)." },
                "widget": { "type": "string", "minLength": 1, "description": "A widget of the workspace: `./widgets/<name>` or a bare name; exclusive with `code`." },
                "code": { "type": "string", "description": "The component's source (TSX), saved as index.tsx; exclusive with `widget`." },
                "replace": { "type": "boolean", "description": "Replace an existing component of that name." },
                "workspaceId": { "type": "string", "minLength": 1, "description": WORKSPACE_ID }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        run: run_save,
    }
}

fn list() -> AgentTool {
    AgentTool {
        name: COMPONENT_LIST,
        label: "List components",
        description: "Lists the user's component library: each `user:<name>` with its files and description. \
                      Mount one with omfp_ui_mount { widget: \"user:<name>\" }; read its source with the read tool \
                      at the folder it names.",
        parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        run: run_list,
    }
}

/// The optional string argument `key`, trimmed and not empty.
pub(super) fn text(arguments: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.trim().to_owned())),
        Some(_) => Err(format!("`{key}` must be a non-empty string")),
    }
}

/// The optional source argument `key`, as written.
fn code(arguments: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(code)) => Ok(Some(code.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

pub(super) fn required(arguments: &Map<String, Value>, key: &str) -> Result<String, String> {
    text(arguments, key)?.ok_or_else(|| format!("`{key}` must be a non-empty string"))
}

fn object(arguments: &Map<String, Value>, key: &str) -> Result<Option<Map<String, Value>>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(object)) => Ok(Some(object.clone())),
        Some(_) => Err(format!("`{key}` must be an object")),
    }
}

fn flag(arguments: &Map<String, Value>, key: &str) -> Result<bool, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// `workspaceId`, else the calling thread's workspace (AGT-066).
pub(super) fn workspace(
    context: &ToolContext,
    arguments: &Map<String, Value>,
) -> Result<String, String> {
    text(arguments, "workspaceId")?
        .or_else(|| context.workspace_id.clone())
        .ok_or_else(|| "`workspaceId` is required: no calling thread names a workspace".to_owned())
}

/// Runs `request` as the context's principal and decodes its result.
pub(super) async fn run_call<T: DeserializeOwned>(
    context: &ToolContext,
    request: Call,
) -> Result<T, String> {
    let result = call(context, request)
        .await
        .map_err(|error| error_text(&error))?;
    serde_json::from_value(result).map_err(|error| error.to_string())
}

/// Where a mount is, as a phrase.
fn placed(mounted: &Mounted) -> String {
    match &mounted.view {
        Some(view) => format!("in view {view:?}"),
        None => format!("as the floating panel {:?}", mounted.id),
    }
}

fn run_mount(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let place = match text(&arguments, "place")?.as_deref() {
            None => None,
            Some("side_tab") => Some(MountPlace::SideTab),
            Some("main_tab") => Some(MountPlace::MainTab),
            Some("end") => Some(MountPlace::End),
            Some(_) => {
                return Err("`place` must be \"side_tab\", \"main_tab\" or \"end\"".to_owned());
            }
        };
        let params = MountParams {
            workspace_id: workspace(&context, &arguments)?,
            id: required(&arguments, "id")?,
            widget: text(&arguments, "widget")?,
            code: code(&arguments, "code")?,
            props: object(&arguments, "props")?,
            view: text(&arguments, "view")?,
            place,
            panel: flag(&arguments, "panel")?,
        };
        let inline = params.code.is_some();
        let mounted: Mounted = run_call(&context, Call::ViewsMount(params)).await?;
        let mut answer = format!(
            "Mounted {} as {:?} {}{}; open windows show it now.",
            mounted.reference,
            mounted.id,
            placed(&mounted),
            if mounted.replaced {
                ", replacing the earlier mount"
            } else {
                ""
            },
        );
        if inline {
            let _ = write!(
                answer,
                " The code is .omfp/widgets/{0}/index.tsx: check it with omfp_widget_check {{ name: \"{0}\" }}.",
                mounted.id
            );
        }
        Ok(answer)
    })
}

fn run_update(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let params = MountUpdateParams {
            workspace_id: workspace(&context, &arguments)?,
            id: required(&arguments, "id")?,
            props: object(&arguments, "props")?,
            code: code(&arguments, "code")?,
        };
        let mounted: Mounted = run_call(&context, Call::ViewsMountUpdate(params)).await?;
        Ok(format!(
            "Updated {} ({} {}); open windows re-render it in place.",
            mounted.id,
            mounted.reference,
            placed(&mounted)
        ))
    })
}

fn run_remove(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let params = UnmountParams {
            workspace_id: workspace(&context, &arguments)?,
            id: required(&arguments, "id")?,
        };
        let id = params.id.clone();
        let removed: Unmounted = run_call(&context, Call::ViewsUnmount(params)).await?;
        let mut parts = Vec::new();
        if removed.panel {
            parts.push("closed its floating panel".to_owned());
        }
        if !removed.views.is_empty() {
            parts.push(format!("took it out of view(s) {}", quoted(&removed.views)));
        }
        if !removed.removed_views.is_empty() {
            parts.push(format!(
                "removed the view(s) it filled, {}",
                quoted(&removed.removed_views)
            ));
        }
        Ok(format!("Removed {id:?}: {}.", parts.join("; ")))
    })
}

fn quoted(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("{id:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn run_save(context: ToolContext, arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let widget = text(&arguments, "widget")?;
        let workspace_id = match widget {
            Some(_) => Some(workspace(&context, &arguments)?),
            None => text(&arguments, "workspaceId")?,
        };
        let params = SaveWidgetParams {
            name: required(&arguments, "name")?,
            workspace_id,
            widget,
            code: code(&arguments, "code")?,
            replace: flag(&arguments, "replace")?,
        };
        let saved: WidgetSaved = run_call(&context, Call::WidgetsSave(params)).await?;
        Ok(format!(
            "Saved {0} to the user library; any workspace can mount it with omfp_ui_mount {{ widget: \"{0}\" }}.",
            saved.reference
        ))
    })
}

fn run_list(context: ToolContext, _arguments: Map<String, Value>) -> ToolFuture {
    Box::pin(async move {
        let library: WidgetLibrary = run_call(&context, Call::WidgetsLibrary(Empty {})).await?;
        if library.components.is_empty() {
            return Ok(
                "The user library is empty: save a widget with omfp_component_save.".to_owned(),
            );
        }
        let mut answer = format!(
            "{} component(s) in the user library:",
            library.components.len()
        );
        for component in &library.components {
            let entry = component.entry.as_deref().unwrap_or("no entry file");
            let description = if component.description.is_empty() {
                "(no description)"
            } else {
                component.description.as_str()
            };
            let _ = write!(
                answer,
                "\n- {} ({entry}; files: {}): {description}",
                component.reference,
                component.files.join(", ")
            );
        }
        Ok(answer)
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::agent::FakeLauncher;
    use crate::git::{FakeGit, Git};
    use crate::paths::DataDir;
    use crate::server::{Principal, Services};
    use crate::store::Store;
    use crate::supervisor::{Supervisor, SupervisorConfig};
    use crate::threads::Threads;
    use crate::widgets::Widgets;
    use crate::workspaces::{NewWorkspace, Workspaces};

    struct Harness {
        _dir: TempDir,
        context: ToolContext,
        folder: PathBuf,
        library: PathBuf,
    }

    async fn harness() -> Harness {
        harness_with(None).await
    }

    /// A harness whose daemon serves the web bundle `static_dir` (its built-in widgets).
    async fn harness_with(static_dir: Option<&Path>) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let root = dir.path().join("workspaces");
        let store = Arc::new(Store::open(&data.db(), &root).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            Arc::new(FakeGit::new()) as Arc<dyn Git>,
            root,
        ));
        let library = data.widgets();
        let widgets = Arc::new(Widgets::new(library.clone(), static_dir));
        let supervisor = Supervisor::new(
            Arc::new(FakeLauncher::new()),
            Arc::clone(&threads),
            Arc::clone(&workspaces),
            data,
            SupervisorConfig::new("omp"),
        );
        let view = workspaces.create(NewWorkspace::new()).unwrap();
        let services = Services::new(
            store,
            threads,
            workspaces,
            supervisor,
            widgets,
            Arc::new(crate::jobs::declared::SystemClock),
        )
        .await
        .unwrap();
        Harness {
            _dir: dir,
            context: ToolContext {
                services,
                principal: Principal::Agent {
                    thread_id: "01THREAD".to_owned(),
                    client: "omp".to_owned(),
                },
                workspace_id: Some(view.workspace.id),
                tool_call_id: None,
            },
            folder: view.workspace.path,
            library,
        }
    }

    fn args(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(arguments) => arguments,
            _ => Map::new(),
        }
    }

    fn manifest(folder: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(folder.join(".omfp/views.json")).unwrap())
            .unwrap()
    }

    async fn run(
        harness: &Harness,
        tool: fn(ToolContext, Map<String, Value>) -> ToolFuture,
        arguments: Value,
    ) -> Result<String, String> {
        tool(harness.context.clone(), args(arguments)).await
    }

    // spec: AGT-061, AGT-062, AGT-063, AGT-066, PRT-080, PRT-081, PRT-082
    #[tokio::test]
    async fn an_agent_mounts_code_in_a_dashboard_updates_its_props_and_removes_it() {
        let harness = harness().await;
        let code = "// Board.\nexport default function Board() { return <p>board</p>; }\n";
        let answer = run(
            &harness,
            run_mount,
            json!({ "id": "board", "code": code, "view": "dash", "props": { "rows": 1 } }),
        )
        .await
        .unwrap();
        assert_eq!(
            answer,
            "Mounted ./widgets/board as \"board\" in view \"dash\"; open windows show it now. The code is \
             .omfp/widgets/board/index.tsx: check it with omfp_widget_check { name: \"board\" }."
        );
        assert_eq!(
            std::fs::read_to_string(harness.folder.join(".omfp/widgets/board/index.tsx")).unwrap(),
            code
        );
        assert_eq!(
            manifest(&harness.folder)["views"]["dash"]["layout"],
            json!(["./widgets/board", { "instance": "board", "rows": 1 }])
        );

        let answer = run(
            &harness,
            run_update,
            json!({ "id": "board", "props": { "rows": 2 } }),
        )
        .await
        .unwrap();
        assert_eq!(
            answer,
            "Updated board (./widgets/board in view \"dash\"); open windows re-render it in place."
        );
        assert_eq!(
            manifest(&harness.folder)["views"]["dash"]["layout"][1]["rows"],
            2
        );

        let answer = run(&harness, run_remove, json!({ "id": "board" }))
            .await
            .unwrap();
        assert_eq!(
            answer,
            "Removed \"board\": removed the view(s) it filled, \"dash\"."
        );
        assert_eq!(manifest(&harness.folder)["views"].get("dash"), None);
        assert!(
            harness
                .folder
                .join(".omfp/widgets/board/index.tsx")
                .is_file(),
            "the folder stays"
        );
    }

    // spec: DMN-183, PRT-080, PRT-081
    #[tokio::test]
    async fn code_never_overwrites_a_folder_no_mount_wrote_nor_shadows_a_built_in() {
        let bundle = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(bundle.path().join("builtin-widgets/chat")).unwrap();
        std::fs::write(
            bundle.path().join("builtin-widgets/chat/index.tsx"),
            "export default () => null;\n",
        )
        .unwrap();
        let harness = harness_with(Some(bundle.path())).await;
        let widgets = harness.folder.join(".omfp/widgets");
        let code = "export default () => null;\n";
        let conflict = |error: String, needle: &str| {
            assert!(error.starts_with("conflict: "), "{error}");
            assert!(error.contains(needle), "{error}");
        };

        let error = run(&harness, run_mount, json!({ "id": "chat", "code": code }))
            .await
            .unwrap_err();
        conflict(error, "built-in omfp:chat");
        assert!(!widgets.join("chat").exists());

        // A hand-made widget, then one with only `index.ts`.
        std::fs::create_dir_all(widgets.join("mine")).unwrap();
        std::fs::write(widgets.join("mine/index.tsx"), "// mine\n").unwrap();
        std::fs::create_dir_all(widgets.join("old")).unwrap();
        std::fs::write(widgets.join("old/index.ts"), "// old\n").unwrap();
        for id in ["mine", "old"] {
            let error = run(&harness, run_mount, json!({ "id": id, "code": code }))
                .await
                .unwrap_err();
            conflict(error, "no mount wrote");
        }
        assert_eq!(
            std::fs::read_to_string(widgets.join("mine/index.tsx")).unwrap(),
            "// mine\n"
        );
        assert!(!widgets.join("old/index.tsx").exists());
        // Mounting the hand-made widget by reference is fine; its code still cannot be replaced.
        run(
            &harness,
            run_mount,
            json!({ "id": "mine", "widget": "mine" }),
        )
        .await
        .unwrap();
        let error = run(&harness, run_update, json!({ "id": "mine", "code": code }))
            .await
            .unwrap_err();
        conflict(error.clone(), "no mount wrote");
        // An update cannot choose another id.
        assert!(
            error.contains("cannot update the code of mount mine"),
            "{error}"
        );
        assert!(!error.contains("choose another id"), "{error}");

        // The mount's own folder: written with the marker, replaced (and said so) by the next mount with code.
        let mounted = harness
            .context
            .services
            .mount(
                serde_json::from_value(json!({
                    "workspaceId": harness.context.workspace_id, "id": "board", "code": code
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(!mounted.replaced);
        assert!(
            widgets
                .join("board")
                .join(crate::widgets::folders::MOUNT_MARKER)
                .is_file()
        );
        run(&harness, run_remove, json!({ "id": "board" }))
            .await
            .unwrap();
        let again = harness
            .context
            .services
            .mount(
                serde_json::from_value(json!({
                    "workspaceId": harness.context.workspace_id, "id": "board", "code": "// v2\n"
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(again.replaced, "the folder's code was replaced");
        assert_eq!(
            std::fs::read_to_string(widgets.join("board/index.tsx")).unwrap(),
            "// v2\n"
        );
        // Another entry file next to ours would shadow or be shadowed.
        std::fs::write(widgets.join("board/index.js"), "// js\n").unwrap();
        let error = run(&harness, run_update, json!({ "id": "board", "code": code }))
            .await
            .unwrap_err();
        conflict(error, "also has index.js");
    }

    // spec: AGT-060, AGT-061, AGT-063, PRT-080
    #[tokio::test]
    async fn a_floating_panel_mount_and_its_errors() {
        let harness = harness().await;
        let answer = run(
            &harness,
            run_mount,
            json!({ "id": "clock", "widget": "user:clock", "panel": true, "props": { "title": "Clock" } }),
        )
        .await
        .unwrap();
        assert_eq!(
            answer,
            "Mounted user:clock as \"clock\" as the floating panel \"clock\"; open windows show it now."
        );
        assert_eq!(
            manifest(&harness.folder)["panels"],
            json!({ "clock": ["user:clock", { "instance": "clock", "title": "Clock" }] })
        );

        for (arguments, reason) in [
            (
                json!({ "id": "a b", "widget": "x" }),
                "bad_request: invalid mount id",
            ),
            (
                json!({ "id": "x" }),
                "bad_request: give exactly one of `widget` and `code`",
            ),
            (
                json!({ "id": "x", "widget": "y", "code": "" }),
                "bad_request: give exactly one of `widget` and `code`",
            ),
            (
                json!({ "id": "x", "widget": "y", "panel": true, "view": "v" }),
                "bad_request: a panel floats over every view: `view` and `place` do not apply",
            ),
            (
                json!({ "id": "x", "widget": "y", "place": "side-tab" }),
                "`place` must be \"side_tab\", \"main_tab\" or \"end\"",
            ),
            (
                json!({ "id": "x", "widget": "y", "props": [] }),
                "`props` must be an object",
            ),
        ] {
            let error = run(&harness, run_mount, arguments.clone())
                .await
                .unwrap_err();
            assert!(error.contains(reason), "{arguments}: {error}");
        }
        let error = run(&harness, run_update, json!({ "id": "nope", "props": {} }))
            .await
            .unwrap_err();
        assert_eq!(error, "not_found: nothing is mounted with the id nope");
        let error = run(&harness, run_update, json!({ "id": "clock", "code": "x" }))
            .await
            .unwrap_err();
        assert!(error.contains("not inline code"), "{error}");

        let answer = run(&harness, run_remove, json!({ "id": "clock" }))
            .await
            .unwrap();
        assert_eq!(answer, "Removed \"clock\": closed its floating panel.");

        std::fs::write(harness.folder.join(".omfp/views.json"), "{").unwrap();
        let error = run(&harness, run_mount, json!({ "id": "x", "widget": "y" }))
            .await
            .unwrap_err();
        assert!(
            error.starts_with("bad_request: .omfp/views.json is invalid ("),
            "{error}"
        );
    }

    // spec: AGT-061, PRT-080
    #[tokio::test]
    async fn the_tool_places_with_the_methods_words() {
        let harness = harness().await;
        // A view without the `side` tabs: the tool and the method name the same argument and values.
        std::fs::create_dir_all(harness.folder.join(".omfp")).unwrap();
        std::fs::write(
            harness.folder.join(".omfp/views.json"),
            r#"{ "default": "a", "views": { "a": { "layout": ["row", {}, "omfp:chat"] } } }"#,
        )
        .unwrap();
        let error = run(&harness, run_mount, json!({ "id": "x", "widget": "y" }))
            .await
            .unwrap_err();
        assert!(error.contains("use place: \"end\""), "{error}");
        assert_eq!(
            mount().parameters["properties"]["place"]["enum"],
            json!(["side_tab", "main_tab", "end"]),
            "views.mount's MountPlace values"
        );
        let mount = |params: Value| {
            let mut params = params;
            params["workspaceId"] = json!(harness.context.workspace_id);
            harness
                .context
                .services
                .mount(serde_json::from_value(params).unwrap())
        };
        let error = mount(json!({ "id": "x", "widget": "y" }))
            .await
            .unwrap_err();
        assert!(
            error.message.contains("use place: \"end\""),
            "{}",
            error.message
        );
        let error = mount(json!({ "id": "x", "widget": "y", "panel": true, "place": "end" }))
            .await
            .unwrap_err();
        assert!(
            error.message.contains("`view` and `place` do not apply"),
            "{}",
            error.message
        );
        let answer = run(
            &harness,
            run_mount,
            json!({ "id": "z", "widget": "y", "place": "end" }),
        )
        .await
        .unwrap();
        assert!(
            answer.starts_with("Mounted ./widgets/y as \"z\" in view \"a\""),
            "{answer}"
        );
    }

    // spec: AGT-066
    #[test]
    fn the_registry_offers_the_tools_and_the_widget_docs_explain_them() {
        let names: Vec<&str> = super::super::registry()
            .iter()
            .map(|tool| tool.name)
            .collect();
        for name in [
            UI_MOUNT,
            UI_UPDATE,
            UI_REMOVE,
            COMPONENT_SAVE,
            COMPONENT_LIST,
        ] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
            assert!(
                crate::widgets::DOCS.contains(name),
                "{name} missing from the docs"
            );
        }
        for needle in [
            "## Mounts, panels and the library",
            "\"panels\"",
            "views.mount",
        ] {
            assert!(
                crate::widgets::DOCS.contains(needle),
                "{needle} missing from the docs"
            );
        }
        for tool in tools() {
            assert_eq!(tool.parameters["type"], "object", "{}", tool.name);
            assert_eq!(
                tool.parameters["additionalProperties"], false,
                "{}",
                tool.name
            );
        }
    }

    // spec: AGT-064, AGT-065, AGT-066, PRT-083, PRT-084
    #[tokio::test]
    async fn components_are_saved_from_a_mount_or_code_and_listed() {
        let harness = harness().await;
        assert_eq!(
            run(&harness, run_list, json!({})).await.unwrap(),
            "The user library is empty: save a widget with omfp_component_save."
        );
        run(
            &harness,
            run_mount,
            json!({ "id": "board", "code": "// A board; props: rows.\nexport default () => null;\n" }),
        )
        .await
        .unwrap();
        let answer = run(
            &harness,
            run_save,
            json!({ "name": "board", "widget": "board" }),
        )
        .await
        .unwrap();
        assert_eq!(
            answer,
            "Saved user:board to the user library; any workspace can mount it with omfp_ui_mount { widget: \
             \"user:board\" }."
        );
        let mut context = harness.context.clone();
        context.workspace_id = None;
        run_save(
            context.clone(),
            args(
                json!({ "name": "clock", "code": "/* A clock. */\nexport default () => null;\n" }),
            ),
        )
        .await
        .unwrap();
        let error = run_save(context, args(json!({ "name": "x", "widget": "board" })))
            .await
            .unwrap_err();
        assert!(error.contains("`workspaceId` is required"), "{error}");
        let error = run(&harness, run_save, json!({ "name": "clock", "code": "" }))
            .await
            .unwrap_err();
        assert!(error.starts_with("conflict: "), "{error}");

        assert_eq!(
            run(&harness, run_list, json!({})).await.unwrap(),
            "2 component(s) in the user library:\n- user:board (index.tsx; files: index.tsx): A board; props: \
             rows.\n- user:clock (index.tsx; files: index.tsx): A clock."
        );
        assert!(harness.library.join("board/index.tsx").is_file());
    }
}
