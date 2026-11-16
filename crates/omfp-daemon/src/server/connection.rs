//! One WebSocket connection: `system.hello` first (PRT-001..005), then requests and topic subscriptions with
//! replay or snapshot (PRT-020..025). Outbound frames go through a bounded queue (PRT-023).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures::{SinkExt, StreamExt};
use omfp_core::LiveOwner;
use omfp_protocol::{
    BashParams, BtwParams, Call, ErrorBody, ErrorCode, EventFrame, HelloResult, LiveConnectParams,
    LiveConnected, LiveStarted, MessageParams, PromptParams, Request, RespondParams, Response,
    ServerFrame, SubscribeParams, SubscribeResult, ThreadRef, Topic, TopicEvent, TransientEvent,
    decode_request,
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::warn;

use super::auth::{Principal, authorize};
use super::convert::{bad_request, internal, supervisor_error};
use super::dispatch::dispatch;
use super::{AppState, OUTBOUND_QUEUE, SUPPORTED_PROTOCOLS};
use crate::event_log::{Position, Sequenced, Start, Subscription};
use crate::supervisor::{WatchGuard, WorkspaceWatchGuard};

/// Close code for a connection that did not authenticate (PRT-001), or not within [`HELLO_TIMEOUT`] (DMN-184).
pub(crate) const CLOSE_UNAUTHORIZED: u16 = 4401;
/// How long a connection has to complete `system.hello` (DMN-184).
pub(crate) const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Close code after `version_mismatch` (PRT-004).
pub(crate) const CLOSE_VERSION_MISMATCH: u16 = 4426;
/// Close code when the daemon shuts down.
const CLOSE_GOING_AWAY: u16 = 1001;

/// A frame for the writer task.
#[derive(Debug)]
pub(crate) enum Outbound {
    Text(String),
    Close(u16, &'static str),
}

impl Outbound {
    pub(crate) fn frame(frame: &ServerFrame) -> Option<Self> {
        match serde_json::to_string(frame) {
            Ok(text) => Some(Self::Text(text)),
            Err(error) => {
                warn!(%error, "cannot encode a frame");
                None
            }
        }
    }

    fn response(id: String, result: Result<Value, ErrorBody>) -> Option<Self> {
        Self::frame(&ServerFrame::Response(Response { id, result }))
    }
}

pub(crate) async fn serve(socket: WebSocket, app: Arc<AppState>, permit: OwnedSemaphorePermit) {
    let (mut sink, mut stream) = socket.split();
    let (out, mut outbox) = mpsc::channel::<Outbound>(OUTBOUND_QUEUE);
    let writer = tokio::spawn(async move {
        while let Some(frame) = outbox.recv().await {
            let (message, last) = match frame {
                Outbound::Text(text) => (Message::Text(text.into()), false),
                Outbound::Close(code, reason) => (
                    Message::Close(Some(CloseFrame {
                        code,
                        reason: reason.into(),
                    })),
                    true,
                ),
            };
            if sink.send(message).await.is_err() || last {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // Names the connection: its widget report (PRT-043) and the live sessions it owns (PRT-051, DMN-103).
    let number = app.services.reports_connection();
    let hello = tokio::time::timeout(
        HELLO_TIMEOUT,
        hello(&app, &mut stream, &out, number, permit),
    );
    let principal = if let Ok(principal) = hello.await {
        principal
    } else {
        let close = Outbound::Close(CLOSE_UNAUTHORIZED, "system.hello timed out");
        send(&out, Some(close)).await;
        None
    };
    if let Some(principal) = principal {
        // The daemon's requests to this window (`window_request`, PRT-124) go through its outbound queue.
        app.services.link_window(number, out.clone());
        let (dropped_tx, dropped) = mpsc::unbounded_channel();
        let mut connection = Connection {
            app: Arc::clone(&app),
            number,
            principal,
            out: out.clone(),
            subscriptions: HashMap::new(),
            dropped: dropped_tx,
            generation: 0,
        };
        connection.run(&mut stream, dropped).await;
        // Closing the socket unsubscribes everything (PRT-024) and forgets the window's report and requests (PRT-043,
        // DMN-205).
        connection.subscriptions.clear();
        app.services.forget_widget_reports(connection.number);
        // The window's audio peer is gone with it: stop the live sessions it started (DMN-103); nobody is left to see
        // the login it shows (DMN-222).
        let supervisor = app.services.supervisor.clone();
        supervisor.cancel_login(Some(number));
        tokio::spawn(async move { supervisor.release_live(number).await });
    }
    drop(out);
    let _ = writer.await;
}

async fn send(out: &mpsc::Sender<Outbound>, frame: Option<Outbound>) {
    if let Some(frame) = frame {
        let _ = out.send(frame).await;
    }
}

/// Waits for `system.hello`; anything else closes the connection with 4401 (PRT-001). `permit` (DMN-184) is released
/// once the token checked out, before the answer goes out.
async fn hello(
    app: &AppState,
    stream: &mut futures::stream::SplitStream<WebSocket>,
    out: &mpsc::Sender<Outbound>,
    number: u64,
    permit: OwnedSemaphorePermit,
) -> Option<Principal> {
    let unauthorized = || Outbound::Close(CLOSE_UNAUTHORIZED, "system.hello required");
    loop {
        let text = match stream.next().await?.ok()? {
            Message::Text(text) => text,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => return None,
            Message::Binary(_) => {
                send(out, Some(unauthorized())).await;
                return None;
            }
        };
        let Ok(Request {
            id,
            call: Call::SystemHello(params),
        }) = decode_request(&text)
        else {
            send(out, Some(unauthorized())).await;
            return None;
        };
        if !app.token.matches(&params.token) {
            let error = ErrorBody::new(ErrorCode::Unauthorized, "invalid token");
            send(out, Outbound::response(id, Err(error))).await;
            send(
                out,
                Some(Outbound::Close(CLOSE_UNAUTHORIZED, "invalid token")),
            )
            .await;
            return None;
        }
        drop(permit);
        if !SUPPORTED_PROTOCOLS.contains(&params.protocol) {
            let error = ErrorBody::version_mismatch(SUPPORTED_PROTOCOLS);
            send(out, Outbound::response(id, Err(error))).await;
            let close = Outbound::Close(CLOSE_VERSION_MISMATCH, "unsupported protocol version");
            send(out, Some(close)).await;
            return None;
        }
        let result = HelloResult {
            protocol: params.protocol,
            daemon: env!("CARGO_PKG_VERSION").to_owned(),
            omp: app.omp_version.clone(),
            warnings: app.warnings.clone(),
            widget_base: app.widget_base.clone(),
            file_base: app.file_base.clone(),
            connection: number,
        };
        send(out, Outbound::response(id, to_value(&result))).await;
        return Some(Principal::Owner {
            client: params.client,
        });
    }
}

fn to_value(value: &impl Serialize) -> Result<Value, ErrorBody> {
    serde_json::to_value(value).map_err(internal)
}

/// Keeps a thread (PRT-025) or a workspace (DMN-023) watched while its subscription lives.
enum Watch {
    Thread { _guard: WatchGuard },
    Workspace { _guard: WorkspaceWatchGuard },
}

/// A live subscription; dropping it stops delivery and stops watching its thread or workspace (PRT-024, PRT-025,
/// DMN-023).
struct Subscribed {
    generation: u64,
    task: JoinHandle<()>,
    /// Forwards the topic's transient frames (PRT-054): a thread's `live_levels` (DMN-110).
    transient: Option<JoinHandle<()>>,
    _watch: Option<Watch>,
}

impl Drop for Subscribed {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(transient) = &self.transient {
            transient.abort();
        }
    }
}

struct Connection {
    app: Arc<AppState>,
    /// Names this connection's widget report (PRT-043) and the live sessions it owns (PRT-051).
    number: u64,
    principal: Principal,
    out: mpsc::Sender<Outbound>,
    subscriptions: HashMap<Topic, Subscribed>,
    /// Forwarders report topics they dropped after an overflow.
    dropped: mpsc::UnboundedSender<(Topic, u64)>,
    generation: u64,
}

impl Connection {
    async fn run(
        &mut self,
        stream: &mut futures::stream::SplitStream<WebSocket>,
        mut dropped: mpsc::UnboundedReceiver<(Topic, u64)>,
    ) {
        let mut shutdown = self.app.shutdown.clone();
        loop {
            tokio::select! {
                message = stream.next() => match message {
                    Some(Ok(Message::Text(text))) => self.on_text(&text).await,
                    Some(Ok(Message::Binary(_))) => {
                        let error = bad_request("binary frames are not part of the protocol");
                        send(&self.out, Outbound::response(String::new(), Err(error))).await;
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                },
                Some((topic, generation)) = dropped.recv() => {
                    if self
                        .subscriptions
                        .get(&topic)
                        .is_some_and(|subscribed| subscribed.generation == generation)
                    {
                        self.subscriptions.remove(&topic);
                    }
                }
                () = async { let _ = shutdown.wait_for(|stopping| *stopping).await; } => {
                    send(&self.out, Some(Outbound::Close(CLOSE_GOING_AWAY, "daemon shutting down"))).await;
                    break;
                }
            }
        }
    }

    async fn on_text(&mut self, text: &str) {
        let Request { id, call } = match decode_request(text) {
            Ok(request) => request,
            Err(error) => {
                let id = error.id.clone().unwrap_or_default();
                send(
                    &self.out,
                    Outbound::response(id, Err(error.to_error_body())),
                )
                .await;
                return;
            }
        };
        if let Err(error) = authorize(&self.principal, call.method()) {
            send(&self.out, Outbound::response(id, Err(error))).await;
            return;
        }
        if let Some(thread_id) = driven_thread(&call) {
            self.app.services.supervisor.used(thread_id, self.number);
        }
        match call {
            Call::SystemHello(_) => {
                let error = bad_request("system.hello was already sent on this connection");
                send(&self.out, Outbound::response(id, Err(error))).await;
            }
            Call::Subscribe(params) => self.subscribe(id, params).await,
            Call::Unsubscribe(params) => {
                // Dropping the subscription stops its forwarder before the response is queued, so no event of the
                // topic follows it (PRT-024).
                self.subscriptions.remove(&params.topic);
                let result = to_value(&omfp_protocol::Empty {});
                send(&self.out, Outbound::response(id, result)).await;
            }
            Call::WidgetsReport(params) => {
                // Kept per connection: a window replaces its own report (PRT-043).
                self.app
                    .services
                    .report_widgets(self.number, self.principal.client(), params);
                let result = to_value(&omfp_protocol::Empty {});
                send(&self.out, Outbound::response(id, result)).await;
            }
            Call::WindowsAnswer(params) => {
                // Only the window a request went to answers it (PRT-124).
                let result = self
                    .app
                    .services
                    .answer_window(self.number, params)
                    .and_then(|()| to_value(&omfp_protocol::Empty {}));
                send(&self.out, Outbound::response(id, result)).await;
            }
            Call::ThreadLiveConnect(params) => {
                // The connection owns the session: closing it stops the session (DMN-103).
                let owner = LiveOwner {
                    client: self.principal.client().to_owned(),
                    connection: self.number,
                };
                let supervisor = self.app.services.supervisor.clone();
                let out = self.out.clone();
                tokio::spawn(async move {
                    let result = supervisor
                        .live_connect(&params.thread_id, owner, params.sdp_offer)
                        .await
                        .map_err(supervisor_error)
                        .and_then(|answer| {
                            to_value(&LiveConnected {
                                sdp_answer: answer.sdp_answer,
                                voice: answer.voice,
                            })
                        });
                    send(&out, Outbound::response(id, result)).await;
                });
            }
            Call::ThreadLiveStart(ThreadRef { thread_id }) => {
                // Owned like a `live_connect` session (DMN-109, DMN-103).
                let owner = LiveOwner {
                    client: self.principal.client().to_owned(),
                    connection: self.number,
                };
                let supervisor = self.app.services.supervisor.clone();
                let out = self.out.clone();
                tokio::spawn(async move {
                    let result = supervisor
                        .live_start(&thread_id, owner)
                        .await
                        .map_err(supervisor_error)
                        .and_then(|voice| to_value(&LiveStarted { voice }));
                    send(&out, Outbound::response(id, result)).await;
                });
            }
            Call::AuthLogin(params) => self.log_in(id, params.provider_id),
            call => {
                // Requests run concurrently; responses may arrive out of order (PRT-010).
                let services = self.app.services.clone();
                let principal = self.principal.clone();
                let out = self.out.clone();
                tokio::spawn(async move {
                    let result = dispatch(&services, &principal, call).await;
                    send(&out, Outbound::response(id, result)).await;
                });
            }
        }
    }

    /// `auth.login` of this window's user: the flow goes to this window (DMN-221).
    fn log_in(&self, id: String, provider_id: String) {
        let services = self.app.services.clone();
        let number = self.number;
        let out = self.out.clone();
        tokio::spawn(async move {
            let result = services
                .auth_login(Some(number), provider_id)
                .await
                .and_then(|logged_in| to_value(&logged_in));
            send(&out, Outbound::response(id, result)).await;
        });
    }

    #[allow(clippy::too_many_lines)] // One arm per topic.
    async fn subscribe(&mut self, id: String, params: SubscribeParams) {
        let SubscribeParams { topic, since } = params;
        let since = since.map(|position| Position {
            epoch: position.epoch,
            seq: position.seq,
        });
        // A new subscription replaces the previous one of the topic.
        self.subscriptions.remove(&topic);
        self.generation += 1;
        let generation = self.generation;
        let services = &self.app.services;
        let mut transient = None;
        let started = match &topic {
            Topic::Index => {
                let subscription = services.index.subscribe(since.as_ref());
                Ok((
                    self.start(
                        id,
                        topic.clone(),
                        since,
                        subscription,
                        TopicEvent::Index,
                        generation,
                    )
                    .await,
                    None,
                ))
            }
            Topic::Thread(thread_id) => {
                let watched = async {
                    let log = services.supervisor.thread_log(thread_id).await?;
                    let levels = services.supervisor.live_levels(thread_id).await?;
                    let guard = services.supervisor.watch(thread_id, self.number).await?;
                    Ok((log, levels, guard))
                }
                .await
                .map_err(supervisor_error);
                match watched {
                    Ok((log, levels, guard)) => {
                        let subscription = log.subscribe(since.as_ref());
                        let task = self
                            .start(
                                id.clone(),
                                topic.clone(),
                                since,
                                subscription,
                                TopicEvent::Thread,
                                generation,
                            )
                            .await;
                        transient = Some(tokio::spawn(forward_levels(
                            topic.clone(),
                            levels,
                            self.out.clone(),
                        )));
                        Ok((task, Some(Watch::Thread { _guard: guard })))
                    }
                    Err(error) => Err((id, error)),
                }
            }
            Topic::Workspace(workspace_id) => match services.workspace_log(workspace_id).await {
                Ok(log) => {
                    let guard = services.supervisor.watch_workspace(workspace_id);
                    let subscription = log.subscribe(since.as_ref());
                    let task = self
                        .start(
                            id.clone(),
                            topic.clone(),
                            since,
                            subscription,
                            TopicEvent::Workspace,
                            generation,
                        )
                        .await;
                    Ok((task, Some(Watch::Workspace { _guard: guard })))
                }
                Err(error) => Err((id, error)),
            },
            Topic::Views(workspace_id) => match services.views_log(workspace_id).await {
                Ok(log) => {
                    // Watched like `workspace/<id>`: the polls read its views every round (DMN-082).
                    let guard = services.supervisor.watch_workspace(workspace_id);
                    let subscription = log.subscribe(since.as_ref());
                    let task = self
                        .start(
                            id.clone(),
                            topic.clone(),
                            since,
                            subscription,
                            TopicEvent::Views,
                            generation,
                        )
                        .await;
                    Ok((task, Some(Watch::Workspace { _guard: guard })))
                }
                Err(error) => Err((id, error)),
            },
            Topic::Job(job_id) => {
                // A run of a declared job the engine no longer holds serves what the store kept (DMN-330).
                let log = match services.jobs.log(job_id) {
                    Ok(log) => Ok(log),
                    Err(error) => services.declared.stored_log(job_id).await.ok_or(error),
                };
                match log {
                    Ok(log) => {
                        let subscription = log.subscribe(since.as_ref());
                        let task = self
                            .start(
                                id,
                                topic.clone(),
                                since,
                                subscription,
                                TopicEvent::Job,
                                generation,
                            )
                            .await;
                        Ok((task, None))
                    }
                    Err(error) => Err((id, error.into())),
                }
            }
            Topic::Terminal(terminal_id) => match services.terminals.log(terminal_id, None) {
                Ok(log) => {
                    let subscription = log.subscribe(since.as_ref());
                    let task = self
                        .start(
                            id,
                            topic.clone(),
                            since,
                            subscription,
                            TopicEvent::Terminal,
                            generation,
                        )
                        .await;
                    Ok((task, None))
                }
                Err(error) => Err((id, error.into())),
            },
            Topic::Jobs(workspace_id) => match services.declared.log(workspace_id).await {
                Ok(log) => {
                    // Watched like `views/<id>`: the polls read its jobs file every round (DMN-322).
                    let guard = services.supervisor.watch_workspace(workspace_id);
                    let subscription = log.subscribe(since.as_ref());
                    let task = self
                        .start(
                            id.clone(),
                            topic.clone(),
                            since,
                            subscription,
                            TopicEvent::Jobs,
                            generation,
                        )
                        .await;
                    Ok((task, Some(Watch::Workspace { _guard: guard })))
                }
                Err(error) => Err((id, error)),
            },
        };
        match started {
            Ok((task, watch)) => {
                self.subscriptions.insert(
                    topic,
                    Subscribed {
                        generation,
                        task,
                        transient,
                        _watch: watch,
                    },
                );
            }
            Err((id, error)) => send(&self.out, Outbound::response(id, Err(error))).await,
        }
    }

    /// Answers the subscribe request, then forwards the replayed and live events in a task.
    async fn start<E, S>(
        &self,
        id: String,
        topic: Topic,
        since: Option<Position>,
        subscription: Subscription<E, S>,
        event: fn(E) -> TopicEvent,
        generation: u64,
    ) -> JoinHandle<()>
    where
        E: Clone + Send + Sync + 'static,
        S: Serialize,
    {
        let Subscription { start, live } = subscription;
        let (result, replay) = match start {
            Start::Replay(events) => {
                let position = since.unwrap_or(Position {
                    epoch: String::new(),
                    seq: 0,
                });
                let result = SubscribeResult::Replay {
                    epoch: position.epoch,
                    from_seq: position.seq + 1,
                };
                (to_value(&result), events)
            }
            Start::Snapshot { epoch, seq, state } => {
                let result = to_value(&state).map(|state| SubscribeResult::Snapshot {
                    epoch: epoch.to_string(),
                    seq,
                    state,
                });
                (result.and_then(|result| to_value(&result)), Vec::new())
            }
        };
        send(&self.out, Outbound::response(id, result)).await;
        let out = self.out.clone();
        let dropped = self.dropped.clone();
        tokio::spawn(async move {
            if forward(&topic, replay, live, &out, event).await == Ended::Overflow {
                let _ = dropped.send((topic, generation));
            }
        })
    }
}

/// The thread a request sends something to on the user's behalf (a message, a shell command, an answer, an abort or
/// a live session): the window that sent it is the one the user last used on that thread (DMN-060).
fn driven_thread(call: &Call) -> Option<&str> {
    match call {
        Call::ThreadPrompt(PromptParams { thread_id, .. })
        | Call::ThreadSteer(MessageParams { thread_id, .. })
        | Call::ThreadFollowUp(MessageParams { thread_id, .. })
        | Call::ThreadAbort(ThreadRef { thread_id })
        | Call::ThreadBash(BashParams { thread_id, .. })
        | Call::ThreadBtw(BtwParams { thread_id, .. })
        | Call::InteractionRespond(RespondParams { thread_id, .. })
        | Call::ThreadLiveConnect(LiveConnectParams { thread_id, .. })
        | Call::ThreadLiveStart(ThreadRef { thread_id }) => Some(thread_id),
        _ => None,
    }
}

/// How a forwarder ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The connection or the topic went away.
    Closed,
    /// The client fell behind; `resync_required` was sent (PRT-023).
    Overflow,
}

fn event_frame<E: Clone>(
    topic: &Topic,
    sequenced: &Sequenced<E>,
    event: fn(E) -> TopicEvent,
) -> Option<Outbound> {
    Outbound::frame(&ServerFrame::Event(EventFrame {
        topic: topic.clone(),
        epoch: sequenced.epoch.to_string(),
        seq: sequenced.seq,
        event: event(sequenced.event.clone()),
    }))
}

/// Sends a thread's `live_levels` as transient frames (PRT-054, DMN-110): a value the client cannot take now is
/// dropped, and a lagging stream skips to the latest, since only the latest levels matter.
async fn forward_levels(
    topic: Topic,
    mut levels: broadcast::Receiver<omp_rpc::LiveLevelsEvent>,
    out: mpsc::Sender<Outbound>,
) {
    loop {
        match levels.recv().await {
            Ok(levels) => {
                let frame = ServerFrame::Transient {
                    topic: topic.clone(),
                    event: TransientEvent::LiveLevels {
                        input: levels.input,
                        output: levels.output,
                    },
                };
                if let Some(frame) = Outbound::frame(&frame)
                    && let Err(mpsc::error::TrySendError::Closed(_)) = out.try_send(frame)
                {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Sends the replayed events, then live ones in order (PRT-022). When the outbound queue is full or the live stream
/// lagged, the topic is dropped with `resync_required` and the connection stays open (PRT-023).
pub(crate) async fn forward<E: Clone>(
    topic: &Topic,
    replay: Vec<Sequenced<E>>,
    mut live: broadcast::Receiver<Arc<Sequenced<E>>>,
    out: &mpsc::Sender<Outbound>,
    event: fn(E) -> TopicEvent,
) -> Ended {
    for sequenced in &replay {
        if let Some(frame) = event_frame(topic, sequenced, event)
            && out.send(frame).await.is_err()
        {
            return Ended::Closed;
        }
    }
    loop {
        match live.recv().await {
            Ok(sequenced) => {
                let Some(frame) = event_frame(topic, &sequenced, event) else {
                    continue;
                };
                match out.try_send(frame) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => break,
                    Err(mpsc::error::TrySendError::Closed(_)) => return Ended::Closed,
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => break,
            Err(broadcast::error::RecvError::Closed) => return Ended::Closed,
        }
    }
    let resync = ServerFrame::ResyncRequired {
        topic: topic.clone(),
    };
    send(out, Outbound::frame(&resync)).await;
    Ended::Overflow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_log::EventLog;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn keep_last(state: &mut i64, event: &omfp_protocol::IndexEvent) {
        let _ = event;
        *state += 1;
    }

    fn removed(id: &str) -> omfp_protocol::IndexEvent {
        omfp_protocol::IndexEvent::ThreadRemoved {
            thread_id: id.to_owned(),
        }
    }

    // spec: PRT-023
    #[tokio::test]
    async fn a_full_outbound_queue_drops_the_topic_with_resync_required() {
        let log = EventLog::new("e1", 0, keep_last, 100, 100);
        let subscription = log.subscribe(None);
        let (out, mut outbox) = mpsc::channel(2);
        for id in ["a", "b", "c", "d"] {
            log.push(removed(id));
        }

        let ended = forward(
            &Topic::Index,
            Vec::new(),
            subscription.live,
            &out,
            TopicEvent::Index,
        );
        let (ended, frames) = tokio::join!(ended, async {
            let mut frames = Vec::new();
            while let Some(Outbound::Text(text)) = outbox.recv().await {
                frames.push(serde_json::from_str::<Value>(&text).unwrap());
                if frames.len() == 3 {
                    break;
                }
            }
            frames
        });

        assert_eq!(ended, Ended::Overflow);
        let kinds: Vec<&str> = frames
            .iter()
            .map(|frame| frame["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["event", "event", "resync_required"]);
        assert_eq!(frames[1]["seq"], 2);
        assert_eq!(frames[2]["topic"], "index");
    }

    // spec: PRT-022
    #[tokio::test]
    async fn replayed_events_come_before_live_ones_in_seq_order() {
        let log = EventLog::new("e1", 0, keep_last, 100, 100);
        log.push(removed("a"));
        log.push(removed("b"));
        let subscription = log.subscribe(Some(&Position {
            epoch: "e1".into(),
            seq: 0,
        }));
        let Start::Replay(replay) = subscription.start else {
            panic!("expected a replay");
        };
        log.push(removed("c"));
        let (out, mut outbox) = mpsc::channel(16);

        let task = tokio::spawn(async move {
            forward(
                &Topic::Index,
                replay,
                subscription.live,
                &out,
                TopicEvent::Index,
            )
            .await
        });
        let mut seqs = Vec::new();
        while seqs.len() < 3 {
            let Some(Outbound::Text(text)) = outbox.recv().await else {
                panic!("forwarder stopped");
            };
            seqs.push(
                serde_json::from_str::<Value>(&text).unwrap()["seq"]
                    .as_u64()
                    .unwrap(),
            );
        }
        task.abort();

        assert_eq!(seqs, [1, 2, 3]);
    }
}
