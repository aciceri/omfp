//! Live voice relay (DMN-100..112): one session per thread, opened on the client's peer (`live_connect`) or on the
//! daemon machine (`live_start`), owned by the connection that opened it.

use std::sync::atomic::Ordering;

use omfp_core::{LiveOwner, ThreadEvent, live_error_message};
use omfp_protocol::LiveVoice;
use omp_rpc::{
    LiveConnectCommand, LiveConnectResult, LiveLevelsEvent, LiveMuteCommand, LiveStartCommand,
    LiveStartResult,
};
use serde_json::Value;
use tokio::sync::broadcast;
use tracing::warn;

use super::{Result, Supervisor, SupervisorError, VoiceSlot, blocking, lock};
use crate::agent::OmpCall;

/// omp's `live_connect`/`live_start` failures that come before a session exists: no `live_end` follows them (S-09).
const LIVE_NOT_STARTED: &[&str] = &[
    "live_connect requires a non-empty sdpOffer",
    "End dictation before starting live voice",
    "Session is changing or shutting down",
    "A live session is already active",
];

/// omp's `live_mute` failure without a session.
const LIVE_NO_SESSION: &str = "No live session is active";

impl Supervisor {
    /// Starts a live voice session for `owner` on the client's peer (DMN-100), speaking with `voice` when given
    /// (DMN-112). Returns omp's SDP answer once the call is signaled.
    pub async fn live_connect(
        &self,
        id: &str,
        owner: LiveOwner,
        sdp_offer: String,
        voice: Option<LiveVoice>,
    ) -> Result<LiveConnectResult> {
        if sdp_offer.trim().is_empty() {
            return Err(SupervisorError::EmptyOffer);
        }
        let answer = self
            .open_live(id, owner, |instructions| {
                OmpCall::LiveConnect(LiveConnectCommand {
                    sdp_offer,
                    voice: voice.map(|voice| voice.as_str().to_owned()),
                    instructions: Some(instructions),
                })
            })
            .await?;
        serde_json::from_value(answer).map_err(|error| {
            SupervisorError::Internal(format!("unexpected live_connect answer: {error}"))
        })
    }

    /// Starts a live voice session for `owner` on the daemon machine's microphone and speakers (DMN-109), speaking
    /// with `voice` when given (DMN-112). Returns the voice once omp records.
    pub async fn live_start(
        &self,
        id: &str,
        owner: LiveOwner,
        voice: Option<LiveVoice>,
    ) -> Result<String> {
        let answer = self
            .open_live(id, owner, |instructions| {
                OmpCall::LiveStart(LiveStartCommand {
                    voice: voice.map(|voice| voice.as_str().to_owned()),
                    instructions: Some(instructions),
                })
            })
            .await?;
        serde_json::from_value::<LiveStartResult>(answer)
            .map(|started| started.voice)
            .map_err(|error| {
                SupervisorError::Internal(format!("unexpected live_start answer: {error}"))
            })
    }

    /// omp's `live_levels` of the thread's `live_start` sessions, which never reach its log (DMN-110).
    pub async fn live_levels(&self, id: &str) -> Result<broadcast::Receiver<LiveLevelsEvent>> {
        Ok(self.actor(id).await?.levels.subscribe())
    }

    /// Opens the thread's one live session (DMN-101) for `owner`: starts omp when needed, records the owner as a
    /// `live_session` event, then sends omp the command `call` builds from the live instructions (DMN-105).
    async fn open_live(
        &self,
        id: &str,
        owner: LiveOwner,
        call: impl FnOnce(String) -> OmpCall,
    ) -> Result<Value> {
        let actor = self.actor(id).await?;
        let slot = self.inner.voice_ids.fetch_add(1, Ordering::SeqCst);
        {
            let mut voice = lock(&actor.voice);
            if let Some(active) = &*voice {
                return Err(SupervisorError::LiveActive {
                    client: active.owner.client.clone(),
                });
            }
            *voice = Some(VoiceSlot {
                id: slot,
                owner: owner.clone(),
            });
        }
        let (process, title) = async {
            let process = self.ensure_running(&actor).await?;
            Ok::<_, SupervisorError>((process, self.thread(id).await?.title))
        }
        .await
        .inspect_err(|_| {
            actor.release_voice(slot);
        })?;
        // The realtime model knows only its instructions, so they carry the thread's conversation (DMN-105).
        let instructions = actor
            .log
            .with_state(|state| crate::live_context::live_instructions(&title, state));
        self.push(&actor, ThreadEvent::LiveSession { owner: Some(owner) });
        let call = call(instructions);
        let answer = blocking(move || process.call(call)).await?;
        actor.touch();
        answer.map_err(|error| {
            match &error {
                // omp refused before a session existed: no `live_end` will close it.
                omp_rpc::Error::Command { error, .. }
                    if LIVE_NOT_STARTED.iter().any(|known| error.contains(known)) =>
                {
                    if actor.release_voice(slot) {
                        self.push(&actor, ThreadEvent::LiveSession { owner: None });
                    }
                }
                // The session started and failed: omp's `live_end` frees the slot.
                omp_rpc::Error::Command { .. } => {}
                // The process is gone, and the session with it.
                _ => {
                    actor.release_voice(slot);
                }
            }
            live_failure(error)
        })
    }

    /// Stops the thread's live voice session (DMN-102); a thread whose omp does not run has none, and omp succeeds
    /// without one.
    pub async fn live_stop(&self, id: &str) -> Result<()> {
        let actor = self.actor(id).await?;
        let Some(process) = actor.process() else {
            return Ok(());
        };
        match blocking(move || process.call(OmpCall::LiveStop)).await? {
            Ok(_) | Err(omp_rpc::Error::Closed) => Ok(()),
            Err(error) => Err(live_failure(error)),
        }
    }

    /// Mutes or unmutes the thread's live voice session (DMN-102); returns the mute omp applied.
    pub async fn live_mute(&self, id: &str, muted: bool) -> Result<bool> {
        let actor = self.actor(id).await?;
        let process = actor
            .process()
            .filter(|_| lock(&actor.voice).is_some())
            .ok_or(SupervisorError::NoLiveSession)?;
        let call = OmpCall::LiveMute(LiveMuteCommand { muted: Some(muted) });
        match blocking(move || process.call(call)).await? {
            Ok(value) => value.get("muted").and_then(Value::as_bool).ok_or_else(|| {
                SupervisorError::Internal(format!("unexpected live_mute answer: {value}"))
            }),
            Err(omp_rpc::Error::Command { error, .. }) if error.contains(LIVE_NO_SESSION) => {
                Err(SupervisorError::NoLiveSession)
            }
            Err(error) => Err(live_failure(error)),
        }
    }

    /// Stops every live voice session started by `connection`, which closed (DMN-103).
    pub async fn release_live(&self, connection: u64) {
        let owned: Vec<String> = lock(&self.inner.actors)
            .values()
            .filter(|actor| {
                lock(&actor.voice)
                    .as_ref()
                    .is_some_and(|slot| slot.owner.connection == connection)
            })
            .map(|actor| actor.id.clone())
            .collect();
        for id in owned {
            if let Err(error) = self.live_stop(&id).await {
                warn!(thread = %id, %error, "cannot stop the live session of a closed connection");
            }
        }
    }
}

/// A failed live command, readable (DMN-104); other errors keep their meaning (a process gone stays one).
fn live_failure(error: omp_rpc::Error) -> SupervisorError {
    match error {
        omp_rpc::Error::Command { error, .. } => SupervisorError::Live {
            message: live_error_message(&error),
            omp: error,
        },
        other => other.into(),
    }
}
