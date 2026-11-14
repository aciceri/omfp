//! `omfp desktop` (M7, spike S-04): one Tauri window per process (D-16) on the local daemon of the data directory,
//! found with [`omfp_daemon::discover`] or started in the background (ARC-003), or with `--connect` on another
//! daemon. The window loads the daemon's own UI over http: no bundled assets, no Tauri IPC, no capabilities.

use std::ffi::OsString;
use std::io::IsTerminal as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use omfp_daemon::paths::DataDir;
use omfp_daemon::theme::default_theme_file;
use omfp_daemon::watch::{self, WatchedFile};
use omfp_protocol::Theme;
use omfp_protocol::method::ThemeSet;
use tauri::webview::{
    NewWindowResponse, PageLoadEvent, PageLoadPayload, PermissionKind, PermissionResponse,
};
use tauri::{Manager as _, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::remote::{
    Connection, Target, data_dir_or_default, reachable, read_token, websocket_url,
};

mod local;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(target_os = "linux")]
mod portal;

pub use local::{LocalDaemon, fallback_bind};
#[cfg(target_os = "linux")]
pub use portal::{portal_theme, portal_value, setting_changed};

/// `omfp desktop`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Opens the window on this daemon (e.g. `https://host:7420`) instead of the local one; the theme file of this
    /// machine is then pushed to it (UI-091, needs --token).
    #[arg(long, value_name = "URL")]
    pub connect: Option<String>,
    /// Access token (DMN-003) [default: the data directory's token file].
    #[arg(long)]
    pub token: Option<String>,
    /// Data directory of the local daemon; also holds the window's web storage (`webview/`) [default: the daemon's,
    /// `$XDG_DATA_HOME/omfp`, else `~/.local/share/omfp`, on macOS `~/Library/Application Support/omfp`].
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Selects this thread in the new window (the tray's waiting threads, UI-218).
    #[arg(long, value_name = "ID")]
    pub thread: Option<String>,
    /// Arguments for `omfp daemon` when the local daemon has to be started, e.g. `-- --static-dir ui/dist`.
    #[arg(last = true, value_name = "DAEMON ARGS")]
    pub daemon_args: Vec<String>,
}

/// The process environment the defaults depend on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Env {
    fn current() -> Self {
        let var = |name| std::env::var_os(name).map(PathBuf::from);
        Self {
            xdg_data_home: var("XDG_DATA_HOME"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            home: var("HOME"),
        }
    }
}

/// `OMFP_SMOKE` (TST-033): the window closes itself once the shell is connected and rendered (exit 0), or the
/// process exits 1 after [`SMOKE_DEADLINE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Smoke {
    /// `OMFP_SMOKE=1`: on an in-process daemon (DMN-004) that stops with the window, so nothing outlives the test.
    InProcess,
    /// `OMFP_SMOKE=local`: on the local daemon, found or started in the background as without the variable.
    Local,
}

impl Smoke {
    /// Reads `OMFP_SMOKE`; unset means a normal window.
    pub fn parse(value: Option<&str>) -> anyhow::Result<Option<Self>> {
        match value {
            None => Ok(None),
            Some("1") => Ok(Some(Self::InProcess)),
            Some("local") => Ok(Some(Self::Local)),
            Some(other) => bail!("OMFP_SMOKE must be `1` or `local`, not `{other}`"),
        }
    }
}

/// How long the smoke check waits for the shell.
#[cfg(not(target_os = "macos"))]
pub const SMOKE_DEADLINE: Duration = Duration::from_secs(60);
/// How long the smoke check waits for the shell: longer on macOS (TST-090), where `WKWebView` throttles a window on a
/// locked screen (about 30 s to get ready).
#[cfg(target_os = "macos")]
pub const SMOKE_DEADLINE: Duration = Duration::from_secs(180);

/// Runs `omfp desktop`; returns the process exit code.
pub fn run(args: &Args) -> anyhow::Result<i32> {
    let smoke = Smoke::parse(std::env::var("OMFP_SMOKE").ok().as_deref())?;
    let env = Env::current();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut in_process = None;
    let (window_url, browser, data_dir) = if let Some(connect) = &args.connect {
        if !args.daemon_args.is_empty() {
            bail!("daemon arguments after `--` need the local daemon; drop them or --connect");
        }
        let base = connect_base(connect)?;
        let data_dir = data_dir_or_default(
            args.data_dir.clone(),
            env.xdg_data_home.clone(),
            env.home.as_deref(),
        )?;
        match theme_push(&base, args.token.as_deref(), &env) {
            ThemePush::Push { target, file } => {
                runtime.spawn(push_theme(target, file, watch::POLL));
            }
            ThemePush::NeedsToken => eprintln!(
                "omfp desktop: the theme file of this machine is not pushed to {base}: pass --token"
            ),
            ThemePush::NoThemeFile => {}
        }
        (
            window_url(&base, args.token.as_deref(), false, args.thread.as_deref()),
            BrowserUrl {
                base,
                token: args.token.clone(),
                token_file: None,
            },
            data_dir,
        )
    } else {
        let plan = LocalDaemon::plan(&args.data_dir, &args.daemon_args, &env)?;
        let (addr, token) = if smoke == Some(Smoke::InProcess) {
            let handle = runtime.block_on(omfp_daemon::start(plan.in_process_config()?))?;
            let started = (handle.addr(), handle.token().expose().to_owned());
            in_process = Some(handle);
            started
        } else {
            let addr = plan.find_or_start()?;
            let token = match &args.token {
                Some(token) => token.clone(),
                None => read_token(&plan.data_dir)?,
            };
            (addr, token)
        };
        let base = local_base(addr)?;
        (
            window_url(&base, Some(&token), true, args.thread.as_deref()),
            BrowserUrl {
                base,
                token: Some(token),
                token_file: Some(DataDir::new(&plan.data_dir).token()),
            },
            plan.data_dir,
        )
    };
    // UI-004: a browser can attach to the same daemon.
    println!("{}", browser.line(std::io::stdout().is_terminal()));
    let new_window = new_window_command(args, &data_dir);
    let code = open_window(window_url, &data_dir.join("webview"), smoke, new_window)?;
    if let Some(handle) = in_process {
        runtime.block_on(handle.shutdown())?;
    }
    Ok(code)
}

/// The base URL of `--connect`: `http(s)://host[:port][/path]`; a URL without a scheme is `http://`.
pub fn connect_base(connect: &str) -> anyhow::Result<Url> {
    let with_scheme = if connect.contains("://") {
        connect.to_owned()
    } else {
        format!("http://{connect}")
    };
    let mut url =
        Url::parse(&with_scheme).with_context(|| format!("--connect: invalid URL `{connect}`"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("--connect: the daemon's UI is an http:// or https:// URL, not `{connect}`");
    }
    url.set_fragment(None);
    url.set_query(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// The base URL of the local daemon at `addr` (a daemon on every interface is reached on loopback).
fn local_base(addr: SocketAddr) -> anyhow::Result<Url> {
    Ok(Url::parse(&format!("http://{}/", reachable(addr)))?)
}

/// The URL a window opens: the token in the fragment (never sent to the server, PRT-002), `local=1` when the window
/// runs on the daemon's machine (the app then uses omp's `live_start` for live voice, S-09), and `thread=<id>` to
/// select a thread (UI-218).
#[must_use]
pub fn window_url(base: &Url, token: Option<&str>, local: bool, thread: Option<&str>) -> Url {
    let parameters: Vec<String> = token
        .map(|token| format!("token={token}"))
        .into_iter()
        .chain(local.then(|| "local=1".to_owned()))
        .chain(thread.map(|thread| format!("thread={}", fragment_value(thread))))
        .collect();
    let mut url = base.clone();
    url.set_fragment(
        (!parameters.is_empty())
            .then(|| parameters.join("&"))
            .as_deref(),
    );
    url
}

/// A fragment parameter's value with `%`, `&`, `#` and `=` escaped (thread ids are plain, but they come from argv).
fn fragment_value(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('&', "%26")
        .replace('#', "%23")
        .replace('=', "%3D")
}

/// The daemon's URL `omfp desktop` prints for a browser (UI-004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserUrl {
    pub base: Url,
    pub token: Option<String>,
    /// Where the token is, for the line printed without it.
    pub token_file: Option<PathBuf>,
}

impl BrowserUrl {
    /// The printed line (DMN-181): the URL with the token only to a terminal; to anything else (the daemon's log when
    /// the tray starts the window, a pipe) the URL without it and where the token is.
    #[must_use]
    pub fn line(&self, terminal: bool) -> String {
        if terminal || self.token.is_none() {
            return format!(
                "omfp desktop: {}",
                window_url(&self.base, self.token.as_deref(), false, None)
            );
        }
        let from = self
            .token_file
            .as_ref()
            .map(|file| format!(" from {}", file.display()))
            .unwrap_or_default();
        format!("omfp desktop: {} (add #token=<token>{from})", self.base)
    }
}

/// What the window does with a navigation of its page or one of its frames (DMN-182).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    /// The daemon's own origin, or a document without one of its own (`about:`, `blob:`, `data:`).
    Allow,
    /// Another web page or a mail link: a web page loads in a frame (a web frame, UI-677) but the top-level page turns
    /// back from it (UI-678); a new-window request for either opens in the system's browser or mail client.
    External,
    /// Anything else (`file:`, other schemes): cancelled.
    Deny,
}

/// Sorts a navigation of the window on the daemon at `origin` (its serialized origin, `http://127.0.0.1:7420`; DMN-182):
/// the window stays on the daemon's UI.
#[must_use]
pub fn navigation(url: &Url, origin: &str) -> Navigation {
    match url.scheme() {
        "http" | "https" if url.origin().ascii_serialization() == origin => Navigation::Allow,
        "http" | "https" | "mailto" => Navigation::External,
        "about" | "blob" | "data" => Navigation::Allow,
        _ => Navigation::Deny,
    }
}

/// What the window does once its top-level page starts loading a URL (UI-678).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageStart {
    /// A page of the daemon's origin or a document without an origin of its own: it loads.
    Stay,
    /// Another page: the window goes back to the daemon's page it showed last, and `browser`, a web page, opens in the
    /// system's browser.
    Return { browser: Option<Url> },
}

/// Sorts the start of a top-level page load of the window on the daemon at `origin` (UI-678). Neither `WebKitGTK` nor
/// `WKWebView` tells the navigation handler whether a frame or the page navigates, so web pages pass it for the web
/// frames (UI-677); the page loads report the top-level page only, so the window turns back there instead.
#[must_use]
pub fn page_start(url: &Url, origin: &str) -> PageStart {
    match navigation(url, origin) {
        Navigation::Allow => PageStart::Stay,
        Navigation::External => PageStart::Return {
            browser: Some(url.clone()),
        },
        Navigation::Deny => PageStart::Return { browser: None },
    }
}

/// The answer to a permission request of the page at `page` in the window on the daemon at `origin` (DMN-182): the
/// UI's notifications, and on macOS the microphone (live voice, UI-251), only for the daemon's own origin; nothing for
/// another page.
#[must_use]
pub fn permission(kind: PermissionKind, page: &Url, origin: &str) -> PermissionResponse {
    if page.origin().ascii_serialization() != origin {
        return PermissionResponse::Deny;
    }
    match kind {
        PermissionKind::Notifications => PermissionResponse::Allow,
        #[cfg(target_os = "macos")]
        PermissionKind::Microphone => PermissionResponse::Allow,
        _ => PermissionResponse::Default,
    }
}

/// Whether and what to push of the theme file of this machine (UI-091). A local daemon follows that file itself
/// (DMN-075); a daemon reached with `--connect` cannot see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemePush {
    /// Follow `file` and send it to `target` with `theme.set`.
    Push { target: Target, file: PathBuf },
    /// `theme.set` needs the daemon's token.
    NeedsToken,
    /// Neither `XDG_CONFIG_HOME` nor `HOME` is set.
    NoThemeFile,
}

/// The theme push of a window opened with `--connect` on `base`.
#[must_use]
pub fn theme_push(base: &Url, token: Option<&str>, env: &Env) -> ThemePush {
    let Some(token) = token else {
        return ThemePush::NeedsToken;
    };
    match default_theme_file(env.xdg_config_home.clone(), env.home.clone()) {
        Some(file) => ThemePush::Push {
            target: Target {
                url: websocket_url(base.as_str()),
                token: token.to_owned(),
            },
            file,
        },
        None => ThemePush::NoThemeFile,
    }
}

/// Follows `file` (checked every `period` once it settled, DMN-091) and sends every change to `target`'s
/// `theme.set`, a removed file as no theme; runs until the process ends. Failures are reported and the next change
/// is tried again.
pub async fn push_theme(target: Target, file: PathBuf, period: Duration) {
    let mut watched = WatchedFile::new(&file);
    let first = watch::first_read(&mut watched);
    let (sender, mut changes) = tokio::sync::mpsc::unbounded_channel();
    if let Some(change) = first {
        // The receiver is alive: it is dropped only when this function returns.
        let _ = sender.send(change);
    }
    let _watcher = watch::watch(watched, period, sender.clone());
    while let Some(css) = changes.recv().await {
        if let Err(error) = set_theme(&target, css).await {
            eprintln!(
                "omfp desktop: cannot push the theme {} to {}: {error:#}",
                file.display(),
                target.url
            );
        }
    }
}

async fn set_theme(target: &Target, css: Option<String>) -> anyhow::Result<()> {
    let mut connection = Connection::open(target).await?;
    connection.call::<ThemeSet>(Theme { css }).await?;
    connection.close().await
}

/// The page state the smoke check polls, a function of `wait` (a CSS selector or `null`): the shell is connected and
/// shows its main area — a view, or the first run's welcome when omp can use no model (UI-500) —, whether `wait` matches
/// an element, and the color scheme the page sees.
const PROBE: &str = r#"((wait) => {
  const q = (s) => document.querySelector(s);
  return {
    ready: !!q("[data-testid=view], [data-testid=welcome]") && !q("[data-testid=connection-banner]"),
    waited: wait === null || !!q(wait),
    scheme: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
    href: location.href,
    rtcPeerConnection: typeof RTCPeerConnection,
    notification: typeof Notification === "function" ? Notification.permission : "undefined",
    mediaDevices: typeof navigator.mediaDevices,
    smoke: document.body.dataset.smoke ?? null,
  };
})"#;

/// Opens the window and runs the event loop until it closes. `new_window` is the command line of one more window
/// (macOS's File menu, UI-282).
#[cfg_attr(
    not(target_os = "macos"),
    expect(
        unused_variables,
        clippy::needless_pass_by_value,
        reason = "macOS's app menu only"
    )
)]
fn open_window(
    url: Url,
    webview_data: &Path,
    smoke: Option<Smoke>,
    new_window: Vec<OsString>,
) -> anyhow::Result<i32> {
    // UI-222: the Dock's and the app switcher's icon, registered before AppKit starts (a folder otherwise).
    #[cfg(target_os = "macos")]
    if !macos::register_app_icon() {
        eprintln!("omfp desktop: cannot set the app icon");
    }
    let builder = tauri::Builder::default();
    // UI-281, UI-282: the app menu with New Window and the zoom items on their macOS keys.
    #[cfg(target_os = "macos")]
    let builder = macos::with_app_menu(builder, new_window);
    let app = builder.build(tauri::generate_context!())?;
    // The window stays on the daemon's UI (DMN-182): its origin is the only one granted anything.
    let origin = url.origin().ascii_serialization();
    // The top-level page, as its loads report it (the permission handler must not call into the webview).
    let page = Arc::new(Mutex::new(url.clone()));
    #[cfg(target_os = "macos")]
    let nonce = macos::notify_nonce();
    let builder = WebviewWindowBuilder::new(&app, "main", WebviewUrl::External(url.clone()))
        .title("omfp")
        .inner_size(1280.0, 800.0)
        .on_page_load(on_page_load(Arc::clone(&page), origin.clone(), url))
        // WebKitGTK denies requests nobody handles: allow the UI's thread notifications. WKWebView answers an unhandled
        // media request with its own prompt on every page load: grant the microphone (live voice, UI-251); macOS still
        // asks the user once per app through TCC (the embedded Info.plist's NSMicrophoneUsageDescription). Only the
        // daemon's page gets them (DMN-182).
        .on_permission_request({
            let (page, origin) = (Arc::clone(&page), origin.clone());
            move |_, kind| {
                let page = page.lock().unwrap_or_else(PoisonError::into_inner);
                permission(kind, &page, &origin)
            }
        })
        .on_navigation({
            let origin = origin.clone();
            #[cfg(target_os = "macos")]
            let nonce = nonce.clone();
            move |url| {
                #[cfg(target_os = "macos")]
                if let Some(allowed) = macos::on_notify(url, &nonce) {
                    return allowed;
                }
                follow(url, &origin)
            }
        })
        // `target="_blank"` links and `window.open`: never a second webview; other pages go to the browser. The same on
        // Linux and macOS: Tauri's handlers, no platform code.
        .on_new_window({
            let origin = origin.clone();
            move |url, _| {
                if opens_externally(&url, &origin) {
                    crate::tray::open_in_browser(url.as_str());
                }
                NewWindowResponse::Deny
            }
        });
    // Per data directory: two daemons on the same port never share the window's storage (UI-005). WebKitGTK takes a
    // directory; WKWebView ignores it and takes a data store identifier instead (UI-250, macOS 14+).
    #[cfg(target_os = "linux")]
    let builder = builder.data_directory(webview_data.to_path_buf());
    // WKWebView has no web notifications: the daemon's page gets a `Notification` whose notifications the window posts
    // to the system's notification center (UI-252).
    #[cfg(target_os = "macos")]
    let builder = builder
        .data_store_identifier(macos::data_store_identifier(webview_data)?)
        .initialization_script(macos::notification_script(&origin, &nonce));
    // UI-512: the system accent the `accent` theme derives from, which the page cannot read itself.
    #[cfg(target_os = "macos")]
    let builder = match macos::system_accent() {
        Some(accent) => builder.initialization_script(macos::accent_script(&origin, &accent)),
        None => builder,
    };
    let window = builder.build()?;
    #[cfg(target_os = "linux")]
    let _color_scheme = portal::follow_color_scheme(&window);
    if smoke.is_some() {
        smoke_check(
            window,
            std::env::var("OMFP_SMOKE_SCHEME").ok(),
            std::env::var("OMFP_SMOKE_WAIT").ok(),
        );
    }
    Ok(app.run_return(|_, _| {}))
}

/// Whether a navigation of the window's page or of one of its frames loads (DMN-182): the daemon's own pages, documents
/// without an origin of their own and other web pages, for the web frames (UI-677) — the top-level page turns back from
/// those once it starts loading one (UI-678, [`page_start`]). Every other navigation is cancelled without a word, a
/// `mailto:` link too: neither `WebKitGTK` nor `WKWebView` tells Tauri whether a frame or the page navigates, and a
/// widget's frame must not open the mail client at every render of every window.
fn follow(url: &Url, origin: &str) -> bool {
    match navigation(url, origin) {
        Navigation::Allow => true,
        Navigation::External => matches!(url.scheme(), "http" | "https"),
        Navigation::Deny => false,
    }
}

/// The window's page-load handler: records the top-level page in `page` (the permission handler reads it) and, when it
/// starts loading a page off the daemon's origin, turns back to the daemon's page it showed last (`home` until then),
/// opening a web page in the system's browser (UI-678).
fn on_page_load(
    page: Arc<Mutex<Url>>,
    origin: String,
    home: Url,
) -> impl Fn(WebviewWindow, PageLoadPayload<'_>) + Send + Sync + 'static {
    let home = Mutex::new(home);
    move |window, payload| {
        let url = payload.url();
        *page.lock().unwrap_or_else(PoisonError::into_inner) = url.clone();
        if payload.event() != PageLoadEvent::Started {
            return;
        }
        let mut home = home.lock().unwrap_or_else(PoisonError::into_inner);
        match page_start(url, &origin) {
            PageStart::Stay if url.origin().ascii_serialization() == origin => *home = url.clone(),
            PageStart::Stay => {}
            PageStart::Return { browser } => {
                if let Some(browser) = browser {
                    crate::tray::open_in_browser(browser.as_str());
                }
                if let Err(error) = window.navigate(home.clone()) {
                    eprintln!("omfp desktop: cannot go back to {home}: {error}");
                }
            }
        }
    }
}

/// Whether a new-window request (`target="_blank"`, `window.open`) goes to the system (DMN-182): an external web page
/// or mail link does; the window never opens a second webview.
fn opens_externally(url: &Url, origin: &str) -> bool {
    navigation(url, origin) == Navigation::External
}

/// A zoom command of the window (UI-281): the View menu's Zoom In, Zoom Out and Actual Size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    In,
    Out,
    Reset,
}

/// The window's page zoom levels (UI-281), the browsers' steps from 50 % to 300 %.
const ZOOM_LEVELS: [f64; 13] = [
    0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

/// The page zoom after `zoom` at `current` (UI-281): the next level up or down, the ends staying, or 100 %.
#[must_use]
pub fn zoomed(current: f64, zoom: Zoom) -> f64 {
    const SAME: f64 = 1e-6;
    match zoom {
        Zoom::In => ZOOM_LEVELS
            .into_iter()
            .find(|level| *level > current + SAME)
            .unwrap_or(current),
        Zoom::Out => ZOOM_LEVELS
            .into_iter()
            .rev()
            .find(|level| *level < current - SAME)
            .unwrap_or(current),
        Zoom::Reset => 1.0,
    }
}

/// The command line of one more window like the one of `args` (UI-282): on the same daemon (`--connect`) or the
/// local one of `data_dir` (found running, so without the daemon arguments), with the same token when one was given;
/// not on the thread of `--thread`. The local case is the menu bar's New window (UI-218).
#[must_use]
pub fn new_window_command(args: &Args, data_dir: &Path) -> Vec<OsString> {
    let mut line = vec![OsString::from("desktop")];
    if let Some(connect) = &args.connect {
        line.extend(["--connect".into(), connect.into()]);
    }
    line.extend(["--data-dir".into(), data_dir.as_os_str().to_owned()]);
    if let Some(token) = &args.token {
        line.extend(["--token".into(), token.into()]);
    }
    line
}

/// Polls [`PROBE`] every 100 ms; once the shell is ready (and the page's color scheme is `scheme`, when given:
/// `OMFP_SMOKE_SCHEME`; and an element matches the CSS selector `wait`, when given: `OMFP_SMOKE_WAIT`) prints
/// `OMFP_SMOKE_READY <state>` and closes the window like the user would (exit 0), else exits 1 after
/// [`SMOKE_DEADLINE`] with the last state.
fn smoke_check(window: WebviewWindow, scheme: Option<String>, wait: Option<String>) {
    let probe = format!("{PROBE}({})", serde_json::Value::from(wait));
    std::thread::spawn(move || {
        let deadline = Instant::now() + SMOKE_DEADLINE;
        let mut last = String::new();
        while Instant::now() < deadline {
            let (sender, receiver) = mpsc::channel();
            if window
                .eval_with_callback(&probe, move |json| {
                    let _ = sender.send(json);
                })
                .is_err()
            {
                break;
            }
            if let Ok(json) = receiver.recv_timeout(Duration::from_secs(2)) {
                let ready = serde_json::from_str::<serde_json::Value>(&json).is_ok_and(|state| {
                    state["ready"] == true
                        && state["waited"] == true
                        && scheme
                            .as_ref()
                            .is_none_or(|scheme| state["scheme"] == **scheme)
                });
                last = json;
                if ready {
                    println!("OMFP_SMOKE_READY {last}");
                    if window.close().is_ok() {
                        return;
                    }
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("omfp desktop: the shell was not ready: {last}");
        window.app_handle().exit(1);
    });
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::Arc;

    use clap::Parser as _;
    use omfp_daemon::agent::FakeLauncher;
    use omfp_daemon::config::{Cli, Config};
    use omfp_daemon::git::FakeGit;
    use omfp_daemon::{Ports, start_with};
    use omfp_protocol::Empty;
    use omfp_protocol::method::ThemeGet;

    use super::*;

    #[derive(Debug, clap::Parser)]
    struct Line {
        #[command(flatten)]
        args: Args,
    }

    fn parse(line: &[&str]) -> Args {
        Line::try_parse_from(std::iter::once("desktop").chain(line.iter().copied()))
            .unwrap()
            .args
    }

    fn env(home: &Path) -> Env {
        Env {
            xdg_data_home: None,
            xdg_config_home: None,
            home: Some(home.to_path_buf()),
        }
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    // spec: ARC-003, DMN-128
    #[test]
    fn arguments_after_the_separator_go_to_the_daemon() {
        let args = parse(&[
            "--data-dir",
            "/d",
            "--",
            "--static-dir",
            "/web",
            "--bind",
            "127.0.0.1:0",
        ]);
        assert_eq!(args.data_dir, Some(PathBuf::from("/d")));
        assert_eq!(
            args.daemon_args,
            ["--static-dir", "/web", "--bind", "127.0.0.1:0"]
        );
        let plan = LocalDaemon::plan(
            &args.data_dir,
            &args.daemon_args,
            &env(Path::new("/home/u")),
        )
        .unwrap();
        assert_eq!(plan.data_dir, PathBuf::from("/d"));
        assert!(plan.explicit_bind);
        assert_eq!(
            strings(&plan.command_line(None)),
            [
                "daemon",
                "--tray",
                "--data-dir",
                "/d",
                "--static-dir",
                "/web",
                "--bind",
                "127.0.0.1:0"
            ]
        );
        assert!(LocalDaemon::plan(&None, &["--no-such-flag".into()], &Env::default()).is_err());
    }

    // spec: ARC-003, DMN-002
    #[test]
    fn the_daemon_is_planned_like_omfp_daemon_resolves_it() {
        let home = tempfile::tempdir().unwrap();
        let plan = LocalDaemon::plan(&None, &[], &env(home.path())).unwrap();
        let home_data = home.path().join(omfp_daemon::config::HOME_DATA);
        assert_eq!(plan.data_dir, home_data.join("omfp"));
        assert_eq!(plan.bind, omfp_daemon::config::DEFAULT_BIND);
        assert!(!plan.explicit_bind);

        // The daemon's data directory may come after `--` instead, and the config file sets the bind address.
        let config = home.path().join(".config/omfp");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.toml"),
            "[daemon]\nbind = \"127.0.0.1:7500\"\n",
        )
        .unwrap();
        let plan = LocalDaemon::plan(
            &None,
            &["--data-dir".into(), "/other".into()],
            &env(home.path()),
        )
        .unwrap();
        assert_eq!(plan.data_dir, PathBuf::from("/other"));
        assert_eq!(plan.bind, "127.0.0.1:7500".parse().unwrap());
        assert!(!plan.explicit_bind);
        assert_eq!(
            strings(&plan.command_line(Some("127.0.0.1:0".parse().unwrap()))),
            [
                "daemon",
                "--tray",
                "--bind",
                "127.0.0.1:0",
                "--data-dir",
                "/other"
            ]
        );

        let twice = LocalDaemon::plan(
            &Some("/d".into()),
            &["--data-dir".into(), "/e".into()],
            &env(home.path()),
        );
        assert!(twice.unwrap_err().to_string().contains("twice"));
    }

    // spec: ARC-003, UI-004
    #[test]
    fn window_urls_carry_the_token_and_the_local_flag_in_the_fragment() {
        let local = local_base("127.0.0.1:7420".parse().unwrap()).unwrap();
        assert_eq!(
            window_url(&local, Some("t0k"), true, None).as_str(),
            "http://127.0.0.1:7420/#token=t0k&local=1"
        );
        assert_eq!(
            window_url(&local, Some("t0k"), false, None).as_str(),
            "http://127.0.0.1:7420/#token=t0k"
        );
        assert_eq!(
            local_base("0.0.0.0:7420".parse().unwrap())
                .unwrap()
                .as_str(),
            "http://127.0.0.1:7420/"
        );

        for (given, expected) in [
            ("https://omfp.example.com", "https://omfp.example.com/"),
            ("http://host:7420/", "http://host:7420/"),
            ("host:7420", "http://host:7420/"),
            (
                "https://example.com/proxy?x=1#token=old",
                "https://example.com/proxy/",
            ),
        ] {
            assert_eq!(connect_base(given).unwrap().as_str(), expected, "{given}");
        }
        let remote = connect_base("https://omfp.example.com").unwrap();
        assert_eq!(
            window_url(&remote, Some("t0k"), false, None).as_str(),
            "https://omfp.example.com/#token=t0k"
        );
        assert_eq!(
            window_url(&remote, None, false, None).as_str(),
            "https://omfp.example.com/"
        );
        assert!(connect_base("ws://host:7420").is_err());
        assert!(connect_base("http://").is_err());
    }

    // spec: UI-218
    #[test]
    fn a_window_on_a_thread_names_it_in_the_fragment() {
        let local = local_base("127.0.0.1:7420".parse().unwrap()).unwrap();
        assert_eq!(
            window_url(&local, Some("t0k"), true, Some("th-1")).as_str(),
            "http://127.0.0.1:7420/#token=t0k&local=1&thread=th-1"
        );
        assert_eq!(
            window_url(&local, None, false, Some("a&b=c#d%")).as_str(),
            "http://127.0.0.1:7420/#thread=a%26b%3Dc%23d%25"
        );
        let args = parse(&["--thread", "th-1", "--data-dir", "/d"]);
        assert_eq!(args.thread.as_deref(), Some("th-1"));
    }

    // spec: DMN-181, UI-004
    #[test]
    fn the_printed_url_carries_the_token_only_to_a_terminal() {
        let browser = BrowserUrl {
            base: local_base("127.0.0.1:7420".parse().unwrap()).unwrap(),
            token: Some("t0k".into()),
            token_file: Some(PathBuf::from("/data/omfp/token")),
        };
        assert_eq!(
            browser.line(true),
            "omfp desktop: http://127.0.0.1:7420/#token=t0k"
        );
        assert_eq!(
            browser.line(false),
            "omfp desktop: http://127.0.0.1:7420/ (add #token=<token> from /data/omfp/token)"
        );
        let remote = BrowserUrl {
            base: connect_base("https://omfp.example.com").unwrap(),
            token: Some("t0k".into()),
            token_file: None,
        };
        assert_eq!(
            remote.line(false),
            "omfp desktop: https://omfp.example.com/ (add #token=<token>)"
        );
        let without = BrowserUrl {
            token: None,
            ..remote
        };
        assert_eq!(
            without.line(false),
            "omfp desktop: https://omfp.example.com/"
        );
    }

    // spec: DMN-182, UI-677
    #[test]
    fn the_window_stays_on_the_daemons_origin() {
        let origin = "http://127.0.0.1:7420";
        let url = |text: &str| Url::parse(text).unwrap();
        for (target, expected, loads) in [
            (
                "http://127.0.0.1:7420/#token=t&local=1",
                Navigation::Allow,
                true,
            ),
            ("http://127.0.0.1:7420/widgets/k/x", Navigation::Allow, true),
            ("about:blank", Navigation::Allow, true),
            ("blob:http://127.0.0.1:7420/1234", Navigation::Allow, true),
            // Web frames (UI-677): other web pages load in a frame; the page turns back from them (UI-678).
            ("https://example.com/docs", Navigation::External, true),
            ("http://127.0.0.1:7421/", Navigation::External, true),
            ("http://localhost:7420/", Navigation::External, true),
            ("mailto:someone@example.com", Navigation::External, false),
            ("file:///etc/passwd", Navigation::Deny, false),
            ("omfp-notify:?title=x", Navigation::Deny, false),
        ] {
            assert_eq!(navigation(&url(target), origin), expected, "{target}");
            // Navigations of the page or a frame never reach the system; new windows do when they are external.
            assert_eq!(follow(&url(target), origin), loads, "{target}");
            assert_eq!(
                opens_externally(&url(target), origin),
                expected == Navigation::External,
                "{target}"
            );
        }
    }

    // spec: UI-678
    #[test]
    fn the_top_level_page_turns_back_from_another_origin_and_the_browser_opens_it() {
        let origin = "http://127.0.0.1:7420";
        let url = |text: &str| Url::parse(text).unwrap();
        for page in [
            "http://127.0.0.1:7420/#token=t&local=1",
            "http://127.0.0.1:7420/?thread=t1",
            "about:blank",
        ] {
            assert_eq!(page_start(&url(page), origin), PageStart::Stay, "{page}");
        }
        for page in [
            "https://example.com/docs",
            "http://localhost:5173/",
            "http://127.0.0.1:7421/",
        ] {
            assert_eq!(
                page_start(&url(page), origin),
                PageStart::Return {
                    browser: Some(url(page))
                },
                "{page}"
            );
        }
        assert_eq!(
            page_start(&url("file:///etc/passwd"), origin),
            PageStart::Return { browser: None }
        );
    }

    // spec: DMN-182, UI-251
    #[test]
    fn only_the_daemons_page_gets_permissions() {
        let origin = "http://127.0.0.1:7420";
        let daemon = Url::parse("http://127.0.0.1:7420/#token=t").unwrap();
        let other = Url::parse("https://example.com/").unwrap();
        assert_eq!(
            permission(PermissionKind::Notifications, &daemon, origin),
            PermissionResponse::Allow
        );
        assert_eq!(
            permission(PermissionKind::Geolocation, &daemon, origin),
            PermissionResponse::Default
        );
        for kind in [
            PermissionKind::Notifications,
            PermissionKind::Microphone,
            PermissionKind::Camera,
        ] {
            assert_eq!(
                permission(kind, &other, origin),
                PermissionResponse::Deny,
                "{kind:?}"
            );
        }
        #[cfg(target_os = "macos")]
        assert_eq!(
            permission(PermissionKind::Microphone, &daemon, origin),
            PermissionResponse::Allow
        );
    }

    // spec: TST-033
    #[test]
    fn the_smoke_variable_picks_the_daemon() {
        assert_eq!(Smoke::parse(None).unwrap(), None);
        assert_eq!(Smoke::parse(Some("1")).unwrap(), Some(Smoke::InProcess));
        assert_eq!(Smoke::parse(Some("local")).unwrap(), Some(Smoke::Local));
        assert!(Smoke::parse(Some("yes")).is_err());
    }

    // spec: UI-091
    #[test]
    fn only_a_connected_window_with_a_token_pushes_the_theme_file() {
        let base = connect_base("https://omfp.example.com").unwrap();
        let env = Env {
            xdg_config_home: Some("/xdg".into()),
            ..env(Path::new("/home/u"))
        };
        assert_eq!(
            theme_push(&base, Some("t0k"), &env),
            ThemePush::Push {
                target: Target {
                    url: "wss://omfp.example.com/api/ws".into(),
                    token: "t0k".into()
                },
                file: "/xdg/omfp/theme.css".into(),
            }
        );
        assert_eq!(theme_push(&base, None, &env), ThemePush::NeedsToken);
        assert_eq!(
            theme_push(&base, Some("t0k"), &Env::default()),
            ThemePush::NoThemeFile
        );
    }

    // spec: UI-091
    #[tokio::test]
    async fn the_theme_file_follows_to_the_connected_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "daemon",
            "--bind",
            "127.0.0.1:0",
            "--data-dir",
            dir.path().join("data").to_str().unwrap(),
            "--workspaces-root",
            dir.path().join("workspaces").to_str().unwrap(),
        ])
        .unwrap();
        let ports = Ports {
            launcher: Arc::new(FakeLauncher::new()),
            git: Arc::new(FakeGit::new()),
            omp_program: "omp".into(),
            omp_version: omfp_core::OMP_VERSION.into(),
            omp_env: Vec::new(),
            warnings: Vec::new(),
        };
        let daemon = start_with(Config::from_cli(cli, None, None).unwrap(), ports)
            .await
            .unwrap();
        let target = Target {
            url: format!("ws://{}/api/ws", daemon.addr()),
            token: daemon.token().expose().to_owned(),
        };
        let file = dir.path().join("theme.css");
        std::fs::write(&file, ":root { --omfp-primary: #123456; }").unwrap();
        let pusher = tokio::spawn(push_theme(
            target.clone(),
            file.clone(),
            Duration::from_millis(20),
        ));

        let theme = |css: Option<&'static str>| {
            let target = target.clone();
            async move {
                loop {
                    let mut connection = Connection::open(&target).await.unwrap();
                    let theme = connection.call::<ThemeGet>(Empty {}).await.unwrap();
                    if theme.css.as_deref() == css {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        };
        let within = |future| tokio::time::timeout(Duration::from_secs(10), future);
        within(theme(Some(":root { --omfp-primary: #123456; }")))
            .await
            .unwrap();
        std::fs::write(&file, ":root { --omfp-primary: #654321; }").unwrap();
        within(theme(Some(":root { --omfp-primary: #654321; }")))
            .await
            .unwrap();
        std::fs::remove_file(&file).unwrap();
        within(theme(None)).await.unwrap();

        pusher.abort();
        daemon.shutdown().await.unwrap();
    }

    // spec: UI-281
    #[test]
    fn zoom_steps_through_the_browsers_levels() {
        assert_eq!(zoomed(1.0, Zoom::In), 1.1);
        assert_eq!(zoomed(1.0, Zoom::Out), 0.9);
        assert_eq!(zoomed(2.5, Zoom::Reset), 1.0);
        // A level between two steps goes to the next one either way.
        assert_eq!(zoomed(1.2, Zoom::In), 1.25);
        assert_eq!(zoomed(1.2, Zoom::Out), 1.1);
        // The ends stay.
        assert_eq!(zoomed(3.0, Zoom::In), 3.0);
        assert_eq!(zoomed(0.5, Zoom::Out), 0.5);
        let mut level = 1.0;
        let mut ins = Vec::new();
        for _ in 0..8 {
            level = zoomed(level, Zoom::In);
            ins.push(level);
        }
        assert_eq!(ins, [1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 3.0]);
    }

    // spec: UI-282
    #[test]
    fn a_new_window_opens_on_the_same_daemon() {
        let dir = Path::new("/data");
        assert_eq!(
            strings(&new_window_command(&parse(&["--thread", "t1"]), dir)),
            ["desktop", "--data-dir", "/data"]
        );
        // The daemon arguments of a daemon already running are not needed again.
        assert_eq!(
            strings(&new_window_command(
                &parse(&["--token", "s", "--", "--bind", "127.0.0.1:0"]),
                dir
            )),
            ["desktop", "--data-dir", "/data", "--token", "s"]
        );
        assert_eq!(
            strings(&new_window_command(
                &parse(&["--connect", "https://host:7420", "--token", "s"]),
                dir
            )),
            [
                "desktop",
                "--connect",
                "https://host:7420",
                "--data-dir",
                "/data",
                "--token",
                "s"
            ]
        );
    }
}
