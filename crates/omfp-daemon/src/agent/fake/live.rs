//! omp's live voice sequence for the fake launcher (TST-015). Every frame is one the pinned omp sends: the
//! no-credential failure of the contract tests, and the phases, transcripts and end of the real call of spike S-09.

use omp_rpc::{
    LiveConnectCommand, LiveEndEvent, LiveLevelsEvent, LiveMuteCommand, LivePhase, LivePhaseEvent,
    LiveRole, LiveStartCommand, LiveTranscriptEvent, RpcNotification,
};
use serde_json::{Value, json};

use super::{FakeProcess, lock};

/// omp's failure without an `openai-codex` login (S-09).
pub const NO_CREDENTIAL: &str = "No Codex OAuth credential is available for a live call.";

/// omp's `live_mute` failure without a session.
const NO_SESSION: &str = "No live session is active";

/// omp's `live_connect`/`live_start` failure while a session is connecting, active or closing.
const ALREADY_ACTIVE: &str = "A live session is already active";

/// How the fake answers `live_connect` and `live_start`; see [`super::FakeLauncher::script_live`].
#[derive(Debug, Clone, PartialEq)]
pub enum FakeLive {
    /// omp signaled the call: it sends `connecting` and `listening`, then `frames`, and answers `{ voice, sdpAnswer }`
    /// with [`answer_for`] the offer (`live_connect`) or `{ voice }` (`live_start`). `voice` is omp's own choice (its
    /// `live.voice` setting); a command's `voice` wins over it, as in omp.
    Connect {
        voice: String,
        frames: Vec<RpcNotification>,
    },
    /// The session failed once started, as without an `openai-codex` login: `connecting`, `error`,
    /// `live_end { error }`, `error`, then the failed response.
    Fail { error: String },
}

impl Default for FakeLive {
    fn default() -> Self {
        Self::connect(Vec::new())
    }
}

impl FakeLive {
    /// A call that connects with voice `sol`, then sends `frames` (e.g. [`FakeLive::call_frames`]).
    #[must_use]
    pub fn connect(frames: Vec<RpcNotification>) -> Self {
        Self::Connect {
            voice: "sol".into(),
            frames,
        }
    }

    /// The pinned omp without an `openai-codex` login.
    #[must_use]
    pub fn no_credential() -> Self {
        Self::Fail {
            error: NO_CREDENTIAL.into(),
        }
    }

    /// The transcript frames of the real call of S-09, in omp's order: user partials, assistant partials, the user's
    /// final after the assistant's first partial, then the assistant's final.
    #[must_use]
    pub fn call_frames() -> Vec<RpcNotification> {
        let line = |role, text: &str, last| transcript(role, 1, text, last);
        vec![
            line(LiveRole::User, "Ciao,", false),
            line(LiveRole::User, "Ciao, mi senti", false),
            line(LiveRole::User, "Ciao, mi senti? Dimmi solo sì", false),
            line(LiveRole::Assistant, "Sì", false),
            line(LiveRole::Assistant, "Sì.", false),
            line(LiveRole::User, "Ciao, mi senti? Dimmi solo sì", true),
            line(LiveRole::Assistant, "Sì.", true),
        ]
    }
}

/// A `live_phase` frame.
#[must_use]
pub fn phase(phase: LivePhase) -> RpcNotification {
    RpcNotification::LivePhase(LivePhaseEvent { phase })
}

/// A `live_transcript` frame.
#[must_use]
pub fn transcript(role: LiveRole, turn: i64, text: &str, last: bool) -> RpcNotification {
    RpcNotification::LiveTranscript(LiveTranscriptEvent {
        role,
        turn,
        text: text.into(),
        r#final: last,
    })
}

/// A `live_end` frame.
#[must_use]
pub fn end(error: Option<&str>) -> RpcNotification {
    RpcNotification::LiveEnd(LiveEndEvent {
        error: error.map(str::to_owned),
    })
}

/// A `live_levels` frame: microphone and speaker RMS (`live_start` sessions only).
#[must_use]
pub fn levels(input: f64, output: f64) -> RpcNotification {
    RpcNotification::LiveLevels(LiveLevelsEvent { input, output })
}

/// omp's failure of `command`.
fn failed(command: &str, error: &str) -> omp_rpc::Error {
    omp_rpc::Error::Command {
        command: command.into(),
        error: error.into(),
        code: None,
    }
}

/// The fake's SDP answer to `offer`: the offer as the answering side (`a=setup:active`, no candidates). The browser
/// accepts it as an answer, but no peer stands behind it: the connection never gets past ICE checking.
#[must_use]
pub fn answer_for(offer: &str) -> String {
    offer
        .split_inclusive('\n')
        .filter(|line| {
            !line.starts_with("a=candidate:") && !line.starts_with("a=end-of-candidates")
        })
        .map(|line| {
            if line.starts_with("a=setup:") {
                let ending = &line[line.trim_end().len()..];
                format!("a=setup:active{ending}")
            } else {
                line.to_owned()
            }
        })
        .collect()
}

/// The fake process's live session: `None` without one, else whether it is muted.
pub(super) type LiveSlot = Option<bool>;

impl FakeProcess {
    pub(super) fn live_connect(
        &self,
        command: &LiveConnectCommand,
        script: FakeLive,
    ) -> Result<Value, omp_rpc::Error> {
        if command.sdp_offer.trim().is_empty() {
            return Err(failed(
                "live_connect",
                "live_connect requires a non-empty sdpOffer",
            ));
        }
        let voice = self.live_open("live_connect", command.voice.as_deref(), script)?;
        Ok(json!({ "voice": voice, "sdpAnswer": answer_for(&command.sdp_offer) }))
    }

    /// `live_start`: as `live_connect`, but omp owns the host's audio, so the scripted frames may hold `live_levels`
    /// and a `speaking` phase (TST-051).
    pub(super) fn live_start(
        &self,
        command: &LiveStartCommand,
        script: FakeLive,
    ) -> Result<Value, omp_rpc::Error> {
        let voice = self.live_open("live_start", command.voice.as_deref(), script)?;
        Ok(json!({ "voice": voice }))
    }

    /// Opens the session of `command` as `script` says; returns the voice: the `requested` one, else the script's.
    fn live_open(
        &self,
        command: &str,
        requested: Option<&str>,
        script: FakeLive,
    ) -> Result<String, omp_rpc::Error> {
        {
            let mut live = lock(&self.shared.live);
            if live.is_some() {
                return Err(failed(command, ALREADY_ACTIVE));
            }
            *live = Some(false);
        }
        match script {
            FakeLive::Connect { voice, frames } => {
                self.emit(vec![
                    phase(LivePhase::Connecting),
                    phase(LivePhase::Listening),
                ]);
                self.emit(frames);
                Ok(requested.map_or(voice, str::to_owned))
            }
            FakeLive::Fail { error } => {
                lock(&self.shared.live).take();
                self.emit(vec![
                    phase(LivePhase::Connecting),
                    phase(LivePhase::Error),
                    end(Some(&error)),
                    phase(LivePhase::Error),
                ]);
                Err(failed(command, &error))
            }
        }
    }

    pub(super) fn live_mute(&self, command: &LiveMuteCommand) -> Result<Value, omp_rpc::Error> {
        let muted = {
            let mut live = lock(&self.shared.live);
            let Some(current) = *live else {
                return Err(omp_rpc::Error::Command {
                    command: "live_mute".into(),
                    error: NO_SESSION.into(),
                    code: None,
                });
            };
            let muted = command.muted.unwrap_or(!current);
            *live = Some(muted);
            muted
        };
        self.emit(vec![phase(if muted {
            LivePhase::Muted
        } else {
            LivePhase::Listening
        })]);
        Ok(json!({ "muted": muted }))
    }

    pub(super) fn live_stop(&self) -> Value {
        if lock(&self.shared.live).take().is_some() {
            self.emit(vec![end(None)]);
        }
        Value::Null
    }

    /// Whether the fake process has a live session.
    #[must_use]
    pub fn live_active(&self) -> bool {
        lock(&self.shared.live).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: TST-015
    #[test]
    fn the_answer_mirrors_the_offer_as_the_answering_side() {
        let offer = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=setup:actpass\r\na=candidate:1 1 udp 1 1.2.3.4 5 typ host\r\na=mid:0\r\n";
        assert_eq!(
            answer_for(offer),
            "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=setup:active\r\na=mid:0\r\n"
        );
        assert_eq!(answer_for("v=0"), "v=0");
    }
}
