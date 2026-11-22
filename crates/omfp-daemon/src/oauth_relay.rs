//! Finishing an OAuth sign-in omp started from a browser on another machine (`oauth.callback`, PRT-212,
//! DMN-340..DMN-342). omp waits for the provider's redirect on a loopback listener of the daemon's host
//! (`http://localhost:<port>/<path>?code=…&state=…`); a browser elsewhere lands on an address that reaches nothing
//! there. The user pastes that address, and the daemon requests it on its own loopback.
//!
//! The daemon only relays to a callback omp is waiting on: it records the `redirect_uri` of every authorization URL omp
//! opened ([`Flows`], per thread and per login) and accepts a pasted address only when it names that callback exactly
//! ([`Flows::target`], the SSRF guard of DMN-341).

use std::time::Duration;

use reqwest::Url;
use reqwest::redirect::Policy;
use tokio::time::Instant;

/// How long omp waits for a callback (`DEFAULT_TIMEOUT` of its callback listener): an older flow is forgotten.
pub const FLOW_TIMEOUT: Duration = Duration::from_mins(5);

/// How long the daemon waits for omp's listener to answer the relayed request.
pub const RELAY_TIMEOUT: Duration = Duration::from_secs(10);

/// The only hosts a callback may name: the loopback omp's listener binds (`localhost` binds `127.0.0.1` too).
const LOOPBACK_HOSTS: [&str; 2] = ["localhost", "127.0.0.1"];

/// The loopback callback an OAuth sign-in of omp waits on: `http://localhost:<port><path>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    port: u16,
    path: String,
}

impl Callback {
    /// The callback `url` names when it is a plain loopback `http` address: no user info, host `localhost` or
    /// `127.0.0.1` after the URL standard's normalization (`LOCALHOST`, `127.1` count; `[::1]`, other addresses and
    /// names do not).
    fn of_loopback(url: &Url) -> Option<Self> {
        let loopback = url.scheme() == "http"
            && url.username().is_empty()
            && url.password().is_none()
            && url
                .host_str()
                .is_some_and(|host| LOOPBACK_HOSTS.contains(&host));
        if !loopback {
            return None;
        }
        Some(Self {
            port: url.port_or_known_default()?,
            path: url.path().to_owned(),
        })
    }

    /// The loopback callback of an authorization URL omp opened: its `redirect_uri` parameter, when that is a loopback
    /// address ([`Callback::of_loopback`]).
    #[must_use]
    pub fn of_authorization(url: &str) -> Option<Self> {
        let url = Url::parse(url).ok()?;
        let (_, redirect) = url.query_pairs().find(|(name, _)| name == "redirect_uri")?;
        Self::of_loopback(&Url::parse(&redirect).ok()?)
    }
}

impl std::fmt::Display for Callback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "http://localhost:{}{}", self.port, self.path)
    }
}

/// Why a pasted address is not relayed (DMN-341).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// It is not a loopback `http` address, or it names another callback than the pending ones.
    Invalid(String),
    /// No sign-in of omp waits for a callback here (none started, or it completed or timed out).
    NotPending,
}

/// A pasted address that may be relayed: the callback it completes and the request to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    pub callback: Callback,
    /// The pasted address on `127.0.0.1`, without its fragment.
    pub url: Url,
}

/// The OAuth sign-ins of one omp (a thread's or the login's) still waiting for their callback (DMN-340).
#[derive(Debug, Default)]
pub struct Flows {
    pending: Vec<(Callback, Instant)>,
}

/// The first line of `/mcp reauth`'s outputs that end its sign-in (omp 19.1.0, `handleReauthCommand`).
const REAUTH_ENDS: [&str; 3] = [
    "✓ Reauthorized \"",
    "Reauthorization cancelled for \"",
    "Failed to reauthorize server \"",
];

impl Flows {
    /// omp opened `url` (an `open_url` request): a sign-in waits on its callback, if it names a loopback one.
    pub fn opened(&mut self, url: &str, now: Instant) {
        if let Some(callback) = Callback::of_authorization(url) {
            self.pending.retain(|(pending, _)| *pending != callback);
            self.pending.push((callback, now));
        }
    }

    /// A `command_output` of the thread: `/mcp reauth`'s end forgets its sign-ins; any other output records the
    /// authorization URLs it names (`Opening your browser to authorize. If it did not open:` and the URL).
    pub fn command_output(&mut self, text: &str, now: Instant) {
        if REAUTH_ENDS.iter().any(|end| text.starts_with(end)) {
            self.clear();
            return;
        }
        for word in text.split_whitespace() {
            if word.starts_with("http://") || word.starts_with("https://") {
                self.opened(word, now);
            }
        }
    }

    /// The sign-in that waited on `callback` got it.
    pub fn complete(&mut self, callback: &Callback) {
        self.pending.retain(|(pending, _)| pending != callback);
    }

    /// omp started or stopped: no listener of the earlier process is left.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// The SSRF guard (DMN-341): `pasted` is relayed only when it is an `http` address on `localhost` or `127.0.0.1`
    /// without user info whose port and path are those of a callback a sign-in still waits on (opened less than
    /// [`FLOW_TIMEOUT`] ago).
    pub fn target(&mut self, pasted: &str, now: Instant) -> Result<Relay, Refusal> {
        self.pending
            .retain(|(_, opened)| now.duration_since(*opened) < FLOW_TIMEOUT);
        if self.pending.is_empty() {
            return Err(Refusal::NotPending);
        }
        let expected = || {
            self.pending
                .iter()
                .map(|(callback, _)| callback.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut url = Url::parse(pasted.trim()).map_err(|_| {
            Refusal::Invalid(format!(
                "not an address: paste the whole address the browser ended on ({})",
                expected()
            ))
        })?;
        let Some(callback) = Callback::of_loopback(&url) else {
            return Err(Refusal::Invalid(format!(
                "only a plain http address on localhost or 127.0.0.1 is relayed ({})",
                expected()
            )));
        };
        if !self.pending.iter().any(|(pending, _)| *pending == callback) {
            return Err(Refusal::Invalid(format!(
                "{callback} is not the callback omp waits on ({})",
                expected()
            )));
        }
        url.set_host(Some("127.0.0.1"))
            .map_err(|error| Refusal::Invalid(error.to_string()))?;
        url.set_fragment(None);
        Ok(Relay { callback, url })
    }
}

/// How the relayed request went (DMN-342).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relayed {
    /// omp's listener answered with this status.
    Answered(u16),
    /// No answer within [`RELAY_TIMEOUT`], or no listener.
    Failed(String),
}

/// Requests `url` (a [`Relay`]'s) once: `GET`, no proxy, no redirect followed, [`RELAY_TIMEOUT`] (DMN-342).
pub async fn relay(url: Url) -> Relayed {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(RELAY_TIMEOUT)
        .build();
    let response = match client {
        Ok(client) => client.get(url).send().await,
        Err(error) => return Relayed::Failed(error.to_string()),
    };
    match response {
        Ok(response) => Relayed::Answered(response.status().as_u16()),
        Err(error) => Relayed::Failed(error.without_url().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHORIZE: &str = "https://auth.example.com/oauth/authorize?client_id=omp&response_type=code\
                             &redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback&state=s1";

    fn flows(now: Instant) -> Flows {
        let mut flows = Flows::default();
        flows.opened(AUTHORIZE, now);
        flows
    }

    fn invalid(result: &Result<Relay, Refusal>) -> bool {
        matches!(result, Err(Refusal::Invalid(_)))
    }

    // spec: DMN-340
    #[test]
    fn an_authorization_url_names_its_loopback_redirect() {
        assert_eq!(
            Callback::of_authorization(AUTHORIZE),
            Some(Callback {
                port: 1455,
                path: "/auth/callback".into()
            })
        );
        let elsewhere = |redirect: &str| {
            Callback::of_authorization(&format!(
                "https://auth.example.com/authorize?redirect_uri={redirect}"
            ))
        };
        assert_eq!(
            elsewhere("http%3A%2F%2F127.0.0.1%3A8080%2Fcallback")
                .map(|callback| callback.to_string()),
            Some("http://localhost:8080/callback".into())
        );
        assert_eq!(elsewhere("https%3A%2F%2Fexample.com%2Fcallback"), None);
        assert_eq!(elsewhere("https%3A%2F%2Flocalhost%3A1455%2Fcallback"), None);
        assert_eq!(
            elsewhere("vscode%3A%2F%2Fgitlab.gitlab-workflow%2Fauth"),
            None
        );
        assert_eq!(
            elsewhere("http%3A%2F%2F%5B%3A%3A1%5D%3A1455%2Fcallback"),
            None
        );
        assert_eq!(
            Callback::of_authorization("https://auth.example.com/device?code=ABCD"),
            None
        );
        assert_eq!(Callback::of_authorization("not a url"), None);
    }

    // spec: DMN-341
    #[test]
    fn the_exact_loopback_redirect_is_relayed_on_127_0_0_1_without_its_fragment() {
        let now = Instant::now();
        let relay = flows(now)
            .target(
                " http://localhost:1455/auth/callback?code=c0de&state=s1#frag \n",
                now,
            )
            .unwrap();
        assert_eq!(
            relay.url.as_str(),
            "http://127.0.0.1:1455/auth/callback?code=c0de&state=s1"
        );
        assert_eq!(
            relay.callback.to_string(),
            "http://localhost:1455/auth/callback"
        );
        // The URL standard normalizes the host: these are the same loopback addresses.
        for same in [
            "http://LOCALHOST:1455/auth/callback?code=c",
            "http://127.0.0.1:1455/auth/callback?code=c",
            "http://127.1:1455/auth/callback?code=c",
            "http://localhost:1455/auth/./callback?code=c",
        ] {
            assert!(flows(now).target(same, now).is_ok(), "{same}");
        }
    }

    // spec: DMN-341
    #[test]
    fn other_hosts_ports_paths_schemes_and_user_info_are_refused() {
        let now = Instant::now();
        for other in [
            "http://example.com:1455/auth/callback?code=c",
            "http://localhost.:1455/auth/callback?code=c",
            "http://localhost.example.com:1455/auth/callback?code=c",
            "http://10.0.0.1:1455/auth/callback?code=c",
            "http://127.0.0.2:1455/auth/callback?code=c",
            "http://0.0.0.0:1455/auth/callback?code=c",
            "http://[::1]:1455/auth/callback?code=c",
            "http://169.254.169.254:1455/auth/callback?code=c",
            "http://user@localhost:1455/auth/callback?code=c",
            "http://user:secret@127.0.0.1:1455/auth/callback?code=c",
            "https://localhost:1455/auth/callback?code=c",
            "file:///etc/passwd",
            "http://localhost:1456/auth/callback?code=c",
            "http://localhost/auth/callback?code=c",
            "http://localhost:1455/auth/callback/x?code=c",
            "http://localhost:1455/launch",
            "http://localhost:1455/?code=c",
            "localhost:1455/auth/callback?code=c",
            "code=c&state=s1",
        ] {
            assert!(invalid(&flows(now).target(other, now)), "{other}");
        }
    }

    // spec: DMN-340, DMN-341
    #[test]
    fn nothing_is_relayed_without_a_pending_sign_in_after_it_completed_or_timed_out() {
        let now = Instant::now();
        let pasted = "http://localhost:1455/auth/callback?code=c0de&state=s1";
        assert_eq!(
            Flows::default().target(pasted, now),
            Err(Refusal::NotPending)
        );

        let mut completed = flows(now);
        let relay = completed.target(pasted, now).unwrap();
        completed.complete(&relay.callback);
        assert_eq!(completed.target(pasted, now), Err(Refusal::NotPending));

        let mut late = flows(now);
        assert!(late.target(pasted, now + FLOW_TIMEOUT / 2).is_ok());
        assert_eq!(
            late.target(pasted, now + FLOW_TIMEOUT),
            Err(Refusal::NotPending)
        );

        // Opened again (omp printed the URL anew), it waits from then.
        let mut reopened = flows(now);
        reopened.opened(AUTHORIZE, now + FLOW_TIMEOUT / 2);
        assert!(reopened.target(pasted, now + FLOW_TIMEOUT).is_ok());

        let mut stopped = flows(now);
        stopped.clear();
        assert_eq!(stopped.target(pasted, now), Err(Refusal::NotPending));
    }

    // spec: DMN-340
    #[test]
    fn reauth_outputs_record_the_url_and_its_end_forgets_it() {
        let now = Instant::now();
        let pasted = "http://localhost:1455/auth/callback?code=c0de&state=s1";
        let mut flows = Flows::default();
        flows.command_output("Workspace roots:\n  /w/app", now);
        assert_eq!(flows.target(pasted, now), Err(Refusal::NotPending));
        flows.command_output(
            &format!(
                "━━━ OAuth Authorization Required ━━━\nOpening your browser to authorize. If it did not open:\n\
                 {AUTHORIZE}"
            ),
            now,
        );
        assert!(flows.target(pasted, now).is_ok());
        for end in [
            "✓ Reauthorized \"gh\" (user config). Credentials apply on the next connection.",
            "Reauthorization cancelled for \"gh\".",
            "Failed to reauthorize server \"gh\": boom",
        ] {
            flows.opened(AUTHORIZE, now);
            flows.command_output(end, now);
            assert_eq!(flows.target(pasted, now), Err(Refusal::NotPending), "{end}");
        }
    }
}
