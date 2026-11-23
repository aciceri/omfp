//! Shared helpers for the daemon's contract tests (L3): an isolated omp wired to the fake LLM, laid out like a
//! thread, the real daemon on it ([`omp_daemon`]), and a recorder turning omp's frames into reducer inputs (fixtures,
//! TST-012).

#![allow(dead_code)] // Each test binary uses a different subset.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fake_llm::{FakeLlm, Script, Step};
use omfp_core::ThreadEvent;
use omfp_daemon::config::Config;
use omfp_daemon::omp::{LaunchSpec, OmpProcess, Startup, startup_events, thread_event};
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_testkit::{
    DenyProxy, FAKE_MODEL, FAKE_PROVIDER, KNOWN_DISCOVERY_HOSTS, OmpEnv, normalize_frame, omp_bin,
    require_pinned_omp,
};
use omp_rpc::{Event, PromptCommand};
use tempfile::TempDir;

pub const TIMEOUT: Duration = Duration::from_secs(60);

/// An isolated omp environment plus a thread-like layout: a workspace folder (cwd), sessions and log.
pub struct Harness {
    pub fake: FakeLlm,
    pub proxy: DenyProxy,
    pub env: OmpEnv,
    pub thread: TempDir,
}

impl Harness {
    pub async fn new(steps: Vec<Step>) -> Self {
        require_pinned_omp();
        let fake = FakeLlm::start(Script::new(FAKE_MODEL, steps))
            .await
            .unwrap();
        let proxy = DenyProxy::start().await.unwrap();
        let env = OmpEnv::new(&fake.base_url(), Some(&proxy)).unwrap();
        Self {
            fake,
            proxy,
            env,
            thread: tempfile::tempdir().unwrap(),
        }
    }

    /// The omp cwd of [`Harness::spec`]: a plain folder, like a workspace.
    pub fn workspace(&self) -> PathBuf {
        self.thread.path().join("workspace")
    }

    pub fn spec(&self, add_dirs: &[&Path]) -> LaunchSpec {
        LaunchSpec {
            program: omp_bin().into(),
            cwd: self.workspace(),
            add_dirs: add_dirs.iter().map(|dir| dir.to_path_buf()).collect(),
            session_dir: self.thread.path().join("sessions"),
            fork: None,
            model: Some(format!("{FAKE_PROVIDER}/{FAKE_MODEL}")),
            approval_mode: None,
            append_system_prompt: None,
            extension: None,
            stderr_log: self.thread.path().join("omp.log"),
            env: self.env.env_overrides(),
        }
    }

    pub fn stderr(&self) -> String {
        std::fs::read_to_string(self.thread.path().join("omp.log")).unwrap_or_default()
    }

    pub fn assert_no_stray_network(&self) {
        let stray = self.proxy.unexpected_attempts(KNOWN_DISCOVERY_HOSTS);
        assert!(stray.is_empty(), "stray network access: {stray:#?}");
        assert!(
            self.fake.failures().is_empty(),
            "unexpected model requests: {:#?}",
            self.fake.failures()
        );
    }

    /// Temporary paths that must not leak into fixtures.
    pub fn roots(&self) -> Vec<String> {
        vec![
            self.thread.path().to_string_lossy().into_owned(),
            self.env.root().to_string_lossy().into_owned(),
        ]
    }
}

/// The real daemon over a new data directory: the real ports, with the pinned omp isolated by `harness`'s environment
/// and `extra` daemon arguments (e.g. `--no-title-generation`: the fake LLM scripts every model call, and a title
/// run would take one, DMN-073).
pub async fn omp_daemon(harness: &Harness, extra: &[&str]) -> (DaemonHandle, TempDir) {
    omp_daemon_with(harness, extra, |_| {}).await
}

/// [`omp_daemon`], its ports changed by `adjust` first.
pub async fn omp_daemon_with(
    harness: &Harness,
    extra: &[&str],
    adjust: impl FnOnce(&mut Ports),
) -> (DaemonHandle, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let omp = omp_bin();
    let args: Vec<&str> = ["--omp-bin", omp.as_str()]
        .into_iter()
        .chain(extra.iter().copied())
        .collect();
    let config = Config::loopback(data.path(), &args);
    let mut ports = Ports::resolve(&config).unwrap();
    ports.omp_env = harness.env.prepare_omfp_profile(data.path()).unwrap();
    adjust(&mut ports);
    (start_with(config, ports).await.unwrap(), data)
}

pub fn prompt(text: &str) -> PromptCommand {
    PromptCommand {
        message: text.into(),
        images: None,
        streaming_behavior: None,
    }
}

/// The system prompt of a recorded chat-completions request.
pub fn system_prompt(request: &serde_json::Value) -> String {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "system" || message["role"] == "developer")
        .map(|message| match &message["content"] {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .collect()
}

/// The `<workspace-roots>` list of a system prompt, one path per entry.
pub fn workspace_roots(prompt: &str) -> Vec<String> {
    let Some(start) = prompt.find("<workspace-roots>") else {
        return Vec::new();
    };
    let end = prompt[start..].find("</workspace-roots>").unwrap() + start;
    prompt[start..end]
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(str::to_owned)
        .collect()
}

pub fn start(spec: &LaunchSpec) -> (OmpProcess, Startup) {
    OmpProcess::start(spec, Vec::new(), Vec::new()).unwrap_or_else(|error| {
        let log = std::fs::read_to_string(&spec.stderr_log).unwrap_or_default();
        panic!("omp failed to start: {error}\nstderr:\n{log}")
    })
}

/// Collects reducer inputs: the startup events, every frame of the event receiver, and events added by the test.
#[derive(Clone, Default)]
pub struct Recorder {
    events: Arc<(Mutex<Vec<ThreadEvent>>, Condvar)>,
}

impl Recorder {
    /// Starts recording a freshly started process, as the agent supervisor will see it.
    pub fn attach(process: &mut OmpProcess, startup: &Startup) -> Self {
        let recorder = Self::default();
        for event in startup_events(startup) {
            recorder.push(event);
        }
        let receiver: Receiver<Event> = process.take_events().expect("events not taken yet");
        let sink = recorder.clone();
        std::thread::spawn(move || {
            for event in receiver {
                sink.push(thread_event(event));
            }
        });
        recorder
    }

    pub fn push(&self, event: ThreadEvent) {
        let (events, changed) = &*self.events;
        events.lock().unwrap().push(event);
        changed.notify_all();
    }

    pub fn events(&self) -> Vec<ThreadEvent> {
        self.events.0.lock().unwrap().clone()
    }

    /// Waits until `predicate` holds for the recorded events, failing after [`TIMEOUT`].
    pub fn wait_until(&self, what: &str, predicate: impl Fn(&[ThreadEvent]) -> bool) {
        let (events, changed) = &*self.events;
        let deadline = Instant::now() + TIMEOUT;
        let mut guard = events.lock().unwrap();
        while !predicate(&guard) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "timed out waiting for {what}; got {guard:#?}"
            );
            guard = changed.wait_timeout(guard, left).unwrap().0;
        }
    }

    /// Number of recorded frames of omp type `kind` (e.g. `session_settled`).
    pub fn count(events: &[ThreadEvent], kind: &str) -> usize {
        events
            .iter()
            .filter(|event| frame_type(event).as_deref() == Some(kind))
            .count()
    }

    /// With `OMFP_RECORD_FIXTURES` set, writes the events to `fixtures/threads/<OMP_VERSION>/<name>.jsonl`,
    /// normalized like omp frames (TST-012).
    pub fn record(&self, harness: &Harness, name: &str) {
        if std::env::var_os("OMFP_RECORD_FIXTURES").is_none() {
            return;
        }
        let dir = fixtures_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let mut out = String::new();
        for event in self.events() {
            let mut value = serde_json::to_value(&event).unwrap();
            for root in harness.roots() {
                normalize_frame(&mut value, &root);
            }
            out.push_str(&value.to_string());
            out.push('\n');
        }
        std::fs::write(dir.join(format!("{name}.jsonl")), out).unwrap();
    }
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/threads")
        .join(omfp_core::OMP_VERSION)
}

/// The omp `type` of a recorded frame, if it is one.
pub fn frame_type(event: &ThreadEvent) -> Option<String> {
    match event {
        ThreadEvent::Omp { .. } => serde_json::to_value(event)
            .ok()
            .and_then(|value| value["frame"]["type"].as_str().map(str::to_owned)),
        _ => None,
    }
}
