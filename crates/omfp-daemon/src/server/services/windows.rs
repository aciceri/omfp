//! The windows (DMN-205..DMN-207): every connected window's latest report (`widgets.report`, PRT-043, PRT-122), the
//! link to its connection, the order the user focused them in, and the requests the daemon sends them
//! (`window_request` frames answered with `windows.answer`, PRT-124). Agents read them with `windows.list` and
//! `windows.state` (PRT-123), run a command in one with `windows.run` and render a widget out of sight in one with
//! `windows.check_widget`.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omfp_protocol::{
    AnswerWindowParams, CheckWidgetParams, ErrorBody, ErrorCode, ListWindowsParams, RanInWindow,
    ReportWidgetsParams, RunFailure, RunInWindowParams, ServerFrame, WidgetChecked,
    WidgetReportState, WidgetSource, WindowAnswer, WindowList, WindowRef, WindowReport,
    WindowRequest, WindowState, WindowSummary,
};
use tokio::sync::{mpsc, oneshot, watch};

use super::super::connection::Outbound;
use super::{Services, blocking, lock};
use crate::server::convert::bad_request;
use crate::widgets::folders;

/// How long a window has to run a command (DMN-206).
pub(crate) const RUN_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a window waits for an out-of-sight widget to render or fail (DMN-207).
pub(crate) const RENDER_TIMEOUT: Duration = Duration::from_secs(4);
/// How much longer than [`RENDER_TIMEOUT`] the daemon waits for the window's answer.
const ANSWER_MARGIN: Duration = Duration::from_secs(2);

/// What the daemon keeps of a report (PRT-043, DMN-195, DMN-205).
pub(super) const MAX_TEXT: usize = 4000;
pub(super) const MAX_ENTRIES: usize = 20;
const MAX_TAIL: usize = 10;
pub(super) const MAX_ENTRY: usize = 2000;
const MAX_VIEWS: usize = 50;
const MAX_COMMANDS: usize = 100;
const MAX_VIEW_WIDGETS: usize = 100;
const MAX_NAME: usize = 200;

/// `text` cut to `max` characters, with `…` when cut.
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// The last `max` entries of `entries`, each clipped.
fn last(entries: &[String], max: usize) -> Vec<String> {
    entries[entries.len().saturating_sub(max)..]
        .iter()
        .map(|entry| clip(entry, MAX_ENTRY))
        .collect()
}

/// A window's state as the daemon keeps it (DMN-205): at most 50 views of 100 widgets each and 100 commands, names
/// and titles clipped.
fn trimmed_window(mut window: WindowReport) -> WindowReport {
    window.views.truncate(MAX_VIEWS);
    for view in &mut window.views {
        view.title = clip(&view.title, MAX_NAME);
        if let Some(widgets) = &mut view.widgets {
            widgets.truncate(MAX_VIEW_WIDGETS);
        }
    }
    window.commands.truncate(MAX_COMMANDS);
    for command in &mut window.commands {
        command.title = clip(&command.title, MAX_NAME);
        command.aliases.truncate(MAX_ENTRIES);
        command.bindings.truncate(MAX_ENTRIES);
    }
    window
}

/// A report as the daemon keeps it (PRT-043, DMN-195, DMN-205).
pub(super) fn trimmed(mut report: ReportWidgetsParams) -> ReportWidgetsParams {
    for widget in &mut report.widgets {
        widget.text = clip(&widget.text, MAX_TEXT);
        widget.errors = last(&widget.errors, MAX_ENTRIES);
        widget.console = last(&widget.console, MAX_ENTRIES);
        for job in &mut widget.jobs {
            job.tail = last(&job.tail, MAX_TAIL);
        }
    }
    let files = &mut report.file_errors;
    files.drain(..files.len().saturating_sub(MAX_ENTRIES));
    for error in files {
        error.message = clip(&error.message, MAX_ENTRY);
    }
    report.window = report.window.map(trimmed_window);
    report
}

/// The latest report of one window (connection).
#[derive(Debug, Clone)]
pub(super) struct Window {
    /// Its connection number: the `windowId` agents name it by.
    pub(super) id: u64,
    /// The `client` of its `system.hello`.
    pub(super) client: String,
    pub(super) report: ReportWidgetsParams,
    /// When the user last focused it, in [`Windows`]' focus order; 0 when never.
    focused_at: u64,
}

impl Window {
    /// Whether its latest report is about `workspace`; any window is when `workspace` is `None`.
    fn shows(&self, workspace: Option<&str>) -> bool {
        workspace.is_none_or(|workspace| self.report.workspace_id.as_deref() == Some(workspace))
    }

    /// What it reports besides its widgets; empty from a client that does not report it.
    fn state(&self) -> WindowReport {
        self.report.window.clone().unwrap_or_default()
    }

    /// Its init file and TSX view errors and the widgets of its view in an error state.
    fn errors(&self) -> u32 {
        let widgets = self
            .report
            .widgets
            .iter()
            .filter(|widget| {
                !matches!(
                    widget.state,
                    WidgetReportState::Loading | WidgetReportState::Rendered
                ) || !widget.console.is_empty()
            })
            .count();
        u32::try_from(self.report.file_errors.len() + widgets).unwrap_or(u32::MAX)
    }

    fn summary(&self) -> WindowSummary {
        let state = self.state();
        WindowSummary {
            window_id: self.id,
            client: self.client.clone(),
            workspace_id: self.report.workspace_id.clone(),
            thread_id: state.thread_id,
            focused: state.focused,
            active_view: state.active_view,
            errors: self.errors(),
        }
    }

    fn full(&self) -> WindowState {
        let state = self.state();
        WindowState {
            window_id: self.id,
            client: self.client.clone(),
            workspace_id: self.report.workspace_id.clone(),
            thread_id: state.thread_id,
            focused: state.focused,
            active_view: state.active_view,
            default_view: state.default_view,
            views: state.views,
            commands: state.commands,
            file_errors: self.report.file_errors.clone(),
            widgets: self.report.widgets.clone(),
        }
    }
}

/// A request a window has not answered yet.
#[derive(Debug)]
struct Pending {
    window: u64,
    reply: oneshot::Sender<WindowAnswer>,
}

/// Every connected window (DMN-205): its latest report by connection, the connection's outbound queue for requests,
/// and the requests waiting for an answer.
#[derive(Debug)]
pub(crate) struct Windows {
    reports: Mutex<BTreeMap<u64, Window>>,
    links: Mutex<BTreeMap<u64, mpsc::Sender<Outbound>>>,
    pending: Mutex<HashMap<String, Pending>>,
    next: AtomicU64,
    requests: AtomicU64,
    focus: AtomicU64,
    /// Bumped on every change of a report, for checks waiting for reports.
    pub(super) changed: watch::Sender<u64>,
}

/// Forgets a request when its caller stops waiting (an answer, a timeout, a cancelled tool).
struct PendingGuard<'a> {
    windows: &'a Windows,
    id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        lock(&self.windows.pending).remove(&self.id);
    }
}

fn not_found(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::NotFound, message)
}

/// Queues `request`, request `id`, on `link`, the outbound queue of `connection`'s window.
fn send(
    link: &mpsc::Sender<Outbound>,
    connection: u64,
    id: String,
    request: WindowRequest,
) -> Result<(), ErrorBody> {
    let frame = Outbound::frame(&ServerFrame::WindowRequest { id, request })
        .ok_or_else(|| bad_request("cannot encode the window request"))?;
    link.try_send(frame).map_err(|_| {
        conflict(format!(
            "omfp window {connection} is busy or closing; try again"
        ))
    })
}

fn conflict(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::Conflict, message)
}

impl Windows {
    pub(crate) fn new() -> Self {
        Self {
            reports: Mutex::new(BTreeMap::new()),
            links: Mutex::new(BTreeMap::new()),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            requests: AtomicU64::new(1),
            focus: AtomicU64::new(0),
            changed: watch::channel(0).0,
        }
    }

    /// A new connection's number.
    pub(crate) fn connection(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Where requests to `connection`'s window go.
    pub(crate) fn link(&self, connection: u64, out: mpsc::Sender<Outbound>) {
        lock(&self.links).insert(connection, out);
    }

    /// Replaces the report of `connection`; a report saying the window has the focus while it had not makes it the
    /// most recently focused window.
    pub(crate) fn set(&self, connection: u64, client: &str, report: ReportWidgetsParams) {
        let report = trimmed(report);
        let focused = report.window.as_ref().is_some_and(|window| window.focused);
        {
            let mut windows = lock(&self.reports);
            let previous = windows.get(&connection);
            let was_focused = previous
                .and_then(|window| window.report.window.as_ref())
                .is_some_and(|window| window.focused);
            let focused_at = if focused && !was_focused {
                self.focus.fetch_add(1, Ordering::Relaxed) + 1
            } else {
                previous.map_or(0, |window| window.focused_at)
            };
            windows.insert(
                connection,
                Window {
                    id: connection,
                    client: client.to_owned(),
                    report,
                    focused_at,
                },
            );
        }
        self.changed.send_modify(|count| *count += 1);
    }

    /// Forgets a closed connection: its report, its link, and its requests (their callers learn it closed).
    pub(crate) fn forget(&self, connection: u64) {
        lock(&self.links).remove(&connection);
        lock(&self.pending).retain(|_, pending| pending.window != connection);
        if lock(&self.reports).remove(&connection).is_some() {
            self.changed.send_modify(|count| *count += 1);
        }
    }

    /// The windows whose latest report is about `workspace`, by connection.
    pub(super) fn showing(&self, workspace: &str) -> Vec<Window> {
        lock(&self.reports)
            .values()
            .filter(|window| window.shows(Some(workspace)))
            .cloned()
            .collect()
    }

    /// Every window that reported, the most recently focused first (then the newest).
    fn by_focus(&self) -> Vec<Window> {
        let mut windows: Vec<Window> = lock(&self.reports).values().cloned().collect();
        windows.sort_by_key(|window| std::cmp::Reverse((window.focused_at, window.id)));
        windows
    }

    fn get(&self, id: u64) -> Option<Window> {
        lock(&self.reports).get(&id).cloned()
    }

    /// The window `id`, else the most recently focused window showing `workspace` (any window without one).
    fn pick(&self, id: Option<u64>, workspace: Option<&str>) -> Result<Window, ErrorBody> {
        if let Some(id) = id {
            return self.get(id).ok_or_else(|| {
                not_found(format!(
                    "no omfp window {id}: windows.list names the open ones"
                ))
            });
        }
        let windows = self.by_focus();
        let none = windows.is_empty();
        windows
            .into_iter()
            .find(|window| window.shows(workspace))
            .ok_or_else(|| match workspace {
                Some(workspace) if !none => not_found(format!(
                    "no omfp window shows workspace {workspace}; windows.list names the open ones and what they show"
                )),
                _ => not_found("no omfp window is open"),
            })
    }

    /// Sends `request` to `window` and waits at most `timeout` for its answer (DMN-206, DMN-207).
    async fn request(
        &self,
        window: &Window,
        request: WindowRequest,
        timeout: Duration,
    ) -> Result<WindowAnswer, ErrorBody> {
        self.request_to(window.id, &window.client, request, timeout)
            .await
    }

    /// The outbound queue of `connection`'s window.
    fn link_of(&self, connection: u64) -> Result<mpsc::Sender<Outbound>, ErrorBody> {
        lock(&self.links)
            .get(&connection)
            .cloned()
            .ok_or_else(|| not_found(format!("omfp window {connection} is closed")))
    }

    /// A new request's id.
    fn request_id(&self) -> String {
        format!("w{}", self.requests.fetch_add(1, Ordering::Relaxed))
    }

    /// Sends `request` to the window of `connection` (`client` names it in errors) and waits at most `timeout` for its
    /// answer; the window need not have reported (DMN-206, DMN-221).
    pub(super) async fn request_to(
        &self,
        connection: u64,
        client: &str,
        request: WindowRequest,
        timeout: Duration,
    ) -> Result<WindowAnswer, ErrorBody> {
        let link = self.link_of(connection)?;
        let id = self.request_id();
        let (reply, answer) = oneshot::channel();
        lock(&self.pending).insert(
            id.clone(),
            Pending {
                window: connection,
                reply,
            },
        );
        let _guard = PendingGuard {
            windows: self,
            id: id.clone(),
        };
        send(&link, connection, id, request)?;
        match tokio::time::timeout(timeout, answer).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => Err(not_found(format!(
                "omfp window {connection} closed before it answered"
            ))),
            Err(_) => Err(conflict(format!(
                "omfp window {connection} ({client}) did not answer within {} s",
                timeout.as_secs()
            ))),
        }
    }

    /// Sends `request`, a notice the window does not answer (`login_url`, `login_progress`), to the window of
    /// `connection` (DMN-221).
    pub(super) fn notify(&self, connection: u64, request: WindowRequest) -> Result<(), ErrorBody> {
        send(
            &self.link_of(connection)?,
            connection,
            self.request_id(),
            request,
        )
    }

    /// The connection of the most recently focused window (DMN-221); `None` when no window reported.
    pub(super) fn focused(&self) -> Option<u64> {
        self.by_focus().first().map(|window| window.id)
    }

    /// An agent's `omfp_notify` about thread `thread` for the most recently focused window, whatever it shows
    /// (DMN-375): `Ok(false)` when no window reported, the error a busy or closing window's.
    pub(crate) fn notify_focused(
        &self,
        thread: &str,
        title: &str,
        body: &str,
    ) -> Result<bool, String> {
        let Some(connection) = self.focused() else {
            return Ok(false);
        };
        self.notify(
            connection,
            WindowRequest::Notify {
                thread_id: thread.to_owned(),
                title: title.to_owned(),
                body: body.to_owned(),
            },
        )
        .map(|()| true)
        .map_err(|error| error.message)
    }

    /// `windows.answer` from `connection`: hands the answer to the request's caller.
    fn answer(&self, connection: u64, params: AnswerWindowParams) -> Result<(), ErrorBody> {
        let mut pending = lock(&self.pending);
        let mine = pending
            .get(&params.request_id)
            .is_some_and(|entry| entry.window == connection);
        if !mine {
            return Err(not_found(format!(
                "no request {} waits for this window's answer",
                params.request_id
            )));
        }
        if let Some(entry) = pending.remove(&params.request_id) {
            // The caller may have stopped waiting meanwhile.
            let _ = entry.reply.send(params.answer);
        }
        Ok(())
    }
}

impl Services {
    /// A new connection's number, naming its window (PRT-043, DMN-205).
    pub fn reports_connection(&self) -> u64 {
        self.windows.connection()
    }

    /// Where the daemon's requests to `connection`'s window go (PRT-124).
    pub(crate) fn link_window(&self, connection: u64, out: mpsc::Sender<Outbound>) {
        self.windows.link(connection, out);
    }

    /// `widgets.report` from `connection` (PRT-043, PRT-122): replaces its report.
    pub fn report_widgets(&self, connection: u64, client: &str, report: ReportWidgetsParams) {
        self.windows.set(connection, client, report);
    }

    /// Forgets a closed connection's window: its report and its requests (PRT-043, DMN-205).
    pub fn forget_widget_reports(&self, connection: u64) {
        self.windows.forget(connection);
    }

    /// `windows.answer` from `connection` (PRT-124).
    pub fn answer_window(
        &self,
        connection: u64,
        params: AnswerWindowParams,
    ) -> Result<(), ErrorBody> {
        self.windows.answer(connection, params)
    }

    async fn known_workspace(&self, workspace_id: &str) -> Result<PathBuf, ErrorBody> {
        let store = Arc::clone(&self.store);
        let id = workspace_id.to_owned();
        blocking(move || Ok(store.workspace(&id)?.path)).await
    }

    /// `windows.list` (PRT-123): every window that reported (those showing `workspaceId` with one), the most
    /// recently focused first.
    pub async fn list_windows(&self, params: ListWindowsParams) -> Result<WindowList, ErrorBody> {
        if let Some(workspace) = &params.workspace_id {
            self.known_workspace(workspace).await?;
        }
        Ok(WindowList {
            windows: self
                .windows
                .by_focus()
                .iter()
                .filter(|window| window.shows(params.workspace_id.as_deref()))
                .map(Window::summary)
                .collect(),
        })
    }

    /// `windows.state` (PRT-123): all window `windowId` reports.
    pub fn window_state(&self, params: &WindowRef) -> Result<WindowState, ErrorBody> {
        self.windows
            .pick(Some(params.window_id), None)
            .map(|window| window.full())
    }

    /// The windows showing `workspace_id`, the most recently focused first, as `windows.state` gives them.
    pub(crate) fn window_states(&self, workspace_id: Option<&str>) -> Vec<WindowState> {
        self.windows
            .by_focus()
            .iter()
            .filter(|window| window.shows(workspace_id))
            .map(Window::full)
            .collect()
    }

    /// `windows.run` (PRT-124, DMN-206): runs a registered command in window `windowId`, else in the most recently
    /// focused window showing `workspaceId`, else in the most recently focused window.
    pub async fn run_in_window(&self, params: RunInWindowParams) -> Result<RanInWindow, ErrorBody> {
        let command = params.command.trim().to_owned();
        if command.is_empty() {
            return Err(bad_request(
                "`command` must name a command (an id or an alias)",
            ));
        }
        if let Some(workspace) = &params.workspace_id {
            self.known_workspace(workspace).await?;
        }
        let window = self
            .windows
            .pick(params.window_id, params.workspace_id.as_deref())?;
        let answer = self
            .windows
            .request(
                &window,
                WindowRequest::RunCommand {
                    command: command.clone(),
                    args: params.args,
                },
                RUN_TIMEOUT,
            )
            .await?;
        let place = format!("omfp window {} ({})", window.id, window.client);
        match answer {
            WindowAnswer::Ran { command } => Ok(RanInWindow {
                window_id: window.id,
                command,
            }),
            WindowAnswer::NotRun {
                reason: RunFailure::Unknown,
                message,
            } => Err(not_found(format!("{place}: {message}"))),
            WindowAnswer::NotRun {
                reason: RunFailure::Disabled,
                message,
            } => Err(conflict(format!("{place} refused it: {message}"))),
            WindowAnswer::NotRun {
                reason: RunFailure::Failed,
                message,
            } => Err(bad_request(format!(
                "`{command}` failed in {place}: {message}"
            ))),
            WindowAnswer::Checked { .. } | WindowAnswer::LoginInput { .. } => Err(conflict(
                format!("{place} answered a command with something else"),
            )),
        }
    }

    /// `windows.check_widget` (PRT-124, DMN-207): renders widget `widget` of workspace `workspaceId` out of sight in
    /// window `windowId` (default: the most recently focused one showing the workspace) and returns what it rendered.
    pub async fn check_in_window(
        &self,
        params: CheckWidgetParams,
    ) -> Result<WidgetChecked, ErrorBody> {
        let path = self.known_workspace(&params.workspace_id).await?;
        let widgets = Arc::clone(&self.widgets);
        let name = params.widget.clone();
        let (source, name, version) = blocking(move || {
            let (source, name, folder) = widgets.locate(&path, &name).map_err(bad_request)?;
            Ok((source, name, folders::version(&folder)))
        })
        .await?;
        let reference = match source {
            WidgetSource::Workspace => format!("./widgets/{name}"),
            WidgetSource::User => format!("user:{name}"),
        };
        // Windows get the folder's current version before they render it.
        self.refresh_views(&params.workspace_id).await;
        self.render_offscreen(
            &params.workspace_id,
            params.window_id,
            &reference,
            Some(version),
        )
        .await
    }

    /// Asks a window showing `workspace_id` (`window_id`, else the most recently focused one) to render `reference`
    /// out of sight, waiting for `version` (DMN-207).
    pub(super) async fn render_offscreen(
        &self,
        workspace_id: &str,
        window_id: Option<u64>,
        reference: &str,
        version: Option<String>,
    ) -> Result<WidgetChecked, ErrorBody> {
        let window = self.windows.pick(window_id, Some(workspace_id))?;
        if !window.shows(Some(workspace_id)) {
            return Err(conflict(format!(
                "omfp window {} shows another workspace: it can render only the widgets of the workspace it shows",
                window.id
            )));
        }
        let answer = self
            .windows
            .request(
                &window,
                WindowRequest::CheckWidget {
                    reference: reference.to_owned(),
                    version,
                    timeout_ms: u64::try_from(RENDER_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                },
                RENDER_TIMEOUT + ANSWER_MARGIN,
            )
            .await?;
        match answer {
            WindowAnswer::Checked { report, settled } => Ok(WidgetChecked {
                window_id: window.id,
                report,
                settled,
            }),
            WindowAnswer::NotRun { message, .. } => Err(conflict(format!(
                "omfp window {} could not render {reference}: {message}",
                window.id
            ))),
            WindowAnswer::Ran { .. } | WindowAnswer::LoginInput { .. } => Err(conflict(format!(
                "omfp window {} answered a widget check with something else",
                window.id
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use omfp_protocol::{
        CommandSource, FileErrorKind, FileErrorReport, ViewSource, WidgetJobReport, WidgetReport,
        WindowCommand, WindowView,
    };

    use super::*;

    fn widget(state: WidgetReportState) -> WidgetReport {
        WidgetReport {
            reference: "./widgets/tests".to_owned(),
            instance: "./widgets/tests".to_owned(),
            source: WidgetSource::Workspace,
            name: "tests".to_owned(),
            version: Some("1".to_owned()),
            state,
            errors: Vec::new(),
            console: Vec::new(),
            text: String::new(),
            jobs: Vec::new(),
        }
    }

    fn report(workspace: Option<&str>, window: Option<WindowReport>) -> ReportWidgetsParams {
        ReportWidgetsParams {
            workspace_id: workspace.map(str::to_owned),
            views: None,
            widgets: Vec::new(),
            file_errors: Vec::new(),
            window,
        }
    }

    fn focused(focused: bool) -> WindowReport {
        WindowReport {
            focused,
            ..WindowReport::default()
        }
    }

    // spec: PRT-043, DMN-195, DMN-205
    #[test]
    fn reports_are_trimmed_to_what_the_daemon_keeps() {
        let mut widget = widget(WidgetReportState::Rendered);
        widget.text = "x".repeat(MAX_TEXT + 10);
        widget.console = (0..30).map(|n| format!("error {n}")).collect();
        widget.errors = vec!["e".repeat(MAX_ENTRY + 5)];
        widget.jobs = vec![WidgetJobReport {
            command: "make test".to_owned(),
            status: "running".to_owned(),
            exit_code: None,
            signal: None,
            runs: 1,
            started_at: None,
            tail: (0..15).map(|n| n.to_string()).collect(),
        }];
        let mut file_errors: Vec<FileErrorReport> = (0..25)
            .map(|n| FileErrorReport {
                kind: FileErrorKind::View,
                file: ".omfp/views/ops.tsx".to_owned(),
                message: format!("error {n}"),
            })
            .collect();
        file_errors[24].message = "m".repeat(MAX_ENTRY + 5);
        let views = (0..60)
            .map(|n| WindowView {
                id: format!("v{n}"),
                title: "t".repeat(MAX_NAME + 3),
                source: ViewSource::ViewsJson,
                file: None,
                widgets: Some((0..120).map(|n| format!("./widgets/w{n}")).collect()),
            })
            .collect();
        let commands = (0..110)
            .map(|n| WindowCommand {
                id: format!("c{n}"),
                title: "c".repeat(MAX_NAME + 3),
                aliases: Vec::new(),
                source: CommandSource::WorkspaceInit,
                widget: None,
                enabled: true,
                bindings: Vec::new(),
            })
            .collect();
        let kept = trimmed(ReportWidgetsParams {
            workspace_id: None,
            views: None,
            widgets: vec![widget],
            file_errors,
            window: Some(WindowReport {
                views,
                commands,
                ..WindowReport::default()
            }),
        });
        let widget = &kept.widgets[0];
        assert_eq!(widget.text.chars().count(), MAX_TEXT + 1, "cut, with …");
        assert_eq!(widget.console.len(), MAX_ENTRIES);
        assert_eq!(widget.console[0], "error 10", "the newest are kept");
        assert_eq!(widget.errors[0].chars().count(), MAX_ENTRY + 1);
        assert_eq!(
            widget.jobs[0].tail,
            (5..15).map(|n| n.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(kept.file_errors.len(), MAX_ENTRIES);
        assert_eq!(kept.file_errors[0].message, "error 5", "the last are kept");
        assert_eq!(
            kept.file_errors[MAX_ENTRIES - 1].message.chars().count(),
            MAX_ENTRY + 1
        );
        let window = kept.window.unwrap();
        assert_eq!(window.views.len(), MAX_VIEWS);
        assert_eq!(window.views[0].title.chars().count(), MAX_NAME + 1);
        assert_eq!(
            window.views[0].widgets.as_ref().map(Vec::len),
            Some(MAX_VIEW_WIDGETS)
        );
        assert_eq!(window.commands.len(), MAX_COMMANDS);
        assert_eq!(window.commands[0].title.chars().count(), MAX_NAME + 1);
    }

    // spec: DMN-205
    #[test]
    fn windows_are_ordered_by_the_last_focus_and_a_closed_one_is_forgotten() {
        let windows = Windows::new();
        windows.set(1, "web", report(Some("01WS"), Some(focused(true))));
        windows.set(2, "desktop", report(Some("01WS"), Some(focused(false))));
        windows.set(3, "web", report(None, None));
        let order = |windows: &Windows| -> Vec<u64> {
            windows.by_focus().iter().map(|window| window.id).collect()
        };
        assert_eq!(order(&windows), [1, 3, 2], "focused first, then the newest");
        // Window 2 takes the focus: it goes first; 1 losing the focus keeps its place among the rest.
        windows.set(1, "web", report(Some("01WS"), Some(focused(false))));
        windows.set(2, "desktop", report(Some("01WS"), Some(focused(true))));
        assert_eq!(order(&windows), [2, 1, 3]);
        // A report repeating the focus does not move it.
        windows.set(1, "web", report(Some("01WS"), Some(focused(true))));
        windows.set(2, "desktop", report(Some("01WS"), Some(focused(true))));
        assert_eq!(order(&windows), [1, 2, 3]);
        assert_eq!(windows.pick(None, Some("01WS")).unwrap().id, 1);
        assert_eq!(windows.pick(None, None).unwrap().id, 1);
        assert_eq!(
            windows.pick(None, Some("01OTHER")).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(windows.pick(Some(3), None).unwrap().id, 3);
        assert_eq!(
            windows.pick(Some(9), None).unwrap_err().code,
            ErrorCode::NotFound
        );
        windows.forget(1);
        assert_eq!(order(&windows), [2, 3]);
        assert_eq!(windows.pick(None, Some("01WS")).unwrap().id, 2);
    }

    // spec: DMN-205
    #[test]
    fn a_summary_counts_the_errors_a_window_shows() {
        let windows = Windows::new();
        let mut shown = report(
            Some("01WS"),
            Some(WindowReport {
                thread_id: Some("01TH".to_owned()),
                focused: true,
                active_view: Some("ops".to_owned()),
                ..WindowReport::default()
            }),
        );
        let mut noisy = widget(WidgetReportState::Rendered);
        noisy.console = vec!["TypeError".to_owned()];
        shown.widgets = vec![
            widget(WidgetReportState::Rendered),
            widget(WidgetReportState::RenderError),
            noisy,
        ];
        shown.file_errors = vec![FileErrorReport {
            kind: FileErrorKind::Init,
            file: "user".to_owned(),
            message: "boom".to_owned(),
        }];
        windows.set(4, "web", shown);
        let summary = windows.get(4).unwrap().summary();
        assert_eq!(
            summary,
            WindowSummary {
                window_id: 4,
                client: "web".to_owned(),
                workspace_id: Some("01WS".to_owned()),
                thread_id: Some("01TH".to_owned()),
                focused: true,
                active_view: Some("ops".to_owned()),
                errors: 3,
            }
        );
        // A client that reports no window state is listed with an empty one.
        windows.set(5, "cli", report(None, None));
        let full = windows.get(5).unwrap().full();
        assert_eq!(full.views, Vec::new());
        assert_eq!(full.active_view, None);
        assert_eq!(full.workspace_id, None);
    }

    /// A window's connection: the frames the daemon queued for it.
    fn link(windows: &Windows, id: u64) -> mpsc::Receiver<Outbound> {
        let (out, frames) = mpsc::channel(8);
        windows.link(id, out);
        frames
    }

    /// The next request queued for a window: its id and the request.
    fn next_request(frames: &mut mpsc::Receiver<Outbound>) -> (String, WindowRequest) {
        match frames.try_recv() {
            Ok(Outbound::Text(text)) => match serde_json::from_str(&text).unwrap() {
                ServerFrame::WindowRequest { id, request } => (id, request),
                other => panic!("not a window request: {other:?}"),
            },
            other => panic!("no request: {other:?}"),
        }
    }

    // spec: DMN-375
    #[test]
    fn an_agent_notification_goes_to_the_most_recently_focused_window() {
        let windows = Windows::new();
        assert_eq!(
            windows.notify_focused("01TH", "Key", "Load it"),
            Ok(false),
            "no window"
        );
        windows.set(1, "web", report(Some("01WS"), Some(focused(true))));
        windows.set(2, "web", report(Some("01OTHER"), Some(focused(false))));
        windows.set(2, "web", report(Some("01OTHER"), Some(focused(true))));
        let mut first = link(&windows, 1);
        let mut second = link(&windows, 2);
        assert_eq!(windows.notify_focused("01TH", "Key", "Load it"), Ok(true));
        assert_eq!(
            next_request(&mut second).1,
            WindowRequest::Notify {
                thread_id: "01TH".to_owned(),
                title: "Key".to_owned(),
                body: "Load it".to_owned(),
            },
            "whatever workspace it shows"
        );
        assert!(first.try_recv().is_err(), "only that window");
        drop(second);
        assert!(
            windows.notify_focused("01TH", "Key", "Load it").is_err(),
            "a closing window's error reaches the agent"
        );
    }

    // spec: DMN-206, PRT-124
    #[tokio::test]
    async fn a_request_goes_to_its_window_and_only_that_window_answers_it() {
        let windows = Arc::new(Windows::new());
        windows.set(1, "web", report(Some("01WS"), Some(focused(true))));
        windows.set(2, "web", report(Some("01WS"), None));
        let mut first = link(&windows, 1);
        let second = link(&windows, 2);
        let window = windows.get(1).unwrap();
        let asking = {
            let windows = Arc::clone(&windows);
            tokio::spawn(async move {
                let run = WindowRequest::RunCommand {
                    command: "view".to_owned(),
                    args: vec!["ops".to_owned()],
                };
                windows.request(&window, run, RUN_TIMEOUT).await
            })
        };
        tokio::task::yield_now().await;
        while first.is_empty() {
            tokio::task::yield_now().await;
        }
        let (id, request) = next_request(&mut first);
        assert_eq!(
            request,
            WindowRequest::RunCommand {
                command: "view".to_owned(),
                args: vec!["ops".to_owned()]
            }
        );
        assert!(second.is_empty(), "the other window gets nothing");
        let answer = |request_id: &str| AnswerWindowParams {
            request_id: request_id.to_owned(),
            answer: WindowAnswer::Ran {
                command: "view.switch".to_owned(),
            },
        };
        assert_eq!(
            windows.answer(2, answer(&id)).unwrap_err().code,
            ErrorCode::NotFound,
            "another window cannot answer it"
        );
        windows.answer(1, answer(&id)).unwrap();
        assert_eq!(
            asking.await.unwrap().unwrap(),
            WindowAnswer::Ran {
                command: "view.switch".to_owned()
            }
        );
        assert_eq!(
            windows.answer(1, answer(&id)).unwrap_err().code,
            ErrorCode::NotFound,
            "answered once"
        );
        assert!(lock(&windows.pending).is_empty());
    }

    // spec: DMN-206
    #[tokio::test(start_paused = true)]
    async fn a_window_that_does_not_answer_times_out_and_a_closed_one_fails_its_requests() {
        let windows = Arc::new(Windows::new());
        windows.set(1, "web", report(Some("01WS"), None));
        let _frames = link(&windows, 1);
        let window = windows.get(1).unwrap();
        let run = || WindowRequest::RunCommand {
            command: "view.next".to_owned(),
            args: Vec::new(),
        };
        let error = windows
            .request(&window, run(), RUN_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(
            error.message,
            "omfp window 1 (web) did not answer within 5 s"
        );
        assert!(
            lock(&windows.pending).is_empty(),
            "the request is forgotten"
        );

        let asking = {
            let windows = Arc::clone(&windows);
            tokio::spawn(async move { windows.request(&window, run(), RUN_TIMEOUT).await })
        };
        while lock(&windows.pending).is_empty() {
            tokio::task::yield_now().await;
        }
        windows.forget(1);
        let error = asking.await.unwrap().unwrap_err();
        assert_eq!(error.message, "omfp window 1 closed before it answered");

        // Without a link (a window that never connected through a socket), nothing is sent.
        windows.set(2, "web", report(Some("01WS"), None));
        let error = windows
            .request(&windows.get(2).unwrap(), run(), RUN_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(error.message, "omfp window 2 is closed");
    }
}
