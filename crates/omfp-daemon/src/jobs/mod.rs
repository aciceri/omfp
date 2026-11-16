//! Jobs (DMN-080, DMN-081): shell commands the daemon runs for a workspace, each as `sh -c <command>` in its own
//! process group, its output lines published on the `job/<id>` topic (PRT-041). Jobs live in memory and belong to
//! their workspace, not to a client.

pub mod cron;
pub mod declared;
pub mod file;

use std::collections::{HashMap, VecDeque};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use omfp_protocol::{ErrorBody, ErrorCode, JobEvent, JobInfo, JobLine, JobStatus, JobStream};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::{oneshot, watch};
use tracing::warn;

use crate::event_log::EventLog;
use crate::store::{new_id, now_ms};

/// A longer output line is cut into pieces of at most this many bytes.
pub const LINE_BYTES: usize = 64 * 1024;
/// The topic's state keeps the newest lines totalling at most this much text.
pub const KEPT_TEXT_BYTES: usize = 1024 * 1024;
/// Events kept for replay; also how far a subscriber may fall behind (PRT-023).
pub const REPLAY_EVENTS: usize = 4096;
/// Finished jobs kept per workspace; the oldest finished one is forgotten first.
pub const FINISHED_KEPT: usize = 32;
/// How long a job's shell has after SIGTERM before its group gets SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_secs(2);
/// A declared run keeps at most this many of its last lines in the store (DMN-330)…
pub const TAIL_LINES: usize = 200;
/// …totalling at most this much text.
pub const TAIL_BYTES: usize = 64 * 1024;

const READ_CHUNK: usize = 8192;

/// A `job/<id>` topic.
pub type JobLog = EventLog<JobEvent, JobTopic>;

/// Why a job request failed.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("job {0} not found")]
    NotFound(String),
    #[error("the command is empty")]
    EmptyCommand,
    #[error("the daemon is shutting down")]
    ShuttingDown,
    #[error("job {0} belongs to another workspace")]
    OtherWorkspace(String),
    #[error("cannot start the job: {0}")]
    Spawn(#[from] std::io::Error),
}

impl From<JobError> for ErrorBody {
    fn from(error: JobError) -> Self {
        let code = match &error {
            JobError::NotFound(_) => ErrorCode::NotFound,
            JobError::EmptyCommand | JobError::OtherWorkspace(_) => ErrorCode::BadRequest,
            JobError::ShuttingDown | JobError::Spawn(_) => ErrorCode::Internal,
        };
        ErrorBody::new(code, error.to_string())
    }
}

/// The state of a `job/<id>` topic, serialized as an [`omfp_protocol::JobState`]; it also counts the bytes of its
/// lines so the 1 MiB cap costs nothing per line.
#[derive(Debug, Clone)]
pub struct JobTopic {
    job: JobInfo,
    lines: VecDeque<JobLine>,
    text_bytes: usize,
    dropped: u64,
}

impl JobTopic {
    fn new(job: JobInfo) -> Self {
        Self {
            job,
            lines: VecDeque::new(),
            text_bytes: 0,
            dropped: 0,
        }
    }
}

/// The `job/<id>` log of a run the daemon no longer runs, from what the store kept of it (DMN-330): its last lines,
/// `dropped` counting the others.
#[must_use]
pub fn stored_log(job: JobInfo, lines: Vec<JobLine>, dropped: u64) -> Arc<JobLog> {
    let text_bytes = lines.iter().map(|line| line.text.len()).sum();
    let topic = JobTopic {
        job,
        lines: lines.into(),
        text_bytes,
        dropped,
    };
    Arc::new(EventLog::new(
        new_id(),
        topic,
        apply,
        REPLAY_EVENTS,
        REPLAY_EVENTS,
    ))
}

/// How a run of a declared job ended (DMN-327, DMN-330).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub ended_at: i64,
    /// The daemon signalled the run (`jobs.kill`, a teardown, the shutdown).
    pub killed: bool,
    /// Its last [`TAIL_LINES`] lines, at most [`TAIL_BYTES`] of text, oldest first.
    pub tail: Vec<JobLine>,
    /// Every line it printed.
    pub line_count: u64,
}

/// The last lines of `topic` the store keeps (DMN-330) and the count of every line.
fn tail(topic: &JobTopic) -> (Vec<JobLine>, u64) {
    let mut bytes = 0;
    let mut tail: Vec<JobLine> = topic
        .lines
        .iter()
        .rev()
        .take(TAIL_LINES)
        .take_while(|line| {
            bytes += line.text.len();
            bytes <= TAIL_BYTES
        })
        .cloned()
        .collect();
    tail.reverse();
    (tail, topic.dropped + topic.lines.len() as u64)
}

/// [`omfp_protocol::JobState`]'s wire shape over borrowed parts.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JobStateRef<'a> {
    job: &'a JobInfo,
    lines: &'a VecDeque<JobLine>,
    dropped: u64,
}

impl Serialize for JobTopic {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        JobStateRef {
            job: &self.job,
            lines: &self.lines,
            dropped: self.dropped,
        }
        .serialize(serializer)
    }
}

/// The reducer of `job/<id>` (PRT-041).
fn apply(topic: &mut JobTopic, event: &JobEvent) {
    match event {
        JobEvent::JobOutput { stream, text } => {
            topic.text_bytes += text.len();
            topic.lines.push_back(JobLine {
                stream: *stream,
                text: text.clone(),
            });
            while topic.text_bytes > KEPT_TEXT_BYTES
                && let Some(oldest) = topic.lines.pop_front()
            {
                topic.text_bytes -= oldest.text.len();
                topic.dropped += 1;
            }
        }
        JobEvent::JobExited {
            code,
            signal,
            ended_at,
        } => {
            topic.job.status = JobStatus::Exited {
                code: *code,
                signal: *signal,
                ended_at: *ended_at,
            };
        }
    }
}

/// Cuts a byte stream into lines (DMN-080): invalid UTF-8 replaced, the line break and a `\r` before it dropped, a
/// line longer than [`LINE_BYTES`] cut into pieces (never inside a UTF-8 character).
#[derive(Debug, Default)]
struct Lines {
    pending: Vec<u8>,
}

impl Lines {
    fn push(&mut self, mut bytes: &[u8], emit: &mut impl FnMut(String)) {
        while let Some(end) = bytes.iter().position(|&byte| byte == b'\n') {
            self.pending.extend_from_slice(&bytes[..end]);
            bytes = &bytes[end + 1..];
            if self.pending.last() == Some(&b'\r') {
                self.pending.pop();
            }
            self.cut(LINE_BYTES, emit);
            emit(text(&self.pending));
            self.pending.clear();
        }
        self.pending.extend_from_slice(bytes);
        // One byte more stays pending: it may be a `\r` whose `\n` has not arrived yet.
        self.cut(LINE_BYTES + 1, emit);
    }

    /// The last line, without a line break.
    fn finish(mut self, emit: &mut impl FnMut(String)) {
        if !self.pending.is_empty() {
            self.cut(LINE_BYTES, emit);
            emit(text(&self.pending));
        }
    }

    fn cut(&mut self, longest: usize, emit: &mut impl FnMut(String)) {
        while self.pending.len() > longest {
            let end = piece_end(&self.pending);
            emit(text(&self.pending[..end]));
            self.pending.drain(..end);
        }
    }
}

/// Where the first piece of a too long line ends: at [`LINE_BYTES`], or before the UTF-8 character crossing it.
fn piece_end(bytes: &[u8]) -> usize {
    let continuation = |at: usize| bytes[at] & 0xC0 == 0x80;
    let mut end = LINE_BYTES;
    while end > LINE_BYTES - 3 && continuation(end) {
        end -= 1;
    }
    if continuation(end) { LINE_BYTES } else { end }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Publishes the lines of one output until it closes.
async fn read_lines(mut output: impl AsyncRead + Unpin, stream: JobStream, log: &JobLog) {
    let mut lines = Lines::default();
    let mut emit = |text| {
        log.push(JobEvent::JobOutput { stream, text });
    };
    let mut chunk = vec![0; READ_CHUNK];
    loop {
        match output.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => lines.push(&chunk[..read], &mut emit),
            Err(error) => {
                warn!(%error, "cannot read a job's output");
                break;
            }
        }
    }
    lines.finish(&mut emit);
}

/// Waits up to `grace` for the shell to exit; otherwise calls `kill` and waits for the exit (DMN-081).
async fn escalate(mut exited: watch::Receiver<bool>, grace: Duration, kill: impl FnOnce()) {
    // The condition's temporaries (the watch's read guard) are gone before the block runs.
    if tokio::time::timeout(grace, exited.wait_for(|exited| *exited))
        .await
        .is_err()
    {
        kill();
        let _ = exited.wait_for(|exited| *exited).await;
    }
}

/// Signals a job's process group (DMN-081). The shell leads the group, so its id is the shell's pid. Once the shell
/// is reaped the group may still hold leftovers (`sleep 60 &`); they are signalled too, after probing the group:
/// the kernel never hands out a pid still used as a process-group id by a live process, so while the probe finds a
/// member the id is still the job's. A group found empty is not signalled. What detached (`setsid`, its own group)
/// is out of reach and keeps the job running while it holds the job's outputs open.
#[derive(Debug)]
struct Control {
    group: Pid,
    /// Set right after the shell is reaped.
    exited: watch::Sender<bool>,
    /// Set once the daemon signalled the group.
    killed: AtomicBool,
}

/// How often a kill whose shell already exited checks whether the leftovers are gone.
const LEFTOVER_POLL: Duration = Duration::from_millis(50);

impl Control {
    /// Whether the group still has a member; only meaningful once the shell exited.
    fn group_alive(&self) -> bool {
        // EPERM: a member exists but is not ours, so the id is in use; anything but ESRCH means alive.
        test_kill_process_group(self.group) != Err(Errno::SRCH)
    }

    fn signal(&self, signal: Signal) {
        // Once the shell exited, the id stays the job's only while the group has a member (see [`Control`]).
        // Remaining race: the group empties and its id is reused between the probe and the signal; the signal
        // would then reach a group of the daemon's own user only (EPERM otherwise).
        if *self.exited.borrow() && !self.group_alive() {
            return;
        }
        match kill_process_group(self.group, signal) {
            // The group may be gone already.
            Err(error) if error != Errno::SRCH => {
                warn!(group = ?self.group, ?signal, %error, "cannot signal a job");
            }
            _ => {}
        }
    }

    /// Sends SIGTERM to the group now; the returned future sends SIGKILL [`KILL_GRACE`] later unless the shell
    /// exited (or, if it had exited already, unless the group emptied), and ends then (DMN-081). `None` when the
    /// shell exited and the group is empty: nothing is sent.
    fn terminate(self: &Arc<Self>) -> Option<impl Future<Output = ()> + Send + 'static> {
        let leftovers = *self.exited.borrow();
        if leftovers && !self.group_alive() {
            return None;
        }
        self.killed.store(true, Ordering::SeqCst);
        self.signal(Signal::TERM);
        let control = Arc::clone(self);
        Some(async move {
            if leftovers {
                let deadline = tokio::time::Instant::now() + KILL_GRACE;
                while control.group_alive() && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(LEFTOVER_POLL).await;
                }
                control.signal(Signal::KILL);
            } else {
                let exited = control.exited.subscribe();
                escalate(exited, KILL_GRACE, || control.signal(Signal::KILL)).await;
            }
        })
    }
}

#[derive(Debug)]
struct Entry {
    workspace_id: String,
    /// The thread whose agent started the job, if an agent did (DMN-162).
    started_by: Option<String>,
    log: Arc<JobLog>,
    control: Arc<Control>,
}

#[derive(Debug, Default)]
struct Registry {
    jobs: HashMap<String, Entry>,
    /// The finished jobs of each workspace, oldest first.
    finished: HashMap<String, VecDeque<String>>,
    /// Set at shutdown: no job starts any more.
    closed: bool,
}

impl Registry {
    /// Removes the job `id`, running or finished, and returns it.
    fn forget(&mut self, id: &str) -> Option<Entry> {
        let entry = self.jobs.remove(id)?;
        if let Some(finished) = self.finished.get_mut(&entry.workspace_id) {
            finished.retain(|kept| kept != id);
        }
        Some(entry)
    }
}

/// Every job of the daemon. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct Jobs {
    registry: Arc<Mutex<Registry>>,
}

impl Jobs {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `sh -c <command>` in `folder` for the workspace (DMN-080): the daemon's environment, stdin from
    /// `/dev/null`, a new process group. `started_by` is the thread whose agent asked, if one did: a cascade stop
    /// kills its jobs (DMN-162). `replaces` is a job of the same workspace this one takes the place of (DMN-215): it is
    /// killed if it still runs and forgotten at once; an unknown id is ignored. Must run inside a Tokio runtime.
    pub fn start(
        &self,
        workspace_id: &str,
        folder: &Path,
        command: &str,
        started_by: Option<&str>,
        replaces: Option<&str>,
    ) -> Result<JobInfo, JobError> {
        self.spawn(workspace_id, folder, command, started_by, replaces, None)
    }

    /// Runs a declared job's command in `folder` (its `cwd`) as [`Jobs::start`] runs a transient one (DMN-327), started
    /// by no thread: no thread owns a declared job's runs (DMN-329). The receiver gets how the run ended.
    pub fn start_run(
        &self,
        workspace_id: &str,
        folder: &Path,
        command: &str,
    ) -> Result<(JobInfo, oneshot::Receiver<RunExit>), JobError> {
        let (sender, receiver) = oneshot::channel();
        let job = self.spawn(workspace_id, folder, command, None, None, Some(sender))?;
        Ok((job, receiver))
    }

    fn spawn(
        &self,
        workspace_id: &str,
        folder: &Path,
        command: &str,
        started_by: Option<&str>,
        replaces: Option<&str>,
        exit: Option<oneshot::Sender<RunExit>>,
    ) -> Result<JobInfo, JobError> {
        if command.trim().is_empty() {
            return Err(JobError::EmptyCommand);
        }
        let mut registry = self.lock();
        if registry.closed {
            return Err(JobError::ShuttingDown);
        }
        if let Some(old) = replaces
            && registry
                .jobs
                .get(old)
                .is_some_and(|entry| entry.workspace_id != workspace_id)
        {
            return Err(JobError::OtherWorkspace(old.to_owned()));
        }
        // The user's shell, on the host when the daemon runs in Flatpak (ENV-064).
        let mut shell = std::process::Command::new("sh");
        shell.arg("-c").arg(command).current_dir(folder);
        let mut child = Command::from(crate::flatpak::on_host(shell))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()?;
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(Pid::from_raw)
            .ok_or_else(|| std::io::Error::other("the shell has no process id"))?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            return Err(std::io::Error::other("the shell's outputs are not piped").into());
        };
        let job = JobInfo {
            id: new_id(),
            workspace_id: workspace_id.to_owned(),
            command: command.to_owned(),
            started_at: now_ms(),
            status: JobStatus::Running,
        };
        let log = Arc::new(EventLog::new(
            new_id(),
            JobTopic::new(job.clone()),
            apply,
            REPLAY_EVENTS,
            REPLAY_EVENTS,
        ));
        let control = Arc::new(Control {
            group,
            exited: watch::Sender::new(false),
            killed: AtomicBool::new(false),
        });
        registry.jobs.insert(
            job.id.clone(),
            Entry {
                workspace_id: job.workspace_id.clone(),
                started_by: started_by.map(str::to_owned),
                log: Arc::clone(&log),
                control: Arc::clone(&control),
            },
        );
        if let Some(old) = replaces.and_then(|old| registry.forget(old))
            && let Some(escalation) = old.control.terminate()
        {
            tokio::spawn(escalation);
        }
        drop(registry);
        tokio::spawn(self.clone().run(
            job.id.clone(),
            child,
            (stdout, stderr),
            (log, control),
            exit,
        ));
        Ok(job)
    }

    /// Follows the job until it finished: shell exited and both outputs closed.
    async fn run(
        self,
        id: String,
        mut child: Child,
        (stdout, stderr): (ChildStdout, ChildStderr),
        (log, control): (Arc<JobLog>, Arc<Control>),
        exit: Option<oneshot::Sender<RunExit>>,
    ) {
        let shell = async {
            let status = child.wait().await;
            control.exited.send_replace(true);
            status
        };
        let (status, (), ()) = tokio::join!(
            shell,
            read_lines(stdout, JobStream::Stdout, &log),
            read_lines(stderr, JobStream::Stderr, &log),
        );
        let (code, signal) = match status {
            Ok(status) => (status.code(), status.signal()),
            Err(error) => {
                warn!(job = %id, %error, "cannot wait for a job's shell");
                (None, None)
            }
        };
        // Kept as finished before anyone sees it finish.
        self.finish(&id);
        let ended_at = now_ms();
        log.push(JobEvent::JobExited {
            code,
            signal,
            ended_at,
        });
        if let Some(exit) = exit {
            let (tail, line_count) = log.with_state(tail);
            // The declared runtime may be gone (shutdown); nothing waits then.
            let _ = exit.send(RunExit {
                code,
                signal,
                ended_at,
                killed: control.killed.load(Ordering::SeqCst),
                tail,
                line_count,
            });
        }
    }

    /// Keeps `id` among its workspace's finished jobs, forgetting the oldest beyond [`FINISHED_KEPT`].
    fn finish(&self, id: &str) {
        let mut guard = self.lock();
        let registry = &mut *guard;
        // A job forgotten with its workspace stays forgotten.
        let Some(entry) = registry.jobs.get(id) else {
            return;
        };
        let finished = registry
            .finished
            .entry(entry.workspace_id.clone())
            .or_default();
        finished.push_back(id.to_owned());
        if finished.len() > FINISHED_KEPT
            && let Some(oldest) = finished.pop_front()
        {
            registry.jobs.remove(&oldest);
        }
    }

    /// The log of `job/<id>`.
    pub fn log(&self, id: &str) -> Result<Arc<JobLog>, JobError> {
        self.lock()
            .jobs
            .get(id)
            .map(|entry| Arc::clone(&entry.log))
            .ok_or_else(|| JobError::NotFound(id.to_owned()))
    }

    /// SIGTERM to the job's group, SIGKILL [`KILL_GRACE`] later if the shell has not exited (DMN-081); once the
    /// shell exited, only a group that still has members is signalled (see [`Control`]).
    pub fn kill(&self, id: &str) -> Result<(), JobError> {
        let control = self
            .lock()
            .jobs
            .get(id)
            .map(|entry| Arc::clone(&entry.control))
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        if let Some(escalation) = control.terminate() {
            tokio::spawn(escalation);
        }
        Ok(())
    }

    /// Kills the running jobs of a workspace (DMN-081) and returns once their shells exited.
    pub async fn kill_workspace(&self, workspace_id: &str) {
        let escalations: Vec<_> = self
            .lock()
            .jobs
            .values()
            .filter(|entry| entry.workspace_id == workspace_id)
            .filter_map(|entry| entry.control.terminate())
            .collect();
        futures::future::join_all(escalations).await;
    }

    /// Kills the running jobs the agents of `threads` started (DMN-162), as [`Jobs::kill`] does.
    pub fn kill_started_by(&self, threads: &[String]) {
        let escalations: Vec<_> = self
            .lock()
            .jobs
            .values()
            .filter(|entry| {
                entry
                    .started_by
                    .as_ref()
                    .is_some_and(|thread| threads.contains(thread))
            })
            .filter_map(|entry| entry.control.terminate())
            .collect();
        for escalation in escalations {
            tokio::spawn(escalation);
        }
    }

    /// The running jobs the agent of `thread` started (DMN-333), oldest first.
    #[must_use]
    pub fn running_started_by(&self, thread: &str) -> Vec<JobInfo> {
        let mut running: Vec<JobInfo> = self
            .lock()
            .jobs
            .values()
            .filter(|entry| entry.started_by.as_deref() == Some(thread))
            .map(|entry| entry.log.with_state(|topic| topic.job.clone()))
            .filter(|job| job.status == JobStatus::Running)
            .collect();
        running.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
        running
    }

    /// Forgets the jobs of a removed workspace; one that started meanwhile is killed.
    pub fn forget_workspace(&self, workspace_id: &str) {
        let mut registry = self.lock();
        registry.finished.remove(workspace_id);
        for (_, entry) in registry
            .jobs
            .extract_if(|_, entry| entry.workspace_id == workspace_id)
        {
            if let Some(escalation) = entry.control.terminate() {
                tokio::spawn(escalation);
            }
        }
    }

    /// Kills every running job (DMN-081) and returns once their shells exited; no job starts afterwards.
    pub async fn shutdown(&self) {
        let escalations: Vec<_> = {
            let mut registry = self.lock();
            registry.closed = true;
            registry
                .jobs
                .values()
                .filter_map(|entry| entry.control.terminate())
                .collect()
        };
        futures::future::join_all(escalations).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use omfp_protocol::JobState;

    use super::*;

    fn lines(chunks: &[&[u8]]) -> Vec<String> {
        let mut out = Vec::new();
        let mut emit = |text| out.push(text);
        let mut lines = Lines::default();
        for chunk in chunks {
            lines.push(chunk, &mut emit);
        }
        lines.finish(&mut emit);
        out
    }

    // spec: DMN-080
    #[test]
    fn output_is_cut_at_line_breaks_without_a_carriage_return_before_them() {
        assert_eq!(
            lines(&[b"one\ntw", b"o\r", b"\nthree\r\n\nmid\rdle\nlast"]),
            ["one", "two", "three", "", "mid\rdle", "last"]
        );
    }

    // spec: DMN-080
    #[test]
    fn invalid_utf8_is_replaced() {
        assert_eq!(lines(&[b"bad \xff byte\n"]), ["bad \u{FFFD} byte"]);
    }

    // spec: DMN-080
    #[test]
    fn a_line_longer_than_64_kib_is_cut_into_pieces() {
        let long = vec![b'a'; 2 * LINE_BYTES + 10];
        let exact = vec![b'b'; LINE_BYTES];
        let pieces = lines(&[&long[..1000], &long[1000..], b"\n", &exact, b"\r\n"]);
        let lengths: Vec<usize> = pieces.iter().map(String::len).collect();
        assert_eq!(lengths, [LINE_BYTES, LINE_BYTES, 10, LINE_BYTES]);
        assert_eq!(pieces[3], "b".repeat(LINE_BYTES));
    }

    // spec: DMN-080
    #[test]
    fn a_cut_never_splits_a_utf8_character() {
        let mut line = vec![b'a'; LINE_BYTES - 1];
        line.extend_from_slice("é".as_bytes());
        line.extend_from_slice(b"z\n");
        let pieces = lines(&[&line]);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].len(), LINE_BYTES - 1);
        assert_eq!(pieces[1], "éz");
    }

    fn job() -> JobInfo {
        JobInfo {
            id: "j1".into(),
            workspace_id: "w1".into(),
            command: "true".into(),
            started_at: 1,
            status: JobStatus::Running,
        }
    }

    fn output(stream: JobStream, text: &str) -> JobEvent {
        JobEvent::JobOutput {
            stream,
            text: text.into(),
        }
    }

    // spec: DMN-080, PRT-041
    #[test]
    fn the_state_keeps_the_newest_mebibyte_of_lines_and_counts_the_dropped_ones() {
        let mut topic = JobTopic::new(job());
        let line = "x".repeat(1024);
        for _ in 0..1024 {
            apply(&mut topic, &output(JobStream::Stdout, &line));
        }
        assert_eq!((topic.lines.len(), topic.dropped), (1024, 0));

        apply(&mut topic, &output(JobStream::Stderr, "y"));
        apply(&mut topic, &output(JobStream::Stdout, "z"));

        let state = JobState {
            job: topic.job.clone(),
            lines: topic.lines.iter().cloned().collect(),
            dropped: topic.dropped,
        };
        assert_eq!(state.dropped, 1);
        assert_eq!(state.lines.len(), 1025);
        assert_eq!(
            state.lines[1023..],
            [
                JobLine {
                    stream: JobStream::Stderr,
                    text: "y".into()
                },
                JobLine {
                    stream: JobStream::Stdout,
                    text: "z".into()
                }
            ]
        );
        assert_eq!(
            serde_json::to_value(&topic).unwrap(),
            serde_json::to_value(&state).unwrap()
        );
    }

    // spec: DMN-080, PRT-041
    #[test]
    fn job_exited_sets_the_status() {
        let mut topic = JobTopic::new(job());
        apply(
            &mut topic,
            &JobEvent::JobExited {
                code: None,
                signal: Some(9),
                ended_at: 7,
            },
        );
        assert_eq!(
            topic.job.status,
            JobStatus::Exited {
                code: None,
                signal: Some(9),
                ended_at: 7
            }
        );
    }

    // spec: DMN-081
    #[tokio::test(start_paused = true)]
    async fn a_shell_still_running_after_the_grace_period_is_killed() {
        let (exited, receiver) = watch::channel(false);
        let kills = AtomicUsize::new(0);
        let started = tokio::time::Instant::now();

        escalate(receiver, KILL_GRACE, || {
            kills.fetch_add(1, Ordering::SeqCst);
            exited.send_replace(true);
        })
        .await;

        assert_eq!(kills.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() >= KILL_GRACE);
    }

    // spec: DMN-081
    #[tokio::test(start_paused = true)]
    async fn a_shell_exiting_within_the_grace_period_is_not_killed() {
        let (exited, receiver) = watch::channel(false);
        let kills = AtomicUsize::new(0);
        let exit = async {
            tokio::time::sleep(KILL_GRACE / 2).await;
            exited.send_replace(true);
        };

        tokio::join!(
            escalate(receiver, KILL_GRACE, || {
                kills.fetch_add(1, Ordering::SeqCst);
            }),
            exit
        );

        assert_eq!(kills.load(Ordering::SeqCst), 0);
    }

    const WAIT: Duration = Duration::from_secs(60);

    /// The job's state once it finished.
    async fn finished(log: &JobLog) -> JobTopic {
        let mut live = log.subscribe(None).live;
        tokio::time::timeout(WAIT, async {
            while log.with_state(|topic| topic.job.status == JobStatus::Running) {
                let _ = live.recv().await;
            }
        })
        .await
        .expect("the job did not finish");
        log.with_state(Clone::clone)
    }

    // spec: DMN-081
    #[tokio::test(flavor = "multi_thread")]
    async fn at_most_32_finished_jobs_are_kept_per_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let gate = dir.path().join("gate");
        let jobs = Jobs::default();
        let running = jobs
            .start(
                "w1",
                dir.path(),
                &format!("while [ ! -e '{}' ]; do sleep 0.01; done", gate.display()),
                None,
                None,
            )
            .unwrap();
        let other = jobs.start("w2", dir.path(), "true", None, None).unwrap();
        finished(&jobs.log(&other.id).unwrap()).await;
        let mut ids = Vec::new();
        for _ in 0..=FINISHED_KEPT {
            let job = jobs.start("w1", dir.path(), "true", None, None).unwrap();
            finished(&jobs.log(&job.id).unwrap()).await;
            ids.push(job.id);
        }

        assert!(matches!(jobs.log(&ids[0]), Err(JobError::NotFound(_))));
        assert!(matches!(jobs.kill(&ids[0]), Err(JobError::NotFound(_))));
        for id in ids[1..].iter().chain([&running.id, &other.id]) {
            assert!(jobs.log(id).is_ok(), "{id} was forgotten");
        }
        std::fs::write(&gate, "").unwrap();
        finished(&jobs.log(&running.id).unwrap()).await;
    }

    // spec: DMN-081
    #[tokio::test(flavor = "multi_thread")]
    async fn no_job_starts_after_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::default();
        jobs.shutdown().await;
        assert!(matches!(
            jobs.start("w1", dir.path(), "true", None, None),
            Err(JobError::ShuttingDown)
        ));
    }

    // spec: DMN-162, AGT-071
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cascade_kills_only_the_jobs_the_stopped_threads_agents_started() {
        let dir = tempfile::tempdir().unwrap();
        let gate = dir.path().join("gate");
        let wait = format!("while [ ! -e '{}' ]; do sleep 0.01; done", gate.display());
        let jobs = Jobs::default();
        let child = jobs
            .start("w1", dir.path(), &wait, Some("01JCHILD"), None)
            .unwrap();
        let other = jobs
            .start("w1", dir.path(), &wait, Some("01JOTHER"), None)
            .unwrap();
        let user = jobs.start("w1", dir.path(), &wait, None, None).unwrap();

        jobs.kill_started_by(&["01JROOT".to_owned(), "01JCHILD".to_owned()]);

        let killed = finished(&jobs.log(&child.id).unwrap()).await;
        assert!(
            matches!(
                killed.job.status,
                JobStatus::Exited {
                    signal: Some(15),
                    ..
                }
            ),
            "{:?}",
            killed.job.status
        );
        for id in [&other.id, &user.id] {
            assert!(
                jobs.log(id)
                    .unwrap()
                    .with_state(|topic| topic.job.status == JobStatus::Running)
            );
        }
        std::fs::write(&gate, "").unwrap();
        for id in [&other.id, &user.id] {
            let done = finished(&jobs.log(id).unwrap()).await;
            assert!(matches!(
                done.job.status,
                JobStatus::Exited { code: Some(0), .. }
            ));
        }
    }

    // spec: DMN-081
    #[tokio::test(flavor = "multi_thread")]
    async fn killing_a_job_whose_shell_exited_kills_its_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::default();
        // The shell exits at once; the background `sleep` keeps the group and the outputs, so the job keeps running.
        let job = jobs
            .start("w1", dir.path(), "sleep 60 & exit 0", None, None)
            .unwrap();
        let control = Arc::clone(&jobs.lock().jobs[&job.id].control);
        let mut exited = control.exited.subscribe();
        tokio::time::timeout(WAIT, exited.wait_for(|exited| *exited))
            .await
            .unwrap()
            .unwrap();
        assert!(control.group_alive());

        jobs.kill(&job.id).unwrap();
        // The job finishes only once the `sleep` closed the outputs, i.e. died.
        finished(&jobs.log(&job.id).unwrap()).await;
    }

    // spec: DMN-081
    #[tokio::test(flavor = "multi_thread")]
    async fn killing_a_finished_job_with_an_empty_group_signals_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::default();
        let job = jobs.start("w1", dir.path(), "exit 0", None, None).unwrap();
        let control = Arc::clone(&jobs.lock().jobs[&job.id].control);
        finished(&jobs.log(&job.id).unwrap()).await;
        assert!(*control.exited.borrow());

        assert!(!control.group_alive());
        assert!(control.terminate().is_none());
        jobs.kill(&job.id).unwrap();
        jobs.kill_workspace("w1").await;
    }

    // spec: DMN-215
    #[tokio::test(flavor = "multi_thread")]
    async fn a_job_replacing_another_kills_it_if_running_and_forgets_it() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::default();
        let first = jobs
            .start("w1", dir.path(), "sleep 60", None, None)
            .unwrap();
        let first_log = jobs.log(&first.id).unwrap();

        let second = jobs
            .start("w1", dir.path(), "exit 0", None, Some(&first.id))
            .unwrap();

        assert!(matches!(jobs.log(&first.id), Err(JobError::NotFound(_))));
        let killed = finished(&first_log).await;
        assert!(
            matches!(
                killed.job.status,
                JobStatus::Exited {
                    signal: Some(15),
                    ..
                }
            ),
            "{:?}",
            killed.job.status
        );
        finished(&jobs.log(&second.id).unwrap()).await;

        // A finished job is forgotten too, and leaves its place among the kept finished jobs.
        let third = jobs
            .start("w1", dir.path(), "exit 0", None, Some(&second.id))
            .unwrap();
        assert!(matches!(jobs.log(&second.id), Err(JobError::NotFound(_))));
        finished(&jobs.log(&third.id).unwrap()).await;
        assert_eq!(
            jobs.lock().finished["w1"].iter().collect::<Vec<_>>(),
            [&third.id]
        );
    }

    // spec: DMN-215
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_replaced_job_is_ignored_and_another_workspaces_job_refused() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::default();
        let other = jobs.start("w2", dir.path(), "exit 0", None, None).unwrap();

        let job = jobs
            .start("w1", dir.path(), "exit 0", None, Some("01JGONE"))
            .unwrap();
        let refused = jobs.start("w1", dir.path(), "exit 0", None, Some(&other.id));

        assert!(
            matches!(&refused, Err(JobError::OtherWorkspace(id)) if *id == other.id),
            "{refused:?}"
        );
        assert_eq!(
            ErrorBody::from(refused.unwrap_err()).code,
            ErrorCode::BadRequest
        );
        finished(&jobs.log(&other.id).unwrap()).await;
        finished(&jobs.log(&job.id).unwrap()).await;
    }
}
