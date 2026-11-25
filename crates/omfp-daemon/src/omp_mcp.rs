//! The MCP servers of a workspace (M13): the workspace's config files omp 19.2.0 reads, parsed by the daemon in
//! omp's precedence order (DMN-190), the entry of one remote server the daemon calls itself (DMN-351), the `/mcp`
//! commands that would make omp write its user config, refused (DMN-350), and the reply of omp's `/mcp test <name>`,
//! the one check of a server omp offers over RPC (DMN-191).
//!
//! MCP servers belong to a workspace: omfp reads no user-level config (omfp's profile `agent/mcp.json`, Claude Code's
//! or Cursor's under `~`) and never has omp write one. omp has no RPC command or frame for its live MCP state:
//! `get_state` lists no MCP tool (they are `xd://` devices), `/mcp list` reads the same config files, `/mcp reload`
//! only refreshes the command list and `/mcp reconnect` is refused outside the TUI (spike in 07-roadmap, M13).

use std::path::{Path, PathBuf};
use std::time::Duration;

use omfp_protocol::{McpServer, McpSource, McpState, McpTransport, RestartReason};
use serde::Deserializer;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// omp's project config of `workspace`, the file `/mcp add` writes and `/mcp test` reads.
#[must_use]
pub fn project_config(workspace: &Path) -> PathBuf {
    workspace.join(".omp").join("mcp.json")
}

/// One config file omp reads, and how.
struct ConfigFile {
    path: PathBuf,
    tool: &'static str,
    /// The key path of the server map: `["mcpServers"]`, or `["mcp", "servers"]` for VS Code.
    key: &'static [&'static str],
    /// omp's `/mcp test` sees its servers: the workspace's `.omp/mcp.json`.
    checkable: bool,
}

/// A group of files of which omp reads only the first that defines servers (Claude Code), or every one.
enum Group {
    Every(Vec<ConfigFile>),
    First(Vec<ConfigFile>),
}

fn file(path: PathBuf, tool: &'static str, checkable: bool) -> ConfigFile {
    ConfigFile {
        path,
        tool,
        key: &["mcpServers"],
        checkable,
    }
}

/// The workspace's files omp reads, in its precedence order (omp `docs/mcp-config.md`, "Discovery and precedence"):
/// omp's own, Claude Code's, Cursor's, VS Code's, then the root `mcp.json`/`.mcp.json` fallbacks. Codex, Gemini CLI,
/// `OpenCode`, Windsurf and plugin configs are not read.
fn config_files(workspace: &Path) -> Vec<Group> {
    let omp = workspace.join(".omp");
    let claude = workspace.join(".claude");
    vec![
        Group::Every(vec![
            file(omp.join("mcp.json"), "omp", true),
            file(omp.join(".mcp.json"), "omp", false),
        ]),
        Group::First(vec![
            file(claude.join(".mcp.json"), "claude", false),
            file(claude.join("mcp.json"), "claude", false),
        ]),
        Group::Every(vec![file(
            workspace.join(".cursor").join("mcp.json"),
            "cursor",
            false,
        )]),
        Group::Every(vec![ConfigFile {
            path: workspace.join(".vscode").join("mcp.json"),
            tool: "vscode",
            key: &["mcp", "servers"],
            checkable: false,
        }]),
        Group::First(vec![
            file(workspace.join("mcp.json"), "omp", false),
            file(workspace.join(".mcp.json"), "omp", false),
        ]),
    ]
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

/// One server entry of the workspace and the file that defines it.
struct Entry {
    path: PathBuf,
    tool: &'static str,
    checkable: bool,
    name: String,
    config: Map<String, Value>,
}

/// Every server entry of the workspace in omp's precedence order, each name once from the first file defining it.
fn entries(workspace: &Path) -> Vec<Entry> {
    let mut found: Vec<Entry> = Vec::new();
    for group in config_files(workspace) {
        let (files, first_only) = match group {
            Group::Every(files) => (files, false),
            Group::First(files) => (files, true),
        };
        for file in files {
            let Some(servers) = servers_of(&file.path, file.key) else {
                continue;
            };
            for (name, config) in servers {
                let Value::Object(config) = config else {
                    continue;
                };
                if found.iter().any(|entry| entry.name == name) {
                    continue;
                }
                found.push(Entry {
                    path: file.path.clone(),
                    tool: file.tool,
                    checkable: file.checkable,
                    name,
                    config,
                });
            }
            if first_only {
                break;
            }
        }
    }
    found
}

fn disabled(config: &Map<String, Value>) -> bool {
    config.get("enabled").and_then(Value::as_bool) == Some(false)
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

fn transport(config: &Map<String, Value>) -> McpTransport {
    match config.get("type").and_then(Value::as_str) {
        Some("http" | "streamable-http" | "streamableHttp") => McpTransport::Http,
        Some("sse") => McpTransport::Sse,
        Some("stdio") => McpTransport::Stdio,
        _ if config.get("url").and_then(Value::as_str).is_some() => McpTransport::Http,
        _ => McpTransport::Stdio,
    }
}

fn transport_and_target(config: &Map<String, Value>) -> (McpTransport, String) {
    let transport = transport(config);
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
        McpTransport::Http | McpTransport::Sse => config
            .get("url")
            .and_then(Value::as_str)
            .map(redact_url)
            .unwrap_or_default(),
    };
    (transport, target)
}

/// The MCP servers of `workspace`, unchecked (DMN-190): each name once, from the first file defining it, `disabled`
/// when its entry says `enabled: false`.
#[must_use]
pub fn configured(workspace: &Path) -> Vec<McpServer> {
    entries(workspace)
        .into_iter()
        .map(|entry| {
            let (transport, target) = transport_and_target(&entry.config);
            let disabled = disabled(&entry.config);
            McpServer {
                // `/mcp test` takes the first word as the name and skips disabled entries.
                checkable: entry.checkable
                    && !disabled
                    && !entry.name.contains(char::is_whitespace),
                name: entry.name,
                transport,
                target,
                source: McpSource {
                    workspace_path: entry
                        .path
                        .strip_prefix(workspace)
                        .ok()
                        .map(|path| path.to_string_lossy().into_owned()),
                    path: entry.path.to_string_lossy().into_owned(),
                    tool: entry.tool.to_owned(),
                },
                state: if disabled {
                    McpState::Disabled
                } else {
                    McpState::Configured
                },
                tools: None,
                error: None,
            }
        })
        .collect()
}

/// The server entries of every MCP config file of `workspace`, as a hash: a change means omp would load other servers
/// or other settings of one (DMN-201). Only the server sections count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigFingerprint(u64);

/// The [`ConfigFingerprint`] of the files [`configured`] reads now.
#[must_use]
pub fn fingerprint(workspace: &Path) -> ConfigFingerprint {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    for group in config_files(workspace) {
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

/// `${VAR}` and `${VAR:-default}` in `text` replaced as omp's discovery does: a set variable gives its value (an empty
/// one too, except under `:-`), else the default, else the placeholder stays as written.
fn expand_env(text: &str, env: &impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let (name, default) = match inner.split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (inner, None),
        };
        if name.is_empty() || name.contains(':') {
            out.push_str(&rest[start..start + 3 + end]);
        } else {
            match (env(name), default) {
                (Some(value), default) if default.is_none() || !value.is_empty() => {
                    out.push_str(&value);
                }
                (_, Some(default)) => out.push_str(default),
                (_, None) => out.push_str(&rest[start..start + 3 + end]),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn expand_env_deep(value: &mut Value, env: &impl Fn(&str) -> Option<String>) {
    match value {
        Value::String(text) => *text = expand_env(text, env),
        Value::Array(items) => items.iter_mut().for_each(|item| expand_env_deep(item, env)),
        Value::Object(map) => map.values_mut().for_each(|item| expand_env_deep(item, env)),
        _ => {}
    }
}

/// A server's `auth` block (omp's `MCPAuthConfig`): which stored credential it uses and, for credentials that do not
/// carry their own, how to refresh it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Auth {
    /// `type: "oauth"`; an `apikey` server uses no stored credential.
    pub oauth: bool,
    pub credential_id: Option<String>,
    pub token_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub resource: Option<String>,
}

/// The entry of one remote server of the workspace, environment variables expanded (DMN-351).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    /// `Http` (Streamable HTTP) or `Sse` (the legacy HTTP+SSE transport).
    pub transport: McpTransport,
    pub url: String,
    /// The configured headers, each value as written: `!command`, the name of an environment variable, or literal.
    pub headers: Vec<(String, String)>,
    /// `headerPolicy: "origin-locked"`: the header values are literal.
    pub literal_headers: bool,
    pub auth: Option<Auth>,
    /// The entry's `timeout` (milliseconds in the file).
    pub timeout: Option<Duration>,
}

/// Why a server of the workspace cannot be called by the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// No config file of the workspace defines it.
    NotFound,
    /// Its entry says `enabled: false`.
    Disabled,
    /// A stdio server: the daemon calls remote servers only.
    Stdio,
    /// A remote entry without a `url`.
    NoUrl,
}

fn text_of(config: &Map<String, Value>, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// The remote server `name` of `workspace` as the daemon calls it (DMN-351), with `env` for `${VAR}` placeholders.
///
/// # Errors
///
/// Why the server cannot be called: unknown, disabled, stdio, or without a URL.
pub fn remote(
    workspace: &Path,
    name: &str,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<Remote, Unavailable> {
    let entry = entries(workspace)
        .into_iter()
        .find(|entry| entry.name == name)
        .ok_or(Unavailable::NotFound)?;
    if disabled(&entry.config) {
        return Err(Unavailable::Disabled);
    }
    let mut config = Value::Object(entry.config);
    expand_env_deep(&mut config, env);
    let Value::Object(config) = config else {
        return Err(Unavailable::NotFound);
    };
    let transport = transport(&config);
    if transport == McpTransport::Stdio {
        return Err(Unavailable::Stdio);
    }
    let url = text_of(&config, "url").ok_or(Unavailable::NoUrl)?;
    let headers = config
        .get("headers")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect();
    let auth = config
        .get("auth")
        .and_then(Value::as_object)
        .map(|auth| Auth {
            oauth: auth.get("type").and_then(Value::as_str) == Some("oauth"),
            credential_id: text_of(auth, "credentialId"),
            token_url: text_of(auth, "tokenUrl"),
            client_id: text_of(auth, "clientId"),
            client_secret: text_of(auth, "clientSecret"),
            resource: text_of(auth, "resource"),
        });
    let timeout = match config.get("timeout") {
        Some(Value::Number(ms)) => ms.as_u64(),
        Some(Value::String(ms)) => ms.trim().parse().ok(),
        _ => None,
    }
    .filter(|ms| *ms > 0)
    .map(Duration::from_millis);
    Ok(Remote {
        name: entry.name,
        transport,
        url,
        headers,
        literal_headers: config.get("headerPolicy").and_then(Value::as_str)
            == Some("origin-locked"),
        auth,
        timeout,
    })
}

/// Whether prompt `text` is a `/mcp` command omp would answer by writing its user MCP config (DMN-350): any `--scope
/// user` (before a `--` that starts a stdio command), and `/mcp enable|disable <name>` of a server the workspace's
/// `.omp/mcp.json` does not define (omp then edits the user config's entry or its `disabledServers`/`enabledServers`).
#[must_use]
pub fn writes_user_config(workspace: &Path, text: &str) -> bool {
    let mut words = text.split_whitespace();
    if words.next() != Some("/mcp") {
        return false;
    }
    let Some(verb) = words.next() else {
        return false;
    };
    let options: Vec<&str> = words.take_while(|word| *word != "--").collect();
    if options.windows(2).any(|pair| pair == ["--scope", "user"]) {
        return true;
    }
    match (verb, options.first()) {
        ("enable" | "disable", Some(name)) => {
            !servers_of(&project_config(workspace), &["mcpServers"])
                .unwrap_or_default()
                .iter()
                .any(|(defined, _)| defined == name)
        }
        _ => false,
    }
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

    fn workspace() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        (root, workspace)
    }

    fn summary(servers: &[McpServer]) -> Vec<(String, &str, McpState, bool)> {
        servers
            .iter()
            .map(|server| {
                (
                    server.name.clone(),
                    server.source.tool.as_str(),
                    server.state,
                    server.checkable,
                )
            })
            .collect()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    // spec: DMN-190
    #[test]
    fn servers_come_from_the_workspace_files_in_omp_order_each_name_once() {
        let (root, ws) = workspace();
        write(
            &ws.join(".omp/mcp.json"),
            r#"{"mcpServers":{"docs":{"type":"http","url":"https://user:pw@docs.example/mcp?key=s3cret"}}}"#,
        );
        write(
            &ws.join(".omp/.mcp.json"),
            r#"{"mcpServers":{"docs":{"command":"ignored"},"fs":{"command":"npx","args":["-y","fs-server"]}}}"#,
        );
        write(
            &ws.join(".claude/.mcp.json"),
            r#"{"mcpServers":{"old":{"command":"old-server","enabled":false},"fs":{"command":"shadowed"}}}"#,
        );
        write(
            &ws.join(".claude/mcp.json"),
            r#"{"mcpServers":{"skipped":{"command":"x"}}}"#,
        );
        write(
            &ws.join(".cursor/mcp.json"),
            r#"{"mcpServers":{"remote":{"type":"sse","url":"https://sse.example/"}}}"#,
        );
        write(
            &ws.join(".vscode/mcp.json"),
            r#"{"mcp":{"servers":{"code":{"command":"code-mcp","enabled":false}}}}"#,
        );
        write(
            &ws.join("mcp.json"),
            r#"{"mcpServers":{"root":{"command":"r"}}}"#,
        );
        write(
            &ws.join(".mcp.json"),
            r#"{"mcpServers":{"hidden":{"command":"h"}}}"#,
        );
        // User-level configs belong to no workspace: never read.
        write(
            &root.path().join("home/.claude.json"),
            r#"{"mcpServers":{"user":{"command":"u"}}}"#,
        );

        let servers = configured(&ws);

        assert_eq!(
            summary(&servers),
            vec![
                ("docs".into(), "omp", McpState::Configured, true),
                ("fs".into(), "omp", McpState::Configured, false),
                ("old".into(), "claude", McpState::Disabled, false),
                ("remote".into(), "cursor", McpState::Configured, false),
                ("code".into(), "vscode", McpState::Disabled, false),
                ("root".into(), "omp", McpState::Configured, false),
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
        assert_eq!(servers[3].transport, McpTransport::Sse);
        assert_eq!(servers[3].target, "https://sse.example");
    }

    // spec: DMN-190
    #[test]
    fn unreadable_files_are_skipped_and_names_with_spaces_cannot_be_checked() {
        let (_root, ws) = workspace();
        write(
            &ws.join(".omp/mcp.json"),
            r#"{"mcpServers":{"a":{"command":"a","enabled":false},"two words":{"command":"b"}}}"#,
        );
        write(&ws.join(".omp/.mcp.json"), "{ not json");

        assert_eq!(
            summary(&configured(&ws)),
            vec![
                ("a".into(), "omp", McpState::Disabled, false),
                ("two words".into(), "omp", McpState::Configured, false),
            ]
        );
        assert_eq!(configured(Path::new("/nowhere")), Vec::new());
    }

    // spec: DMN-351
    #[test]
    fn a_remote_entry_expands_environment_placeholders_like_omp() {
        let (_root, ws) = workspace();
        write(
            &ws.join(".omp/mcp.json"),
            r#"{"mcpServers":{
                "linear":{"type":"http","url":"https://${HOST}/mcp?team=${TEAM:-core}&x=${UNSET}",
                  "headers":{"X-Token":"!pass show linear","X-Env":"LINEAR_KEY","X-Lit":"${EMPTY:-fallback}"},
                  "auth":{"type":"oauth","credentialId":"mcp_oauth_abc","tokenUrl":"https://auth.example/token"},
                  "timeout":"2500"},
                "legacy":{"type":"sse","url":"https://sse.example/sse","headerPolicy":"origin-locked","timeout":0},
                "local":{"command":"fs"},
                "off":{"type":"http","url":"https://off.example","enabled":false},
                "nourl":{"type":"http"}
            }}"#,
        );
        let env = |name: &str| match name {
            "HOST" => Some("mcp.linear.app".to_owned()),
            "EMPTY" => Some(String::new()),
            _ => None,
        };

        let linear = remote(&ws, "linear", &env).unwrap();
        assert_eq!(linear.transport, McpTransport::Http);
        assert_eq!(
            linear.url,
            "https://mcp.linear.app/mcp?team=core&x=${UNSET}"
        );
        let mut headers = linear.headers.clone();
        headers.sort();
        assert_eq!(
            headers,
            vec![
                ("X-Env".to_owned(), "LINEAR_KEY".to_owned()),
                ("X-Lit".to_owned(), "fallback".to_owned()),
                ("X-Token".to_owned(), "!pass show linear".to_owned()),
            ]
        );
        assert!(!linear.literal_headers);
        assert_eq!(
            linear.auth,
            Some(Auth {
                oauth: true,
                credential_id: Some("mcp_oauth_abc".into()),
                token_url: Some("https://auth.example/token".into()),
                ..Auth::default()
            })
        );
        assert_eq!(linear.timeout, Some(Duration::from_millis(2500)));

        let legacy = remote(&ws, "legacy", &no_env).unwrap();
        assert_eq!(legacy.transport, McpTransport::Sse);
        assert!(legacy.literal_headers);
        assert_eq!((legacy.auth, legacy.timeout), (None, None));

        assert_eq!(remote(&ws, "local", &no_env), Err(Unavailable::Stdio));
        assert_eq!(remote(&ws, "off", &no_env), Err(Unavailable::Disabled));
        assert_eq!(remote(&ws, "nourl", &no_env), Err(Unavailable::NoUrl));
        assert_eq!(remote(&ws, "missing", &no_env), Err(Unavailable::NotFound));
    }

    // spec: DMN-350
    #[test]
    fn only_mcp_commands_that_would_write_the_user_config_are_caught() {
        let (_root, ws) = workspace();
        write(
            &ws.join(".omp/mcp.json"),
            r#"{"mcpServers":{"docs":{"type":"http","url":"https://docs.example/mcp"}}}"#,
        );
        for text in [
            "/mcp add gh --scope user --url https://gh.example/mcp",
            "/mcp remove gh --scope user",
            "/mcp smithery-search github --scope user",
            "/mcp disable imported",
            "/mcp enable imported",
        ] {
            assert!(writes_user_config(&ws, text), "{text}");
        }
        for text in [
            "/mcp add gh --url https://gh.example/mcp",
            "/mcp add fs --scope project -- npx fs --scope user",
            "/mcp disable docs",
            "/mcp enable",
            "/mcp test docs",
            "/mcp reauth docs",
            "/mcp",
            "please run /mcp add gh --scope user",
        ] {
            assert!(!writes_user_config(&ws, text), "{text}");
        }
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
    }

    // spec: DMN-201
    #[test]
    fn the_fingerprint_changes_with_the_servers_omp_reads_only() {
        let (_root, ws) = workspace();
        let empty = fingerprint(&ws);
        write(&ws.join(".omp/mcp.json"), r#"{"mcpServers":{}}"#);
        assert_eq!(fingerprint(&ws), empty);

        write(
            &ws.join(".omp/mcp.json"),
            r#"{"mcpServers":{"fs":{"command":"fs"}}}"#,
        );
        let project = fingerprint(&ws);
        assert_ne!(project, empty);
        write(
            &ws.join(".omp/mcp.json"),
            r#"{"mcpServers":{"fs":{"command":"fs","args":["--ro"]}}}"#,
        );
        let changed = fingerprint(&ws);
        assert_ne!(changed, project);

        // Only the server sections count.
        write(&ws.join(".claude/.mcp.json"), r#"{"other":1}"#);
        assert_eq!(fingerprint(&ws), changed);
        write(
            &ws.join(".claude/.mcp.json"),
            r#"{"other":2,"mcpServers":{"gh":{"type":"http","url":"https://gh.example/mcp"}}}"#,
        );
        assert_ne!(fingerprint(&ws), changed);
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
