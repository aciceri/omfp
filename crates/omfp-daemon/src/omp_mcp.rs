//! The MCP servers omp loads for a workspace (M13): the config files omp 18.18.6 reads, parsed by the daemon in omp's
//! precedence order (DMN-190), and the reply of omp's `/mcp test <name>`, the one check of a server omp offers over
//! RPC (DMN-191).
//!
//! omp has no RPC command or frame for its live MCP state: `get_state` lists no MCP tool (they are `xd://` devices),
//! `/mcp list` reads the same config files, `/mcp reload` only refreshes the command list and `/mcp reconnect` is
//! refused outside the TUI (spike in 07-roadmap, M13).

use std::path::{Path, PathBuf};

use omfp_protocol::{McpScope, McpServer, McpSource, McpState, McpTransport, RestartReason};
use serde::Deserializer;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// Where omp looks for MCP config besides the workspace: the user's home and omp's agent directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Homes {
    pub home: Option<PathBuf>,
    /// `<data-dir>/omp-profile/agent` in production (ARC-021).
    pub agent_dir: Option<PathBuf>,
}

/// omp's project config of `workspace`, the file `/mcp add` writes and `/mcp test` reads.
#[must_use]
pub fn project_config(workspace: &Path) -> PathBuf {
    workspace.join(".omp").join("mcp.json")
}

/// omp's user config, the file `/mcp add --scope user` writes and `/mcp test` reads.
#[must_use]
pub fn user_config(homes: &Homes) -> Option<PathBuf> {
    homes.agent_dir.as_ref().map(|dir| dir.join("mcp.json"))
}

/// One config file omp reads, and how.
struct ConfigFile {
    path: PathBuf,
    scope: McpScope,
    tool: &'static str,
    /// The key path of the server map: `["mcpServers"]`, or `["mcp", "servers"]` for VS Code.
    key: &'static [&'static str],
    /// omp's `/mcp test` sees its servers: omp's own project or user `mcp.json`.
    checkable: bool,
}

/// A group of files of which omp reads only the first that defines servers (Claude Code, Cursor), or every one.
enum Group {
    Every(Vec<ConfigFile>),
    First(Vec<ConfigFile>),
}

fn file(path: PathBuf, scope: McpScope, tool: &'static str, checkable: bool) -> ConfigFile {
    ConfigFile {
        path,
        scope,
        tool,
        key: &["mcpServers"],
        checkable,
    }
}

/// The files omp reads, in its precedence order (omp `docs/mcp-config.md`, "Discovery and precedence"): omp's own
/// (project, then user), Claude Code, Cursor, VS Code, then the root `mcp.json`/`.mcp.json` fallbacks. Codex, Gemini
/// CLI, `OpenCode`, Windsurf and plugin configs are not read.
fn config_files(workspace: &Path, homes: &Homes) -> Vec<Group> {
    use McpScope::{Project, User};
    let omp = workspace.join(".omp");
    let mut native = vec![
        file(omp.join("mcp.json"), Project, "omp", true),
        file(omp.join(".mcp.json"), Project, "omp", false),
    ];
    if let Some(agent) = &homes.agent_dir {
        native.push(file(agent.join("mcp.json"), User, "omp", true));
        native.push(file(agent.join(".mcp.json"), User, "omp", false));
    }
    let claude = workspace.join(".claude");
    let mut groups = vec![
        Group::Every(native),
        Group::First(vec![
            file(claude.join(".mcp.json"), Project, "claude", false),
            file(claude.join("mcp.json"), Project, "claude", false),
        ]),
    ];
    if let Some(home) = &homes.home {
        groups.push(Group::First(vec![
            file(home.join(".claude.json"), User, "claude", false),
            file(home.join(".claude").join("mcp.json"), User, "claude", false),
        ]));
    }
    groups.push(Group::Every(vec![file(
        workspace.join(".cursor").join("mcp.json"),
        Project,
        "cursor",
        false,
    )]));
    if let Some(home) = &homes.home {
        groups.push(Group::Every(vec![file(
            home.join(".cursor").join("mcp.json"),
            User,
            "cursor",
            false,
        )]));
    }
    groups.push(Group::Every(vec![ConfigFile {
        path: workspace.join(".vscode").join("mcp.json"),
        scope: Project,
        tool: "vscode",
        key: &["mcp", "servers"],
        checkable: false,
    }]));
    groups.push(Group::First(vec![
        file(workspace.join("mcp.json"), Project, "omp", false),
        file(workspace.join(".mcp.json"), Project, "omp", false),
    ]));
    groups
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Descends `key` in a JSON document and collects the object there as entries in file order: omp keeps the order a
/// file lists its servers in, `serde_json::Map` would sort them.
struct Entries<'k> {
    key: &'k [&'k str],
}

impl<'de> DeserializeSeed<'de> for Entries<'_> {
    type Value = Option<Vec<(String, Value)>>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Entries<'_> {
    type Value = Option<Vec<(String, Value)>>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut found = None;
        match self.key.split_first() {
            None => {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    entries.push(entry);
                }
                found = Some(entries);
            }
            Some((first, rest)) => {
                while let Some(name) = map.next_key::<String>()? {
                    if name == *first {
                        found = map.next_value_seed(Entries { key: rest })?;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
            }
        }
        Ok(found)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(None)
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
}

/// The server entries of a config file at `key`, in file order, if it has any.
fn servers_of(path: &Path, key: &[&str]) -> Option<Vec<(String, Value)>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    let entries = Entries { key }.deserialize(&mut deserializer).ok()??;
    deserializer.end().ok()?;
    (!entries.is_empty()).then_some(entries)
}

fn names(json: Option<&Value>, key: &str) -> Vec<String> {
    json.and_then(|json| json.get(key))
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A remote URL without its query, fragment and user info, as omp's `/mcp list` shows it.
fn redact_url(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or_default();
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let (authority, path) = rest.find('/').map_or((rest, ""), |at| rest.split_at(at));
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let path = if path == "/" { "" } else { path };
    format!("{scheme}://{host}{path}")
}

fn transport_and_target(config: &Map<String, Value>) -> (McpTransport, String) {
    let url = config.get("url").and_then(Value::as_str);
    let transport = match config.get("type").and_then(Value::as_str) {
        Some("http" | "streamable-http" | "streamableHttp") => McpTransport::Http,
        Some("sse") => McpTransport::Sse,
        Some("stdio") => McpTransport::Stdio,
        _ if url.is_some() => McpTransport::Http,
        _ => McpTransport::Stdio,
    };
    let target = match transport {
        McpTransport::Stdio => {
            let command = config.get("command").and_then(Value::as_str).unwrap_or("");
            let args = config
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            std::iter::once(command)
                .chain(args)
                .collect::<Vec<_>>()
                .join(" ")
        }
        McpTransport::Http | McpTransport::Sse => url.map(redact_url).unwrap_or_default(),
    };
    (transport, target)
}

/// The MCP servers omp would load for `workspace`, unchecked (DMN-190): each name once, from the first file defining
/// it, `disabled` when its entry says `enabled: false` (unless the user config's `enabledServers` names it) or the
/// user config's `disabledServers` names it.
#[must_use]
pub fn configured(workspace: &Path, homes: &Homes) -> Vec<McpServer> {
    let user = user_config(homes).and_then(|path| read_json(&path));
    let denied = names(user.as_ref(), "disabledServers");
    let allowed = names(user.as_ref(), "enabledServers");
    let mut servers: Vec<McpServer> = Vec::new();
    for group in config_files(workspace, homes) {
        let (files, first_only) = match group {
            Group::Every(files) => (files, false),
            Group::First(files) => (files, true),
        };
        for file in files {
            let Some(entries) = servers_of(&file.path, file.key) else {
                continue;
            };
            for (name, config) in entries {
                let Some(config) = config.as_object() else {
                    continue;
                };
                if servers.iter().any(|server| server.name == name) {
                    continue;
                }
                let (transport, target) = transport_and_target(config);
                let off = config.get("enabled").and_then(Value::as_bool) == Some(false)
                    && !allowed.contains(&name);
                let disabled = off || denied.contains(&name);
                // `/mcp test` takes the first word as the name and skips disabled entries.
                let checkable = file.checkable && !disabled && !name.contains(char::is_whitespace);
                servers.push(McpServer {
                    name,
                    transport,
                    target,
                    source: McpSource {
                        path: file.path.to_string_lossy().into_owned(),
                        workspace_path: file
                            .path
                            .strip_prefix(workspace)
                            .ok()
                            .map(|path| path.to_string_lossy().into_owned()),
                        scope: file.scope,
                        tool: file.tool.to_owned(),
                    },
                    state: if disabled {
                        McpState::Disabled
                    } else {
                        McpState::Configured
                    },
                    checkable,
                    tools: None,
                    error: None,
                });
            }
            if first_only {
                break;
            }
        }
    }
    servers
}

/// The server entries of every MCP config file omp reads for `workspace` plus the user config's enable lists, as a
/// hash: a change means omp would load other servers or other settings of one (DMN-201). Only the server sections
/// count, so `~/.claude.json`, which Claude Code rewrites all the time, changes it only when its servers change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigFingerprint(u64);

/// The [`ConfigFingerprint`] of the files [`configured`] reads now.
#[must_use]
pub fn fingerprint(workspace: &Path, homes: &Homes) -> ConfigFingerprint {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    let user = user_config(homes).and_then(|path| read_json(&path));
    names(user.as_ref(), "disabledServers").hash(&mut hasher);
    names(user.as_ref(), "enabledServers").hash(&mut hasher);
    for group in config_files(workspace, homes) {
        let (Group::Every(files) | Group::First(files)) = group;
        for file in files {
            file.path.hash(&mut hasher);
            servers_of(&file.path, file.key)
                .unwrap_or_default()
                .iter()
                .map(|(name, config)| (name.as_str(), config.to_string()))
                .for_each(|entry| entry.hash(&mut hasher));
        }
    }
    ConfigFingerprint(hasher.finish())
}

/// Why one `command_output` text of omp says the thread's omp must connect anew to get new MCP tools (DMN-202,
/// DMN-228): `✓ Reauthorized "<name>" …` (over RPC omp applies the credential on the next connection, unless it says
/// it reconnected the server itself) or `Added MCP server "<name>" …` (`/mcp add` wrote a config file).
#[must_use]
pub fn restart_reason(text: &str) -> Option<RestartReason> {
    if text.starts_with("✓ Reauthorized \"") && !text.trim_end().ends_with("Server reconnected.")
    {
        Some(RestartReason::McpReauth)
    } else if text.starts_with("Added MCP server \"") {
        Some(RestartReason::McpConfig)
    } else {
        None
    }
}

/// omp's answer to `/mcp test <name>` (DMN-191).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// `Server "<name>" connected (<n> tools).`
    Connected(u32),
    /// `Connection to "<name>" failed: <error>`
    Failed(String),
    /// `Server "<name>" not found. …`: omp's own files do not define it (any more).
    NotFound,
}

/// The check of server `name` in one `command_output` text of omp, if it is one.
#[must_use]
pub fn parse_check(name: &str, text: &str) -> Option<Check> {
    let quoted = format!("\"{name}\"");
    if let Some(rest) = text.strip_prefix(&format!("Server {quoted} connected (")) {
        let count = rest.split_once(" tool")?.0;
        return count.parse().ok().map(Check::Connected);
    }
    if text.starts_with(&format!("Server {quoted} not found")) {
        return Some(Check::NotFound);
    }
    let error = text.strip_prefix(&format!("Connection to {quoted} failed: "))?;
    Some(Check::Failed(error.trim().to_owned()))
}

/// The server with its check applied (PRT-114).
#[must_use]
pub fn checked(mut server: McpServer, check: Option<Check>) -> McpServer {
    match check {
        Some(Check::Connected(tools)) => {
            server.state = McpState::Connected;
            server.tools = Some(tools);
        }
        Some(Check::Failed(error)) => {
            server.state = McpState::Failed;
            server.error = Some(error);
        }
        Some(Check::NotFound) | None => {}
    }
    server
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    struct Layout {
        _root: tempfile::TempDir,
        workspace: PathBuf,
        homes: Homes,
    }

    fn layout() -> Layout {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("ws");
        let home = root.path().join("home");
        std::fs::create_dir_all(&workspace).unwrap();
        Layout {
            homes: Homes {
                agent_dir: Some(home.join(".omp").join("agent")),
                home: Some(home),
            },
            workspace,
            _root: root,
        }
    }

    fn summary(servers: &[McpServer]) -> Vec<(String, String, McpState, bool)> {
        servers
            .iter()
            .map(|server| {
                (
                    server.name.clone(),
                    format!("{:?}:{}", server.source.scope, server.source.tool),
                    server.state,
                    server.checkable,
                )
            })
            .collect()
    }

    // spec: DMN-190
    #[test]
    fn servers_come_from_omp_files_first_then_imported_ones_each_name_once() {
        let l = layout();
        let home = l.homes.home.clone().unwrap();
        write(
            &l.workspace.join(".omp/mcp.json"),
            r#"{"mcpServers":{"docs":{"type":"http","url":"https://user:pw@docs.example/mcp?key=s3cret"}}}"#,
        );
        write(
            &home.join(".omp/agent/mcp.json"),
            r#"{"mcpServers":{"docs":{"command":"ignored"},"fs":{"command":"npx","args":["-y","fs-server"]}},"disabledServers":["old"]}"#,
        );
        write(
            &l.workspace.join(".claude/.mcp.json"),
            r#"{"mcpServers":{"old":{"command":"old-server"},"fs":{"command":"shadowed"}}}"#,
        );
        write(
            &l.workspace.join(".claude/mcp.json"),
            r#"{"mcpServers":{"skipped":{"command":"x"}}}"#,
        );
        write(
            &home.join(".claude.json"),
            r#"{"mcpServers":{"remote":{"type":"sse","url":"https://sse.example/"}}}"#,
        );
        write(
            &l.workspace.join(".vscode/mcp.json"),
            r#"{"mcp":{"servers":{"code":{"command":"code-mcp","enabled":false}}}}"#,
        );
        write(
            &l.workspace.join("mcp.json"),
            r#"{"mcpServers":{"root":{"command":"r"}}}"#,
        );
        write(
            &l.workspace.join(".mcp.json"),
            r#"{"mcpServers":{"hidden":{"command":"h"}}}"#,
        );

        let servers = configured(&l.workspace, &l.homes);

        assert_eq!(
            summary(&servers),
            vec![
                (
                    "docs".into(),
                    "Project:omp".into(),
                    McpState::Configured,
                    true
                ),
                ("fs".into(), "User:omp".into(), McpState::Configured, true),
                (
                    "old".into(),
                    "Project:claude".into(),
                    McpState::Disabled,
                    false
                ),
                (
                    "remote".into(),
                    "User:claude".into(),
                    McpState::Configured,
                    false
                ),
                (
                    "code".into(),
                    "Project:vscode".into(),
                    McpState::Disabled,
                    false
                ),
                (
                    "root".into(),
                    "Project:omp".into(),
                    McpState::Configured,
                    false
                ),
            ]
        );
        assert_eq!(servers[0].transport, McpTransport::Http);
        assert_eq!(servers[0].target, "https://docs.example/mcp");
        assert_eq!(
            servers[0].source.workspace_path.as_deref(),
            Some(".omp/mcp.json")
        );
        assert_eq!(servers[1].transport, McpTransport::Stdio);
        assert_eq!(servers[1].target, "npx -y fs-server");
        assert_eq!(servers[1].source.workspace_path, None);
        assert_eq!(servers[3].transport, McpTransport::Sse);
        assert_eq!(servers[3].target, "https://sse.example");
    }

    // spec: DMN-190
    #[test]
    fn the_user_allowlist_enables_an_entry_and_unreadable_files_are_skipped() {
        let l = layout();
        let home = l.homes.home.clone().unwrap();
        write(
            &l.workspace.join(".omp/mcp.json"),
            r#"{"mcpServers":{"a":{"command":"a","enabled":false},"two words":{"command":"b"}}}"#,
        );
        write(&l.workspace.join(".omp/.mcp.json"), "{ not json");
        write(
            &home.join(".omp/agent/mcp.json"),
            r#"{"mcpServers":{},"enabledServers":["a"]}"#,
        );

        assert_eq!(
            summary(&configured(&l.workspace, &l.homes)),
            vec![
                ("a".into(), "Project:omp".into(), McpState::Configured, true),
                (
                    "two words".into(),
                    "Project:omp".into(),
                    McpState::Configured,
                    false
                ),
            ]
        );
        assert_eq!(configured(&l.workspace, &Homes::default()).len(), 2);
    }

    // spec: DMN-191
    #[test]
    fn omp_test_replies_become_checks() {
        assert_eq!(
            parse_check(
                "good",
                "Server \"good\" connected (2 tools).\n  - ping\n  - pong"
            ),
            Some(Check::Connected(2))
        );
        assert_eq!(
            parse_check(
                "broken",
                "Connection to \"broken\" failed: ENOENT: no such file or directory, posix_spawn '/x'"
            ),
            Some(Check::Failed(
                "ENOENT: no such file or directory, posix_spawn '/x'".into()
            ))
        );
        assert_eq!(
            parse_check("good", "Connection to \"other\" failed: nope"),
            None
        );
        assert_eq!(
            parse_check(
                "gone",
                "Server \"gone\" not found. Run /mcp list to see configured servers."
            ),
            Some(Check::NotFound)
        );
        let server = configured(Path::new("/nowhere"), &Homes::default());
        assert_eq!(server, Vec::new());
    }

    // spec: DMN-201
    #[test]
    fn the_fingerprint_changes_with_the_servers_omp_reads_only() {
        let l = layout();
        let home = l.homes.home.clone().unwrap();
        let empty = fingerprint(&l.workspace, &l.homes);
        write(&l.workspace.join(".omp/mcp.json"), r#"{"mcpServers":{}}"#);
        assert_eq!(fingerprint(&l.workspace, &l.homes), empty);

        write(
            &l.workspace.join(".omp/mcp.json"),
            r#"{"mcpServers":{"fs":{"command":"fs"}}}"#,
        );
        let project = fingerprint(&l.workspace, &l.homes);
        assert_ne!(project, empty);
        write(
            &l.workspace.join(".omp/mcp.json"),
            r#"{"mcpServers":{"fs":{"command":"fs","args":["--ro"]}}}"#,
        );
        let changed = fingerprint(&l.workspace, &l.homes);
        assert_ne!(changed, project);

        // Claude Code rewrites `~/.claude.json` all the time: only its servers count.
        write(&home.join(".claude.json"), r#"{"numStartups":1}"#);
        assert_eq!(fingerprint(&l.workspace, &l.homes), changed);
        write(
            &home.join(".claude.json"),
            r#"{"numStartups":2,"mcpServers":{"gh":{"type":"http","url":"https://gh.example/mcp"}}}"#,
        );
        let imported = fingerprint(&l.workspace, &l.homes);
        assert_ne!(imported, changed);

        write(
            &home.join(".omp/agent/mcp.json"),
            r#"{"mcpServers":{},"disabledServers":["gh"]}"#,
        );
        assert_ne!(fingerprint(&l.workspace, &l.homes), imported);
    }

    // spec: DMN-202, DMN-228
    #[test]
    fn reauthorized_and_added_servers_want_a_restart_with_their_reason() {
        assert_eq!(
            restart_reason(
                "✓ Reauthorized \"gh\" (user config). Credentials apply on the next connection; restart the session \
                 if the server stays disconnected."
            ),
            Some(RestartReason::McpReauth)
        );
        assert_eq!(
            restart_reason("Added MCP server \"fs\" (project)."),
            Some(RestartReason::McpConfig)
        );
        assert_eq!(
            restart_reason("✓ Reauthorized \"gh\" (user config). Server reconnected."),
            None
        );
        assert_eq!(
            restart_reason("Reauthorization cancelled for \"gh\"."),
            None
        );
        assert_eq!(restart_reason("Server \"gh\" connected (2 tools)."), None);
    }
}
