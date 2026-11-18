//! Desktop tests (M7, M11) of the real `omfp desktop` window. On Linux under a virtual X server, never the user's
//! display: they need `xvfb-run`, and `just test-desktop` runs them one at a time (each `xvfb-run -a` picks a free
//! display, which races when two start at once). On macOS (TST-090) the window opens on the logged-in user's screen,
//! from an SSH session too, and closes itself once ready. Both need the release web bundle (`just build-web`).
#![cfg(feature = "desktop")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rustix::process::{Pid, Signal, kill_process};

/// The web bundle built by `just build-web` (`ui/packages/app/dist`), or `OMFP_WEB_BUNDLE` (`just test-desktop` builds a
/// private one, which no other build can replace while the tests run).
fn web_bundle() -> PathBuf {
    std::env::var_os("OMFP_WEB_BUNDLE").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/packages/app/dist"),
        PathBuf::from,
    )
}

/// `omfp desktop <args>` with `OMFP_SMOKE=<smoke>` and a temporary home.
fn desktop(home: &Path, smoke: &str, args: &[&str]) -> Output {
    desktop_with(home, &[("OMFP_SMOKE", smoke)], args)
}

/// `omfp desktop <args>` (under `xvfb-run` on Linux) with `env` and a temporary home; it must print
/// `OMFP_SMOKE_READY`.
fn desktop_with(home: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    desktop_with_store(home, env, args, false)
}

/// Keeps the store between launches when a test checks restoration.
fn desktop_with_store(
    home: &Path,
    env: &[(&str, &str)],
    args: &[&str],
    keep_store: bool,
) -> Output {
    #[cfg(target_os = "linux")]
    let _ = keep_store;
    #[cfg(target_os = "linux")]
    let mut command = {
        // A wedged WebKit main loop cannot process the smoke deadline or close request.
        let mut command = Command::new("timeout");
        command
            .args([
                "--kill-after=5s",
                "90s",
                "xvfb-run",
                "-a",
                "-s",
                "-screen 0 1280x800x24",
            ])
            .arg(env!("CARGO_BIN_EXE_omfp"))
            // Never the user's session bus (the window reads the settings portal, the background daemon shows a tray
            // icon): a bus that does not exist unless the test passes its own.
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", home.join("no-bus").display()),
            )
            // Never the user's display.
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("GDK_BACKEND", "x11");
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new(env!("CARGO_BIN_EXE_omfp"));
    let output = command
        .arg("desktop")
        .args(args)
        .envs(env.iter().copied())
        .env("HOME", home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap();
    #[cfg(target_os = "macos")]
    if !keep_store
        && let Some(data) = args
            .iter()
            .position(|arg| *arg == "--data-dir")
            .and_then(|at| args.get(at + 1))
    {
        forget_web_store(Path::new(data));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("OMFP_SMOKE_READY"),
        "omfp desktop failed ({}):\n{stdout}\n{stderr}",
        output.status
    );
    output
}

/// Deletes the window's `WKWebsiteDataStore` of the data directory `data` (UI-250): `WebKit` keeps it outside the test's
/// temporary directory, whatever the window's `HOME`, in the user's
/// `~/Library/WebKit/dev.omfp.desktop/WebsiteDataStore/<the store id as a UUID>` (about 0.7 MB per window).
#[cfg(target_os = "macos")]
fn forget_web_store(data: &Path) {
    use std::fmt::Write as _;
    let (Ok(id), Some(home)) = (
        std::fs::read(data.join("webview/store-id")),
        std::env::var_os("HOME"),
    ) else {
        return;
    };
    let hex = id.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    if hex.len() != 32 {
        return;
    }
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    );
    let _ = std::fs::remove_dir_all(
        Path::new(&home)
            .join("Library/WebKit/dev.omfp.desktop/WebsiteDataStore")
            .join(uuid),
    );
}

#[cfg(target_os = "macos")]
struct RetainedWebStore(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for RetainedWebStore {
    fn drop(&mut self) {
        forget_web_store(&self.0);
    }
}

// spec: TST-033, TST-090, DMN-004
#[test]
fn the_desktop_window_renders_the_shell() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspaces = dir.path().join("workspaces");
    desktop(
        dir.path(),
        "1",
        &[
            "--data-dir",
            data.to_str().unwrap(),
            "--",
            "--static-dir",
            web_bundle().to_str().unwrap(),
            "--workspaces-root",
            workspaces.to_str().unwrap(),
        ],
    );
    // The in-process daemon stopped with the window.
    assert_eq!(omfp_daemon::discover(&data).unwrap(), None);
}

// spec: TST-090, UI-250, UI-251, UI-252, UI-253
#[cfg(target_os = "macos")]
#[test]
fn the_macos_window_has_webrtc_the_microphone_and_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspaces = dir.path().join("workspaces");
    let output = desktop(
        dir.path(),
        "1",
        &[
            "--data-dir",
            data.to_str().unwrap(),
            "--",
            "--static-dir",
            web_bundle().to_str().unwrap(),
            "--workspaces-root",
            workspaces.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let state = stdout
        .lines()
        .find_map(|line| line.strip_prefix("OMFP_SMOKE_READY "))
        .expect("the ready line");
    let state: serde_json::Value = serde_json::from_str(state).unwrap();
    // WKWebView's WebRTC (live voice, UI-251), `getUserMedia` thanks to the embedded Info.plist, the window's
    // `Notification` (UI-252).
    assert_eq!(state["rtcPeerConnection"], "function", "{state}");
    assert_eq!(state["mediaDevices"], "object", "{state}");
    assert_eq!(state["notification"], "granted", "{state}");
    // UI-253: the page's color scheme is the system appearance's (`AppleInterfaceStyle` is set in Dark mode only).
    let dark = Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .unwrap();
    let system = if String::from_utf8_lossy(&dark.stdout).trim() == "Dark" {
        "dark"
    } else {
        "light"
    };
    assert_eq!(state["scheme"], system, "{state}");
    // UI-250: the web storage of the data directory.
    let id = std::fs::read(data.join("webview/store-id")).unwrap();
    assert_eq!(id.len(), 16);
}

/// The page a web frame shows in [`a_web_frame_loads_another_origin_and_the_window_stays_on_the_daemon`]: it tells the
/// window it loaded, then tries to navigate the window away (its sandbox forbids it, UI-672).
const FRAMED_PAGE: &str = r#"<!doctype html><title>Framed</title><h1>Framed page</h1>
<script>
  parent.postMessage("framed page loaded", "*");
  try { top.location.href = "/escaped"; } catch {}
</script>
"#;

/// Serves [`FRAMED_PAGE`] on a loopback port until the test ends; returns its base URL and the request lines seen.
fn serve_framed_page() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead as _, BufReader, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = std::sync::Arc::clone(&requests);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            // The rest of the head.
            let mut header = String::new();
            while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
                header.clear();
            }
            seen.lock().unwrap().push(line.trim_end().to_owned());
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{FRAMED_PAGE}",
                FRAMED_PAGE.len()
            );
        }
    });
    (base, requests)
}

/// A page whose first decoded frame proves the Linux desktop has an H.264 pipeline; an error reports without waiting
/// for the smoke deadline.
#[cfg(target_os = "linux")]
const H264_PAGE: &str = r#"<!doctype html><title>H.264</title>
<video muted preload="auto" src="/h264.mp4"></video>
<script>
  const video = document.querySelector("video");
  const report = (state) => parent.postMessage(`h264 ${state}`, "*");
  video.addEventListener("loadeddata", () =>
    report(video.videoWidth === 16 && video.videoHeight === 16 ? "loaded" : "wrong-size"));
  video.addEventListener("error", () => report(`error-${video.error?.code ?? 0}`));
  video.load();
</script>
"#;

#[cfg(target_os = "linux")]
const H264: &[u8] = include_bytes!("fixtures/h264.mp4");

/// Serves [`H264_PAGE`] and its tiny H.264 video on a loopback port until the test process ends.
#[cfg(target_os = "linux")]
fn serve_h264_page() -> String {
    use std::io::{BufRead as _, BufReader, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut header = String::new();
            while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
                header.clear();
            }
            let (content_type, body): (&str, &[u8]) = if line.starts_with("GET /h264.mp4 ") {
                ("video/mp4", H264)
            } else {
                ("text/html; charset=utf-8", H264_PAGE.as_bytes())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    base
}

// spec: TST-033, UI-677, UI-670, UI-672
#[test]
fn a_web_frame_loads_another_origin_and_the_window_stays_on_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspaces = dir.path().join("workspaces");
    let (base, requests) = serve_framed_page();
    // The user's init file makes the default view `omfp:web` on the page (there is no workspace, so no panel) and
    // records the page's message.
    let config = dir.path().join(".config/omfp");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("init.ts"),
        format!(
            r#"export default function init(omfp) {{
  const loaded = (event) => {{
    if (event.data === "framed page loaded") document.body.dataset.framedPage = event.origin;
  }};
  window.addEventListener("message", loaded);
  omfp.views.transform((views) =>
    views.update("default", () => ["row", {{}}, ["omfp:web", {{ url: "{base}/page", title: "Framed" }}]]),
  );
  return () => window.removeEventListener("message", loaded);
}}
"#
        ),
    )
    .unwrap();
    // The background daemon (`OMFP_SMOKE=local`) follows the user's init file, the in-process one does not.
    let _daemon = BackgroundDaemon(&data);
    let output = desktop_with(
        dir.path(),
        &[
            ("OMFP_SMOKE", "local"),
            (
                "OMFP_SMOKE_WAIT",
                &format!("body[data-framed-page=\"{base}\"]"),
            ),
            // A model omp can use, so the window shows its view, not the first run's welcome (UI-500); nothing calls
            // the model.
            ("ANTHROPIC_API_KEY", "sk-ant-omfp-test"),
        ],
        &[
            "--data-dir",
            data.to_str().unwrap(),
            "--",
            "--static-dir",
            web_bundle().to_str().unwrap(),
            "--workspaces-root",
            workspaces.to_str().unwrap(),
            // A random port: the default one may be the user's daemon.
            "--bind",
            "127.0.0.1:0",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let state: serde_json::Value = stdout
        .lines()
        .find_map(|line| line.strip_prefix("OMFP_SMOKE_READY "))
        .map(|json| serde_json::from_str(json).unwrap())
        .unwrap();
    // The window is still on the daemon's page; the frame loaded the page and nothing else.
    let href = state["href"].as_str().unwrap();
    assert!(
        href.starts_with("http://127.0.0.1:") && !href.starts_with(&base),
        "{state}"
    );
    let pages: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|line| !line.contains("/favicon.ico"))
        .cloned()
        .collect();
    assert_eq!(pages, ["GET /page HTTP/1.1"], "{state}");
}

// spec: TST-033, ENV-030, UI-541
#[cfg(target_os = "linux")]
#[test]
fn the_linux_desktop_decodes_h264_and_stays_responsive() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspaces = dir.path().join("workspaces");
    let base = serve_h264_page();
    let config = dir.path().join(".config/omfp");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("init.ts"),
        format!(
            r#"export default function init(omfp) {{
  const media = (event) => {{
    if (typeof event.data === "string" && event.data.startsWith("h264 ")) {{
      document.body.dataset.smoke = event.data.slice(5);
    }}
  }};
  window.addEventListener("message", media);
  omfp.views.transform((views) =>
    views.update("default", () => ["row", {{}}, ["omfp:web", {{ url: "{base}/page", title: "H.264" }}]]),
  );
  return () => window.removeEventListener("message", media);
}}
"#
        ),
    )
    .unwrap();
    let _daemon = BackgroundDaemon(&data);
    let output = desktop_with(
        dir.path(),
        &[
            ("OMFP_SMOKE", "local"),
            ("OMFP_SMOKE_WAIT", "body[data-smoke]"),
            ("ANTHROPIC_API_KEY", "sk-ant-omfp-test"),
        ],
        &[
            "--data-dir",
            data.to_str().unwrap(),
            "--",
            "--static-dir",
            web_bundle().to_str().unwrap(),
            "--workspaces-root",
            workspaces.to_str().unwrap(),
            "--bind",
            "127.0.0.1:0",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let state: serde_json::Value = stdout
        .lines()
        .find_map(|line| line.strip_prefix("OMFP_SMOKE_READY "))
        .map(|json| serde_json::from_str(json).unwrap())
        .unwrap();
    assert_eq!(state["smoke"], "loaded", "{state}");
}

/// Stops the background daemon of a test, also when the test fails.
struct Daemon(i32);

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(pid) = Pid::from_raw(self.0) {
            let _ = kill_process(pid, Signal::TERM);
        }
    }
}

/// Stops the background daemon of the data directory, if one runs, when the test ends (also when it fails).
struct BackgroundDaemon<'a>(&'a Path);

impl Drop for BackgroundDaemon<'_> {
    fn drop(&mut self) {
        if let Ok(Some(info)) = omfp_daemon::discover(self.0) {
            drop(Daemon(i32::try_from(info.pid).unwrap_or(0)));
        }
    }
}

// spec: ARC-003, UI-004, UI-005, DMN-128, DMN-180, DMN-181
#[test]
fn a_second_window_reuses_the_background_daemon_that_outlives_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspaces = dir.path().join("workspaces");
    let web = web_bundle();
    let args = [
        "--data-dir",
        data.to_str().unwrap(),
        "--",
        "--static-dir",
        web.to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        // A random port: the default one may be the user's daemon.
        "--bind",
        "127.0.0.1:0",
    ];

    let first = desktop(dir.path(), "local", &args);
    // The first window closed; its background daemon still runs (D-16).
    let info = omfp_daemon::discover(&data)
        .unwrap()
        .expect("the daemon outlives the window");
    let daemon = Daemon(i32::try_from(info.pid).unwrap());
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert!(
        stderr.contains(&format!("started the daemon (pid {})", info.pid)),
        "{stderr}"
    );
    let log = std::fs::read_to_string(data.join("daemon.log")).unwrap();
    assert!(log.contains("omfp daemon listening on"), "{log}");
    // DMN-180, DMN-181: neither the log nor its directory are open to others, and the log never holds the token.
    let token = std::fs::read_to_string(data.join("token")).unwrap();
    let token = token.trim();
    assert!(!log.contains(token), "{log}");
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&data), 0o700);
        assert_eq!(mode(&data.join("daemon.log")), 0o600);
    }
    // DMN-128: started with `--tray`; on Linux the test's session bus does not exist, on macOS the item is in the
    // menu bar (DMN-171).
    #[cfg(target_os = "linux")]
    assert!(log.contains("no tray icon: no session bus"), "{log}");
    #[cfg(target_os = "macos")]
    assert!(!log.contains("no menu bar item"), "{log}");
    // UI-004: the URL a browser opens on the same daemon; with the token only on a terminal, not this pipe (DMN-181).
    let url = format!(
        "omfp desktop: http://{}/ (add #token=<token> from {})",
        info.addr,
        data.join("token").display()
    );
    assert!(
        String::from_utf8_lossy(&first.stdout).contains(&url),
        "{url}"
    );

    let second = desktop(dir.path(), "local", &args);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains(&format!("found the daemon (pid {})", info.pid)),
        "{stderr}"
    );
    assert!(!stderr.contains("started the daemon"), "{stderr}");
    assert!(String::from_utf8_lossy(&second.stdout).contains(&url));
    assert_eq!(omfp_daemon::discover(&data).unwrap(), Some(info));

    // SIGTERM stops it cleanly (DMN-005): its data directory's lock is free once it exited.
    drop(daemon);
    std::fs::File::open(data.join("daemon.lock"))
        .unwrap()
        .lock_shared()
        .unwrap();
    assert_eq!(omfp_daemon::discover(&data).unwrap(), None);
}

// spec: TST-033, UI-115, UI-541, UI-542
#[cfg(target_os = "linux")]
#[expect(
    clippy::too_many_lines,
    reason = "the three launches form one persistence scenario"
)]
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_audio_tab_keeps_the_desktop_responsive_and_can_be_closed() {
    use clap::Parser as _;
    use omfp::remote::{Connection, Target};
    use omfp_daemon::agent::FakeLauncher;
    use omfp_daemon::config::{Cli, Config};
    use omfp_daemon::git::FakeGit;
    use omfp_daemon::{Ports, start_with};
    use omfp_protocol::method::{ThreadGit, ThreadsCreate, WorkspacesRescan};
    use omfp_protocol::{
        CreateThreadParams, CreateWorkspaceParams, ThreadRef, ThreadWorkspace, WorkspaceRef,
    };
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspaces = dir.path().join("workspaces");
    let init = dir.path().join("init.ts");
    std::fs::write(&init, r#"
export default function init(omfp) {
  const stage = localStorage.getItem("test.restore") || "open";
  let restored = false;
  const tick = setInterval(() => {
    const tab = document.querySelector('[data-testid=file-tab][data-file="media/tone.wav"][aria-selected="true"]');
    const player = document.querySelector('[data-testid=file-player]');
    const error = document.querySelector('[data-testid=file-media-error]');
    if (player && player.paused) {
      player.muted = true;
      player.play().catch(() => {});
    }
    const media = (player && player.currentTime > 0) || error?.textContent?.includes("This window cannot play this audio file");
    if (stage === "open" && !tab) omfp.run("file.open", ["media/tone.wav"]);
    if (stage === "open" && tab && media && localStorage.getItem("omfp.openFiles")?.includes("tone.wav")) {
      localStorage.setItem("test.restore", "close");
      document.body.dataset.restoreTest = "opened";
    } else if (stage === "close" && tab && media) {
      restored = true;
      tab.querySelector('[data-testid=file-tab-close]')?.click();
    } else if (stage === "close" && restored && !tab && !localStorage.getItem("omfp.openFiles")?.includes("tone.wav")) {
      localStorage.setItem("test.restore", "done");
      document.body.dataset.restoreTest = "closed";
    } else if (stage === "done" && !tab && !localStorage.getItem("omfp.openFiles")?.includes("tone.wav")) {
      document.body.dataset.restoreTest = "done";
    }
  }, 100);
  return () => clearInterval(tick);
}
"#).unwrap();
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--static-dir",
        web_bundle().to_str().unwrap(),
    ])
    .unwrap();
    let mut config = Config::from_cli(cli, None, None).unwrap();
    config.init_file = Some(init);
    let handle = start_with(
        config,
        Ports {
            launcher: Arc::new(FakeLauncher::new()),
            git: Arc::new(FakeGit::new()),
            omp_program: "omp".into(),
            omp_version: omfp_core::OMP_VERSION.into(),
            omp_env: Vec::new(),
            user_omp_env: Vec::new(),
            warnings: Vec::new(),
            clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
        },
    )
    .await
    .unwrap();
    let mut client = Connection::open(&Target {
        url: format!("ws://{}/api/ws", handle.addr()),
        token: handle.token().expose().to_owned(),
    })
    .await
    .unwrap();
    let thread = client
        .call::<ThreadsCreate>(CreateThreadParams {
            title: None,
            workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                name: Some("media".into()),
                link: None,
            }),
            model: None,
            thinking_level: None,
            approval_mode: None,
            prompt: None,
            images: Vec::new(),
        })
        .await
        .unwrap();
    let git = client
        .call::<ThreadGit>(ThreadRef {
            thread_id: thread.id.clone(),
        })
        .await
        .unwrap();
    let workspace = client
        .call::<WorkspacesRescan>(WorkspaceRef {
            workspace_id: git.workspace_id,
        })
        .await
        .unwrap();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32 + 16000).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8000_u32.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.resize(16044, 0);
    std::fs::create_dir_all(Path::new(&workspace.path).join("media")).unwrap();
    std::fs::write(Path::new(&workspace.path).join("media/tone.wav"), wav).unwrap();
    let args = [
        "--data-dir",
        data.to_str().unwrap(),
        "--thread",
        thread.id.as_str(),
    ];
    for stage in ["opened", "closed", "done"] {
        desktop_with(
            dir.path(),
            &[
                ("OMFP_SMOKE", "local"),
                (
                    "OMFP_SMOKE_WAIT",
                    &format!("body[data-restore-test=\"{stage}\"]"),
                ),
            ],
            &args,
        );
    }
    handle.shutdown().await.unwrap();
}

/// Two ASCII PDF pages with a selectable text layer.
fn matrix_pdf() -> Vec<u8> {
    use std::io::Write as _;
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [4 0 R 6 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    for (index, text) in ["Hello PDF one", "Hello PDF two"].iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * index
        ));
        let stream = format!("BT /F1 36 Tf 72 680 Td ({text}) Tj ET\n");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}endstream",
            stream.len()
        ));
    }
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        write!(bytes, "{} 0 obj\n{object}\nendobj\n", index + 1).unwrap();
    }
    let xref = bytes.len();
    write!(
        bytes,
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    )
    .unwrap();
    for offset in offsets {
        writeln!(bytes, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        bytes,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    bytes
}

/// A 9 MB 24-bit image, larger than the daemon's inline image limit.
fn matrix_bmp(red: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(9_000_054);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&9_000_054_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&2000_u32.to_le_bytes());
    bytes.extend_from_slice(&1500_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for _ in 0..3_000_000 {
        bytes.extend_from_slice(if red { &[32, 32, 224] } else { &[224, 80, 32] });
    }
    bytes
}

// spec: TST-033, UI-115, UI-250, UI-541, UI-542, UI-543, UI-544, UI-545, UI-546, UI-547, UI-548, UI-549, UI-551, UI-553, UI-560, UI-561, UI-562, UI-563, UI-567
#[expect(
    clippy::too_many_lines,
    reason = "the fixture matrix shares one daemon and one persistent webview store"
)]
#[tokio::test(flavor = "multi_thread")]
async fn desktop_file_viewers_open_restore_and_close() {
    use clap::Parser as _;
    use omfp::remote::{Connection, Target};
    use omfp_daemon::agent::FakeLauncher;
    use omfp_daemon::config::{Cli, Config};
    use omfp_daemon::git::FakeGit;
    use omfp_daemon::{Ports, start_with};
    use omfp_protocol::method::{ThreadGit, ThreadsCreate, WorkspacesRescan};
    use omfp_protocol::{
        CreateThreadParams, CreateWorkspaceParams, ThreadRef, ThreadWorkspace, WorkspaceRef,
    };
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    #[cfg(target_os = "macos")]
    let _web_store = RetainedWebStore(data.clone());
    let workspaces = dir.path().join("workspaces");
    let init = dir.path().join("init.ts");
    std::fs::write(&init, include_str!("fixtures/desktop-viewers/probe.ts")).unwrap();
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--static-dir",
        web_bundle().to_str().unwrap(),
    ])
    .unwrap();
    let mut config = Config::from_cli(cli, None, None).unwrap();
    config.init_file = Some(init);
    let git = Arc::new(FakeGit::new());
    let handle = start_with(
        config,
        Ports {
            launcher: Arc::new(FakeLauncher::new()),
            git: git.clone(),
            omp_program: "omp".into(),
            omp_version: omfp_core::OMP_VERSION.into(),
            omp_env: Vec::new(),
            user_omp_env: Vec::new(),
            warnings: Vec::new(),
            clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
        },
    )
    .await
    .unwrap();
    let mut client = Connection::open(&Target {
        url: format!("ws://{}/api/ws", handle.addr()),
        token: handle.token().expose().to_owned(),
    })
    .await
    .unwrap();
    let thread = client
        .call::<ThreadsCreate>(CreateThreadParams {
            title: None,
            workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                name: Some("viewers".into()),
                link: None,
            }),
            model: None,
            thinking_level: None,
            approval_mode: None,
            prompt: None,
            images: Vec::new(),
        })
        .await
        .unwrap();
    let thread_git = client
        .call::<ThreadGit>(ThreadRef {
            thread_id: thread.id.clone(),
        })
        .await
        .unwrap();
    let workspace = client
        .call::<WorkspacesRescan>(WorkspaceRef {
            workspace_id: thread_git.workspace_id.clone(),
        })
        .await
        .unwrap();
    let folder = Path::new(&workspace.path);
    let viewers = folder.join("viewers");
    std::fs::create_dir_all(&viewers).unwrap();
    for (name, bytes) in [
        (
            "video.mp4",
            include_bytes!("fixtures/desktop-viewers/video.mp4").as_slice(),
        ),
        (
            "video.webm",
            include_bytes!("fixtures/desktop-viewers/video.webm").as_slice(),
        ),
        (
            "video.ogv",
            include_bytes!("fixtures/desktop-viewers/video.ogv").as_slice(),
        ),
        (
            "unsupported.mkv",
            include_bytes!("fixtures/desktop-viewers/unsupported.mkv").as_slice(),
        ),
        (
            "audio.mp3",
            include_bytes!("fixtures/desktop-viewers/audio.mp3").as_slice(),
        ),
        (
            "audio.ogg",
            include_bytes!("fixtures/desktop-viewers/audio.ogg").as_slice(),
        ),
        (
            "audio.opus",
            include_bytes!("fixtures/desktop-viewers/audio.opus").as_slice(),
        ),
        (
            "audio.flac",
            include_bytes!("fixtures/desktop-viewers/audio.flac").as_slice(),
        ),
        (
            "audio.m4a",
            include_bytes!("fixtures/desktop-viewers/audio.m4a").as_slice(),
        ),
        (
            "image.png",
            include_bytes!("fixtures/desktop-viewers/image.png").as_slice(),
        ),
        (
            "image.jpg",
            include_bytes!("fixtures/desktop-viewers/image.jpg").as_slice(),
        ),
        (
            "image.gif",
            include_bytes!("fixtures/desktop-viewers/image.gif").as_slice(),
        ),
        (
            "image.webp",
            include_bytes!("fixtures/desktop-viewers/image.webp").as_slice(),
        ),
    ] {
        std::fs::write(viewers.join(name), bytes).unwrap();
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32 + 16000).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8000_u32.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.resize(16044, 0);
    std::fs::write(viewers.join("audio.wav"), wav).unwrap();
    std::fs::write(viewers.join("guide.pdf"), matrix_pdf()).unwrap();
    for name in ["guide.md", "guide.mdx"] {
        std::fs::write(
            viewers.join(name),
            "# Guide\n\n![sample](image.png)\n\n[Next](next.md)\n",
        )
        .unwrap();
    }
    for (name, text) in [
        ("next.md", "# Next\n"),
        ("data.csv", "name,value\nalpha,1\n"),
        ("data.tsv", "name\tvalue\nalpha\t1\n"),
        ("data.json", "{\"name\":\"alpha\"}\n"),
        ("data.jsonc", "{ // comment\n \"name\":\"alpha\"\n}\n"),
        ("data.yaml", "name: alpha\n"),
    ] {
        std::fs::write(viewers.join(name), text).unwrap();
    }
    git.add_repo(folder, "main");
    git.commit_file(folder, "viewers/large.bmp", &matrix_bmp(false));
    std::fs::write(viewers.join("large.bmp"), matrix_bmp(true)).unwrap();
    client
        .call::<WorkspacesRescan>(WorkspaceRef {
            workspace_id: thread_git.workspace_id,
        })
        .await
        .unwrap();
    let args = [
        "--data-dir",
        data.to_str().unwrap(),
        "--thread",
        thread.id.as_str(),
    ];
    let files = [
        "video.mp4",
        "video.webm",
        "video.ogv",
        "unsupported.mkv",
        "audio.wav",
        "audio.mp3",
        "audio.ogg",
        "audio.opus",
        "audio.flac",
        "audio.m4a",
        "guide.pdf",
        "image.png",
        "image.jpg",
        "image.gif",
        "image.webp",
        "large.bmp",
        "guide.md",
        "guide.mdx",
        "data.csv",
        "data.tsv",
        "data.json",
        "data.jsonc",
        "data.yaml",
    ];
    for name in files {
        for phase in ["open", "restore", "reopen"] {
            let output = desktop_with_store(
                dir.path(),
                &[
                    ("OMFP_SMOKE", "local"),
                    ("OMFP_SMOKE_WAIT", "body[data-matrix=ready]"),
                ],
                &args,
                true,
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            let state: serde_json::Value = stdout
                .lines()
                .find_map(|line| line.strip_prefix("OMFP_SMOKE_READY "))
                .map(|line| serde_json::from_str(line).unwrap())
                .unwrap();
            let result: serde_json::Value =
                serde_json::from_str(state["smoke"].as_str().unwrap()).unwrap();
            assert_eq!(result["name"], name, "{result}");
            assert_eq!(result["phase"], phase, "{result}");
            println!("viewer: {result}");
        }
    }
    desktop_with_store(
        dir.path(),
        &[
            ("OMFP_SMOKE", "local"),
            ("OMFP_SMOKE_WAIT", "body[data-matrix=done]"),
        ],
        &args,
        true,
    );
    handle.shutdown().await.unwrap();
}

/// The settings portal's color scheme (UI-195, Linux only: on macOS the window follows `AppKit`'s appearance, UI-253).
#[cfg(target_os = "linux")]
mod portal {
    use std::io::{BufRead as _, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;

    use gio::glib::{self, ToVariant as _};

    use super::{desktop_with, web_bundle};

    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
    const PORTAL_SETTINGS: &str = "org.freedesktop.portal.Settings";
    const PORTAL_XML: &str = r#"<node>
  <interface name="org.freedesktop.portal.Settings">
    <method name="ReadOne">
      <arg type="s" name="namespace" direction="in"/>
      <arg type="s" name="key" direction="in"/>
      <arg type="v" name="value" direction="out"/>
    </method>
    <signal name="SettingChanged">
      <arg type="s" name="namespace"/>
      <arg type="s" name="key"/>
      <arg type="v" name="value"/>
    </signal>
  </interface>
</node>"#;

    /// A private session bus (`dbus-daemon`, stopped on drop) holding a stub settings portal: it answers
    /// `org.freedesktop.appearance color-scheme` with `initial`, then, when `then` is given, announces that value with
    /// `SettingChanged` right after the answer, as a live switch on the desktop would.
    struct Portal {
        bus: Child,
        address: String,
    }

    impl Portal {
        fn start(initial: u32, then: Option<u32>) -> Self {
            let mut bus = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", session_config().display()))
                .args(["--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut address = String::new();
            BufReader::new(bus.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            let address = address.trim().to_owned();
            let (ready, registered) = mpsc::channel();
            let bus_address = address.clone();
            std::thread::spawn(move || {
                let context = glib::MainContext::new();
                context
                    .with_thread_default(|| {
                        let connection = gio::DBusConnection::for_address_sync(
                            &bus_address,
                            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                            None,
                            None::<&gio::Cancellable>,
                        )
                        .unwrap();
                        let interface = gio::DBusNodeInfo::for_xml(PORTAL_XML)
                            .unwrap()
                            .lookup_interface(PORTAL_SETTINGS)
                            .unwrap();
                        let _registration = connection
                            .register_object(
                                PORTAL_PATH,
                                &interface,
                                move |connection, _, _, _, _, parameters, invocation| {
                                    let (namespace, key) =
                                        parameters.get::<(String, String)>().unwrap_or_default();
                                    if namespace != "org.freedesktop.appearance"
                                        || key != "color-scheme"
                                    {
                                        invocation.return_dbus_error(
                                            "org.freedesktop.portal.Error.NotFound",
                                            "no such setting",
                                        );
                                        return;
                                    }
                                    invocation
                                        .return_value(Some(&(initial.to_variant(),).to_variant()));
                                    if let Some(then) = then {
                                        connection
                                            .emit_signal(
                                                None,
                                                PORTAL_PATH,
                                                PORTAL_SETTINGS,
                                                "SettingChanged",
                                                Some(
                                                    &(namespace, key, then.to_variant())
                                                        .to_variant(),
                                                ),
                                            )
                                            .unwrap();
                                    }
                                },
                                |_, _, _, _, _| "".to_variant(),
                                |_, _, _, _, _, _| false,
                            )
                            .unwrap();
                        // Owned after the object is registered, so the window never sees the name without it.
                        connection
                            .call_sync(
                                Some("org.freedesktop.DBus"),
                                "/org/freedesktop/DBus",
                                "org.freedesktop.DBus",
                                "RequestName",
                                Some(&("org.freedesktop.portal.Desktop", 4u32).to_variant()),
                                None,
                                gio::DBusCallFlags::NONE,
                                -1,
                                None::<&gio::Cancellable>,
                            )
                            .unwrap();
                        ready.send(()).unwrap();
                        glib::MainLoop::new(Some(&context), false).run();
                    })
                    .unwrap();
            });
            registered.recv().unwrap();
            Self { bus, address }
        }
    }

    impl Drop for Portal {
        fn drop(&mut self) {
            let _ = self.bus.kill();
            let _ = self.bus.wait();
        }
    }

    /// The session bus configuration installed with the `dbus-daemon` on `PATH` (`../share/dbus-1/session.conf`):
    /// `--session` reads `/etc/dbus-1/session.conf`, which not every host has (GitHub's runner after the disk purge).
    fn session_config() -> std::path::PathBuf {
        let daemon = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join("dbus-daemon"))
            .find(|path| path.is_file())
            .expect("dbus-daemon on PATH");
        let daemon = daemon.canonicalize().unwrap();
        daemon
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("share/dbus-1/session.conf")
    }

    /// The page's color scheme in a smoke window on `portal`'s session bus (`OMFP_SMOKE_SCHEME=<expected>` waits for it).
    fn scheme_on(portal: &Portal, expected: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let workspaces = dir.path().join("workspaces");
        let web = web_bundle();
        let output = desktop_with(
            dir.path(),
            &[
                ("OMFP_SMOKE", "1"),
                ("OMFP_SMOKE_SCHEME", expected),
                ("DBUS_SESSION_BUS_ADDRESS", &portal.address),
            ],
            &[
                "--data-dir",
                data.to_str().unwrap(),
                "--",
                "--static-dir",
                web.to_str().unwrap(),
                "--workspaces-root",
                workspaces.to_str().unwrap(),
            ],
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    // spec: UI-195
    #[test]
    fn the_window_takes_the_dark_color_scheme_of_the_settings_portal() {
        let portal = Portal::start(1, None);
        assert!(scheme_on(&portal, "dark").contains(r#""scheme":"dark""#));
    }

    // spec: UI-195
    #[test]
    fn the_window_follows_a_color_scheme_change_of_the_settings_portal() {
        // Dark at startup, then switched to light while the window is open.
        let portal = Portal::start(1, Some(2));
        assert!(scheme_on(&portal, "light").contains(r#""scheme":"light""#));
    }
}
