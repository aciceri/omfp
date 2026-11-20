//! Authorization (PRT-034): a connection's [`Principal`] is resolved once by `system.hello`, and every request is
//! checked in one place against a method → [`Permission`] table, so scoped principals (per-thread agent tokens,
//! 08-agent-control) only touch this module. Before that, the WebSocket upgrade checks the browser's `Origin`
//! ([`origin_allowed`], PRT-003) against the daemon's own origin and the configured allow-list ([`parse_origin`],
//! DMN-140, DMN-141).

use axum::http::Uri;
use omfp_protocol::{ErrorBody, ErrorCode};

use crate::store::Provenance;

/// Who makes a request: the other end of a connection, a thread's agent calling through the agent tool registry, or
/// an MCP client outside omfp calling through the MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// Holder of the daemon's token; `client` is the name sent in `system.hello` (PRT-033).
    Owner { client: String },
    /// The agent of thread `thread_id` (DMN-150): full access except the connection-level methods; `client` names
    /// the program calling for it (`omp` for the thread's host tools).
    Agent { thread_id: String, client: String },
    /// An MCP client of the MCP server (AGT-051): an agent without a thread, with the same access as a thread's
    /// agent; `client` is its `clientInfo.name`.
    Mcp { client: String },
}

impl Principal {
    /// The `client` of `system.hello`, the agent's client or the MCP client, recorded as provenance (PRT-033).
    #[must_use]
    pub fn client(&self) -> &str {
        match self {
            Self::Owner { client } | Self::Agent { client, .. } | Self::Mcp { client } => client,
        }
    }

    /// Who created what this principal creates (ARC-009).
    #[must_use]
    pub fn created_by(&self) -> Provenance {
        match self {
            Self::Owner { .. } => Provenance::User,
            Self::Agent { thread_id, .. } => Provenance::Thread(thread_id.clone()),
            Self::Mcp { .. } => Provenance::Mcp,
        }
    }
}

/// What a method needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Session plumbing: hello, subscriptions.
    Connect,
    /// Read workspaces, threads, their state, repository suggestions and directory listings.
    Read,
    /// Create, change or delete workspaces and threads.
    Manage,
    /// Drive a thread's agent: prompts, answers, process control.
    Drive,
}

/// The method → permission table; `None` for an unknown method.
#[must_use]
#[allow(clippy::too_many_lines)] // One flat table of every method.
pub fn permission(method: &str) -> Option<Permission> {
    use Permission::{Connect, Drive, Manage, Read};
    Some(match method {
        "system.hello" | "subscribe" | "unsubscribe" | "watch" | "widgets.report"
        | "windows.answer" => Connect,
        "repos.suggest"
        | "fs.complete"
        | "threads.search"
        | "activity.list"
        | "prompts.history"
        | "threads.list"
        | "workspaces.list"
        | "workspaces.status"
        | "workspaces.diff"
        | "workspaces.read_file"
        | "workspaces.find_file"
        | "thread.models"
        | "thread.thinking_levels"
        | "thread.commands"
        | "thread.tree"
        | "thread.subagents"
        | "thread.subagent_messages"
        | "thread.git"
        | "thread.state"
        | "thread.session"
        | "theme.get"
        | "settings.get"
        | "widgets.library"
        | "terminals.screen"
        | "jobs.list"
        | "jobs.runs"
        | "views.errors"
        | "windows.list"
        | "windows.state"
        | "workspaces.mcp_servers"
        | "thread.mcp_servers"
        | "mcp.tools"
        | "auth.status"
        | "auth.models"
        | "auth.providers" => Read,
        "workspaces.create"
        | "workspaces.update"
        | "workspaces.remove"
        | "workspaces.rescan"
        | "workspaces.add_repo"
        | "workspaces.remove_repo"
        | "workspaces.write_file"
        | "workspaces.reorder"
        | "threads.create"
        | "threads.update"
        | "threads.reorder"
        | "threads.archive"
        | "threads.unarchive"
        | "threads.delete"
        | "threads.fork"
        | "theme.set"
        | "settings.set"
        | "views.set"
        | "widgets.copy"
        | "widgets.state_set"
        | "views.mount"
        | "views.mount_update"
        | "views.unmount"
        | "widgets.save"
        | "jobs.define"
        | "jobs.remove"
        | "jobs.enable"
        | "oauth.callback" => Manage,
        "thread.prompt"
        | "thread.steer"
        | "thread.follow_up"
        | "thread.abort"
        | "thread.queue.remove"
        | "thread.queue.promote"
        | "thread.set_model"
        | "thread.set_thinking"
        | "thread.compact"
        | "thread.navigate"
        | "thread.fork"
        | "thread.label"
        | "thread.subagent_steer"
        | "thread.subagent_cancel"
        | "thread.goal"
        | "thread.btw"
        | "thread.bash"
        | "thread.restart"
        | "thread.stop"
        | "thread.live_connect"
        | "thread.live_start"
        | "thread.live_stop"
        | "thread.live_mute"
        | "thread.mcp_reconnect"
        | "mcp.call"
        | "interaction.respond"
        | "jobs.start"
        | "jobs.kill"
        | "jobs.run"
        | "terminals.start"
        | "terminals.input"
        | "terminals.resize"
        | "terminals.kill"
        | "windows.run"
        | "windows.check_widget"
        | "auth.login"
        | "auth.login_cancel"
        | "auth.logout" => Drive,
        _ => return None,
    })
}

/// The single authorization check for a request of `method` by `principal`.
pub fn authorize(principal: &Principal, method: &str) -> Result<(), ErrorBody> {
    let Some(permission) = permission(method) else {
        return Err(ErrorBody::new(
            ErrorCode::BadRequest,
            format!("unknown method `{method}`"),
        ));
    };
    match (principal, permission) {
        // The owner token grants everything; an agent, inside omfp or an MCP client outside, has full access (D-17,
        // DMN-150, AGT-051), but no connection to hello, subscribe on or report from.
        (
            Principal::Owner { .. },
            Permission::Connect | Permission::Read | Permission::Manage | Permission::Drive,
        )
        | (
            Principal::Agent { .. } | Principal::Mcp { .. },
            Permission::Read | Permission::Manage | Permission::Drive,
        ) => Ok(()),
        (Principal::Agent { .. } | Principal::Mcp { .. }, Permission::Connect) => {
            Err(ErrorBody::new(
                ErrorCode::Unauthorized,
                format!("`{method}` needs a window's connection; an agent cannot call it"),
            ))
        }
    }
}

/// Whether a WebSocket upgrade with this `Origin` may proceed (PRT-003, PRT-070): no `Origin` (not a browser), the
/// daemon's own origin (`http(s)://<Host>`), or exactly one of `allowed`, the entries of [`parse_origin`]. A browser
/// serializes `Origin` as `scheme://host[:port]`, lowercase, without its scheme's default port, so the match is a
/// string comparison: no wildcard, no subdomain, no other scheme or port.
#[must_use]
pub fn origin_allowed(origin: Option<&str>, host: Option<&str>, allowed: &[String]) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    allowed.iter().any(|allowed| allowed == origin)
        || host.is_some_and(|host| {
            origin == format!("http://{host}") || origin == format!("https://{host}")
        })
}

/// An entry of the `Origin` allow-list (DMN-141) in the form a browser sends: `http(s)://host[:port]` with the scheme
/// and host lowercased, the scheme's default port dropped (`https://x:443` is `https://x`) and a trailing `/`
/// allowed. `*`, other schemes, a path, a query, a fragment or user info are errors that say why.
pub fn parse_origin(entry: &str) -> Result<String, String> {
    let invalid = |why: &str| {
        format!(
            "`{entry}` is not an origin: {why}; write `http(s)://host[:port]`, e.g. `https://omfp.example.ts.net`"
        )
    };
    if entry.contains('*') {
        return Err(format!(
            "`{entry}`: wildcards are not allowed: any web page matching it could open a WebSocket to the daemon \
             (cross-site WebSocket hijacking); list every origin the UI is reached at, e.g. \
             `https://omfp.example.ts.net`"
        ));
    }
    if entry.contains(['?', '#']) {
        return Err(invalid("an origin has no query or fragment"));
    }
    if entry.contains('@') {
        return Err(invalid("an origin has no user info"));
    }
    let uri: Uri = entry.parse().map_err(|_| invalid("not a URL"))?;
    let scheme = uri.scheme_str().map(str::to_ascii_lowercase);
    let default_port = match scheme.as_deref() {
        Some("http") => 80,
        Some("https") => 443,
        _ => return Err(invalid("the scheme must be http:// or https://")),
    };
    let Some(authority) = uri.authority() else {
        return Err(invalid("no host"));
    };
    if !matches!(uri.path(), "" | "/") {
        return Err(invalid("an origin has no path"));
    }
    let host = authority.host().to_ascii_lowercase();
    if host.is_empty() {
        return Err(invalid("no host"));
    }
    let port = authority
        .as_str()
        .strip_prefix(authority.host())
        .and_then(|rest| rest.strip_prefix(':'))
        .filter(|port| !port.is_empty());
    let port = match port.map(str::parse::<u16>) {
        Some(Err(_)) => return Err(invalid("the port must be a number up to 65535")),
        Some(Ok(port)) if port != default_port => format!(":{port}"),
        _ => String::new(),
    };
    Ok(format!("{}://{host}{port}", scheme.unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-141
    #[test]
    fn allowed_origins_are_normalized_to_what_a_browser_sends() {
        for (entry, expected) in [
            ("https://omfp.example.ts.net", "https://omfp.example.ts.net"),
            (
                "https://omfp.example.ts.net/",
                "https://omfp.example.ts.net",
            ),
            ("HTTPS://OMFP.Example.TS.net", "https://omfp.example.ts.net"),
            ("https://omfp.example.com:443", "https://omfp.example.com"),
            ("http://omfp.example.com:80", "http://omfp.example.com"),
            (
                "https://omfp.example.com:8443",
                "https://omfp.example.com:8443",
            ),
            ("http://100.64.0.7:7420", "http://100.64.0.7:7420"),
            ("http://[fd7a:115c::1]:7420", "http://[fd7a:115c::1]:7420"),
        ] {
            assert_eq!(parse_origin(entry).as_deref(), Ok(expected), "{entry}");
        }
    }

    // spec: DMN-141
    #[test]
    fn wildcards_and_anything_but_an_origin_are_refused_with_the_reason() {
        for (entry, reason) in [
            ("*", "wildcards are not allowed"),
            ("https://*.ts.net", "cross-site WebSocket hijacking"),
            (
                "omfp.example.ts.net",
                "the scheme must be http:// or https://",
            ),
            (
                "ws://omfp.example.ts.net",
                "the scheme must be http:// or https://",
            ),
            ("null", "the scheme must be http:// or https://"),
            ("https://omfp.example.ts.net/app", "no path"),
            ("https://omfp.example.ts.net/?a=1", "no query or fragment"),
            ("https://omfp.example.ts.net#x", "no query or fragment"),
            ("https://user@omfp.example.ts.net", "no user info"),
            (
                "https://omfp.example.ts.net:99999",
                "the port must be a number up to 65535",
            ),
            ("https://", "not a URL"),
            ("", "not a URL"),
        ] {
            let error = parse_origin(entry).expect_err(entry);
            assert!(error.contains(reason), "{entry}: {error}");
            assert!(error.contains(&format!("`{entry}`")), "{entry}: {error}");
        }
    }

    // spec: PRT-003, PRT-070
    #[test]
    fn only_the_own_origin_and_exactly_the_configured_origins_may_upgrade() {
        let allowed = vec!["https://omfp.example.ts.net".to_owned()];
        let host = Some("127.0.0.1:7420");

        assert!(origin_allowed(None, host, &allowed));
        for origin in [
            "http://127.0.0.1:7420",
            "https://127.0.0.1:7420",
            "https://omfp.example.ts.net",
        ] {
            assert!(origin_allowed(Some(origin), host, &allowed), "{origin}");
        }
        for origin in [
            "https://evil.example",
            "http://127.0.0.1:7421",
            "http://omfp.example.ts.net",
            "https://omfp.example.ts.net:8443",
            "https://evil.omfp.example.ts.net",
            "https://omfp.example.ts.net.evil.example",
            "null",
        ] {
            assert!(!origin_allowed(Some(origin), host, &allowed), "{origin}");
        }
        assert!(!origin_allowed(Some("http://127.0.0.1:7420"), None, &[]));
        assert!(
            origin_allowed(Some("http://127.0.0.1:7420"), host, &[]),
            "without a list the own origin still upgrades"
        );
    }

    // spec: PRT-034
    #[test]
    fn every_method_has_exactly_one_permission() {
        let missing: Vec<&str> = omfp_protocol::METHODS
            .iter()
            .copied()
            .filter(|method| permission(method).is_none())
            .collect();

        assert_eq!(missing, Vec::<&str>::new());
        assert_eq!(permission("thread.prompt"), Some(Permission::Drive));
        assert_eq!(permission("thread.state"), Some(Permission::Read));
        assert_eq!(permission("no.such.method"), None);
    }

    // spec: PRT-034
    #[test]
    fn the_owner_may_call_every_method() {
        let owner = Principal::Owner {
            client: "omfp/web".into(),
        };

        for method in omfp_protocol::METHODS {
            assert_eq!(authorize(&owner, method), Ok(()), "{method}");
        }
        assert_eq!(
            authorize(&owner, "no.such.method").unwrap_err().code,
            ErrorCode::BadRequest
        );
    }

    // spec: PRT-033
    #[test]
    fn the_owner_creates_entities_as_the_user_and_names_its_client() {
        let owner = Principal::Owner {
            client: "script/ci".into(),
        };

        assert_eq!(owner.created_by(), Provenance::User);
        assert_eq!(owner.client(), "script/ci");
    }

    // spec: DMN-150, PRT-034, AGT-051
    #[test]
    fn an_agent_or_an_mcp_client_may_call_every_method_that_needs_no_connection() {
        let agents = [
            Principal::Agent {
                thread_id: "01J".into(),
                client: "omp".into(),
            },
            Principal::Mcp {
                client: "claude-code".into(),
            },
        ];

        for agent in &agents {
            for method in omfp_protocol::METHODS {
                let allowed = authorize(agent, method);
                if permission(method) == Some(Permission::Connect) {
                    let error = allowed.expect_err(method);
                    assert_eq!(error.code, ErrorCode::Unauthorized, "{method}");
                    assert!(error.message.contains("connection"), "{}", error.message);
                } else {
                    assert_eq!(allowed, Ok(()), "{method}");
                }
            }
            assert_eq!(
                authorize(agent, "no.such.method").unwrap_err().code,
                ErrorCode::BadRequest
            );
        }
    }

    // spec: AGT-051, PRT-093, PRT-033
    #[test]
    fn an_mcp_client_creates_entities_as_mcp_and_names_its_client() {
        let mcp = Principal::Mcp {
            client: "claude-code".into(),
        };

        assert_eq!(mcp.created_by(), Provenance::Mcp);
        assert_eq!(mcp.client(), "claude-code");
    }

    // spec: DMN-150, ARC-009, PRT-033
    #[test]
    fn an_agent_creates_entities_as_its_thread_and_names_its_client() {
        let agent = Principal::Agent {
            thread_id: "01J".into(),
            client: "omp".into(),
        };

        assert_eq!(agent.created_by(), Provenance::Thread("01J".into()));
        assert_eq!(agent.client(), "omp");
    }

    // spec: PRT-090, PRT-091
    #[test]
    fn the_lists_are_reads() {
        assert_eq!(permission("threads.list"), Some(Permission::Read));
        assert_eq!(permission("workspaces.list"), Some(Permission::Read));
    }

    // spec: PRT-123, PRT-124
    #[test]
    fn agents_read_and_drive_windows_but_only_a_window_answers_a_request() {
        assert_eq!(permission("windows.list"), Some(Permission::Read));
        assert_eq!(permission("windows.state"), Some(Permission::Read));
        assert_eq!(permission("windows.run"), Some(Permission::Drive));
        assert_eq!(permission("windows.check_widget"), Some(Permission::Drive));
        assert_eq!(permission("windows.answer"), Some(Permission::Connect));
    }

    // spec: PRT-129, PRT-130, PRT-132, PRT-133, PRT-140
    #[test]
    fn listing_providers_or_the_usable_models_reads_and_logging_in_or_out_drives() {
        assert_eq!(permission("auth.status"), Some(Permission::Read));
        assert_eq!(permission("auth.providers"), Some(Permission::Read));
        for method in ["auth.login", "auth.login_cancel", "auth.logout"] {
            assert_eq!(permission(method), Some(Permission::Drive), "{method}");
        }
    }

    // spec: PRT-212
    #[test]
    fn relaying_an_oauth_callback_manages() {
        assert_eq!(permission("oauth.callback"), Some(Permission::Manage));
    }
}
