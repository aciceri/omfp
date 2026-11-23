//! A native protocol client (`tokio-tungstenite`) for the daemon's integration (L2) and contract (L3) tests, and the
//! fake-launcher daemon of the L2 tests ([`TestDaemon`]).

#![allow(dead_code)] // Each test binary uses a different subset.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use omfp_core::{ThreadEvent, ThreadState};
use omfp_daemon::agent::FakeLauncher;
use omfp_daemon::config::Config;
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_protocol::Method;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

/// How long a test waits for a frame before failing.
pub const WAIT: Duration = Duration::from_secs(60);

/// A daemon on port 0 over a temporary data directory, with the fake launcher and fake git (ARC-012); the user's own
/// omp is absent ([`omfp_testkit::absent_standalone_omp`]).
pub struct TestDaemon {
    pub handle: DaemonHandle,
    pub launcher: FakeLauncher,
    pub git: Arc<FakeGit>,
    pub dir: TempDir,
}

impl TestDaemon {
    /// [`Config::loopback`] with `extra` arguments, over a new directory.
    pub async fn start(extra: &[&str]) -> Self {
        Self::start_in(tempfile::tempdir().unwrap(), extra).await
    }

    /// [`Config::loopback`] with `extra` arguments, over `dir` (e.g. a stopped daemon's).
    pub async fn start_in(dir: TempDir, extra: &[&str]) -> Self {
        let config = Config::loopback(dir.path(), extra);
        Self::launch(dir, config, |_| {}).await
    }

    /// A daemon over `dir` with `config`, its ports changed by `adjust` first.
    pub async fn launch(dir: TempDir, config: Config, adjust: impl FnOnce(&mut Ports)) -> Self {
        let launcher = FakeLauncher::new();
        let git = Arc::new(FakeGit::new());
        let mut ports = fake_ports(dir.path(), &launcher, &git);
        adjust(&mut ports);
        let handle = start_with(config, ports).await.unwrap();
        Self {
            handle,
            launcher,
            git,
            dir,
        }
    }

    /// A client said hello as `name`.
    pub async fn client(&self, name: &str) -> Client {
        Client::hello(self.handle.addr(), self.handle.token().expose(), name).await
    }

    /// Stops the daemon and starts another one over the same directory.
    pub async fn restart(self) -> Self {
        self.handle.shutdown().await.unwrap();
        Self::start_in(self.dir, &[]).await
    }

    /// A shell loop running until the file `gate` of the test's directory exists.
    pub fn hold(&self, gate: &str) -> String {
        format!(
            "while [ ! -e '{}' ]; do sleep 0.01; done",
            self.dir.path().join(gate).display()
        )
    }
}

/// The fake ports of a daemon over `dir`, sharing `launcher` and `git`.
pub fn fake_ports(dir: &Path, launcher: &FakeLauncher, git: &Arc<FakeGit>) -> Ports {
    Ports::fake(
        Arc::new(launcher.clone()),
        Arc::clone(git) as _,
        omfp_testkit::absent_standalone_omp(dir),
    )
}

pub struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    /// Frames read while waiting for a response.
    buffered: VecDeque<Value>,
}

pub fn url(addr: SocketAddr) -> String {
    format!("ws://{addr}/api/ws")
}

impl Client {
    /// Opens a connection without saying hello.
    pub async fn connect(addr: SocketAddr) -> Self {
        let (ws, _) = connect_async(url(addr)).await.unwrap();
        Self::from(ws)
    }

    /// Opens a connection with an `Origin` and a `Host` header, as a browser behind a reverse proxy that passes its
    /// `Host` on (or replaces it) does.
    pub async fn connect_with_origin_and_host(
        addr: SocketAddr,
        origin: &str,
        host: &str,
    ) -> Result<Self, tungstenite::Error> {
        let mut request = url(addr).into_client_request().unwrap();
        let headers = request.headers_mut();
        headers.insert("origin", origin.parse().unwrap());
        headers.insert("host", host.parse().unwrap());
        connect_async(request).await.map(|(ws, _)| Self::from(ws))
    }

    fn from(ws: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        Self {
            ws,
            next_id: 0,
            buffered: VecDeque::new(),
        }
    }

    /// Connects and authenticates as `client`.
    pub async fn hello(addr: SocketAddr, token: &str, client: &str) -> Self {
        let mut connection = Self::connect(addr).await;
        let result = connection
            .call(
                "system.hello",
                json!({ "token": token, "protocol": 1, "client": client }),
            )
            .await;
        assert!(result.is_ok(), "hello failed: {result:?}");
        connection
    }

    pub async fn send_text(&mut self, text: String) {
        self.ws.send(Message::Text(text.into())).await.unwrap();
    }

    /// Sends a request and returns its response frame; other frames are kept for [`Client::frame`].
    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = format!("c{}", self.next_id);
        self.send_text(
            json!({ "type": "request", "id": id, "method": method, "params": params }).to_string(),
        )
        .await;
        loop {
            let frame = self
                .read()
                .await
                .unwrap_or_else(|| panic!("connection closed waiting for {method}"));
            if frame["type"] == "response" && frame["id"] == id {
                return frame;
            }
            self.buffered.push_back(frame);
        }
    }

    /// The result, or the error object.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let response = self.request(method, params).await;
        if response["ok"] == true {
            Ok(response["result"].clone())
        } else {
            Err(response["error"].clone())
        }
    }

    /// The result; panics with the error.
    pub async fn ok(&mut self, method: &str, params: Value) -> Value {
        self.call(method, params)
            .await
            .unwrap_or_else(|error| panic!("{method} failed: {error}"))
    }

    /// Calls `M` with typed params; panics with the error, or when the result is not `M`'s declared type (AGT-007).
    pub async fn typed<M: Method>(&mut self, params: M::Params) -> M::Result {
        let result = self
            .ok(M::NAME, serde_json::to_value(params).unwrap())
            .await;
        serde_json::from_value(result.clone()).unwrap_or_else(|error| {
            panic!(
                "{} returned {result}, not its declared result: {error}",
                M::NAME
            )
        })
    }

    /// The next frame that is not the response to one of our requests.
    pub async fn frame(&mut self) -> Value {
        if let Some(frame) = self.buffered.pop_front() {
            return frame;
        }
        self.read().await.expect("connection closed")
    }

    /// The next event of `topic`.
    pub async fn event(&mut self, topic: &str) -> Value {
        loop {
            let frame = self.frame().await;
            if frame["type"] == "event" && frame["topic"] == topic {
                return frame;
            }
        }
    }

    /// The next transient frame of `topic` (PRT-054); the frames before it stay unread.
    pub async fn transient(&mut self, topic: &str) -> Value {
        let mut skipped = Vec::new();
        loop {
            let frame = self.frame().await;
            if frame["type"] == "transient" && frame["topic"] == topic {
                for earlier in skipped.into_iter().rev() {
                    self.buffered.push_front(earlier);
                }
                return frame;
            }
            skipped.push(frame);
        }
    }

    /// Reads one text frame; `None` once closed.
    async fn read(&mut self) -> Option<Value> {
        loop {
            let message = tokio::time::timeout(WAIT, self.ws.next())
                .await
                .expect("timed out waiting for a frame")?;
            match message.ok()? {
                Message::Text(text) => return Some(serde_json::from_str(&text).unwrap()),
                Message::Close(_) => return None,
                _ => {}
            }
        }
    }

    /// Reads until the daemon closes the connection; returns its close code.
    pub async fn close_code(&mut self) -> Option<u16> {
        loop {
            let message = tokio::time::timeout(WAIT, self.ws.next())
                .await
                .expect("timed out waiting for the close")?;
            match message {
                Ok(Message::Close(frame)) => return frame.map(|frame| frame.code.into()),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    /// Frames received but not consumed yet.
    pub fn take_buffered(&mut self) -> Vec<Value> {
        self.buffered.drain(..).collect()
    }
}

/// A thread followed from a subscription: the snapshot, then every event applied with the shared reducer (the
/// client side of PRT-021).
pub struct Follower {
    pub topic: String,
    pub epoch: String,
    pub seq: u64,
    pub state: ThreadState,
}

impl Follower {
    /// Subscribes to `thread/<id>` from `since`, expecting a snapshot.
    pub async fn snapshot(
        client: &mut Client,
        thread_id: &str,
        since: Option<(&str, u64)>,
    ) -> Self {
        let topic = format!("thread/{thread_id}");
        let mut params = json!({ "topic": topic });
        if let Some((epoch, seq)) = since {
            params["since"] = json!({ "epoch": epoch, "seq": seq });
        }
        let result = client.ok("subscribe", params).await;
        assert_eq!(result["mode"], "snapshot", "{result}");
        Self {
            topic,
            epoch: result["epoch"].as_str().unwrap().to_owned(),
            seq: result["seq"].as_u64().unwrap(),
            state: serde_json::from_value(result["state"].clone()).unwrap(),
        }
    }

    /// Applies one event frame; checks it continues the stream without a gap (PRT-022).
    pub fn apply(&mut self, frame: &Value) {
        let epoch = frame["epoch"].as_str().unwrap();
        let seq = frame["seq"].as_u64().unwrap();
        if epoch == self.epoch {
            assert_eq!(seq, self.seq + 1, "gap in {}: {frame}", self.topic);
        } else {
            // A new process: numbering restarts (PRT-022).
            assert_eq!(seq, 1, "a new epoch starts at 1: {frame}");
            epoch.clone_into(&mut self.epoch);
        }
        self.seq = seq;
        let event: ThreadEvent = serde_json::from_value(frame["event"].clone()).unwrap();
        self.state.apply(&event);
    }

    /// Applies events until `done` holds.
    pub async fn until(&mut self, client: &mut Client, done: impl Fn(&ThreadState) -> bool) {
        while !done(&self.state) {
            let frame = client.event(&self.topic).await;
            self.apply(&frame);
        }
    }
}
