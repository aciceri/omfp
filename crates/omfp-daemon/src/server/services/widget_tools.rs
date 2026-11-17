//! What the widget check does with the windows' reports (`widgets.report`, PRT-043, AGT-026, kept by `windows`) and
//! with the views manifest: `omfp_widget_check` (DMN-063, with the out-of-sight render of DMN-207 and the views that
//! use the widget, DMN-209). The init file and TSX view checks and errors (DMN-196, DMN-197) are in `file_checks`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use omfp_protocol::{Position, WidgetChecked, WidgetReport, WidgetReportState, WidgetSource};

use super::file_checks::CheckedFile;
use super::windows::Window;
use super::{Services, blocking};
use crate::agent_tools::windows::view_source_name;
use crate::host_tools::WidgetHost;
use crate::server::convert::internal;
use crate::widgets::compile::{CompileError, IMPORT_MAP};
use crate::widgets::layout;
use crate::widgets::native::{self, NativeControl};
use crate::widgets::{Widgets, folders, keymap, shell, views_folder};

/// How long `omfp_widget_check` waits for the windows to report the current version (DMN-063).
pub(crate) const CHECK_WAIT: Duration = Duration::from_secs(5);
/// How often a waiting check looks at the cancellation flag.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// The widget being checked.
#[derive(Debug)]
struct Target {
    source: WidgetSource,
    name: String,
    folder: PathBuf,
    version: String,
    entry: Option<&'static str>,
    compile: Vec<CompileError>,
    /// The native form controls of its sources (AGT-089).
    native: Vec<NativeControl>,
    manifest_error: Option<String>,
    /// Why `.omfp/keymap.json` is refused (DMN-122); `None` when valid or absent.
    keymap_error: Option<String>,
    /// A valid `views.json` exists and none of its views has the `main` slot (AGT-028).
    main_missing: bool,
}

impl Target {
    fn reference(&self) -> String {
        match self.source {
            WidgetSource::Workspace => format!("./widgets/{}", self.name),
            WidgetSource::User => format!("user:{}", self.name),
        }
    }

    fn instances<'a>(&self, window: &'a Window) -> Vec<&'a WidgetReport> {
        window
            .report
            .widgets
            .iter()
            .filter(|widget| widget.source == self.source && widget.name == self.name)
            .collect()
    }

    /// Why `window` has not settled on the current version yet; `None` once it has (DMN-063).
    fn unsettled(&self, window: &Window, views: Option<&Position>) -> Option<String> {
        if let Some(why) = behind(window, views) {
            return Some(why);
        }
        for instance in self.instances(window) {
            if instance.state == WidgetReportState::Loading {
                return Some(format!("is still loading {}", instance.instance));
            }
            if let Some(job) = instance.jobs.iter().find(|job| job.status == "pending") {
                return Some(format!(
                    "is still starting the job `{}` of {}",
                    job.command, instance.instance
                ));
            }
            if instance.version.as_deref() != Some(self.version.as_str()) {
                return Some(format!(
                    "shows version {} of {}, not the current one",
                    instance.version.as_deref().unwrap_or("none"),
                    instance.instance
                ));
            }
        }
        None
    }
}

/// Why `window` has not caught up with `views`, the latest position of the workspace's views topic (`None` before the
/// topic exists); `None` once it has (DMN-063).
pub(super) fn behind(window: &Window, views: Option<&Position>) -> Option<String> {
    let caught_up = match (views, &window.report.views) {
        (None, _) => true,
        (Some(current), Some(seen)) => seen.epoch == current.epoch && seen.seq >= current.seq,
        (Some(_), None) => false,
    };
    (!caught_up)
        .then(|| "has not caught up with the latest change of the workspace's views".to_owned())
}

fn state_name(state: WidgetReportState) -> &'static str {
    match state {
        WidgetReportState::Loading => "loading",
        WidgetReportState::Rendered => "rendered",
        WidgetReportState::CompileError => "compile error",
        WidgetReportState::LoadError => "load error",
        WidgetReportState::RenderError => "render error",
    }
}

pub(super) fn list(text: &mut String, title: &str, entries: &[String]) {
    if entries.is_empty() {
        let _ = writeln!(text, "  {title}: none");
        return;
    }
    let _ = writeln!(text, "  {title}:");
    for line in entries {
        entry(text, "    ", line);
    }
}

/// Appends a report entry `<indent>- <line>`, its continuation lines indented below it.
pub(super) fn entry(text: &mut String, indent: &str, line: &str) {
    let mut lines = line.lines();
    let _ = writeln!(text, "{indent}- {}", lines.next().unwrap_or_default());
    for line in lines {
        let _ = writeln!(text, "{indent}  {line}");
    }
}

/// `Window <id> (<client>)[, after waiting <s> s, <why>]`.
pub(super) fn window_title(
    window: &Window,
    unsettled: Option<&String>,
    waited: Duration,
) -> String {
    let mut title = format!("Window {} ({})", window.id, window.client);
    if let Some(why) = unsettled {
        let _ = write!(title, ", after waiting {} s, {why}", waited.as_secs());
    }
    title
}

/// A view that holds the checked widget (DMN-209): its id and where it comes from, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Use {
    view: String,
    from: String,
}

/// The out-of-sight render of the checked widget in a window (DMN-207), or why there is none.
type Offscreen = Result<WidgetChecked, String>;

/// The answer of `omfp_widget_check` (DMN-063); `windows` are those showing the workspace, with why each one has not
/// settled (`None` when it has). When none shows the widget, `uses` are the views that hold it (DMN-209) and
/// `offscreen` its out-of-sight render (DMN-207).
fn describe(
    target: &Target,
    windows: &[(Window, Option<String>)],
    waited: Duration,
    uses: &[Use],
    offscreen: Option<&Offscreen>,
) -> String {
    let reference = target.reference();
    let mut text = format!(
        "Widget {reference} (folder {}, version {}, entry {})\n\n",
        target.folder.display(),
        target.version,
        target.entry.unwrap_or("missing")
    );
    if target.entry.is_none() {
        text.push_str(
            "No entry file (index.tsx, index.ts, index.jsx or index.js): windows cannot load the widget.\n",
        );
    }
    if target.compile.is_empty() {
        let _ = writeln!(
            text,
            "Daemon compile: every file compiles and imports only its own files and {}.",
            IMPORT_MAP.join(", ")
        );
    } else {
        let _ = writeln!(
            text,
            "Daemon compile: {} file(s) fail; windows show the first error in place of the widget:",
            target.compile.len()
        );
        for error in &target.compile {
            let _ = writeln!(text, "- {}: {}", location("", error), error.message);
        }
    }
    describe_native(&mut text, &target.native);
    if let Some(error) = &target.manifest_error {
        let _ = writeln!(
            text,
            "views.json is invalid ({error}): windows show the default view until it is fixed."
        );
    }
    if let Some(error) = &target.keymap_error {
        let _ = writeln!(
            text,
            "keymap.json is invalid ({error}): windows keep its last valid bindings until it is fixed."
        );
    }
    if target.main_missing {
        text.push_str(
            "Warning: no view of views.json has the main slot [\"tabs\", {\"id\": \"main\"}, …\"omfp:chat\"], so files \
             the user opens float over the view instead of opening as tabs next to the conversation. Keep that slot: \
             put it back around omfp:chat, and place widgets with omfp_ui_mount.\n",
        );
    }
    text.push('\n');
    if windows.is_empty() {
        text.push_str(
            "No omfp window shows this workspace now, so nothing rendered the widget, not even out of sight: only \
             the daemon's compile check above applies.\n",
        );
        return text;
    }
    let showing = windows
        .iter()
        .filter(|(window, _)| !target.instances(window).is_empty())
        .count();
    if showing == 0 {
        let _ = write!(
            text,
            "{} window(s) show this workspace, but none shows {reference} in its current view.",
            windows.len()
        );
        describe_uses(&mut text, target, uses);
        if let Some(offscreen) = offscreen {
            describe_offscreen(&mut text, windows, offscreen);
        }
        return text;
    }
    for (window, unsettled) in windows {
        let _ = writeln!(
            text,
            "{}:",
            window_title(window, unsettled.as_ref(), waited)
        );
        let instances = target.instances(window);
        if instances.is_empty() {
            let _ = writeln!(text, "- does not show {reference} in its current view");
        }
        for instance in instances {
            describe_instance(&mut text, instance);
        }
    }
    text
}

/// Where the widget is placed, when no window shows it (DMN-209): `omfp_ui_mount` only when no view holds it.
fn describe_uses(text: &mut String, target: &Target, uses: &[Use]) {
    let Some(first) = uses.first() else {
        let _ = writeln!(
            text,
            " No view uses it: mount it with omfp_ui_mount {{ \"id\": \"{}\", \"widget\": \"{}\" }}.",
            target.name,
            target.reference()
        );
        return;
    };
    let views: Vec<String> = uses
        .iter()
        .map(|used| format!("\"{}\" ({})", used.view, used.from))
        .collect();
    let _ = writeln!(
        text,
        " It is in view {}: show it with omfp_window_run {{ \"command\": \"view.switch\", \"args\": [\"{}\"] }} \
         instead of mounting it again.",
        views.join(", "),
        first.view
    );
}

/// The out-of-sight render (DMN-207).
fn describe_offscreen(
    text: &mut String,
    windows: &[(Window, Option<String>)],
    offscreen: &Offscreen,
) {
    match offscreen {
        Ok(checked) => {
            let client = windows
                .iter()
                .find(|(window, _)| window.id == checked.window_id)
                .map_or("window", |(window, _)| window.client.as_str());
            let _ = write!(
                text,
                "Out-of-sight render in window {} ({client}), mounted in a hidden container for this check",
                checked.window_id
            );
            if !checked.settled {
                let _ = write!(
                    text,
                    ", which neither rendered nor failed within {} s",
                    super::windows::RENDER_TIMEOUT.as_secs()
                );
            }
            text.push_str(":\n");
            describe_instance(text, &checked.report);
        }
        Err(why) => {
            let _ = writeln!(text, "No out-of-sight render: {why}.");
        }
    }
}

/// The warning about native form controls (AGT-089): one line per control with the kit component to use instead.
fn describe_native(text: &mut String, controls: &[NativeControl]) {
    if controls.is_empty() {
        return;
    }
    let _ = writeln!(
        text,
        "Warning: {} native form control(s): their opened lists and pickers are drawn by the platform, not the \
         theme (a white GTK menu in the Linux desktop window). Use the omfp/kit components (omfp://docs/widgets, Form \
         controls):",
        controls.len()
    );
    for control in controls {
        let _ = writeln!(
            text,
            "- {}:{}: {} → {}",
            control.file, control.line, control.element, control.replacement
        );
    }
}

/// Where a compile error is: `<prefix><file>:<line>:<column>`, as far as it is known.
pub(super) fn location(prefix: &str, error: &CompileError) -> String {
    match (error.line, error.column) {
        (Some(line), Some(column)) => format!("{prefix}{}:{line}:{column}", error.file),
        (Some(line), None) => format!("{prefix}{}:{line}", error.file),
        _ => format!("{prefix}{}", error.file),
    }
}

/// One widget instance of a window's report, for `omfp_widget_check` (DMN-063).
fn describe_instance(text: &mut String, instance: &WidgetReport) {
    let _ = writeln!(
        text,
        "- {}: {} (version {})",
        instance.instance,
        state_name(instance.state),
        instance.version.as_deref().unwrap_or("none")
    );
    list(text, "errors", &instance.errors);
    list(text, "console errors", &instance.console);
    let jobs: Vec<String> = instance
        .jobs
        .iter()
        .map(|job| {
            let mut line = format!("`{}`: {}, run {}", job.command, job.status, job.runs);
            if let Some(code) = job.exit_code {
                let _ = write!(line, ", exit code {code}");
            }
            if let Some(signal) = job.signal {
                let _ = write!(line, ", signal {signal}");
            }
            if !job.tail.is_empty() {
                let _ = write!(line, "\nlast lines:\n{}", job.tail.join("\n"));
            }
            line
        })
        .collect();
    list(text, "jobs", &jobs);
    if instance.text.trim().is_empty() {
        text.push_str("  text: (empty)\n");
    } else {
        text.push_str("  text:\n");
        for line in instance.text.lines().filter(|line| !line.trim().is_empty()) {
            let _ = writeln!(text, "    {}", line.trim_end());
        }
    }
}

impl Services {
    /// `omfp_widget_check` (DMN-063): the daemon's compile of the widget `name` of workspace `workspace_id`, then
    /// what the windows showing the workspace report, waiting at most `wait` for them to settle on the current version.
    /// An init file or a TSX view `name` is checked as DMN-197 says.
    pub async fn check_widget(
        &self,
        workspace_id: &str,
        name: &str,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        wait: Duration,
    ) -> Result<String, String> {
        if let Some(file) = CheckedFile::parse(name) {
            return self
                .check_file(workspace_id, file, is_cancelled, wait)
                .await;
        }
        let (store, widgets) = (Arc::clone(&self.store), Arc::clone(&self.widgets));
        let (id, name) = (workspace_id.to_owned(), name.to_owned());
        let (source, name, folder) = blocking(move || {
            let path = store.workspace(&id)?.path;
            widgets.locate(&path, &name).map_err(|error| {
                internal(format!(
                    "{error}; check an init file with `init` or `user:init` and a TSX view with `views/<file>`"
                ))
            })
        })
        .await
        .map_err(|error| error.message)?;
        // Windows get the folder's current version (and the manifest) before they are asked about it.
        self.refresh_views(workspace_id).await;
        let (store, widgets) = (Arc::clone(&self.store), Arc::clone(&self.widgets));
        let id = workspace_id.to_owned();
        let target = blocking(move || {
            let workspace = store.workspace(&id)?.path;
            let (manifest, manifest_error) = Widgets::manifest(&workspace);
            Ok(Target {
                source,
                name,
                version: folders::version(&folder),
                entry: folders::entry(&folder),
                compile: widgets.check(&folder),
                native: native::scan_folder(&folder),
                manifest_error,
                keymap_error: keymap::read(&keymap::path(&workspace)).err(),
                main_missing: manifest
                    .as_ref()
                    .is_some_and(|manifest| !layout::has_main_slot(manifest)),
                folder,
            })
        })
        .await
        .map_err(|error| error.message)?;

        let windows = self
            .settled_windows(workspace_id, is_cancelled, wait, |window, views| {
                target.unsettled(window, views)
            })
            .await?;
        let shown = windows
            .iter()
            .any(|(window, _)| !target.instances(window).is_empty());
        if windows.is_empty() || shown {
            return Ok(describe(&target, &windows, wait, &[], None));
        }
        // No window shows it: where it is placed, and a render out of sight (DMN-207, DMN-209).
        let reference = target.reference();
        let uses = self.views_using(workspace_id, &reference, &windows).await;
        let offscreen = until_cancelled(
            self.render_offscreen(workspace_id, None, &reference, Some(target.version.clone())),
            is_cancelled,
        )
        .await?
        .map_err(|error| error.message);
        Ok(describe(&target, &windows, wait, &uses, Some(&offscreen)))
    }

    /// The views that hold widget `reference` (DMN-209): those the windows report (a TSX view none rendered when its
    /// file names the widget), then those of `views.json`, each once.
    async fn views_using(
        &self,
        workspace_id: &str,
        reference: &str,
        windows: &[(Window, Option<String>)],
    ) -> Vec<Use> {
        let mut uses: Vec<Use> = Vec::new();
        let mut unrendered: Vec<(String, String, String)> = Vec::new();
        for (window, _) in windows {
            for view in window.report.window.iter().flat_map(|state| &state.views) {
                let from = match &view.file {
                    Some(file) => format!("{}, TSX view {file}", view_source_name(view.source)),
                    None => view_source_name(view.source).to_owned(),
                };
                match (&view.widgets, &view.file) {
                    (Some(widgets), _) if widgets.iter().any(|widget| widget == reference) => {
                        uses.push(Use {
                            view: view.id.clone(),
                            from,
                        });
                    }
                    (None, Some(file)) => unrendered.push((view.id.clone(), from, file.clone())),
                    _ => {}
                }
            }
        }
        let store = Arc::clone(&self.store);
        let (id, wanted) = (workspace_id.to_owned(), reference.to_owned());
        let found = blocking(move || {
            let workspace = store.workspace(&id)?.path;
            Ok(placed_in_files(&workspace, &wanted, unrendered))
        })
        .await
        .unwrap_or_default();
        for found in found {
            if !uses.iter().any(|used| used.view == found.view) {
                uses.push(found);
            }
        }
        uses
    }
}

/// The views of `unrendered` (id, source in words, `./views/<file>`) whose TSX file names `reference`, then the views
/// of the workspace's `views.json` whose layout holds it (DMN-209).
fn placed_in_files(
    workspace: &Path,
    reference: &str,
    unrendered: Vec<(String, String, String)>,
) -> Vec<Use> {
    let mut uses = Vec::new();
    for (view, from, file) in unrendered {
        let path = views_folder(workspace).join(file.trim_start_matches("./views/"));
        if std::fs::read_to_string(path).is_ok_and(|source| source.contains(reference)) {
            uses.push(Use { view, from });
        }
    }
    if let (Some(manifest), None) = Widgets::manifest(workspace) {
        for view in layout::views_using(&manifest, reference) {
            uses.push(Use {
                view,
                from: "views.json".to_owned(),
            });
        }
    }
    uses
}

/// Runs `work` until it ends or `is_cancelled` says so, looking every [`CANCEL_POLL`].
async fn until_cancelled<T>(
    work: impl Future<Output = T>,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<T, String> {
    let mut work = std::pin::pin!(work);
    loop {
        if is_cancelled() {
            return Err("cancelled".to_owned());
        }
        tokio::select! {
            result = &mut work => return Ok(result),
            () = tokio::time::sleep(CANCEL_POLL) => {}
        }
    }
}

impl Services {
    /// The windows showing workspace `workspace_id`, each with why it has not settled (`None` when it has), waiting at
    /// most `wait` until all have (DMN-063): `unsettled` judges a window against the latest position of the views
    /// topic.
    pub(super) async fn settled_windows(
        &self,
        workspace_id: &str,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        wait: Duration,
        unsettled: impl Fn(&Window, Option<&Position>) -> Option<String>,
    ) -> Result<Vec<(Window, Option<String>)>, String> {
        let views = self
            .views_log_position(workspace_id)
            .map(|position| Position {
                epoch: position.epoch,
                seq: position.seq,
            });
        let mut changes = self.windows.changed.subscribe();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            changes.borrow_and_update();
            let windows: Vec<(Window, Option<String>)> = self
                .windows
                .showing(workspace_id)
                .into_iter()
                .map(|window| {
                    let why = unsettled(&window, views.as_ref());
                    (window, why)
                })
                .collect();
            let settled = windows.iter().all(|(_, why)| why.is_none());
            if settled || tokio::time::Instant::now() >= deadline {
                return Ok(windows);
            }
            if is_cancelled() {
                return Err("cancelled".to_owned());
            }
            let tick = deadline.min(tokio::time::Instant::now() + CANCEL_POLL);
            let _ = tokio::time::timeout_at(tick, changes.changed()).await;
        }
    }

    /// What the daemon's `[keymap]` and the `.omfp/keymap.json` of workspace `workspace_id` add to the default keys,
    /// for `omfp://keymap` (DMN-199).
    pub async fn keymap_additions(&self, workspace_id: &str) -> shell::Additions {
        let store = Arc::clone(&self.store);
        let id = workspace_id.to_owned();
        let workspace = blocking(move || {
            let path = store.workspace(&id)?.path;
            Ok(keymap::read(&keymap::path(&path)).map(Option::unwrap_or_default))
        })
        .await
        .unwrap_or_else(|error| Err(error.message));
        shell::Additions {
            daemon: self.settings().keymap,
            workspace: Some(workspace),
        }
    }
}

/// The server's [`WidgetHost`]: runs the services' futures on the daemon's runtime from the `omp-rpc` handler
/// thread a host tool runs on.
pub(crate) struct ServicesWidgetHost {
    services: Services,
    runtime: tokio::runtime::Handle,
}

impl ServicesWidgetHost {
    pub(crate) fn new(services: Services) -> Self {
        Self {
            services,
            runtime: tokio::runtime::Handle::current(),
        }
    }
}

impl WidgetHost for ServicesWidgetHost {
    fn check_widget(
        &self,
        workspace_id: &str,
        name: &str,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<String, String> {
        self.runtime.block_on(self.services.check_widget(
            workspace_id,
            name,
            is_cancelled,
            CHECK_WAIT,
        ))
    }

    fn keymap_additions(&self, workspace_id: &str) -> shell::Additions {
        self.runtime
            .block_on(self.services.keymap_additions(workspace_id))
    }

    fn windows(&self, workspace_id: &str) -> String {
        let states = self.services.window_states(Some(workspace_id));
        let others = self
            .services
            .window_states(None)
            .len()
            .saturating_sub(states.len());
        crate::agent_tools::windows::describe_windows(Some(workspace_id), &states, others)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use omfp_protocol::{
        AnswerWindowParams, FileErrorKind, FileErrorReport, ReportWidgetsParams, ServerFrame,
        ViewErrors, ViewSource, WidgetJobReport, WindowAnswer, WindowFileErrors, WindowReport,
        WindowRequest, WindowView,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::agent::FakeLauncher;
    use crate::git::{FakeGit, Git};
    use crate::paths::DataDir;
    use crate::server::Outbound;
    use crate::store::{Store, Workspace};
    use crate::supervisor::{Supervisor, SupervisorConfig};
    use crate::threads::Threads;
    use crate::workspaces::{NewWorkspace, Workspaces};

    struct Harness {
        _dir: TempDir,
        services: Services,
        workspace: String,
        folder: PathBuf,
    }

    async fn services(dir: &TempDir, git: Arc<FakeGit>) -> Services {
        let data = DataDir::new(dir.path().join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let root = dir.path().join("workspaces");
        let store = Arc::new(Store::open(&data.db(), &root).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            git as Arc<dyn Git>,
            root,
        ));
        let widgets = Arc::new(Widgets::new(data.widgets(), None));
        let supervisor = Supervisor::new(
            Arc::new(FakeLauncher::new()),
            Arc::clone(&threads),
            Arc::clone(&workspaces),
            data,
            SupervisorConfig::new("omp"),
        );
        Services::new(
            store,
            threads,
            workspaces,
            supervisor,
            widgets,
            Arc::new(crate::jobs::declared::SystemClock),
        )
        .await
        .unwrap()
    }

    /// Services with one workspace holding the widget `tests`.
    async fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let services = services(&dir, Arc::new(FakeGit::new())).await;
        let view = services.workspaces.create(NewWorkspace::new()).unwrap();
        let folder = view.workspace.path.clone();
        write(
            &folder,
            ".omfp/widgets/tests/index.tsx",
            "export default function T() { return <p>ok</p>; }\n",
        );
        Harness {
            _dir: dir,
            services,
            workspace: view.workspace.id,
            folder,
        }
    }

    fn write(folder: &Path, relative: &str, text: &str) {
        let path = folder.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    impl Harness {
        fn version(&self) -> String {
            folders::version(&self.folder.join(".omfp/widgets/tests"))
        }

        fn report(&self, connection: u64, views: Option<Position>, widgets: Vec<WidgetReport>) {
            self.report_files(connection, views, widgets, Vec::new());
        }

        fn report_files(
            &self,
            connection: u64,
            views: Option<Position>,
            widgets: Vec<WidgetReport>,
            file_errors: Vec<FileErrorReport>,
        ) {
            self.services.report_widgets(
                connection,
                "web",
                ReportWidgetsParams {
                    workspace_id: Some(self.workspace.clone()),
                    views,
                    widgets,
                    file_errors,
                    window: None,
                },
            );
        }

        /// Window `connection` shows the workspace with `views` and none of its widgets in its current view.
        fn report_views(&self, connection: u64, views: Vec<WindowView>) {
            self.services.report_widgets(
                connection,
                "web",
                ReportWidgetsParams {
                    workspace_id: Some(self.workspace.clone()),
                    views: None,
                    widgets: Vec::new(),
                    file_errors: Vec::new(),
                    window: Some(WindowReport {
                        views,
                        ..WindowReport::default()
                    }),
                },
            );
        }

        /// Window `connection` answers every request of the daemon with `answer`.
        fn answering(
            &self,
            connection: u64,
            answer: impl Fn(WindowRequest) -> WindowAnswer + Send + 'static,
        ) -> tokio::task::JoinHandle<Vec<WindowRequest>> {
            let (out, mut frames) = tokio::sync::mpsc::channel(8);
            self.services.link_window(connection, out);
            let services = self.services.clone();
            tokio::spawn(async move {
                let mut asked = Vec::new();
                while let Some(Outbound::Text(text)) = frames.recv().await {
                    if let Ok(ServerFrame::WindowRequest { id, request }) =
                        serde_json::from_str(&text)
                    {
                        asked.push(request.clone());
                        let params = AnswerWindowParams {
                            request_id: id,
                            answer: answer(request),
                        };
                        services.answer_window(connection, params).unwrap();
                    }
                }
                asked
            })
        }

        async fn check_named(&self, name: &str, wait: Duration) -> String {
            self.services
                .check_widget(&self.workspace, name, &|| false, wait)
                .await
                .unwrap()
        }

        async fn check(&self, wait: Duration) -> String {
            self.services
                .check_widget(&self.workspace, "tests", &|| false, wait)
                .await
                .unwrap()
        }
    }

    fn instance(version: Option<String>, state: WidgetReportState) -> WidgetReport {
        WidgetReport {
            reference: "./widgets/tests".to_owned(),
            instance: "./widgets/tests".to_owned(),
            source: WidgetSource::Workspace,
            name: "tests".to_owned(),
            version,
            state,
            errors: vec!["index.tsx:3:7: boom".to_owned()],
            console: vec!["TypeError: x is undefined\n    at Tests (index.tsx:4:2)".to_owned()],
            text: "Tests 1/3\n\nPASS 1".to_owned(),
            jobs: vec![WidgetJobReport {
                command: "make test".to_owned(),
                status: "running".to_owned(),
                exit_code: None,
                signal: None,
                runs: 2,
                started_at: Some(1),
                tail: vec!["PASS 1".to_owned()],
            }],
        }
    }

    // spec: DMN-063, DMN-122
    #[tokio::test]
    async fn without_windows_the_check_gives_the_compile_result_and_says_nothing_rendered() {
        let h = harness().await;
        let text = h.check(Duration::ZERO).await;
        assert!(
            text.starts_with("Widget ./widgets/tests (folder "),
            "{text}"
        );
        assert!(
            text.contains(&format!("version {}, entry index.tsx", h.version())),
            "{text}"
        );
        assert!(
            text.contains("Daemon compile: every file compiles"),
            "{text}"
        );
        assert!(
            text.contains("No omfp window shows this workspace now"),
            "{text}"
        );

        write(
            &h.folder,
            ".omfp/widgets/tests/lib.ts",
            "export const = 1;\n",
        );
        write(&h.folder, ".omfp/views.json", "{ \"views\": {} }");
        let text = h.check(Duration::ZERO).await;
        assert!(text.contains("Daemon compile: 1 file(s) fail"), "{text}");
        assert!(text.contains("- lib.ts:1:"), "{text}");
        assert!(text.contains("views.json is invalid ("), "{text}");
        assert!(!text.contains("keymap.json"), "no keymap: {text}");
        write(&h.folder, ".omfp/keymap.json", "{ \"bindings\": [1] }");
        let text = h.check(Duration::ZERO).await;
        assert!(
            text.contains(
                "keymap.json is invalid (bindings[0]: expected an object with keys and command): windows keep \
                 its last valid bindings until it is fixed."
            ),
            "{text}"
        );

        let error = h
            .services
            .check_widget(&h.workspace, "nothing", &|| false, Duration::ZERO)
            .await
            .unwrap_err();
        assert!(error.contains("no widget nothing"), "{error}");
    }

    // spec: AGT-028
    #[tokio::test]
    async fn the_check_warns_when_views_json_lost_the_main_slot() {
        let h = harness().await;
        let warning = "Warning: no view of views.json has the main slot";
        let text = h.check(Duration::ZERO).await;
        assert!(!text.contains(warning), "the default views keep it: {text}");

        write(
            &h.folder,
            ".omfp/views.json",
            r#"{ "views": { "default": { "layout": ["row", {}, "omfp:chat", "./widgets/tests"] } } }"#,
        );
        let text = h.check(Duration::ZERO).await;
        assert!(text.contains(warning), "{text}");
        assert!(text.contains("Keep that slot"), "{text}");
    }

    // spec: AGT-089
    #[tokio::test]
    async fn the_check_warns_about_native_form_controls_with_their_line_and_kit_replacement() {
        let h = harness().await;
        let text = h.check(Duration::ZERO).await;
        assert!(!text.contains("native form control"), "{text}");

        write(
            &h.folder,
            ".omfp/widgets/tests/index.tsx",
            "import { Filter } from \"./filter.tsx\";\n\
             export default function T() {\n\
             \x20 return <div><Filter /><input type=\"checkbox\" /></div>;\n\
             }\n",
        );
        write(
            &h.folder,
            ".omfp/widgets/tests/filter.tsx",
            "export function Filter() {\n  return <select><option>All</option></select>;\n}\n",
        );
        let text = h.check(Duration::ZERO).await;
        for needle in [
            "Daemon compile: every file compiles",
            "Warning: 2 native form control(s): their opened lists and pickers are drawn by the platform, not the \
             theme",
            "- filter.tsx:2: <select> → Select from omfp/kit\n",
            "- index.tsx:3: <input type=\"checkbox\"> → Checkbox from omfp/kit\n",
        ] {
            assert!(text.contains(needle), "{needle:?} missing: {text}");
        }
    }

    // spec: DMN-063, PRT-043, AGT-026, DMN-209
    #[tokio::test]
    async fn the_check_reports_what_each_window_shows_and_names_omfp_ui_mount_when_no_view_uses_it()
    {
        let h = harness().await;
        h.report(7, None, Vec::new());
        let text = h.check(Duration::ZERO).await;
        assert!(
            text.contains("1 window(s) show this workspace, but none shows ./widgets/tests"),
            "{text}"
        );
        assert!(
            text.contains(
                "No view uses it: mount it with omfp_ui_mount { \"id\": \"tests\", \"widget\": \"./widgets/tests\" }."
            ),
            "{text}"
        );
        assert!(
            text.contains("No out-of-sight render: omfp window 7 is closed."),
            "a window without a connection renders nothing: {text}"
        );

        h.report(
            7,
            None,
            vec![instance(Some(h.version()), WidgetReportState::RenderError)],
        );
        let text = h.check(Duration::ZERO).await;
        for needle in [
            "Window 7 (web):\n- ./widgets/tests: render error (version ",
            "  errors:\n    - index.tsx:3:7: boom\n",
            "  console errors:\n    - TypeError: x is undefined\n          at Tests (index.tsx:4:2)\n",
            "  jobs:\n    - `make test`: running, run 2\n      last lines:\n      PASS 1\n",
            "  text:\n    Tests 1/3\n    PASS 1\n",
        ] {
            assert!(text.contains(needle), "{needle:?} missing: {text}");
        }
        assert!(!text.contains("out-of-sight"), "{text}");

        // A closed window's report goes with it.
        h.services.forget_widget_reports(7);
        let text = h.check(Duration::ZERO).await;
        assert!(
            text.contains(
                "No omfp window shows this workspace now, so nothing rendered the widget, not even out of sight"
            ),
            "{text}"
        );
    }

    fn view(
        id: &str,
        source: ViewSource,
        file: Option<&str>,
        widgets: Option<&[&str]>,
    ) -> WindowView {
        WindowView {
            id: id.to_owned(),
            title: id.to_owned(),
            source,
            file: file.map(str::to_owned),
            widgets: widgets
                .map(|widgets| widgets.iter().map(|&widget| widget.to_owned()).collect()),
        }
    }

    // spec: DMN-209
    #[tokio::test]
    async fn the_check_names_the_views_that_hold_the_widget_instead_of_mounting_it_again() {
        let h = harness().await;
        // An init file's view holds it, a TSX view no window rendered names it, views.json holds it too.
        write(
            &h.folder,
            ".omfp/views/ops.tsx",
            "export default () => <Row><Widget use=\"./widgets/tests\" /></Row>;\n",
        );
        write(
            &h.folder,
            ".omfp/views/other.tsx",
            "export default () => <Row><Widget use=\"omfp:chat\" /></Row>;\n",
        );
        h.report_views(
            7,
            vec![
                view("default", ViewSource::ViewsJson, None, Some(&["omfp:chat"])),
                view(
                    "solo",
                    ViewSource::WorkspaceInit,
                    None,
                    Some(&["./widgets/tests"]),
                ),
                view("ops", ViewSource::UserInit, Some("./views/ops.tsx"), None),
                view(
                    "other",
                    ViewSource::ViewsJson,
                    Some("./views/other.tsx"),
                    None,
                ),
            ],
        );
        let text = h.check(Duration::ZERO).await;
        assert!(
            text.contains(
                " It is in view \"solo\" (the workspace's init file), \"ops\" (the user's init file, TSX view \
                 ./views/ops.tsx): show it with omfp_window_run { \"command\": \"view.switch\", \"args\": [\"solo\"] } \
                 instead of mounting it again."
            ),
            "{text}"
        );
        assert!(!text.contains("No view uses it"), "{text}");

        write(
            &h.folder,
            ".omfp/views.json",
            r#"{ "views": { "default": { "layout": ["row", {}, "omfp:chat", ["./widgets/tests", { "instance": "t" }]] } } }"#,
        );
        h.report_views(7, Vec::new());
        let text = h.check(Duration::ZERO).await;
        assert!(
            text.contains(" It is in view \"default\" (views.json): show it with omfp_window_run"),
            "views.json counts when no window lists its views: {text}"
        );
    }

    // spec: DMN-207, DMN-208
    #[tokio::test]
    async fn the_check_renders_a_widget_no_window_shows_out_of_sight_in_the_last_focused_window() {
        let h = harness().await;
        h.report_views(7, Vec::new());
        h.report_views(8, Vec::new());
        h.services.report_widgets(
            8,
            "desktop",
            ReportWidgetsParams {
                workspace_id: Some(h.workspace.clone()),
                views: None,
                widgets: Vec::new(),
                file_errors: Vec::new(),
                window: Some(WindowReport {
                    focused: true,
                    ..WindowReport::default()
                }),
            },
        );
        let version = h.version();
        let unused = h.answering(7, |_| unreachable!("the focused window renders it"));
        let answered = h.answering(8, |request| {
            let WindowRequest::CheckWidget {
                reference, version, ..
            } = request
            else {
                unreachable!("{request:?}")
            };
            let mut report = instance(version, WidgetReportState::RenderError);
            report.instance = format!("{reference}@check");
            report.errors = vec!["x is not defined".to_owned()];
            report.jobs = Vec::new();
            WindowAnswer::Checked {
                report,
                settled: true,
            }
        });
        let text = h.check(Duration::ZERO).await;
        for needle in [
            "2 window(s) show this workspace, but none shows ./widgets/tests in its current view.",
            "Out-of-sight render in window 8 (desktop), mounted in a hidden container for this check:\n\
             - ./widgets/tests@check: render error (version ",
            "  errors:\n    - x is not defined\n",
        ] {
            assert!(text.contains(needle), "{needle:?} missing: {text}");
        }
        h.services.forget_widget_reports(7);
        h.services.forget_widget_reports(8);
        assert_eq!(unused.await.unwrap(), Vec::new());
        assert_eq!(
            answered.await.unwrap(),
            vec![WindowRequest::CheckWidget {
                reference: "./widgets/tests".to_owned(),
                version: Some(version),
                timeout_ms: 4000,
            }],
            "the window waits for the folder's current version"
        );
    }

    // spec: DMN-207
    #[tokio::test]
    async fn a_window_out_of_time_says_so() {
        let h = harness().await;
        h.report_views(7, Vec::new());
        let _window = h.answering(7, |_| WindowAnswer::Checked {
            report: instance(None, WidgetReportState::Loading),
            settled: false,
        });
        let text = h.check(Duration::ZERO).await;
        assert!(
            text.contains(
                "Out-of-sight render in window 7 (web), mounted in a hidden container for this check, which neither \
                 rendered nor failed within 4 s:\n- ./widgets/tests: loading (version none)"
            ),
            "{text}"
        );
    }

    // spec: DMN-063, AGT-026
    #[tokio::test]
    async fn the_check_waits_for_windows_to_render_the_current_version_and_names_a_late_one() {
        let h = harness().await;
        // A window follows the views topic: the check waits until it reported that topic's latest position.
        h.services.views_log(&h.workspace).await.unwrap();
        let old = Position {
            epoch: "old".to_owned(),
            seq: 1,
        };
        h.report(
            1,
            Some(old),
            vec![instance(Some("0".to_owned()), WidgetReportState::Loading)],
        );
        let text = h.check(Duration::from_millis(50)).await;
        assert!(
            text.contains(
                "Window 1 (web), after waiting 0 s, has not caught up with the latest change"
            ),
            "{text}"
        );

        let current = h.services.views_log_position(&h.workspace).unwrap();
        let current = Position {
            epoch: current.epoch,
            seq: current.seq,
        };
        let (services, workspace, version) = (h.services.clone(), h.workspace.clone(), h.version());
        let seen = current.clone();
        let late = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            services.report_widgets(
                1,
                "web",
                ReportWidgetsParams {
                    workspace_id: Some(workspace),
                    views: Some(seen),
                    widgets: vec![instance(Some(version), WidgetReportState::Rendered)],
                    file_errors: Vec::new(),
                    window: None,
                },
            );
        });
        let text = h.check(Duration::from_secs(30)).await;
        late.await.unwrap();
        assert!(
            text.contains("Window 1 (web):\n- ./widgets/tests: rendered"),
            "{text}"
        );

        h.report(
            1,
            Some(current.clone()),
            vec![instance(Some(h.version()), WidgetReportState::Loading)],
        );
        let text = h.check(Duration::from_millis(50)).await;
        assert!(text.contains("is still loading ./widgets/tests"), "{text}");

        // A job the widget is still starting is waited for too.
        let mut starting = instance(Some(h.version()), WidgetReportState::Rendered);
        starting.jobs[0].status = "pending".to_owned();
        h.report(1, Some(current), vec![starting]);
        let text = h.check(Duration::from_millis(50)).await;
        assert!(
            text.contains("is still starting the job `make test` of ./widgets/tests"),
            "{text}"
        );

        let cancelled = h
            .services
            .check_widget(&h.workspace, "tests", &|| true, Duration::from_secs(30))
            .await;
        assert_eq!(cancelled, Err("cancelled".to_owned()));
    }

    fn file_error(kind: FileErrorKind, file: &str, message: &str) -> FileErrorReport {
        FileErrorReport {
            kind,
            file: file.to_owned(),
            message: message.to_owned(),
        }
    }

    // spec: PRT-116, DMN-195
    #[tokio::test]
    async fn views_errors_lists_every_window_showing_the_workspace_with_its_file_errors() {
        let h = harness().await;
        assert_eq!(
            h.services.view_errors(&h.workspace).await.unwrap(),
            ViewErrors {
                windows: Vec::new()
            }
        );
        let broken = file_error(FileErrorKind::Init, ".omfp/init.ts", "boom");
        h.report_files(1, None, Vec::new(), vec![broken.clone()]);
        h.report(2, None, Vec::new());
        // A window showing another workspace is not listed.
        h.services.report_widgets(
            3,
            "desktop",
            ReportWidgetsParams {
                workspace_id: Some("01OTHER".to_owned()),
                views: None,
                widgets: Vec::new(),
                file_errors: vec![broken.clone()],
                window: None,
            },
        );
        assert_eq!(
            h.services.view_errors(&h.workspace).await.unwrap(),
            ViewErrors {
                windows: vec![
                    WindowFileErrors {
                        client: "web".to_owned(),
                        errors: vec![broken],
                    },
                    WindowFileErrors {
                        client: "web".to_owned(),
                        errors: Vec::new(),
                    },
                ],
            }
        );
        h.services.forget_widget_reports(1);
        assert_eq!(
            h.services
                .view_errors(&h.workspace)
                .await
                .unwrap()
                .windows
                .len(),
            1,
            "a closed window's errors go with it"
        );
        let error = h.services.view_errors("01NOPE").await.unwrap_err();
        assert_eq!(error.code, omfp_protocol::ErrorCode::NotFound, "{error:?}");
    }

    // spec: DMN-196
    #[tokio::test]
    async fn omfp_views_errors_describes_each_windows_init_and_view_errors() {
        let h = harness().await;
        let describe = || async {
            h.services
                .describe_view_errors(&h.workspace, &|| false, Duration::ZERO)
                .await
                .unwrap()
        };
        let text = describe().await;
        assert!(
            text.starts_with("No omfp window shows this workspace now"),
            "{text}"
        );
        h.report(1, None, Vec::new());
        assert_eq!(
            describe().await,
            "1 window(s) show this workspace; none reports an init file or TSX view error.\n\
             Window 1 (web): no init file or TSX view errors.\n"
        );
        h.report_files(
            1,
            None,
            Vec::new(),
            vec![
                file_error(
                    FileErrorKind::Init,
                    "user",
                    "TypeError: x\n    at init.ts:2:1",
                ),
                file_error(
                    FileErrorKind::View,
                    ".omfp/views/ops.tsx",
                    "Row: bad layout",
                ),
            ],
        );
        h.report(2, None, Vec::new());
        assert_eq!(
            describe().await,
            "2 window(s) show this workspace.\n\
             Window 1 (web):\n\
             - the user's init file (init): TypeError: x\n\
             \x20     at init.ts:2:1\n\
             - .omfp/views/ops.tsx (view): Row: bad layout\n\
             Window 2 (web): no init file or TSX view errors.\n"
        );
        // Windows behind the views topic are waited for, then named.
        h.services.views_log(&h.workspace).await.unwrap();
        let text = h
            .services
            .describe_view_errors(&h.workspace, &|| false, Duration::from_millis(20))
            .await
            .unwrap();
        assert!(
            text.contains("Window 2 (web), after waiting 0 s, has not caught up"),
            "{text}"
        );
        assert_eq!(
            h.services
                .describe_view_errors(&h.workspace, &|| true, Duration::from_secs(30))
                .await,
            Err("cancelled".to_owned())
        );
    }

    // spec: DMN-197
    #[tokio::test]
    async fn the_check_compiles_the_workspace_init_file_and_lists_what_windows_report_for_it() {
        let h = harness().await;
        for name in ["init", ".omfp/init.ts"] {
            let text = h.check_named(name, Duration::ZERO).await;
            assert!(text.starts_with("No file .omfp/init.ts ("), "{text}");
            assert!(
                text.contains("No omfp window shows this workspace now"),
                "{text}"
            );
        }
        write(&h.folder, ".omfp/init.ts", "export const n: number = 1;\n");
        let text = h.check_named("init", Duration::ZERO).await;
        let version = crate::widgets::init::read_version(&h.folder.join(".omfp/init.ts")).unwrap();
        assert!(text.starts_with("Init file .omfp/init.ts at "), "{text}");
        assert!(text.contains(&format!("version {version})")), "{text}");
        assert!(text.contains("Daemon compile: it compiles.\n"), "{text}");

        write(&h.folder, ".omfp/init.ts", "\nexport const = 1;\n");
        h.report_files(
            1,
            None,
            Vec::new(),
            vec![
                file_error(FileErrorKind::Init, ".omfp/init.ts", "SyntaxError: boom"),
                file_error(FileErrorKind::Init, "user", "not this one"),
            ],
        );
        h.report(2, None, Vec::new());
        let text = h.check_named("init", Duration::ZERO).await;
        for needle in [
            "Daemon compile: it fails; windows load a stand-in that shows this error instead:\n- .omfp/init.ts:2:",
            "2 window(s) show this workspace:\nWindow 1 (web):\n- SyntaxError: boom\nWindow 2 (web): no error for \
             .omfp/init.ts\n",
        ] {
            assert!(text.contains(needle), "{needle:?} missing: {text}");
        }
        assert!(!text.contains("not this one"), "{text}");
        assert!(!text.contains("No window reports an error"), "{text}");

        h.report(1, None, Vec::new());
        let text = h.check_named("init", Duration::ZERO).await;
        assert!(
            text.ends_with("No window reports an error for it.\n"),
            "{text}"
        );
    }

    // spec: DMN-197
    #[tokio::test]
    async fn the_check_of_the_user_init_file_names_the_followed_file() {
        let h = harness().await;
        let text = h.check_named("user:init", Duration::ZERO).await;
        assert!(
            text.starts_with("This omfp daemon follows no user init file."),
            "{text}"
        );
        let path = h.folder.join("config/init.ts");
        *super::super::lock(&h.services.init_file) = Some(path.clone());
        let text = h.check_named("user:init", Duration::ZERO).await;
        assert!(
            text.starts_with(&format!("No file {}: ", path.display())),
            "{text}"
        );
        write(&h.folder, "config/init.ts", "export const = 1;\n");
        h.report_files(
            1,
            None,
            Vec::new(),
            vec![file_error(FileErrorKind::Init, "user", "TypeError: x")],
        );
        let text = h.check_named("user:init", Duration::ZERO).await;
        assert!(
            text.starts_with(&format!(
                "The user's init file {} (version ",
                path.display()
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!("- {}:1:", path.display())),
            "the error names the file: {text}"
        );
        assert!(text.contains("Window 1 (web):\n- TypeError: x\n"), "{text}");
    }

    // spec: DMN-197
    #[tokio::test]
    async fn the_check_of_a_tsx_view_compiles_it_and_leaves_the_layout_to_the_windows() {
        let h = harness().await;
        let text = h.check_named("views/ops.tsx", Duration::ZERO).await;
        assert!(
            text.starts_with("No TSX view file .omfp/views/ops.tsx in "),
            "{text}"
        );
        write(
            &h.folder,
            ".omfp/views/ops.tsx",
            "export default function Ops() {\n  return <p>;\n}\n",
        );
        h.report_files(
            1,
            None,
            Vec::new(),
            vec![file_error(
                FileErrorKind::View,
                ".omfp/views/ops.tsx",
                "ops.tsx:2:10: Unexpected token",
            )],
        );
        for name in [
            "views/ops.tsx",
            ".omfp/views/ops.tsx",
            "./views/ops",
            " views/ops.tsx ",
        ] {
            let text = h.check_named(name, Duration::ZERO).await;
            for needle in [
                "TSX view .omfp/views/ops.tsx (",
                ".omfp/views/ version ",
                "Daemon compile: it fails; windows load a stand-in that shows this error instead:\n- \
                 .omfp/views/ops.tsx:",
                "The daemon cannot check the layout",
                "Window 1 (web):\n- ops.tsx:2:10: Unexpected token\n",
            ] {
                assert!(text.contains(needle), "{name}: {needle:?} missing: {text}");
            }
        }
        let error = h
            .services
            .check_widget(&h.workspace, "nothing", &|| false, Duration::ZERO)
            .await
            .unwrap_err();
        assert!(
            error.contains("`init` or `user:init`") && error.contains("`views/<file>`"),
            "the widget error names the other forms: {error}"
        );
    }

    // spec: DMN-199
    #[tokio::test]
    async fn the_keymap_additions_are_the_daemons_keymap_and_the_workspaces_bindings() {
        let h = harness().await;
        let additions = h.services.keymap_additions(&h.workspace).await;
        assert_eq!(additions.daemon, h.services.settings().keymap);
        assert_eq!(additions.workspace, Some(Ok(Vec::new())), "no file");
        write(&h.folder, ".omfp/keymap.json", "{ \"bindings\": [1] }");
        let additions = h.services.keymap_additions(&h.workspace).await;
        assert_eq!(
            additions.workspace,
            Some(Err(
                "bindings[0]: expected an object with keys and command".to_owned()
            ))
        );
        let unknown = h.services.keymap_additions("01NOPE").await;
        assert!(matches!(unknown.workspace, Some(Err(_))), "{unknown:?}");
    }

    // spec: DMN-120, DMN-121, PRT-060
    #[tokio::test]
    async fn the_views_topic_carries_the_workspace_keymap_and_an_invalid_file_keeps_the_last_valid_one()
     {
        use omfp_protocol::{KeymapMode, ViewsEvent, WorkspaceBinding};

        let h = harness().await;
        let file = h.folder.join(".omfp/keymap.json");
        let log = h.services.views_log(&h.workspace).await.unwrap();
        let keymap = || log.with_state(|state| (state.keymap.clone(), state.keymap_error.clone()));
        assert_eq!(keymap(), (Vec::new(), None), "no file: no bindings");
        let mut live = log.subscribe(Some(&log.position())).live;
        let mut next = move || match live.try_recv().ok().map(|event| event.event.clone()) {
            Some(ViewsEvent::ViewsKeymap { keymap, error }) => Some((keymap, error)),
            _ => None,
        };

        let valid =
            r#"{ "bindings": [{ "keys": "g t", "command": "tests.run", "args": ["unit"] }] }"#;
        write(&h.folder, ".omfp/keymap.json", valid);
        h.services.refresh_views(&h.workspace).await;
        let bound = vec![WorkspaceBinding {
            mode: KeymapMode::Normal,
            keys: "g t".to_owned(),
            command: "tests.run".to_owned(),
            args: vec!["unit".to_owned()],
        }];
        assert_eq!(keymap(), (bound.clone(), None));
        assert_eq!(next(), Some((bound.clone(), None)), "pushed on change");
        h.services.refresh_views(&h.workspace).await;
        assert_eq!(next(), None, "nothing changed, nothing pushed");

        let invalid = r#"{ "bindings": [{ "keys": "", "command": "tests.run" }] }"#;
        write(&h.folder, ".omfp/keymap.json", invalid);
        h.services.refresh_views(&h.workspace).await;
        let error = "bindings[0].keys: expected a key sequence such as \"Space t\"".to_owned();
        assert_eq!(
            next(),
            Some((bound.clone(), Some(error.clone()))),
            "the last valid bindings stay, with the error"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            invalid,
            "never rewritten"
        );

        std::fs::remove_file(&file).unwrap();
        h.services.refresh_views(&h.workspace).await;
        assert_eq!(
            next(),
            Some((Vec::new(), None)),
            "a removed file drops them"
        );
    }

    // spec: DMN-088, DMN-030
    #[tokio::test]
    async fn startup_writes_the_default_views_json_where_it_is_missing_but_never_into_a_repository()
    {
        let dir = tempfile::tempdir().unwrap();
        let git = Arc::new(FakeGit::new());
        let root = dir.path().join("workspaces");
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let store = Store::open(&dir.path().join("data/omfp.db"), &root).unwrap();
        let mut folders = Vec::new();
        for name in ["old", "kept", "repo"] {
            let path = root.join(name);
            std::fs::create_dir_all(&path).unwrap();
            store
                .insert_workspace(&Workspace {
                    id: format!("01{name}"),
                    name: name.to_owned(),
                    path: path.clone(),
                    created_by: crate::store::Provenance::User,
                    created_at: 0,
                })
                .unwrap();
            folders.push(path);
        }
        write(&folders[1], ".omfp/views.json", "{ \"mine\": true }");
        git.add_repo(&folders[2], "main");
        drop(store);

        let _services = services(&dir, git).await;

        let views = |folder: &Path| std::fs::read_to_string(folder.join(".omfp/views.json")).ok();
        assert_eq!(
            views(&folders[0]).as_deref(),
            Some(crate::widgets::DEFAULT_MANIFEST)
        );
        assert_eq!(views(&folders[1]).as_deref(), Some("{ \"mine\": true }"));
        assert_eq!(views(&folders[2]), None, "the folder is a repository");
    }
}
