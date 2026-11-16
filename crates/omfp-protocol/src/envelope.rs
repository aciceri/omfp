//! Frames on the WebSocket: client requests, and daemon responses, events and `resync_required`.

use std::fmt;

use omfp_core::ThreadEvent;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::dto::{
    JobStream, JobView, RepoStatus, RunInfo, Settings, Theme, ThreadSummary, WidgetInfo, Workspace,
    WorkspaceBinding,
};
use crate::method::Call;
use crate::topic::Topic;

/// A client request (PRT-010).
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Chosen by the client, unique per connection; echoed by the response.
    pub id: String,
    pub call: Call,
}

impl Request {
    pub fn new(id: impl Into<String>, call: Call) -> Self {
        Self {
            id: id.into(),
            call,
        }
    }
}

impl Serialize for Request {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("type", "request")?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("method", self.call.method())?;
        self.call.serialize_params(&mut map)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        decode_value(Value::deserialize(deserializer)?)
            .map_err(|error| D::Error::custom(error.message))
    }
}

/// A client frame that is not a valid request; answered with `bad_request` (PRT-011, PRT-012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    /// The request id, when the frame had one to answer to.
    pub id: Option<String>,
    pub message: String,
}

impl ProtocolError {
    fn new(id: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            id: id.map(str::to_owned),
            message: message.into(),
        }
    }

    /// Always [`ErrorCode::BadRequest`].
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        ErrorCode::BadRequest
    }

    /// The error to send back.
    #[must_use]
    pub fn to_error_body(&self) -> ErrorBody {
        ErrorBody::new(self.code(), self.message.clone())
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProtocolError {}

/// Decodes a client text frame.
///
/// Unknown methods are rejected, unknown fields (in the envelope or in `params`) are ignored, and absent or `null`
/// params mean `{}`.
pub fn decode_request(text: &str) -> Result<Request, ProtocolError> {
    let value = serde_json::from_str(text)
        .map_err(|error| ProtocolError::new(None, format!("invalid JSON: {error}")))?;
    decode_value(value)
}

fn decode_value(value: Value) -> Result<Request, ProtocolError> {
    let Value::Object(mut frame) = value else {
        return Err(ProtocolError::new(None, "a request must be a JSON object"));
    };
    let Some(Value::String(id)) = frame.remove("id") else {
        return Err(ProtocolError::new(None, "a request needs a string `id`"));
    };
    if frame.get("type").and_then(Value::as_str) != Some("request") {
        return Err(ProtocolError::new(
            Some(&id),
            "expected `\"type\": \"request\"`",
        ));
    }
    let Some(Value::String(method)) = frame.remove("method") else {
        return Err(ProtocolError::new(
            Some(&id),
            "a request needs a string `method`",
        ));
    };
    let params = match frame.remove("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(params @ Value::Object(_)) => params,
        Some(_) => return Err(ProtocolError::new(Some(&id), "`params` must be an object")),
    };
    if method == "threads.create" && params.get("workspace").is_none() {
        return Err(ProtocolError::new(
            Some(&id),
            "threads.create requires workspace: use { id: \"…\" } for an existing workspace or { new: { … } } for a new workspace",
        ));
    }
    match Call::decode(&method, params) {
        Some(Ok(call)) => Ok(Request { id, call }),
        Some(Err(error)) => Err(ProtocolError::new(
            Some(&id),
            format!("invalid params for `{method}`: {error}"),
        )),
        None => Err(ProtocolError::new(
            Some(&id),
            format!("unknown method `{method}`"),
        )),
    }
}

/// Error codes (PRT-011); a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    BadRequest,
    NotFound,
    Conflict,
    AlreadyResolved,
    LimitReached,
    VersionMismatch,
    ThreadFailed,
    /// omp refused the command; its own code is in `ompCode` (DMN-071).
    OmpError,
    GitError,
    Internal,
}

impl ErrorCode {
    /// Every code, in spec order.
    pub const ALL: [Self; 11] = [
        Self::Unauthorized,
        Self::BadRequest,
        Self::NotFound,
        Self::Conflict,
        Self::AlreadyResolved,
        Self::LimitReached,
        Self::VersionMismatch,
        Self::ThreadFailed,
        Self::OmpError,
        Self::GitError,
        Self::Internal,
    ];
}

/// The `error` of a failed response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    /// omp's `code`, for `omp_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omp_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            omp_code: None,
            details: None,
        }
    }

    #[must_use]
    pub fn with_omp_code(mut self, omp_code: impl Into<String>) -> Self {
        self.omp_code = Some(omp_code.into());
        self
    }

    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// `version_mismatch` listing the supported protocol versions in `details.supported` (PRT-004).
    #[must_use]
    pub fn version_mismatch(supported: &[u32]) -> Self {
        Self::new(
            ErrorCode::VersionMismatch,
            format!("unsupported protocol version; supported: {supported:?}"),
        )
        .with_details(json!({ "supported": supported }))
    }
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawResponse", into = "RawResponse")]
pub struct Response {
    pub id: String,
    /// The method's result as JSON (decode it with the method's `Result` type), or the error.
    pub result: Result<Value, ErrorBody>,
}

impl Response {
    /// A successful response carrying `result`.
    pub fn ok(id: impl Into<String>, result: &impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self {
            id: id.into(),
            result: Ok(serde_json::to_value(result)?),
        })
    }

    pub fn err(id: impl Into<String>, error: ErrorBody) -> Self {
        Self {
            id: id.into(),
            result: Err(error),
        }
    }
}

/// Wire form of [`Response`].
#[derive(Serialize, Deserialize)]
struct RawResponse {
    id: String,
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<ErrorBody>,
}

impl TryFrom<RawResponse> for Response {
    type Error = &'static str;

    fn try_from(raw: RawResponse) -> Result<Self, Self::Error> {
        let result = match (raw.ok, raw.error) {
            (true, None) => Ok(raw.result.unwrap_or(Value::Null)),
            (false, Some(error)) if raw.result.is_none() => Err(error),
            (true, Some(_)) => return Err("a successful response has no `error`"),
            (false, _) => return Err("a failed response needs an `error` and no `result`"),
        };
        Ok(Self { id: raw.id, result })
    }
}

impl From<Response> for RawResponse {
    fn from(response: Response) -> Self {
        match response.result {
            Ok(result) => Self {
                id: response.id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                id: response.id,
                ok: false,
                result: None,
                error: Some(error),
            },
        }
    }
}

/// A frame from the daemon.
#[allow(clippy::large_enum_variant)] // Events carry omp-rpc's unboxed frames, like `omfp_core::ThreadEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Response(Response),
    Event(EventFrame),
    /// The client's queue overflowed: the topic was dropped and must be resubscribed (PRT-023).
    ResyncRequired {
        topic: Topic,
    },
    /// A momentary value of a topic, outside its event stream (PRT-054): no `seq`, not replayed, not part of the
    /// topic's state; dropped rather than queued when the client falls behind.
    Transient {
        topic: Topic,
        event: TransientEvent,
    },
    /// A request of the daemon to this window (PRT-124), answered with `windows.answer { requestId: id, answer }`.
    WindowRequest {
        id: String,
        request: crate::windows::WindowRequest,
    },
}

/// The `event` of a transient frame (PRT-054).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransientEvent {
    /// omp's `live_levels` of a `live_start` session on `thread/<id>` (DMN-110): microphone (`input`) and speaker
    /// (`output`) RMS in [0, 1], at most ten a second.
    LiveLevels { input: f64, output: f64 },
}

/// One event of a topic (PRT-022).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventFrame {
    pub topic: Topic,
    /// Identifies the stream; a new epoch restarts `seq`.
    pub epoch: String,
    /// Strictly increasing, without gaps, within an epoch.
    pub seq: u64,
    /// Matches the topic's kind.
    pub event: TopicEvent,
}

#[derive(Deserialize)]
struct RawEventFrame {
    topic: Topic,
    epoch: String,
    seq: u64,
    event: Value,
}

impl<'de> Deserialize<'de> for EventFrame {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawEventFrame::deserialize(deserializer)?;
        let event = match &raw.topic {
            Topic::Index => serde_json::from_value(raw.event).map(TopicEvent::Index),
            Topic::Thread(_) => serde_json::from_value(raw.event).map(TopicEvent::Thread),
            Topic::Workspace(_) => serde_json::from_value(raw.event).map(TopicEvent::Workspace),
            Topic::Views(_) => serde_json::from_value(raw.event).map(TopicEvent::Views),
            Topic::Job(_) => serde_json::from_value(raw.event).map(TopicEvent::Job),
            Topic::Terminal(_) => serde_json::from_value(raw.event).map(TopicEvent::Terminal),
            Topic::Jobs(_) => serde_json::from_value(raw.event).map(TopicEvent::Jobs),
        }
        .map_err(|error| {
            D::Error::custom(format!("invalid event for topic {}: {error}", raw.topic))
        })?;
        Ok(Self {
            topic: raw.topic,
            epoch: raw.epoch,
            seq: raw.seq,
            event,
        })
    }
}

/// The `event` of an event frame; its kind follows from the topic.
#[allow(clippy::large_enum_variant)] // See `ServerFrame`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TopicEvent {
    Index(IndexEvent),
    Thread(ThreadEvent),
    Workspace(WorkspaceEvent),
    Views(ViewsEvent),
    Job(JobEvent),
    Terminal(TerminalEvent),
    Jobs(JobsEvent),
}
/// Change on the `index` topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IndexEvent {
    WorkspaceUpserted {
        workspace: Workspace,
    },
    WorkspaceRemoved {
        #[serde(rename = "workspaceId")]
        workspace_id: String,
    },
    ThreadUpserted {
        thread: ThreadSummary,
    },
    ThreadRemoved {
        #[serde(rename = "threadId")]
        thread_id: String,
    },
    /// The user's theme changed (DMN-075).
    ThemeChanged {
        theme: Theme,
    },
    /// The UI settings of the config file changed (DMN-009, UI-097).
    SettingsChanged {
        settings: Settings,
    },
    /// The user's init file appeared, changed or went away (PRT-095): replaces `init`.
    InitChanged {
        init: Option<String>,
    },
    /// The daemon checked how many models omp can use and found another count (PRT-141, DMN-235..DMN-237):
    /// replaces `models`.
    ModelsChanged {
        models: u32,
    },
    /// A login started, its flow moved on (its page, a progress message) or it ended (DMN-290): replaces `login`.
    LoginChanged {
        login: Option<crate::LoginActivity>,
    },
}

/// Change on a `workspace/<id>` topic, whose state is the git status of each repository of the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceEvent {
    /// The git status of the workspace's repositories changed; replaces the previous list.
    Status { repos: Vec<RepoStatus> },
}

/// Change on a `views/<workspaceId>` topic, whose state is a `ViewsState` (PRT-040).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewsEvent {
    /// The manifest changed: replaces `manifest` and `error`.
    ViewsManifest {
        manifest: Option<Value>,
        error: Option<String>,
    },
    /// A widget folder appeared, changed or went away: replaces `widgets`.
    ViewsWidgets { widgets: Vec<WidgetInfo> },
    /// The workspace's `.omfp/keymap.json` changed (DMN-121): replaces `keymap` and `keymapError`. An invalid file
    /// carries the last valid bindings with its error.
    ViewsKeymap {
        keymap: Vec<WorkspaceBinding>,
        error: Option<String>,
    },
    /// The workspace's `.omfp/init.ts` appeared, changed or went away (PRT-095): replaces `init`.
    ViewsInit { init: Option<String> },
    /// A file of the workspace's `.omfp/views/` (TSX views) appeared, changed or went away, or the
    /// folder did (PRT-096): replaces `viewFiles`.
    ViewsFiles {
        #[serde(rename = "viewFiles")]
        view_files: Option<String>,
    },
    /// A widget stored a value (`null`: removed the key, DMN-086).
    WidgetStored {
        instance: String,
        key: String,
        value: Value,
    },
}

/// Change on a `job/<id>` topic, whose state is a `JobState` (PRT-041).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobEvent {
    /// One more output line.
    JobOutput { stream: JobStream, text: String },
    /// The shell exited and both outputs closed.
    JobExited {
        code: Option<i32>,
        signal: Option<i32>,
        #[serde(rename = "endedAt")]
        ended_at: i64,
    },
}

/// Change on a `terminal/<id>` topic (TRM-100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalEvent {
    /// Raw PTY bytes, base64-encoded for JSON transport.
    Output { data: String },
    /// The PTY child exited.
    Exited {
        code: Option<i32>,
        #[serde(rename = "endedAt")]
        ended_at: i64,
    },
}

/// Change on a `jobs/<workspaceId>` topic, whose state is a `JobsState` (PRT-200).
#[allow(clippy::large_enum_variant)] // A wire type, built once per event; boxing would only add allocations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobsEvent {
    /// The whole state: a file was applied or refused.
    JobsSet {
        jobs: Vec<JobView>,
        error: Option<String>,
    },
    /// One job changed, or appeared; replace it by name.
    JobUpdated { job: JobView },
    /// A run started; the job's state comes with `job_updated`.
    RunStarted { run: RunInfo },
    /// A run ended; the job's state comes with `job_updated`.
    RunEnded { run: RunInfo },
}
