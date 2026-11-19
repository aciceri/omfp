//! Params and results of the MCP servers of a workspace (M13, PRT-110..PRT-114) and of the daemon's calls to their
//! tools (PRT-220, PRT-221).

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
    /// In omp's precedence order (the first definition of a name wins): omp's own file, then the imported ones.
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
    /// omp can check the server over RPC (`/mcp test`): it comes from the workspace's `.omp/mcp.json`.
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
    /// The tool whose config it is: `omp`, `claude`, `cursor`, `vscode` (omp imports the others' files).
    pub tool: String,
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
    /// `enabled: false`: omp does not load it.
    Disabled,
}

/// `mcp.tools` params (PRT-220).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpToolsParams {
    pub workspace_id: String,
    /// The server's name in the workspace's MCP config.
    pub server: String,
}

/// `mcp.tools` result (PRT-220): every tool the server lists, in its order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpTools {
    pub workspace_id: String,
    pub server: String,
    pub tools: Vec<McpTool>,
}

/// One tool of an MCP server, as its `tools/list` describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The JSON Schema of the tool's `arguments`.
    pub input_schema: serde_json::Value,
    /// The JSON Schema of the tool's `structuredContent`, when the server declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// Hints such as `readOnlyHint` or `destructiveHint`, as the server sends them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
}

/// `mcp.call` params (PRT-221).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpCallParams {
    pub workspace_id: String,
    pub server: String,
    pub tool: String,
    /// The tool's arguments, an object; none sends `{}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Map<String, serde_json::Value>>,
}

/// `mcp.call` result (PRT-221): the server's `tools/call` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpCallResult {
    /// The result's content blocks (`{ type: "text", text }`, `image`, `audio`, `resource_link`, `resource`), as sent.
    pub content: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<serde_json::Value>,
    /// The tool reported a failure (the call itself went through).
    pub is_error: bool,
}
