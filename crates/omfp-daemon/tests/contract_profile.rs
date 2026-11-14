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
use fake_llm::{Reply, Step};
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

// spec: ARC-021, DMN-030, DMN-190, DMN-219, DMN-223, DMN-235, PRT-110
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One end-to-end isolation acceptance scenario.
async fn fresh_profile_never_imports_standalone_state_and_mutations_are_bidirectionally_isolated() {
    let harness = Harness::new(vec![Step::reply(Reply::text("profile response"))]).await;
    // Existing standalone XDG roots must not redirect omfp's runtime or credentials either.
    let standalone_xdg = harness.env.root().join("xdg");
    for category in ["data", "state", "cache"] {
        std::fs::create_dir_all(standalone_xdg.join(category).join("omp")).unwrap();
    }
    let standalone_env = harness.env.env_overrides();
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
    assert_eq!(
        listing["userConfig"],
        agent.join("mcp.json").to_str().unwrap()
    );
    let names: Vec<_> = listing["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|server| server["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["project", "isolated"]);
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
