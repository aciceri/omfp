//! `omfp mcp config` (DMN-159, AGT-054): the `mcp.json` document that points an MCP client (omp outside omfp, any
//! other agent) at a daemon's MCP server (DMN-155) with its token (DMN-158).

use std::path::Path;

use omfp_daemon::paths::DataDir;
use serde_json::{Value, json};

use crate::remote::{DaemonArgs, local_addr};

/// The environment variable the printed entry reads the token from (AGT-054): omp expands `${VAR}` in `mcp.json`, so
/// no file holds the token itself.
pub const TOKEN_VARIABLE: &str = "OMFP_TOKEN";

/// `omfp mcp …`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub enum Command {
    /// Prints the daemon's MCP entry for external clients: put it in standalone omp's `~/.omp/agent/mcp.json`
    /// or a project's `.omp/mcp.json`. Its header reads `$OMFP_TOKEN` (set it where the external client runs);
    /// `--literal-token` writes the token itself instead, which then must never be committed or shared.
    Config(ConfigArgs),
}

/// `omfp mcp config`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ConfigArgs {
    #[command(flatten)]
    pub daemon: DaemonArgs,
    /// The server's name in `mcp.json`; omp names its tools `mcp__<name>_…`.
    #[arg(long, default_value = "omfp")]
    pub name: String,
    /// Writes the token itself into the header instead of `${OMFP_TOKEN}`, for clients without environment
    /// expansion; the file then grants full access to the daemon (keep it private, never commit it).
    #[arg(long)]
    pub literal_token: bool,
}

/// Runs `omfp mcp …`; returns what to print.
pub fn run(command: &Command) -> anyhow::Result<String> {
    match command {
        Command::Config(args) => {
            let printed = config(args)?;
            if !args.literal_token {
                let token_file = match &args.daemon.token {
                    Some(_) => None,
                    None => args
                        .daemon
                        .data_dir()
                        .ok()
                        .map(|dir| DataDir::new(dir).token()),
                };
                eprintln!("{}", variable_hint(token_file.as_deref()));
            }
            Ok(printed)
        }
    }
}

fn config(args: &ConfigArgs) -> anyhow::Result<String> {
    let url = match &args.daemon.daemon {
        Some(daemon) => mcp_url(daemon)?,
        None => format!("http://{}/mcp", local_addr(&args.daemon.data_dir()?)?),
    };
    let token = if args.literal_token {
        args.daemon.token()?
    } else {
        format!("${{{TOKEN_VARIABLE}}}")
    };
    Ok(serde_json::to_string_pretty(&document(
        &args.name, &url, &token,
    ))?)
}

/// What `omfp mcp config` says on stderr about the variable (AGT-054): where its value comes from.
#[must_use]
pub fn variable_hint(token_file: Option<&Path>) -> String {
    let value = match token_file {
        Some(file) => format!("\"$(cat '{}')\"", file.display()),
        None => "<the daemon's token>".to_owned(),
    };
    format!(
        "omfp mcp: the entry reads the token from ${{{TOKEN_VARIABLE}}}; set it where the MCP client runs, e.g. \
         export {TOKEN_VARIABLE}={value}"
    )
}

/// `{ "mcpServers": { <name>: … } }`: Streamable HTTP at `url`, `token` (the token, or `${OMFP_TOKEN}`) as bearer, and
/// no request timeout, so `omfp_thread_wait` may outlast omp's 30 s default (DMN-159).
#[must_use]
pub fn document(name: &str, url: &str, token: &str) -> Value {
    json!({
        "mcpServers": {
            name: {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") },
                "timeout": 0,
            }
        }
    })
}

/// The MCP endpoint of `--daemon`: `http(s)://` or `ws(s)://` (a missing scheme means `http://`), its path ignored.
pub fn mcp_url(daemon: &str) -> anyhow::Result<String> {
    let (scheme, rest) = match daemon.split_once("://") {
        Some(("http" | "ws", rest)) => ("http", rest),
        Some(("https" | "wss", rest)) => ("https", rest),
        Some((other, _)) => {
            anyhow::bail!("--daemon `{daemon}`: the scheme `{other}` is not http(s):// or ws(s)://")
        }
        None => ("http", daemon),
    };
    let authority = rest.split('/').next().unwrap_or_default();
    anyhow::ensure!(!authority.is_empty(), "--daemon `{daemon}` names no host");
    Ok(format!("{scheme}://{authority}/mcp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-159
    #[test]
    fn the_daemon_url_becomes_its_mcp_endpoint() {
        for (given, expected) in [
            ("http://127.0.0.1:7420", "http://127.0.0.1:7420/mcp"),
            ("http://127.0.0.1:7420/", "http://127.0.0.1:7420/mcp"),
            ("ws://127.0.0.1:7420/api/ws", "http://127.0.0.1:7420/mcp"),
            (
                "https://omfp.example.ts.net",
                "https://omfp.example.ts.net/mcp",
            ),
            (
                "wss://omfp.example.ts.net/api/ws",
                "https://omfp.example.ts.net/mcp",
            ),
            ("localhost:7420", "http://localhost:7420/mcp"),
        ] {
            assert_eq!(mcp_url(given).unwrap(), expected, "{given}");
        }
        for wrong in ["ftp://host", "http://", ""] {
            assert!(mcp_url(wrong).is_err(), "{wrong}");
        }
    }

    // spec: DMN-159, AGT-054
    #[test]
    fn the_document_names_the_endpoint_the_token_and_no_timeout() {
        assert_eq!(
            document("omfp", "http://127.0.0.1:7420/mcp", "abc"),
            json!({ "mcpServers": { "omfp": {
                "type": "http",
                "url": "http://127.0.0.1:7420/mcp",
                "headers": { "Authorization": "Bearer abc" },
                "timeout": 0,
            } } })
        );
    }

    // spec: DMN-159, AGT-054
    #[test]
    fn the_entry_reads_the_token_from_the_environment_unless_asked_for_the_literal_token() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("token"), "abc\n").unwrap();
        let mut args = ConfigArgs {
            daemon: DaemonArgs {
                daemon: Some("http://127.0.0.1:1".into()),
                token: None,
                data_dir: Some(dir.path().into()),
            },
            name: "work".into(),
            literal_token: false,
        };
        let printed: Value = serde_json::from_str(&config(&args).unwrap()).unwrap();
        assert_eq!(
            printed["mcpServers"]["work"]["headers"]["Authorization"],
            "Bearer ${OMFP_TOKEN}"
        );
        assert_eq!(
            printed["mcpServers"]["work"]["url"],
            "http://127.0.0.1:1/mcp"
        );
        assert_eq!(
            variable_hint(Some(dir.path().join("token").as_path())),
            format!(
                "omfp mcp: the entry reads the token from ${{OMFP_TOKEN}}; set it where the MCP client runs, e.g. \
                 export OMFP_TOKEN=\"$(cat '{}')\"",
                dir.path().join("token").display()
            )
        );
        assert!(variable_hint(None).ends_with("export OMFP_TOKEN=<the daemon's token>"));

        args.literal_token = true;
        let printed: Value = serde_json::from_str(&config(&args).unwrap()).unwrap();
        assert_eq!(
            printed["mcpServers"]["work"]["headers"]["Authorization"],
            "Bearer abc"
        );

        let no_daemon = ConfigArgs {
            daemon: DaemonArgs {
                daemon: None,
                ..args.daemon
            },
            ..args
        };
        let error = config(&no_daemon).unwrap_err().to_string();
        assert!(error.contains("no daemon is running for"), "{error}");
    }
}
