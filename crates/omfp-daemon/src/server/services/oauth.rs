//! `oauth.callback` (PRT-212, DMN-340..DMN-342): the address a browser on another machine ended on after an OAuth
//! sign-in omp started, requested on the daemon's own loopback so omp's callback listener completes the sign-in.

use omfp_protocol::{ErrorBody, ErrorCode, OAuthCallbackParams, OAuthCallbackRelayed};
use serde_json::json;
use tracing::info;

use super::super::convert::{bad_request, supervisor_error};
use super::Services;
use crate::oauth_relay::{self, Refusal, Relayed};

impl Services {
    /// `oauth.callback { threadId?, url }` (PRT-212): relays `url` to the sign-in of the thread, else of the running
    /// login, once the guard let it through (DMN-341); a 2xx answer of omp's listener completes the sign-in (DMN-342).
    pub async fn oauth_callback(
        &self,
        params: OAuthCallbackParams,
    ) -> Result<OAuthCallbackRelayed, ErrorBody> {
        let target = self
            .supervisor
            .oauth_target(params.thread_id.as_deref(), &params.url)
            .await
            .map_err(supervisor_error)?;
        let (owner, relay) = target.map_err(|refusal| match refusal {
            Refusal::Invalid(message) => bad_request(message),
            Refusal::NotPending => ErrorBody::new(
                ErrorCode::Conflict,
                match params.thread_id {
                    Some(_) => "no OAuth sign-in of this thread's omp waits for its callback",
                    None => "no login waits for its callback",
                },
            ),
        })?;
        info!(?owner, callback = %relay.callback, "relaying an OAuth callback");
        match oauth_relay::relay(relay.url).await {
            Relayed::Answered(status) if (200..300).contains(&status) => {
                self.supervisor.oauth_completed(&owner, &relay.callback);
                Ok(OAuthCallbackRelayed { status })
            }
            Relayed::Answered(status) => Err(ErrorBody::new(
                ErrorCode::OmpError,
                format!(
                    "omp's OAuth callback at {} answered HTTP {status}",
                    relay.callback
                ),
            )
            .with_details(json!({ "status": status }))),
            Relayed::Failed(error) => Err(ErrorBody::new(
                ErrorCode::OmpError,
                format!(
                    "omp's OAuth callback at {} did not answer: {error}",
                    relay.callback
                ),
            )),
        }
    }
}
