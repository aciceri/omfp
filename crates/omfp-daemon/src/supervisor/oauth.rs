//! The OAuth sign-ins omp started and that wait for their loopback callback (DMN-340): a thread's (`/mcp reauth`'s
//! output, an `open_url` request) and the running login's. `oauth.callback` (PRT-212) relays a pasted address only to
//! one of them (`crate::oauth_relay`).

use tokio::time::Instant;

use super::{Result, Supervisor, lock};
use crate::oauth_relay::{Callback, Flows, Refusal, Relay};

/// Whose sign-in a pasted address completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthOwner {
    Thread(String),
    /// The running login, by its number ([`Supervisor::begin_login`]).
    Login(u64),
}

impl Supervisor {
    /// The login omp of login `login` opened `url` (DMN-340).
    pub fn login_opened_url(&self, login: u64, url: &str) {
        self.login_oauth(login, |oauth| oauth.opened(url, Instant::now()));
    }

    /// The login omp of login `login` is gone (exited, crashed or stopped): its OAuth listeners died with it, and
    /// another process may take their ports, so none of its sign-ins waits any more (DMN-340).
    pub fn login_omp_ended(&self, login: u64) {
        self.login_oauth(login, Flows::clear);
    }

    /// Runs `update` on the sign-ins of login `login` while it runs.
    fn login_oauth(&self, login: u64, update: impl FnOnce(&mut Flows)) {
        if let Some(running) = lock(&self.inner.login)
            .as_mut()
            .filter(|running| running.id == login)
        {
            update(&mut running.oauth);
        }
    }

    /// The relay of `pasted` to the sign-in of thread `thread_id`, else of the running login, if the guard lets it
    /// through (DMN-341). An unknown thread is `not_found`.
    pub async fn oauth_target(
        &self,
        thread_id: Option<&str>,
        pasted: &str,
    ) -> Result<std::result::Result<(OAuthOwner, Relay), Refusal>> {
        let now = Instant::now();
        if let Some(thread_id) = thread_id {
            let actor = self.actor(thread_id).await?;
            let target = lock(&actor.oauth).target(pasted, now);
            return Ok(target.map(|relay| (OAuthOwner::Thread(thread_id.to_owned()), relay)));
        }
        let mut login = lock(&self.inner.login);
        Ok(match login.as_mut() {
            Some(running) => running
                .oauth
                .target(pasted, now)
                .map(|relay| (OAuthOwner::Login(running.id), relay)),
            None => Err(Refusal::NotPending),
        })
    }

    /// omp's listener took the callback of `owner`'s sign-in: it waits no more (DMN-340).
    pub fn oauth_completed(&self, owner: &OAuthOwner, callback: &Callback) {
        match owner {
            OAuthOwner::Thread(thread_id) => {
                if let Some(actor) = self.lookup(thread_id) {
                    lock(&actor.oauth).complete(callback);
                }
            }
            OAuthOwner::Login(id) => self.login_oauth(*id, |oauth| oauth.complete(callback)),
        }
    }
}
