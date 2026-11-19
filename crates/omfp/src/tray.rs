//! The daemon's tray icon (DMN-128..130, UI-217..219): `omfp daemon --tray`, which `omfp desktop` passes to the
//! daemon it starts. The model (status, menu, window command) is shared; the platform shows it: on Linux a
//! KDE/freedesktop `StatusNotifierItem` with a `com.canonical.dbusmenu` menu over gio's D-Bus API ([`sni`]), on macOS
//! an `NSStatusItem` in the menu bar through Tauri's tray (`macos`, DMN-171, UI-254).

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use omfp_core::ProcessState;
use omfp_daemon::{DaemonHandle, StopRequest};
use omfp_protocol::ThreadSummary;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod sni;
#[cfg(target_os = "macos")]
pub use macos::run_daemon;
#[cfg(target_os = "linux")]
pub use sni::Tray;

/// What the tray shows of the thread index (UI-217): non-archived threads, those whose agent works, and those waiting
/// for the user (newest first).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayStatus {
    pub threads: usize,
    pub running: usize,
    pub need_you: Vec<NeedsYou>,
}

/// A thread waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsYou {
    pub id: String,
    pub title: String,
}

/// The tray icon (UI-219).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayIcon {
    /// No agent works and nobody waits.
    Idle,
    /// At least one agent works.
    Running,
    /// At least one thread waits for the user.
    NeedsYou,
}

impl TrayStatus {
    /// The status of `threads` as the left column counts them (UI-020): attention first, then a working process
    /// (starting, or running and not settled).
    #[must_use]
    pub fn of(threads: &[ThreadSummary]) -> Self {
        let live: Vec<&ThreadSummary> = threads
            .iter()
            .filter(|thread| thread.archived_at.is_none())
            .collect();
        let mut waiting: Vec<&ThreadSummary> = live
            .iter()
            .copied()
            .filter(|thread| thread.needs_attention)
            .collect();
        waiting.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
        Self {
            threads: live.len(),
            running: live
                .iter()
                .filter(|thread| {
                    !thread.needs_attention
                        && match thread.process {
                            ProcessState::Starting => true,
                            ProcessState::Running => !thread.settled,
                            _ => false,
                        }
                })
                .count(),
            need_you: waiting
                .into_iter()
                .map(|thread| NeedsYou {
                    id: thread.id.clone(),
                    title: thread.title.clone(),
                })
                .collect(),
        }
    }

    /// The icon: a thread waiting for the user wins over a working one.
    #[must_use]
    pub fn icon(&self) -> TrayIcon {
        if !self.need_you.is_empty() {
            TrayIcon::NeedsYou
        } else if self.running > 0 {
            TrayIcon::Running
        } else {
            TrayIcon::Idle
        }
    }

    /// The menu's status line and the tooltip: `N threads · N running · N need you`.
    #[must_use]
    pub fn summary(&self) -> String {
        let threads = if self.threads == 1 {
            "1 thread".to_owned()
        } else {
            format!("{} threads", self.threads)
        };
        let need_you = if self.need_you.len() == 1 {
            "1 needs you".to_owned()
        } else {
            format!("{} need you", self.need_you.len())
        };
        format!("{threads} · {} running · {need_you}", self.running)
    }

    /// The threads whose work the quit interrupts: working or waiting for the user.
    #[must_use]
    pub fn busy(&self) -> usize {
        self.running + self.need_you.len()
    }

    /// `Quit daemon (stops N running threads)`, or `Quit daemon` when nothing is busy (DMN-130).
    #[must_use]
    pub fn quit_label(&self) -> String {
        match self.busy() {
            0 => "Quit daemon".to_owned(),
            1 => "Quit daemon (stops 1 running thread)".to_owned(),
            busy => format!("Quit daemon (stops {busy} running threads)"),
        }
    }
}

/// What a menu entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A new window on the daemon (`omfp desktop`).
    NewWindow,
    /// The daemon's UI in the browser, with its token.
    OpenInBrowser,
    /// A new window on this thread (`omfp desktop --thread <id>`).
    OpenThread(String),
    /// Stops the daemon (DMN-130).
    Quit,
}

/// One entry of the dbusmenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: i32,
    /// The dbusmenu label (`_` doubled: a single one marks a mnemonic); empty for a separator.
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub action: Option<Action>,
}

const NEW_WINDOW: i32 = 1;
const OPEN_IN_BROWSER: i32 = 2;
const STATUS: i32 = 3;
const QUIT: i32 = 4;
const SEPARATOR_TOP: i32 = 5;
const SEPARATOR_BOTTOM: i32 = 6;
/// The first id of a thread entry.
const FIRST_THREAD_ID: i32 = 100;
/// Longer thread titles are cut with an ellipsis.
const TITLE_CHARS: usize = 60;

/// Stable menu ids of the waiting threads: a thread keeps its id while it stays in the menu, so a click on a menu the
/// host shows from an older layout never opens another thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadIds {
    ids: HashMap<String, i32>,
    next: i32,
}

impl Default for ThreadIds {
    fn default() -> Self {
        Self {
            ids: HashMap::new(),
            next: FIRST_THREAD_ID,
        }
    }
}

impl ThreadIds {
    /// The ids of `threads`, new ones numbered on; threads no longer listed lose theirs.
    fn assign(&mut self, threads: &[NeedsYou]) -> Vec<i32> {
        let mut ids = HashMap::with_capacity(threads.len());
        let assigned = threads
            .iter()
            .map(|thread| {
                let id = self.ids.get(&thread.id).copied().unwrap_or_else(|| {
                    let id = self.next;
                    self.next += 1;
                    id
                });
                ids.insert(thread.id.clone(), id);
                id
            })
            .collect();
        self.ids = ids;
        assigned
    }
}

/// The tray menu (UI-218): `New window`, `Open in browser`, the status line (not clickable), the threads waiting for
/// the user, `Quit daemon`.
#[must_use]
pub fn menu(status: &TrayStatus, ids: &mut ThreadIds) -> Vec<MenuItem> {
    let item = |id, label: String, action: Option<Action>| MenuItem {
        id,
        enabled: action.is_some(),
        label,
        separator: false,
        action,
    };
    let separator = |id| MenuItem {
        id,
        label: String::new(),
        enabled: true,
        separator: true,
        action: None,
    };
    let mut items = vec![
        item(NEW_WINDOW, "New window".into(), Some(Action::NewWindow)),
        item(
            OPEN_IN_BROWSER,
            "Open in browser".into(),
            Some(Action::OpenInBrowser),
        ),
        separator(SEPARATOR_TOP),
        item(STATUS, status.summary(), None),
    ];
    let thread_ids = ids.assign(&status.need_you);
    items.extend(status.need_you.iter().zip(thread_ids).map(|(thread, id)| {
        item(
            id,
            format!("◆ {}", menu_label(&thread.title)),
            Some(Action::OpenThread(thread.id.clone())),
        )
    }));
    items.push(separator(SEPARATOR_BOTTOM));
    items.push(item(QUIT, status.quit_label(), Some(Action::Quit)));
    items
}

/// A thread title as a dbusmenu label: `New thread` before the first message (as the UI shows it), cut to
/// [`TITLE_CHARS`], underscores doubled.
fn menu_label(title: &str) -> String {
    let title = title.trim();
    let title = if title.is_empty() {
        "New thread"
    } else {
        title
    };
    let mut label: String = title.chars().take(TITLE_CHARS).collect();
    if title.chars().nth(TITLE_CHARS).is_some() {
        label.push('…');
    }
    label.replace('_', "__")
}

/// The command line of a new window on the daemon of `data_dir`, on `thread` when given (UI-218).
#[must_use]
pub fn window_command(data_dir: &Path, thread: Option<&str>) -> Vec<OsString> {
    let mut line = vec![
        OsString::from("desktop"),
        "--data-dir".into(),
        data_dir.as_os_str().to_owned(),
    ];
    if let Some(thread) = thread {
        line.extend(["--thread".into(), thread.into()]);
    }
    line
}

/// What the tray acts on: this executable (for new windows), the daemon's data directory, URL and stop.
#[derive(Debug, Clone)]
struct Daemon {
    exe: PathBuf,
    data_dir: PathBuf,
    /// The UI's URL with the token in the fragment.
    url: String,
    stop: StopRequest,
}

impl Daemon {
    /// The target of `daemon`'s tray; `None` (said in the log, `item` naming what is not shown) when this executable
    /// cannot be found.
    fn of(daemon: &DaemonHandle, item: &str) -> Option<Self> {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(error) => {
                note!("omfp daemon: no {item}, cannot find this executable: {error}");
                return None;
            }
        };
        Some(Self {
            exe,
            data_dir: daemon.data_dir().to_path_buf(),
            url: daemon.url(),
            stop: daemon.stop_request(),
        })
    }

    /// Runs a menu entry's action; `place` names the tray in the log, `open_in_browser` opens the UI's URL.
    fn act(&self, action: &Action, place: &str, open_in_browser: impl FnOnce(&str)) {
        match action {
            Action::NewWindow => self.open_window(None),
            Action::OpenThread(thread) => self.open_window(Some(thread)),
            Action::OpenInBrowser => open_in_browser(&self.url),
            Action::Quit => {
                note!("omfp daemon: quit from the {place}");
                self.stop.request();
            }
        }
    }

    /// `omfp desktop --data-dir <dir> [--thread <id>]`, detached like the daemon itself.
    fn open_window(&self, thread: Option<&str>) {
        spawn_detached(
            Command::new(&self.exe).args(window_command(&self.data_dir, thread)),
            "omfp desktop",
            "omfp daemon",
        );
    }
}

/// Opens `url` in the system's browser (UI-218, DMN-182): through the desktop portal's `OpenURI` (else `xdg-open`) on
/// Linux, `open` on macOS.
pub fn open_in_browser(url: &str) {
    #[cfg(target_os = "linux")]
    match gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
        Ok(connection) => sni::open_uri(&connection, url.to_owned()),
        Err(_) => spawn_detached(Command::new("xdg-open").arg(url), "xdg-open", "omfp daemon"),
    }
    #[cfg(target_os = "macos")]
    spawn_detached(
        Command::new("/usr/bin/open").arg(url),
        "open",
        "omfp daemon",
    );
}

/// Starts `command` (`name` in the log of `from`) detached like the daemon itself (ARC-003): its own process group,
/// stdin and stdout `/dev/null` (a child's stdout never reaches `daemon.log`: `omfp desktop` prints the daemon's URL
/// there, DMN-181), stderr the caller's; reaped by a thread while the caller lives. Also the new windows of a window
/// (UI-282).
pub(crate) fn spawn_detached(command: &mut Command, name: &str, from: &str) {
    match command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .process_group(0)
        .spawn()
    {
        Ok(mut child) => {
            // Reaped while the caller lives.
            std::thread::spawn(move || child.wait());
        }
        Err(error) => note!("{from}: cannot start {name}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use omfp_protocol::{CreatedBy, Provenance, TitleSource};

    use super::*;

    pub(super) fn thread(id: &str, title: &str, process: ProcessState) -> ThreadSummary {
        ThreadSummary {
            id: id.into(),
            title: title.into(),
            title_source: TitleSource::Set,
            workspace_id: "w".into(),
            process,
            settled: true,
            needs_attention: false,
            restart_pending: Vec::new(),
            provenance: Provenance {
                created_by: CreatedBy::User,
                client: None,
            },
            created_at: 1,
            updated_at: 1,
            archived_at: None,
            position: 0,
            forked_from: None,
        }
    }

    pub(super) fn working(id: &str) -> ThreadSummary {
        ThreadSummary {
            settled: false,
            ..thread(id, id, ProcessState::Running)
        }
    }

    pub(super) fn waiting(id: &str, title: &str, updated_at: i64) -> ThreadSummary {
        ThreadSummary {
            needs_attention: true,
            settled: false,
            updated_at,
            ..thread(id, title, ProcessState::Running)
        }
    }

    // spec: UI-217, UI-219
    #[test]
    fn the_status_counts_threads_like_the_left_column() {
        assert_eq!(
            TrayStatus::of(&[]).summary(),
            "0 threads · 0 running · 0 need you"
        );
        assert_eq!(TrayStatus::of(&[]).icon(), TrayIcon::Idle);
        assert_eq!(TrayStatus::of(&[]).quit_label(), "Quit daemon");

        let idle = thread("idle", "idle", ProcessState::Running);
        let stopped = thread("stopped", "stopped", ProcessState::Stopped);
        let starting = thread("starting", "starting", ProcessState::Starting);
        let archived = ThreadSummary {
            archived_at: Some(5),
            ..working("archived")
        };
        let status = TrayStatus::of(&[idle.clone(), stopped.clone(), archived.clone()]);
        assert_eq!(status.summary(), "2 threads · 0 running · 0 need you");
        assert_eq!(status.icon(), TrayIcon::Idle);

        let status = TrayStatus::of(&[idle.clone(), working("a"), starting, stopped.clone()]);
        assert_eq!(status.summary(), "4 threads · 2 running · 0 need you");
        assert_eq!(status.icon(), TrayIcon::Running);
        assert_eq!(status.quit_label(), "Quit daemon (stops 2 running threads)");

        // Waiting wins over working, the newest first; a waiting thread is not counted as running.
        let status = TrayStatus::of(&[
            idle,
            working("a"),
            waiting("old", "old one", 10),
            waiting("new", "new one", 20),
            ThreadSummary {
                archived_at: Some(1),
                ..waiting("gone", "archived", 30)
            },
        ]);
        assert_eq!(status.summary(), "4 threads · 1 running · 2 need you");
        assert_eq!(status.icon(), TrayIcon::NeedsYou);
        assert_eq!(
            status.need_you,
            [
                NeedsYou {
                    id: "new".into(),
                    title: "new one".into()
                },
                NeedsYou {
                    id: "old".into(),
                    title: "old one".into()
                }
            ]
        );
        assert_eq!(status.quit_label(), "Quit daemon (stops 3 running threads)");

        let one = TrayStatus::of(&[waiting("w", "w", 1)]);
        assert_eq!(one.summary(), "1 thread · 0 running · 1 needs you");
        assert_eq!(one.quit_label(), "Quit daemon (stops 1 running thread)");
    }

    fn labels(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| {
                if item.separator {
                    "---".to_owned()
                } else if item.enabled {
                    item.label.clone()
                } else {
                    format!("({})", item.label)
                }
            })
            .collect()
    }

    // spec: UI-218, DMN-130
    #[test]
    fn the_menu_lists_the_actions_the_status_and_the_waiting_threads() {
        let mut ids = ThreadIds::default();
        let status = TrayStatus::of(&[working("a")]);
        let items = menu(&status, &mut ids);
        assert_eq!(
            labels(&items),
            [
                "New window",
                "Open in browser",
                "---",
                "(1 thread · 1 running · 0 need you)",
                "---",
                "Quit daemon (stops 1 running thread)"
            ]
        );
        let actions: Vec<_> = items
            .iter()
            .filter_map(|item| item.action.clone())
            .collect();
        assert_eq!(
            actions,
            [Action::NewWindow, Action::OpenInBrowser, Action::Quit]
        );

        let long = "x".repeat(70);
        let status = TrayStatus::of(&[
            waiting("t1", "fix_the_login", 2),
            waiting("t2", "", 1),
            waiting("t3", &long, 0),
        ]);
        let items = menu(&status, &mut ids);
        assert_eq!(
            labels(&items)[4..7],
            [
                "◆ fix__the__login".to_owned(),
                "◆ New thread".to_owned(),
                format!("◆ {}…", "x".repeat(60)),
            ]
        );
        let t2 = items
            .iter()
            .find(|item| item.action == Some(Action::OpenThread("t2".into())))
            .unwrap()
            .id;

        // A thread keeps its id while it waits; a new one gets a fresh id, never a former one's.
        let items = menu(
            &TrayStatus::of(&[waiting("t2", "", 1), waiting("t4", "four", 0)]),
            &mut ids,
        );
        let id_of = |thread: &str| {
            items
                .iter()
                .find(|item| item.action == Some(Action::OpenThread(thread.into())))
                .map(|item| item.id)
        };
        assert_eq!(id_of("t2"), Some(t2));
        assert!(id_of("t4").unwrap() > t2 + 1);
        let all: Vec<i32> = items.iter().map(|item| item.id).collect();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len());
    }

    // spec: UI-218
    #[test]
    fn a_window_command_names_the_data_directory_and_the_thread() {
        let strings = |line: Vec<OsString>| {
            line.into_iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            strings(window_command(Path::new("/d"), None)),
            ["desktop", "--data-dir", "/d"]
        );
        assert_eq!(
            strings(window_command(Path::new("/d"), Some("t1"))),
            ["desktop", "--data-dir", "/d", "--thread", "t1"]
        );
    }
}
