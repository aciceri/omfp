//! L2: views and widgets over the real daemon (`start_with`, real router, fake launcher and git) — the `views/<id>`
//! topic, `views.set`, `widgets.copy`, `widgets.state_set`, the mounts and the component library (`views.mount`,
//! `views.mount_update`, `views.unmount`, `widgets.save`, `widgets.library`), and widget files served under
//! `widgetBase`.

mod client;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use client::Client;
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon on port 0 over `data`; `static_dir` is its web bundle; it follows the user init file `<data>/init.ts`.
async fn start(data: &Path, static_dir: Option<&Path>) -> DaemonHandle {
    let workspaces = data.join("workspaces");
    let mut args = vec![
        "daemon".to_owned(),
        "--bind".into(),
        "127.0.0.1:0".into(),
        "--data-dir".into(),
        data.display().to_string(),
        "--workspaces-root".into(),
        workspaces.display().to_string(),
    ];
    if let Some(dir) = static_dir {
        args.extend(["--static-dir".into(), dir.display().to_string()]);
    }
    let mut config = Config::from_cli(Cli::try_parse_from(args).unwrap(), None, None).unwrap();
    config.init_file = Some(data.join("init.ts"));
    let ports = Ports {
        launcher: Arc::new(FakeLauncher::new()),
        git: Arc::new(FakeGit::new()),
        omp_program: "omp".into(),
        omp_version: omfp_core::OMP_VERSION.into(),
        omp_env: Vec::new(),
        user_omp_env: omfp_testkit::absent_standalone_omp(data),
        warnings: Vec::new(),
        clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
    };
    start_with(config, ports).await.unwrap()
}

struct Daemon {
    handle: DaemonHandle,
    data: TempDir,
    /// The web bundle, when the daemon serves one.
    bundle: Option<TempDir>,
}

impl Daemon {
    async fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let handle = start(data.path(), None).await;
        Self {
            handle,
            data,
            bundle: None,
        }
    }

    /// A daemon serving a bundle with the built-in widget `workspace` and the export list of the import map.
    async fn with_bundle() -> Self {
        let data = tempfile::tempdir().unwrap();
        let bundle = tempfile::tempdir().unwrap();
        write(
            &bundle.path().join("builtin-widgets/workspace/index.tsx"),
            "import { Card } from \"omfp/kit\";\nimport { Repos } from \"./repos\";\n\
             export default function Workspace() { return <Card><Repos /></Card>; }\n",
        );
        write(
            &bundle.path().join("builtin-widgets/workspace/repos.tsx"),
            "export function Repos() { return null; }\n",
        );
        write(
            &bundle.path().join("widget-exports.json"),
            r#"{ "react": ["createElement", "useState"], "react/jsx-runtime": ["jsx", "jsxs", "Fragment"],
                 "react-dom": ["createPortal"], "omfp/sdk": ["useJob", "useStored"],
                 "omfp/kit": ["Card", "Log", "Progress"] }"#,
        );
        let handle = start(data.path(), Some(bundle.path())).await;
        Self {
            handle,
            data,
            bundle: Some(bundle),
        }
    }

    async fn client(&self) -> Client {
        Client::hello(self.handle.addr(), self.handle.token().expose(), "test").await
    }

    /// `widgetBase` from `system.hello` (PRT-042).
    async fn widget_base(&self) -> String {
        let mut client = Client::connect(self.handle.addr()).await;
        let hello = client
            .call(
                "system.hello",
                json!({ "token": self.handle.token().expose(), "protocol": 1, "client": "test" }),
            )
            .await
            .unwrap();
        hello["widgetBase"].as_str().unwrap().to_owned()
    }

    /// The user init file the daemon follows (DMN-185).
    fn user_init(&self) -> PathBuf {
        self.data.path().join("init.ts")
    }

    fn user_library(&self) -> PathBuf {
        self.data.path().join("widgets")
    }

    /// Restarts the daemon on the same data directory.
    async fn restart(self) -> Self {
        let bundle = self.bundle.as_ref().map(|dir| dir.path().to_owned());
        self.handle.shutdown().await.unwrap();
        let handle = start(self.data.path(), bundle.as_deref()).await;
        Self { handle, ..self }
    }

    /// `GET path` over HTTP/1.1: status, headers (lowercase names) and body.
    async fn get(&self, path: &str) -> (u16, Vec<(String, String)>, String) {
        let mut stream = tokio::net::TcpStream::connect(self.handle.addr())
            .await
            .unwrap();
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response)
            .await
            .unwrap();
        let response = String::from_utf8_lossy(&response).into_owned();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let mut lines = head.lines();
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|line| line.split_once(": "))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
            .collect();
        (status, headers, body.to_owned())
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A new workspace: its id and folder.
async fn workspace(client: &mut Client) -> (String, PathBuf) {
    let ws = client.ok("workspaces.create", json!({})).await;
    (
        ws["id"].as_str().unwrap().to_owned(),
        PathBuf::from(ws["path"].as_str().unwrap()),
    )
}

/// Subscribes to `views/<id>`; returns the snapshot state.
async fn views(client: &mut Client, id: &str) -> Value {
    let result = client
        .ok("subscribe", json!({ "topic": format!("views/{id}") }))
        .await;
    assert_eq!(result["mode"], "snapshot");
    result["state"].clone()
}

/// The next event of `views/<id>`.
async fn views_event(client: &mut Client, id: &str) -> Value {
    client.event(&format!("views/{id}")).await["event"].clone()
}

fn names(widgets: &Value) -> Vec<(String, String)> {
    widgets
        .as_array()
        .unwrap()
        .iter()
        .map(|widget| {
            (
                widget["source"].as_str().unwrap().to_owned(),
                widget["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

// spec: DMN-082, PRT-040, AGT-011, AGT-012, AGT-023
#[tokio::test(flavor = "multi_thread")]
async fn the_views_topic_carries_the_manifest_or_its_error_and_the_workspace_then_user_widgets() {
    let daemon = Daemon::new().await;
    let mut client = daemon.client().await;
    let error = client
        .call("subscribe", json!({ "topic": "views/NOPE" }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");

    let (id, folder) = workspace(&mut client).await;
    write(
        &folder.join(".omfp/views.json"),
        r#"{ "views": { "a": { "layout": ["row", {}] } } }"#,
    );
    write(
        &folder.join(".omfp/widgets/tests/index.tsx"),
        "export default 1;",
    );
    write(&folder.join(".omfp/widgets/notes/readme.md"), "no entry");
    write(
        &daemon.user_library().join("charts/index.js"),
        "export default 1;",
    );

    let state = views(&mut client, &id).await;

    assert_eq!(state["manifest"], Value::Null);
    assert_eq!(state["error"], "views.a.layout: row has no children");
    assert_eq!(
        names(&state["widgets"]),
        [
            ("workspace".to_owned(), "notes".to_owned()),
            ("workspace".to_owned(), "tests".to_owned()),
            ("user".to_owned(), "charts".to_owned()),
        ]
    );
    assert_eq!(state["widgets"][0]["entry"], Value::Null);
    assert_eq!(state["widgets"][1]["entry"], "index.tsx");
    assert_eq!(state["widgets"][2]["entry"], "index.js");
    assert_eq!(state["stored"], json!({}));

    // A fixed file and a saved widget reach the topic on the next rescan (DMN-023), each change once.
    let version = state["widgets"][1]["version"].clone();
    write(
        &folder.join(".omfp/views.json"),
        r#"{ "views": { "a": { "layout": "omfp:chat" } } }"#,
    );
    write(
        &folder.join(".omfp/widgets/tests/index.tsx"),
        "export default 22;",
    );
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;
    let manifest = views_event(&mut client, &id).await;
    assert_eq!(
        manifest,
        json!({ "kind": "views_manifest", "manifest": { "views": { "a": { "layout": "omfp:chat" } } }, "error": null })
    );
    let widgets = views_event(&mut client, &id).await;
    assert_eq!(widgets["kind"], "views_widgets");
    assert_ne!(widgets["widgets"][1]["version"], version, "AGT-013");
}

// spec: DMN-088, AGT-010, PRT-043
#[tokio::test(flavor = "multi_thread")]
async fn a_new_workspace_has_the_default_views_and_a_window_reports_its_widgets() {
    let daemon = Daemon::new().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    assert_eq!(
        std::fs::read_to_string(folder.join(".omfp/views.json")).unwrap(),
        omfp_daemon::widgets::DEFAULT_MANIFEST
    );
    let state = views(&mut client, &id).await;
    assert_eq!(state["manifest"], omfp_daemon::widgets::default_manifest());
    assert_eq!(state["error"], Value::Null);

    // A window's report is accepted on its own connection (session plumbing, no `Manage` needed).
    let report = json!({
        "workspaceId": id,
        "views": { "epoch": "e", "seq": 1 },
        "widgets": [{
            "reference": "./widgets/tests", "instance": "./widgets/tests", "source": "workspace", "name": "tests",
            "version": null, "state": "loading", "errors": [], "console": [], "text": "", "jobs": []
        }]
    });
    assert_eq!(client.ok("widgets.report", report).await, json!({}));
    assert_eq!(
        client
            .ok(
                "widgets.report",
                json!({ "workspaceId": null, "views": null, "widgets": [] })
            )
            .await,
        json!({})
    );
    let error = client
        .call(
            "widgets.report",
            json!({ "workspaceId": id, "widgets": [{ "state": "melted" }] }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
}

// spec: PRT-115, PRT-116, DMN-195
#[tokio::test(flavor = "multi_thread")]
async fn a_windows_file_errors_are_kept_and_views_errors_lists_them() {
    let daemon = Daemon::new().await;
    let mut client = daemon.client().await;
    let (id, _) = workspace(&mut client).await;
    let mut reader = daemon.client().await;
    assert_eq!(
        reader
            .ok("views.errors", json!({ "workspaceId": id }))
            .await,
        json!({ "windows": [] })
    );
    let error =
        json!({ "kind": "view", "file": ".omfp/views/ops.tsx", "message": "ops.tsx:2:3: boom" });
    let report = json!({ "workspaceId": id, "views": null, "widgets": [], "fileErrors": [error] });
    assert_eq!(client.ok("widgets.report", report).await, json!({}));
    let listed = reader
        .ok("views.errors", json!({ "workspaceId": id }))
        .await;
    assert_eq!(
        listed["windows"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );
    assert_eq!(listed["windows"][0]["errors"], json!([error]));
    // A report without `fileErrors` (an older window) has none.
    let report = json!({ "workspaceId": id, "views": null, "widgets": [] });
    assert_eq!(client.ok("widgets.report", report).await, json!({}));
    let listed = reader
        .ok("views.errors", json!({ "workspaceId": id }))
        .await;
    assert_eq!(listed["windows"][0]["errors"], json!([]), "{listed}");

    let missing = reader
        .call("views.errors", json!({ "workspaceId": "01NOPE" }))
        .await
        .unwrap_err();
    assert_eq!(missing["code"], "not_found");
}

// spec: DMN-083, AGT-010
#[tokio::test(flavor = "multi_thread")]
async fn views_set_checks_writes_sorted_json_and_publishes_before_answering() {
    let daemon = Daemon::new().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    views(&mut client, &id).await;

    let error = client
        .call(
            "views.set",
            json!({ "workspaceId": id, "manifest": { "views": { "a": { "layout": [] } } } }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
    assert_eq!(
        error["message"],
        "views.a.layout: expected a widget name, [widget, props] or [container, props, ...children]"
    );
    let error = client
        .call(
            "views.set",
            json!({ "workspaceId": "NOPE", "manifest": { "views": { "a": { "layout": "omfp:chat" } } } }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");

    let manifest = json!({
        "views": { "work": { "title": "Work", "layout": ["row", { "sizes": [3, 1] }, "omfp:chat", "omfp:workspace"] } },
        "default": "work",
        "$schema": "omfp://schemas/views.json"
    });
    client
        .ok(
            "views.set",
            json!({ "workspaceId": id, "manifest": manifest }),
        )
        .await;

    // Published before the answer: a window subscribing now sees it; the subscribed one gets the event.
    let mut other = daemon.client().await;
    assert_eq!(views(&mut other, &id).await["manifest"], manifest);
    let event = views_event(&mut client, &id).await;
    assert_eq!(event["kind"], "views_manifest");
    assert_eq!(event["manifest"], manifest);
    assert_eq!(
        std::fs::read_to_string(folder.join(".omfp/views.json")).unwrap(),
        "{\n  \"$schema\": \"omfp://schemas/views.json\",\n  \"default\": \"work\",\n  \"views\": {\n    \"work\": {\n      \
         \"layout\": [\n        \"row\",\n        {\n          \"sizes\": [\n            3,\n            1\n          ]\n        },\n        \
         \"omfp:chat\",\n        \"omfp:workspace\"\n      ],\n      \"title\": \"Work\"\n    }\n  }\n}\n"
    );
}

// spec: DMN-086, AGT-018, PRT-040
#[tokio::test(flavor = "multi_thread")]
async fn widget_state_is_published_kept_across_restarts_and_bounded() {
    let daemon = Daemon::new().await;
    let mut client = daemon.client().await;
    let (id, _) = workspace(&mut client).await;
    views(&mut client, &id).await;
    let set = |instance: &str, key: &str, value: Value| json!({ "workspaceId": id, "instance": instance, "key": key, "value": value });

    client
        .ok(
            "widgets.state_set",
            set("tests", "job:make test", json!("J1")),
        )
        .await;
    assert_eq!(
        views_event(&mut client, &id).await,
        json!({ "kind": "widget_stored", "instance": "tests", "key": "job:make test", "value": "J1" })
    );
    client
        .ok(
            "widgets.state_set",
            set("omfp:chat", "draft", json!({ "text": "hi" })),
        )
        .await;
    client
        .ok("widgets.state_set", set("omfp:chat", "draft", Value::Null))
        .await;
    let too_big = "x".repeat(256 * 1024);
    let error = client
        .call("widgets.state_set", set("tests", "log", json!(too_big)))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
    let error = client
        .call(
            "widgets.state_set",
            json!({ "workspaceId": "NOPE", "instance": "i", "key": "k", "value": 1 }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
    let state = views(&mut client, &id).await;
    assert_eq!(
        state["stored"],
        json!({ "tests": { "job:make test": "J1" } })
    );
    drop(client);

    let daemon = daemon.restart().await;
    let mut client = daemon.client().await;
    let state = views(&mut client, &id).await;
    assert_eq!(
        state["stored"],
        json!({ "tests": { "job:make test": "J1" } }),
        "AGT-018"
    );

    // Removing the workspace deletes its state.
    client
        .ok(
            "workspaces.remove",
            json!({ "workspaceId": id, "force": true }),
        )
        .await;
    let error = client
        .call("subscribe", json!({ "topic": format!("views/{id}") }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
}

// spec: DMN-087, AGT-021, AGT-023
#[tokio::test(flavor = "multi_thread")]
async fn widgets_copy_copies_built_ins_and_user_widgets_into_the_workspace() {
    let daemon = Daemon::with_bundle().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    write(
        &daemon.user_library().join("charts/index.ts"),
        "export default 1;\n",
    );
    write(
        &daemon.user_library().join("charts/lib/axis.ts"),
        "export const axis = 1;\n",
    );
    views(&mut client, &id).await;

    let copied = client
        .ok(
            "widgets.copy",
            json!({ "workspaceId": id, "widget": "omfp:workspace" }),
        )
        .await;
    assert_eq!(copied, json!({ "name": "workspace" }));
    let mut other = daemon.client().await;
    assert_eq!(
        names(&views(&mut other, &id).await["widgets"])[0],
        ("workspace".to_owned(), "workspace".to_owned()),
        "published before the answer"
    );
    let event = views_event(&mut client, &id).await;
    assert_eq!(event["kind"], "views_widgets");
    assert_eq!(event["widgets"][0]["name"], "workspace");
    let bundle = daemon.bundle.as_ref().unwrap().path();
    for file in ["index.tsx", "repos.tsx"] {
        assert_eq!(
            std::fs::read(folder.join(".omfp/widgets/workspace").join(file)).unwrap(),
            std::fs::read(bundle.join("builtin-widgets/workspace").join(file)).unwrap()
        );
    }

    let copied = client
        .ok(
            "widgets.copy",
            json!({ "workspaceId": id, "widget": "user:charts", "name": "charts2" }),
        )
        .await;
    assert_eq!(copied, json!({ "name": "charts2" }));
    assert_eq!(
        std::fs::read_to_string(folder.join(".omfp/widgets/charts2/lib/axis.ts")).unwrap(),
        "export const axis = 1;\n"
    );
    let state = views(&mut client, &id).await;
    assert_eq!(
        names(&state["widgets"]),
        [
            ("workspace".to_owned(), "charts2".to_owned()),
            ("workspace".to_owned(), "workspace".to_owned()),
            ("user".to_owned(), "charts".to_owned()),
        ]
    );

    for (params, code) in [
        (
            json!({ "workspaceId": id, "widget": "omfp:workspace" }),
            "conflict",
        ),
        (
            json!({ "workspaceId": id, "widget": "omfp:nothing" }),
            "not_found",
        ),
        (
            json!({ "workspaceId": id, "widget": "user:nothing" }),
            "not_found",
        ),
        (
            json!({ "workspaceId": "NOPE", "widget": "omfp:workspace" }),
            "not_found",
        ),
        (
            json!({ "workspaceId": id, "widget": "omfp:workspace", "name": "../up" }),
            "bad_request",
        ),
        (
            json!({ "workspaceId": id, "widget": "omfp:workspace", "name": ".hidden" }),
            "bad_request",
        ),
        (
            json!({ "workspaceId": id, "widget": "./widgets/charts2" }),
            "bad_request",
        ),
    ] {
        let error = client
            .call("widgets.copy", params.clone())
            .await
            .unwrap_err();
        assert_eq!(error["code"], code, "{params}");
    }
}

// spec: DMN-087
#[tokio::test(flavor = "multi_thread")]
async fn without_a_bundle_there_are_no_built_ins_to_copy() {
    let plain = Daemon::new().await;
    let mut client = plain.client().await;
    let (id, _) = workspace(&mut client).await;
    let error = client
        .call(
            "widgets.copy",
            json!({ "workspaceId": id, "widget": "omfp:workspace" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");
}

// spec: DMN-084, PRT-042, AGT-012
#[tokio::test(flavor = "multi_thread")]
async fn widget_files_are_served_with_the_key_transpiled_and_never_cached() {
    let daemon = Daemon::with_bundle().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    let widget = folder.join(".omfp/widgets/tests");
    write(
        &widget.join("index.tsx"),
        "import { Card } from \"omfp/kit\";\nimport { label } from \"./lib/label\";\n\
         type Props = { command: string };\n\
         export default function Tests({ command }: Props) { return <Card title={label(command)} />; }\n",
    );
    write(
        &widget.join("lib/label.ts"),
        "export const label = (c: string): string => c;\n",
    );
    write(&widget.join("plain.js"), "export const plain = 1;\n");
    write(&widget.join("data.json"), "{\"a\":1}");
    let base = daemon.widget_base().await;
    assert!(base.starts_with("/widgets/"), "{base}");
    assert_eq!(base.len(), "/widgets/".len() + 32, "128-bit key in hex");

    let (status, headers, body) = daemon.get(&format!("{base}/{id}/tests/v1/index.tsx")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        header(&headers, "content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    assert!(body.contains("from \"react/jsx-runtime\""), "{body}");
    assert!(body.contains("from \"./lib/label\""), "{body}");
    assert!(!body.contains("Props"), "types stripped: {body}");
    assert!(
        body.contains("//# sourceMappingURL=data:application/json"),
        "{body}"
    );

    // A path without extension resolves to the first of .tsx, .ts, .jsx, .js; any version serves the current files.
    let (status, _, body) = daemon
        .get(&format!("{base}/{id}/tests/other/lib/label"))
        .await;
    assert_eq!(status, 200);
    assert!(body.contains("export const label = (c) => c;"), "{body}");
    let (status, headers, body) = daemon.get(&format!("{base}/{id}/tests/v1/plain.js")).await;
    assert_eq!((status, body.as_str()), (200, "export const plain = 1;\n"));
    assert_eq!(
        header(&headers, "content-type"),
        Some("text/javascript; charset=utf-8")
    );
    let (status, headers, _) = daemon.get(&format!("{base}/{id}/tests/v1/data.json")).await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), Some("application/json"));
}

// spec: DMN-084
#[tokio::test(flavor = "multi_thread")]
async fn wrong_keys_unknown_sources_missing_files_and_paths_leaving_the_folder_are_404() {
    let daemon = Daemon::new().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    let widgets = folder.join(".omfp/widgets");
    write(&widgets.join("tests/index.js"), "export default 1;\n");
    write(&folder.join("secret.js"), "export const secret = 1;\n");
    std::os::unix::fs::symlink(folder.join("secret.js"), widgets.join("tests/escape.js")).unwrap();
    std::os::unix::fs::symlink(
        widgets.join("tests/index.js"),
        widgets.join("tests/inside.js"),
    )
    .unwrap();
    let base = daemon.widget_base().await;
    assert_eq!(
        daemon.get(&format!("{base}/{id}/tests/v/index.js")).await.0,
        200
    );
    assert_eq!(
        daemon
            .get(&format!("{base}/{id}/tests/v/inside.js"))
            .await
            .0,
        200
    );

    let other_key = format!("/widgets/{}", "0".repeat(32));
    for path in [
        format!("{other_key}/{id}/tests/v/index.js"),
        format!("/widgets/short/{id}/tests/v/index.js"),
        format!("{base}/NOPE/tests/v/index.js"),
        format!("{base}/{id}/nothing/v/index.js"),
        format!("{base}/{id}/tests/v/missing.js"),
        format!("{base}/{id}/tests/v/escape.js"),
        format!("{base}/{id}/tests/v/../../../secret.js"),
        format!("{base}/{id}/tests/v/%2E%2E/%2E%2E/%2E%2E/secret.js"),
        format!("{base}/{id}/..%2F..%2Fsecret.js/v/x.js"),
        format!("{base}/{id}/tests/v"),
    ] {
        let (status, headers, _) = daemon.get(&path).await;
        assert_eq!(status, 404, "{path}");
        assert_eq!(
            header(&headers, "cache-control"),
            Some("no-store"),
            "{path}"
        );
    }
}

// spec: DMN-085, AGT-014
#[tokio::test(flavor = "multi_thread")]
async fn a_widget_that_does_not_compile_is_replaced_by_a_stand_in_until_fixed() {
    let daemon = Daemon::with_bundle().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    let widget = folder.join(".omfp/widgets/w");
    write(
        &widget.join("index.tsx"),
        "import { b } from \"./lib/b\";\nexport default () => <p>{b}</p>;\n",
    );
    write(&widget.join("lib/b.ts"), "export const b = ;\n");
    write(
        &widget.join("lib/c.ts"),
        "import _ from \"lodash\";\nexport const c = _;\n",
    );
    let base = daemon.widget_base().await;
    let entry = format!("{base}/{id}/w/v/index.tsx");
    let compile_error = |body: &str| -> Value {
        let start = body.find("export const compileError = ").unwrap()
            + "export const compileError = ".len();
        let end = start + body[start..].find(";\n").unwrap();
        serde_json::from_str(&body[start..end]).unwrap()
    };

    // The entry stands in for the first failing file in path order, here a syntax error (status 200).
    let (status, headers, body) = daemon.get(&entry).await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    assert!(
        body.starts_with("import { createElement } from \"react\";"),
        "{body}"
    );
    assert!(
        body.contains("export default function CompileError()"),
        "{body}"
    );
    let error = compile_error(&body);
    assert_eq!(
        (&error["file"], &error["line"], &error["column"]),
        (&json!("lib/b.ts"), &json!(1), &json!(18))
    );

    // A file served on its own gets its own stand-in: a forbidden specifier.
    let (status, _, body) = daemon.get(&format!("{base}/{id}/w/v/lib/c.ts")).await;
    assert_eq!(status, 200);
    let error = compile_error(&body);
    assert_eq!(error["file"], "lib/c.ts");
    assert!(
        error["message"].as_str().unwrap().contains("\"lodash\""),
        "{error}"
    );

    // The next save of the first file shows the next failure; a name the kit does not export fails too.
    write(&widget.join("lib/b.ts"), "export const b = 1;\n");
    let error = compile_error(&daemon.get(&entry).await.2);
    assert_eq!(error["file"], "lib/c.ts");
    write(
        &widget.join("lib/c.ts"),
        "import { Gauge } from \"omfp/kit\";\nexport const c = Gauge;\n",
    );
    let error = compile_error(&daemon.get(&entry).await.2);
    assert_eq!(error["message"], "omfp/kit has no export named Gauge");

    // Fixed: the entry is the widget again.
    write(
        &widget.join("lib/c.ts"),
        "import { Card } from \"omfp/kit\";\nexport const c = Card;\n",
    );
    let (status, _, body) = daemon.get(&entry).await;
    assert_eq!(status, 200);
    assert!(!body.contains("compileError"), "{body}");
    assert!(body.contains("from \"./lib/b\""), "{body}");
}

// spec: AGT-023, DMN-084
#[tokio::test(flavor = "multi_thread")]
async fn user_library_widgets_are_served_under_the_user_source() {
    let daemon = Daemon::new().await;
    write(
        &daemon.user_library().join("charts/index.jsx"),
        "export default function Charts() { return <svg />; }\n",
    );
    let base = daemon.widget_base().await;

    let (status, _, body) = daemon.get(&format!("{base}/user/charts/v/index.jsx")).await;

    assert_eq!(status, 200);
    assert!(body.contains("from \"react/jsx-runtime\""), "{body}");
    assert_eq!(
        daemon
            .get(&format!("{base}/user/nothing/v/index.jsx"))
            .await
            .0,
        404
    );
}

// spec: PRT-080, PRT-081, PRT-082, PRT-083, PRT-084, AGT-060, AGT-061, AGT-064
#[tokio::test(flavor = "multi_thread")]
async fn mounts_publish_the_manifest_before_answering_and_saved_components_reach_every_window() {
    let daemon = Daemon::new().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    views(&mut client, &id).await;

    let mounted = client
        .ok(
            "views.mount",
            json!({
                "workspaceId": id,
                "id": "note",
                "code": "// A note.\nexport default function Note() { return <p>note</p>; }\n",
                "panel": true,
                "props": { "title": "Note" }
            }),
        )
        .await;
    assert_eq!(
        mounted,
        json!({ "id": "note", "reference": "./widgets/note", "view": null, "panel": true, "replaced": false })
    );
    // Published before the answer: the manifest and the new widget folder (a poll round may publish the folder
    // first).
    let mut events = [
        views_event(&mut client, &id).await,
        views_event(&mut client, &id).await,
    ];
    events.sort_by_key(|event| event["kind"].as_str().unwrap_or_default().to_owned());
    assert_eq!(events[0]["kind"], "views_manifest");
    assert_eq!(
        events[0]["manifest"]["panels"],
        json!({ "note": ["./widgets/note", { "instance": "note", "title": "Note" }] })
    );
    assert_eq!(events[1]["kind"], "views_widgets");
    assert_eq!(
        names(&events[1]["widgets"]),
        vec![("workspace".to_owned(), "note".to_owned())]
    );

    let updated = client
        .ok(
            "views.mount_update",
            json!({ "workspaceId": id, "id": "note", "props": { "title": "Notes" } }),
        )
        .await;
    assert_eq!(updated["replaced"], true);
    let event = views_event(&mut client, &id).await;
    assert_eq!(event["manifest"]["panels"]["note"][1]["title"], "Notes");

    saves_the_mounted_widget_to_the_user_library(&daemon, &mut client, &id).await;

    let removed = client
        .ok("views.unmount", json!({ "workspaceId": id, "id": "note" }))
        .await;
    assert_eq!(
        removed,
        json!({ "views": [], "removedViews": [], "panel": true })
    );
    let event = views_event(&mut client, &id).await;
    assert_eq!(event["manifest"].get("panels"), None);
    let error = client
        .call("views.unmount", json!({ "workspaceId": id, "id": "note" }))
        .await
        .unwrap_err();
    assert_eq!(error["code"], "not_found");

    write(&folder.join(".omfp/views.json"), "{");
    let error = client
        .call(
            "views.mount",
            json!({ "workspaceId": id, "id": "x", "widget": "user:note" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "bad_request");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with(".omfp/views.json is invalid ("),
        "{error}"
    );
}

/// Saves `./widgets/note` to the user library: every followed views topic lists it, and the library describes it.
async fn saves_the_mounted_widget_to_the_user_library(
    daemon: &Daemon,
    client: &mut Client,
    id: &str,
) {
    let saved = client
        .ok(
            "widgets.save",
            json!({ "name": "note", "workspaceId": id, "widget": "./widgets/note" }),
        )
        .await;
    assert_eq!(saved, json!({ "name": "note", "reference": "user:note" }));
    let event = views_event(client, id).await;
    assert_eq!(
        names(&event["widgets"]),
        vec![
            ("workspace".to_owned(), "note".to_owned()),
            ("user".to_owned(), "note".to_owned())
        ]
    );
    let library = client.ok("widgets.library", json!({})).await;
    assert_eq!(library["components"][0]["reference"], "user:note");
    assert_eq!(library["components"][0]["description"], "A note.");
    assert!(daemon.user_library().join("note/index.tsx").is_file());
    let error = client
        .call(
            "widgets.save",
            json!({ "name": "note", "code": "export default () => null;\n" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "conflict");
}

fn is_version(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|v| v.len() == 16 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}

// spec: DMN-185, PRT-095
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_init_file_is_versioned_on_the_views_topic_and_served_compiled() {
    let daemon = Daemon::with_bundle().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    let base = daemon.widget_base().await;
    assert_eq!(views(&mut client, &id).await["init"], Value::Null);
    let file = folder.join(".omfp/init.ts");
    assert_eq!(
        daemon.get(&format!("{base}/init/{id}/v/init.ts")).await.0,
        404,
        "no init file"
    );

    write(
        &file,
        "import { useStored } from \"omfp/sdk\";\nexport const n: number = 1;\nexport { useStored };\n",
    );
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;
    let event = views_event(&mut client, &id).await;
    assert_eq!(event["kind"], "views_init");
    assert!(is_version(&event["init"]), "{event}");
    let first = event["init"].clone();
    let url = format!("{base}/init/{id}/{}/init.ts", first.as_str().unwrap());
    let (status, headers, body) = daemon.get(&url).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        header(&headers, "content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    assert!(body.contains("useStored"), "{body}");
    assert!(!body.contains(": number"), "types stripped: {body}");

    // An unknown `omfp/sdk` name does not compile: the stand-in module is served; any version serves the file.
    write(&file, "import { nothing } from \"omfp/sdk\";\n");
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;
    let event = views_event(&mut client, &id).await;
    assert_eq!(event["kind"], "views_init");
    assert!(
        is_version(&event["init"]) && event["init"] != first,
        "{event}"
    );
    let (status, _, body) = daemon.get(&format!("{base}/init/{id}/old/init.ts")).await;
    assert_eq!(status, 200);
    assert!(body.contains("compileError"), "{body}");

    let other_key = format!("/widgets/{}", "0".repeat(32));
    for path in [
        format!("{other_key}/init/{id}/v/init.ts"),
        format!("{base}/init/NOPE/v/init.ts"),
        format!("{base}/init/{id}/v/other.ts"),
        format!("{base}/init/{id}/v/../init.ts"),
        format!("{base}/init/user/v/init.ts"),
    ] {
        let (status, headers, _) = daemon.get(&path).await;
        assert_eq!(status, 404, "{path}");
        assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    }

    std::fs::remove_file(&file).unwrap();
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;
    assert_eq!(
        views_event(&mut client, &id).await,
        json!({ "kind": "views_init", "init": null })
    );
    assert_eq!(
        daemon.get(&format!("{base}/init/{id}/v/init.ts")).await.0,
        404
    );
}

/// The next `init_changed` event of the `index` topic.
async fn init_changed(client: &mut Client) -> Value {
    loop {
        let event = client.event("index").await["event"].clone();
        if event["kind"] == "init_changed" {
            return event;
        }
    }
}

// spec: DMN-185, PRT-095
#[tokio::test(flavor = "multi_thread")]
async fn the_user_init_file_is_followed_on_the_index_topic_and_served_compiled() {
    let daemon = Daemon::with_bundle().await;
    let mut client = daemon.client().await;
    let index = client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["init"], Value::Null);
    let base = daemon.widget_base().await;

    write(
        &daemon.user_init(),
        "import { useJob } from \"omfp/sdk\";\nexport const job: string = String(useJob);\n",
    );
    let event = init_changed(&mut client).await;
    assert!(is_version(&event["init"]), "{event}");
    let (status, headers, body) = daemon.get(&format!("{base}/init/user/x/init.ts")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    assert!(body.contains("useJob"), "{body}");
    assert!(!body.contains(": string"), "{body}");

    // Index refreshes keep it.
    workspace(&mut client).await;
    let mut other = daemon.client().await;
    let index = other.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(index["state"]["init"], event["init"]);

    std::fs::remove_file(daemon.user_init()).unwrap();
    assert_eq!(
        init_changed(&mut client).await,
        json!({ "kind": "init_changed", "init": null })
    );
    assert_eq!(
        daemon.get(&format!("{base}/init/user/x/init.ts")).await.0,
        404
    );
}

/// The next `views_files` event of `views/<id>`.
async fn views_files(client: &mut Client, id: &str) -> Value {
    loop {
        let event = views_event(client, id).await;
        if event["kind"] == "views_files" {
            return event;
        }
    }
}

// spec: DMN-186, PRT-096, AGT-074
#[tokio::test(flavor = "multi_thread")]
async fn tsx_view_files_are_versioned_on_the_views_topic_and_served_compiled() {
    let daemon = Daemon::with_bundle().await;
    let mut client = daemon.client().await;
    let (id, folder) = workspace(&mut client).await;
    let base = daemon.widget_base().await;
    assert_eq!(views(&mut client, &id).await["viewFiles"], Value::Null);
    assert_eq!(
        daemon.get(&format!("{base}/views/{id}/v/ops.tsx")).await.0,
        404,
        "no views folder"
    );

    let manifest = json!({
        "default": "ops",
        "views": {
            "ops": { "title": "Ops", "view": "./views/ops.tsx" },
            "work": { "layout": "omfp:chat" }
        }
    });
    client
        .ok(
            "views.set",
            json!({ "workspaceId": id, "manifest": manifest }),
        )
        .await;
    let views_dir = folder.join(".omfp/views");
    write(
        &views_dir.join("ops.tsx"),
        "import { Card } from \"omfp/kit\";\nimport { label } from \"./lib/label\";\n\
         const n: number = 2;\nexport default function Ops() { return <Card>{label(n)}</Card>; }\n",
    );
    write(
        &views_dir.join("lib/label.ts"),
        "export const label = (n: number): string => `#${n}`;\n",
    );
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;
    let event = views_files(&mut client, &id).await;
    assert!(is_version(&event["viewFiles"]), "{event}");
    let first = event["viewFiles"].clone();
    let mut other = daemon.client().await;
    let state = views(&mut other, &id).await;
    assert_eq!(state["viewFiles"], first);
    assert_eq!(state["manifest"], manifest);
    assert_eq!(state["error"], Value::Null);

    let version = first.as_str().unwrap();
    let (status, headers, body) = daemon
        .get(&format!("{base}/views/{id}/{version}/ops.tsx"))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        header(&headers, "content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    assert!(body.contains("./lib/label"), "{body}");
    assert!(!body.contains(": number"), "types stripped: {body}");
    assert!(!body.contains("compileError"), "{body}");
    // The relative import resolves by URL to its sibling in the folder.
    let (status, _, body) = daemon
        .get(&format!("{base}/views/{id}/{version}/lib/label.ts"))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("export const label"), "{body}");
    assert!(!body.contains(": string"), "{body}");

    // A save is a new version; a file that does not compile is served as the stand-in.
    write(
        &views_dir.join("ops.tsx"),
        "export default function Ops() {\n  return <Card>;\n}\n",
    );
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;
    let event = views_files(&mut client, &id).await;
    assert!(
        is_version(&event["viewFiles"]) && event["viewFiles"] != first,
        "{event}"
    );
    let (status, _, body) = daemon.get(&format!("{base}/views/{id}/old/ops.tsx")).await;
    assert_eq!(status, 200);
    assert!(body.contains("export const compileError"), "{body}");
    assert!(body.contains("\"file\":\"ops.tsx\""), "{body}");

    let other_key = format!("/widgets/{}", "0".repeat(32));
    for path in [
        format!("{other_key}/views/{id}/v/ops.tsx"),
        format!("{base}/views/NOPE/v/ops.tsx"),
        format!("{base}/views/{id}/v/missing.tsx"),
        format!("{base}/views/{id}/v/../views.json"),
        format!("{base}/views/{id}/v/lib/../../views.json"),
        format!("{base}/views/{id}/v/"),
    ] {
        let (status, headers, _) = daemon.get(&path).await;
        assert_eq!(status, 404, "{path}");
        assert_eq!(header(&headers, "cache-control"), Some("no-store"));
    }

    std::fs::remove_dir_all(&views_dir).unwrap();
    client
        .ok("workspaces.rescan", json!({ "workspaceId": id }))
        .await;
    assert_eq!(
        views_files(&mut client, &id).await,
        json!({ "kind": "views_files", "viewFiles": null })
    );
}
