//! Params and results of the MCP servers omp loads for a workspace or a thread (M13, PRT-110..PRT-114).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `thread.mcp_reconnect` params (PRT-112).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpReconnectParams {
    pub thread_id: String,
    /// The server checked again after the restart; without it every checkable server is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `workspaces.mcp_servers`, `thread.mcp_servers` and `thread.mcp_reconnect` result (PRT-110..PRT-112).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpServers {
    pub workspace_id: String,
    /// The thread whose omp checked the servers; `null` for `workspaces.mcp_servers`.
    pub thread_id: Option<String>,
    /// The thread's omp process runs: its servers were checked. `false` without a thread.
    pub running: bool,
    /// omp's project config of the workspace, `<workspace>/.omp/mcp.json`, whether it exists or not.
    pub project_config: String,
    /// omfp's isolated user config, `<data-dir>/omp-profile/agent/mcp.json`, whether it exists or not (ARC-021).
    pub user_config: Option<String>,
    /// In omp's precedence order (the first definition of a name wins): omp's own files, then the imported ones.
    pub servers: Vec<McpServer>,
}

/// One MCP server configured for the workspace (PRT-113).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    pub transport: McpTransport,
    /// The command of a stdio server (with its arguments), or the URL of a remote one without its query and user info.
    pub target: String,
    pub source: McpSource,
    pub state: McpState,
    /// omp can check the server over RPC (`/mcp test`): it comes from omp's own project or user `mcp.json`.
    pub checkable: bool,
    /// Tools the server offered when checked (`connected`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<u32>,
    /// Why the check failed (`failed`), as omp reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How omp talks to a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
}

/// Where a server is defined (PRT-113).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpSource {
    /// Absolute path of the config file.
    pub path: String,
    /// The path relative to the workspace folder when the file is in it (a workspace file the UI can open).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    pub scope: McpScope,
    /// The tool whose config it is: `omp`, `claude`, `cursor`, `vscode` (omp imports the others' files).
    pub tool: String,
}

/// Whether a config belongs to the workspace or to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpScope {
    Project,
    User,
}

/// A server's state as the daemon knows it (PRT-114).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpState {
    /// Configured and enabled, not checked: no thread, the thread is not running, or omp cannot check it.
    Configured,
    /// omp connected to it and listed its tools.
    Connected,
    /// omp could not connect to it or list its tools.
    Failed,
    /// `enabled: false`, or named in the user config's `disabledServers`: omp does not load it.
    Disabled,
}
