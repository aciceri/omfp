//! The complete ARC-021 boundary against the pinned omp: fresh migration, independent credentials/configuration,
//! native MCP discovery and unchanged project settings. All credentials are inert fixtures; all model work is fake.

mod client;
mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use clap::Parser;
use client::{Client, Follower};
use common::{Harness, system_prompt};
use fake_llm::{Reply, Step, ToolCall};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::paths::DataDir;
use omfp_daemon::{Ports, omp_profile, start_with};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

type Environment = Vec<(std::ffi::OsString, Option<std::ffi::OsString>)>;

fn command(harness: &Harness, env: &Environment, args: &[&str]) -> Command {
    let mut command = harness.env.command(args);
    for (key, value) in env {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command
}

async fn auth(harness: &Harness, env: &Environment, key: Option<&str>) -> Value {
    let mut command = if let Some(key) = key {
        let file = harness.env.root().join("synthetic-credentials.json");
        std::fs::write(&file, json!({
            "version": 1,
            "credentials": [
                { "provider": "openai", "credential": { "type": "api_key", "key": key, "source": "login" } },
                { "provider": "mcp:fixture", "credential": {
                    "type": "oauth", "access": key, "refresh": "inert-refresh", "expires": 4_102_444_800_000_i64
                } }
            ]
        }).to_string()).unwrap();
        command(harness, env, &["auth", "import", file.to_str().unwrap()])
    } else {
        command(harness, env, &["auth", "export"])
    };
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if key.is_some() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn snapshot(root: &Path) -> BTreeMap<std::path::PathBuf, [u8; 20]> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<std::path::PathBuf, [u8; 20]>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    Sha1::digest(std::fs::read(path).unwrap()).into(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Disables omp's keyless providers in the fresh profile of the data directory `data`: a local ollama (or another
/// keyless server on the developer's machine) would count as usable models, so only a stored key may make any.
fn disable_keyless_providers(data: &Path) {
    write(
        &DataDir::new(data)
            .omp_profile()
            .join("agent")
            .join("config.yml"),
        "disabledProviders: [local, web, ollama, llama.cpp, lm-studio, apple]\n",
    );
}

// spec: ARC-021, DMN-030, DMN-190, DMN-219, DMN-223, DMN-235, PRT-110, UI-389
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One end-to-end isolation acceptance scenario.
async fn fresh_profile_never_imports_standalone_state_and_mutations_are_bidirectionally_isolated() {
    let harness = Harness::new(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "host_config",
            "bash",
            json!({ "command": "cat \"$XDG_CONFIG_HOME/gh/hosts.yml\"" }),
        )])),
        Step::reply(Reply::text("profile response")),
    ])
    .await;
    // Existing standalone XDG roots must not redirect omfp's runtime or credentials either.
    let standalone_xdg = harness.env.root().join("xdg");
    for category in ["data", "state", "cache"] {
        std::fs::create_dir_all(standalone_xdg.join(category).join("omp")).unwrap();
    }
    write(
        &standalone_xdg.join("config/gh/hosts.yml"),
        "HOST_CLI_CONFIG_MARKER",
    );
    let mut standalone_env = harness.env.env_overrides();
    standalone_env.push((
        "XDG_CONFIG_HOME".into(),
        Some(standalone_xdg.join("config").into_os_string()),
    ));
    auth(&harness, &standalone_env, Some("standalone-key")).await;
    let standalone = harness.env.home().join(".omp");
    let standalone_auth = auth(&harness, &standalone_env, None).await;
    let models = std::fs::read_to_string(harness.env.agent_dir().join("models.yml")).unwrap();
    write(
        &harness.env.agent_dir().join("APPEND_SYSTEM.md"),
        "STANDALONE_APPEND_MUST_NOT_LOAD",
    );
    write(
        &harness.env.agent_dir().join("AGENTS.md"),
        "STANDALONE_AGENT_MUST_NOT_LOAD",
    );
    write(
        &harness.env.agent_dir().join("mcp.json"),
        r#"{"mcpServers":{"standalone":{"command":"never-run","enabled":false}}}"#,
    );
    let before = snapshot(&standalone);
    let xdg_before = snapshot(&standalone_xdg);

    let data = tempfile::tempdir().unwrap();
    let workspaces = data.path().join("workspaces");
    disable_keyless_providers(data.path());
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.path().to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--no-title-generation",
    ])
    .unwrap();
    let config = Config::from_cli(cli, None, None).unwrap();
    let mut ports = Ports::resolve(&config).unwrap();
    ports.omp_env.clone_from(&standalone_env);
    ports
        .omp_env
        .push(("OMP_PROFILE".into(), Some("standalone-work".into())));
    ports.omp_env.push((
        "PI_CONFIG_FILES".into(),
        Some(harness.env.agent_dir().join("config.yml").into()),
    ));
    let daemon = start_with(config, ports).await.unwrap();
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    assert_eq!(
        client.ok("auth.status", json!({})).await,
        json!({"models":0})
    );
    assert_eq!(
        client
            .ok("auth.logout", json!({"providerId":"openai"}))
            .await["removed"],
        0
    );
    assert_eq!(
        snapshot(&standalone),
        before,
        "fresh startup/auth must never touch standalone files"
    );
    assert_eq!(snapshot(&standalone_xdg), xdg_before);

    let profile = DataDir::new(data.path()).omp_profile();
    let agent = profile.join("agent");
    write(&agent.join("models.yml"), &models);
    // Deliberately unusable profile default: only the project's config can select the fake model.
    write(
        &agent.join("config.yml"),
        "modelRoles:\n  default: fake/not-the-project-model\n",
    );
    write(&agent.join("APPEND_SYSTEM.md"), "ISOLATED_APPEND_MARKER");
    write(&agent.join("AGENTS.md"), "ISOLATED_AGENT_MARKER");
    write(
        &agent.join("mcp.json"),
        r#"{"mcpServers":{"isolated":{"command":"never-run","enabled":false}}}"#,
    );
    let profile_env =
        omp_profile::environment(&DataDir::new(data.path()), standalone_env.clone()).unwrap();
    auth(&harness, &profile_env, Some("isolated-key")).await;
    let profile_auth = auth(&harness, &profile_env, None).await;
    assert!(profile_auth.to_string().contains("isolated-key"));
    assert!(!profile_auth.to_string().contains("standalone-key"));

    let ws = client
        .ok("workspaces.create", json!({"name":"profile-project"}))
        .await;
    let ws_path = Path::new(ws["path"].as_str().unwrap());
    write(
        &ws_path.join(".omp/config.yml"),
        "modelRoles:\n  default: fake/scripted\n",
    );
    write(&ws_path.join(".omp/AGENTS.md"), "PROJECT_AGENT_MARKER");
    write(
        &ws_path.join(".omp/mcp.json"),
        r#"{"mcpServers":{"project":{"command":"never-run","enabled":false}}}"#,
    );
    let listing = client
        .ok("workspaces.mcp_servers", json!({"workspaceId":ws["id"]}))
        .await;
    // The profile's user-level config belongs to no workspace: only the workspace's own servers are listed.
    assert_eq!(listing.get("userConfig"), None);
    let names: Vec<_> = listing["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|server| server["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["project"]);
    let thread = client
        .ok(
            "threads.create",
            json!({"workspace":{"id":ws["id"]},"prompt":"use the project model"}),
        )
        .await;
    let mut view = Follower::snapshot(&mut client, thread["id"].as_str().unwrap(), None).await;
    view.until(&mut client, |state| {
        state.settled
            && state
                .transcript
                .iter()
                .any(|entry| matches!(entry.message, omfp_core::wire::AgentMessage::Assistant(_)))
    })
    .await;
    let requests = harness.fake.requests();
    assert_eq!(requests[0]["model"], "scripted");
    let system = system_prompt(&requests[0]);
    assert!(system.contains("ISOLATED_APPEND_MARKER"));
    assert!(!system.contains("STANDALONE_APPEND_MUST_NOT_LOAD"));
    assert!(
        requests[1].to_string().contains("HOST_CLI_CONFIG_MARKER"),
        "agent shell tools must see host CLI configuration"
    );
    assert!(system.contains("ISOLATED_AGENT_MARKER"));
    assert!(system.contains("PROJECT_AGENT_MARKER"));
    assert!(!system.contains("STANDALONE_AGENT_MUST_NOT_LOAD"));
    assert_eq!(
        client
            .ok("auth.logout", json!({"providerId":"openai"}))
            .await["removed"],
        1
    );
    daemon.shutdown().await.unwrap();
    assert_eq!(
        snapshot(&standalone_xdg),
        xdg_before,
        "omfp must not alter standalone XDG state"
    );
    assert_eq!(
        snapshot(&standalone),
        before,
        "launch/config/logout cannot alter standalone"
    );
    assert_eq!(auth(&harness, &standalone_env, None).await, standalone_auth);
    let after = auth(&harness, &profile_env, None).await;
    assert_eq!(
        after["credentials"].as_array().unwrap().len(),
        1,
        "only the MCP credential remains"
    );
    assert_eq!(after["credentials"][0]["provider"], "mcp:fixture");
    let profile_before = snapshot(&profile);
    auth(&harness, &standalone_env, Some("standalone-changed")).await;
    write(
        &harness.env.agent_dir().join("config.yml"),
        "modelRoles:\n  default: fake/standalone-changed\n",
    );
    assert_eq!(
        snapshot(&profile),
        profile_before,
        "standalone auth/config must never touch omfp"
    );
    assert_eq!(auth(&harness, &profile_env, None).await, after);
    harness.assert_no_stray_network();
}

/// The provider credentials of an `omp auth export` document: `(provider, credential type)`.
fn credentials(export: &Value) -> Vec<(String, String)> {
    export["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["provider"].as_str().unwrap().to_owned(),
                entry["credential"]["type"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Where standalone omp keeps its credential store in [`copy_scenario`].
#[derive(Clone, Copy)]
enum StandaloneStore {
    /// `$XDG_DATA_HOME/omp/agent.db`: the XDG data directory exists.
    Xdg,
    /// `~/.omp/agent/agent.db`, with a `~/.env` naming another `XDG_DATA_HOME`.
    Home,
}

/// The trees a copy must leave as they were: standalone omp's home tree, its XDG roots, and a directory only a
/// `.env` file names.
fn untouched(harness: &Harness) -> Vec<BTreeMap<std::path::PathBuf, [u8; 20]>> {
    let root = harness.env.root();
    let mut trees = vec![
        snapshot(&harness.env.home().join(".omp")),
        snapshot(&root.join("xdg")),
        snapshot(&root.join("other")),
    ];
    if let Ok(env) = std::fs::read(harness.env.home().join(".env")) {
        trees.push(BTreeMap::from([(".env".into(), Sha1::digest(env).into())]));
    }
    trees
}

/// The snapshot directories left in `<data>/login/`.
fn snapshots(data: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(data.join("login"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("snapshot-"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A requested copy against the pinned omp, standalone omp's store at `store`: only the provider login reaches the
/// fresh profile, exported from a private snapshot that is gone afterwards, and nothing of standalone omp changes.
#[allow(clippy::too_many_lines)] // One end-to-end copy acceptance scenario.
async fn copy_scenario(store: StandaloneStore) {
    let harness = Harness::new(Vec::new()).await;
    let root = harness.env.root().to_owned();
    let standalone_xdg = root.join("xdg");
    let categories: &[&str] = match store {
        StandaloneStore::Xdg => &["data", "state", "cache"],
        StandaloneStore::Home => &["state", "cache"],
    };
    for category in categories {
        std::fs::create_dir_all(standalone_xdg.join(category).join("omp")).unwrap();
    }
    std::fs::create_dir_all(standalone_xdg.join("data")).unwrap();
    let mut standalone_env = harness.env.env_overrides();
    standalone_env.push((
        "XDG_CONFIG_HOME".into(),
        Some(standalone_xdg.join("config").into_os_string()),
    ));
    // An `openai` API key and the `mcp:fixture` MCP server's OAuth credential, stored by omp itself.
    auth(&harness, &standalone_env, Some("standalone-key")).await;
    let stored = match store {
        StandaloneStore::Xdg => standalone_xdg.join("data").join("omp").join("agent.db"),
        StandaloneStore::Home => harness.env.agent_dir().join("agent.db"),
    };
    assert!(
        stored.is_file(),
        "standalone omp's store is at {}",
        stored.display()
    );
    // A directory only a `.env` file names: omp reads `~/.env` for variables left empty, which omfp never does.
    std::fs::create_dir_all(root.join("other").join("omp")).unwrap();
    if matches!(store, StandaloneStore::Home) {
        write(
            &harness.env.home().join(".env"),
            &format!("XDG_DATA_HOME={}\n", root.join("other").display()),
        );
    }
    write(
        &harness.env.agent_dir().join("mcp.json"),
        r#"{"mcpServers":{"standalone":{"command":"never-run","enabled":false}}}"#,
    );
    let standalone_files: Vec<(&str, String)> = ["config.yml", "models.yml", "mcp.json"]
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(harness.env.agent_dir().join(name)).unwrap();
            (name, text)
        })
        .collect();
    let before = untouched(&harness);

    let data = tempfile::tempdir().unwrap();
    let workspaces = data.path().join("workspaces");
    disable_keyless_providers(data.path());
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.path().to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--no-title-generation",
    ])
    .unwrap();
    let config = Config::from_cli(cli, None, None).unwrap();
    let mut ports = Ports::resolve(&config).unwrap();
    ports.omp_env.clone_from(&standalone_env);
    ports.user_omp_env.clone_from(&standalone_env);
    let daemon = start_with(config, ports).await.unwrap();
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    client.ok("subscribe", json!({ "topic": "index" })).await;
    assert_eq!(
        client.ok("auth.status", json!({})).await,
        json!({"models":0}),
        "the fresh profile holds no login"
    );
    let openai = json!({"providers":[{"providerId":"openai","oauth":false}]});

    assert_eq!(client.ok("auth.importable", json!({})).await, openai);
    assert_eq!(
        untouched(&harness),
        before,
        "reading the logins leaves standalone omp as it was"
    );
    assert_eq!(snapshots(data.path()), Vec::<std::path::PathBuf>::new());
    let profile = DataDir::new(data.path()).omp_profile();
    let profile_env =
        omp_profile::environment(&DataDir::new(data.path()), standalone_env.clone()).unwrap();
    assert_eq!(credentials(&auth(&harness, &profile_env, None).await), []);
    assert_eq!(client.ok("auth.import", json!({})).await, openai);

    let copied = auth(&harness, &profile_env, None).await;
    assert_eq!(
        credentials(&copied),
        [("openai".to_owned(), "api_key".to_owned())],
        "only the provider login, no MCP credential"
    );
    assert_eq!(
        copied["credentials"][0]["credential"]["key"],
        "standalone-key"
    );
    for (name, text) in &standalone_files {
        let path = profile.join("agent").join(name);
        let copy = std::fs::read_to_string(&path).unwrap_or_default();
        assert_ne!(&copy, text, "{name} is not copied");
        assert!(!copy.contains("standalone"), "{name} is not copied");
    }
    // The import checks the usable models again; the index publishes the new count (DMN-236).
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let frame = client.event("index").await;
            if frame["event"]["kind"] == "models_changed"
                && frame["event"]["models"].as_u64().unwrap() > 0
            {
                return;
            }
        }
    })
    .await
    .expect("the copied key makes models usable");

    assert_eq!(
        client.ok("auth.import", json!({})).await,
        openai,
        "copying again is harmless"
    );
    assert_eq!(
        credentials(&auth(&harness, &profile_env, None).await),
        [("openai".to_owned(), "api_key".to_owned())],
        "omp's import replaces the provider's key"
    );
    assert_eq!(snapshots(data.path()), Vec::<std::path::PathBuf>::new());
    daemon.shutdown().await.unwrap();
    assert_eq!(
        untouched(&harness),
        before,
        "copying leaves standalone omp as it was"
    );
    harness.assert_no_stray_network();
}

// spec: ARC-021, DMN-229, DMN-230, PRT-136, PRT-137
#[tokio::test(flavor = "multi_thread")]
async fn a_requested_copy_from_an_xdg_located_store_brings_only_its_provider_logins() {
    copy_scenario(StandaloneStore::Xdg).await;
}

// spec: ARC-021, DMN-229, DMN-230, PRT-136, PRT-137
#[tokio::test(flavor = "multi_thread")]
async fn a_requested_copy_from_the_home_store_ignores_omps_env_files_and_brings_only_its_logins() {
    copy_scenario(StandaloneStore::Home).await;
}
