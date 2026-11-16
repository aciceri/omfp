//! End-to-end tests (docs/specs/06-testing.md, layer L5): headless Chromium driven over WebDriver (`thirtyfour` and
//! `chromedriver`) against the TypeScript web bundle served by a real daemon, which runs the pinned omp against the
//! fake LLM in an isolated environment (`omfp_testkit`). Run through `just test-e2e`, which builds the bundle first.
//!
//! Every selector is a `data-testid` (TST-031). Waiting polls the page until a condition holds or [`TIMEOUT`] passes;
//! nothing waits for a fixed time. Title generation (DMN-073) is off, so the fake LLM only answers scripted turns and
//! threads keep the placeholder title of their first message.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Parser;
use fake_llm::{FakeLlm, Script, Step};
use omfp::remote::{Connection, Target};
use omfp_daemon::agent::{FakeLauncher, FakeProcess};
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::git::FakeGit;
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_protocol::method::ThreadsCreate;
use omfp_protocol::{ApprovalMode, CreateThreadParams, CreateWorkspaceParams, ThreadWorkspace};
use omfp_testkit::{
    DenyProxy, FAKE_MODEL, KNOWN_DISCOVERY_HOSTS, OmpEnv, omp_bin, require_pinned_omp,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use thirtyfour::ChromeCapabilities;
use thirtyfour::prelude::*;

/// Longest wait for any condition (omp startup and a model turn included).
pub const TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(50);

/// The element that has the focus, for [`Journey::report`]: its tag, test id and text field value.
const FOCUSED: &str = "
const e = document.activeElement;
if (e === null || e === document.body) { return 'nothing'; }
const id = e.getAttribute('data-testid');
const value = typeof e.value === 'string' && e.value !== '' ? ` holding ${JSON.stringify(e.value)}` : '';
return `<${e.tagName.toLowerCase()}${id === null ? '' : ` data-testid=${id}`}>${value}`;";

/// The web bundle in `OMFP_WEB_BUNDLE` (`just test-e2e` sets it to a bundle no other build writes, TST-074), else the
/// one built by `just build-web` (`ui/packages/app/dist`).
#[must_use]
pub fn web_bundle() -> PathBuf {
    std::env::var_os("OMFP_WEB_BUNDLE").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/packages/app/dist"),
        PathBuf::from,
    )
}

/// The CSS selector of the sidebar row of thread `id`.
#[must_use]
pub fn row(id: &str) -> String {
    format!("[data-testid=thread-row][data-thread-id=\"{id}\"]")
}

/// The latest omp of a thread, excluding transient provider/model discovery processes.
#[must_use]
pub fn fake_thread(launcher: &FakeLauncher, thread: &str) -> Option<FakeProcess> {
    launcher
        .launches()
        .into_iter()
        .rev()
        .find(|launch| {
            launch
                .spec
                .session_dir
                .parent()
                .is_some_and(|dir| dir.ends_with(thread))
        })
        .map(|launch| launch.process)
}

/// The browsers' fontconfig file, never the host's fonts: `OMFP_E2E_FONTCONFIG_FILE`, `DejaVu` from the dev shell. CI's
/// purged runner has no fonts left (TST-075), and Chromium's renderer crashes without any; the same fonts everywhere
/// also give every machine the same text metrics.
fn fontconfig_file() -> anyhow::Result<OsString> {
    std::env::var_os("OMFP_E2E_FONTCONFIG_FILE")
        .context("OMFP_E2E_FONTCONFIG_FILE is unset (run inside `nix develop`)")
}

/// `chromedriver` on the port it picks itself; killed on drop.
struct ChromeDriver {
    child: Child,
    url: String,
}

/// The port of chromedriver's ready line (`ChromeDriver was started successfully on port 43063.`).
fn ready_port(line: &str) -> Option<u16> {
    let (_, rest) = line.split_once("started successfully on port ")?;
    rest.trim_end().trim_end_matches('.').parse().ok()
}

/// How many times a chromedriver that could not bind its port is started again.
const CHROMEDRIVER_ATTEMPTS: usize = 5;

impl ChromeDriver {
    /// Starts chromedriver on a port it picks itself (`--port=0`), so parallel journeys never race for a port probed
    /// free beforehand. Its picked port is free on IPv4 but may already be taken on IPv6 (`bind() failed: Address
    /// already in use`); such a start is retried with a fresh pick.
    fn start() -> anyhow::Result<Self> {
        let mut failure = None;
        for _ in 0..CHROMEDRIVER_ATTEMPTS {
            match Self::start_once()? {
                Ok(driver) => return Ok(driver),
                Err(error) if error.contains("Address already in use") => failure = Some(error),
                Err(error) => bail!("{error}"),
            }
        }
        bail!(
            "{}",
            failure.unwrap_or_else(|| "chromedriver did not start".to_owned())
        )
    }

    /// One start: the driver, or why chromedriver exited before it was ready.
    fn start_once() -> anyhow::Result<Result<Self, String>> {
        let mut child = Command::new("chromedriver")
            .arg("--port=0")
            // Never the user's session bus: Chromium would move itself into a systemd scope of the user's session
            // (`app-org.chromium.Chromium-<pid>.scope`), out of the memory limit the tests run under.
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/omfp-e2e-no-session-bus",
            )
            .env("FONTCONFIG_FILE", fontconfig_file()?)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("cannot start chromedriver (run inside `nix develop`)")?;
        let stdout = child.stdout.take().context("chromedriver stdout")?;
        let stderr = child.stderr.take().context("chromedriver stderr")?;
        // Keep draining its error output so it never blocks on a full pipe; it explains a failed start.
        let errors = std::thread::spawn(move || {
            BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<_>>()
        });
        let mut lines = BufReader::new(stdout).lines();
        let port = loop {
            match lines.next() {
                Some(Ok(line)) => {
                    if let Some(port) = ready_port(&line) {
                        break port;
                    }
                }
                Some(Err(error)) => return Err(error.into()),
                None => {
                    let status = child.wait()?;
                    let errors = errors.join().unwrap_or_default().join("\n");
                    return Ok(Err(format!(
                        "chromedriver exited before it was ready ({status}): {errors}"
                    )));
                }
            }
        };
        // Keep draining its output so it never blocks on a full pipe.
        std::thread::spawn(move || lines.for_each(drop));
        Ok(Ok(Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
        }))
    }
}

impl Drop for ChromeDriver {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `WebKitWebDriver` (`OMFP_WEBKIT_WEBDRIVER`, set by the dev shell) under its own X server: `xvfb-run`, never the
/// user's display. It drives nixpkgs' `MiniBrowser`, the `WebKitGTK` engine of `omfp desktop`. The whole process group
/// is killed on drop.
#[derive(Debug)]
pub struct WebKitDriver {
    child: Child,
    url: String,
    /// The `MiniBrowser` next to the driver (`libexec/webkit2gtk-4.1/`).
    browser: PathBuf,
}

impl WebKitDriver {
    /// Starts the driver and waits until it listens.
    pub async fn start() -> anyhow::Result<Self> {
        use std::os::unix::process::CommandExt;
        let driver = std::env::var_os("OMFP_WEBKIT_WEBDRIVER")
            .map(PathBuf::from)
            .context("set OMFP_WEBKIT_WEBDRIVER to WebKitWebDriver (run inside `nix develop`)")?;
        let browser = driver
            .parent()
            .and_then(Path::parent)
            .map(|prefix| prefix.join("libexec/webkit2gtk-4.1/MiniBrowser"))
            .filter(|path| path.is_file())
            .with_context(|| format!("no MiniBrowser next to {}", driver.display()))?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        let child = Command::new("xvfb-run")
            .args(["-a", "-s", "-screen 0 1600x1200x24"])
            .arg(&driver)
            .arg(format!("--port={port}"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("GDK_BACKEND", "x11")
            .env("FONTCONFIG_FILE", fontconfig_file()?)
            .process_group(0)
            .spawn()
            .context("cannot start xvfb-run (run inside `nix develop`)")?;
        let started = Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
            browser,
        };
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Ok(started);
            }
            tokio::time::sleep(POLL).await;
        }
        bail!("WebKitWebDriver did not start")
    }

    /// A new `MiniBrowser` session in a 1400×1000 window.
    pub async fn session(&self) -> anyhow::Result<WebDriver> {
        let mut caps = thirtyfour::Capabilities::new();
        caps.set("browserName", "MiniBrowser")?;
        caps.set(
            "webkitgtk:browserOptions",
            json!({ "binary": self.browser, "args": ["--automation"] }),
        )?;
        let driver = WebDriver::new(&self.url, caps).await?;
        driver.set_window_rect(0, 0, 1400, 1000).await?;
        Ok(driver)
    }

    /// `DISPLAY` and `XAUTHORITY` of the driver's X server, for real input events there (e.g. `xdotool`): the
    /// environment `xvfb-run` gave `WebKitWebDriver`.
    pub fn x_environment(&self) -> anyhow::Result<Vec<(String, String)>> {
        let mut pending = vec![self.child.id().to_string()];
        while let Some(pid) = pending.pop() {
            let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            let vars: Vec<(String, String)> = environ
                .split(|byte| *byte == 0)
                .filter_map(|entry| {
                    let (name, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
                    ["DISPLAY", "XAUTHORITY"]
                        .contains(&name)
                        .then(|| (name.to_owned(), value.to_owned()))
                })
                .collect();
            if vars.iter().any(|(name, _)| name == "DISPLAY") {
                return Ok(vars);
            }
            for task in std::fs::read_dir(format!("/proc/{pid}/task"))? {
                let children = std::fs::read_to_string(task?.path().join("children"))?;
                pending.extend(children.split_whitespace().map(str::to_owned));
            }
        }
        bail!("no process under xvfb-run has a DISPLAY")
    }
}

impl Drop for WebKitDriver {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(format!("-{}", self.child.id()))
            .status();
        let _ = self.child.wait();
    }
}

/// One journey: fake LLM, isolated omp, daemon serving the web bundle, and a headless browser on its URL.
pub struct Journey {
    pub fake: FakeLlm,
    proxy: DenyProxy,
    pub env: OmpEnv,
    data: TempDir,
    _profile: TempDir,
    daemon: Option<DaemonHandle>,
    pub driver: WebDriver,
    _chromedriver: ChromeDriver,
    /// The daemon's git of a [`Journey::start_fake`] journey (repositories and their `HEAD` set by the test);
    /// `None` with the real omp, whose daemon runs git itself.
    pub git: Option<Arc<FakeGit>>,
    started: Instant,
}

impl std::fmt::Debug for Journey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journey")
            .field("data", &self.data.path())
            .finish_non_exhaustive()
    }
}

impl Journey {
    /// Starts everything with the model replying `steps` in order, then opens the UI with the token in the URL.
    pub async fn start(steps: Vec<Step>) -> anyhow::Result<Self> {
        require_pinned_omp();
        Self::launch(steps, None, web_bundle()).await
    }

    /// Starts a journey whose daemon runs `launcher` (the fake omp of `omfp_daemon::agent`, scripted by the test
    /// through its clone) instead of omp: for what the pinned omp cannot do offline, e.g. a live voice call (E2E-14).
    pub async fn start_fake(launcher: FakeLauncher) -> anyhow::Result<Self> {
        Self::launch(Vec::new(), Some(launcher), web_bundle()).await
    }

    /// [`Journey::start_fake`] serving the web bundle at `bundle` (e.g. a copy with a measuring script, TST-052).
    pub async fn start_fake_with_bundle(
        launcher: FakeLauncher,
        bundle: PathBuf,
    ) -> anyhow::Result<Self> {
        Self::launch(Vec::new(), Some(launcher), bundle).await
    }

    async fn launch(
        steps: Vec<Step>,
        launcher: Option<FakeLauncher>,
        bundle: PathBuf,
    ) -> anyhow::Result<Self> {
        trace_daemon();
        let started = Instant::now();
        if !bundle.join("index.html").is_file() {
            bail!(
                "no web bundle at {}: run `just build-web` (or `just test-e2e`)",
                bundle.display()
            );
        }
        let fake = FakeLlm::start(Script::new(FAKE_MODEL, steps)).await?;
        let proxy = DenyProxy::start().await?;
        let env = OmpEnv::new(&fake.base_url(), Some(&proxy))?;
        let data = tempfile::tempdir()?;
        let git = launcher.as_ref().map(|_| Arc::new(FakeGit::new()));
        let daemon = start_daemon(&env, data.path(), &bundle, launcher, git.clone()).await?;
        let profile = tempfile::tempdir()?;
        let chromedriver = ChromeDriver::start()?;
        let driver = WebDriver::new(&chromedriver.url, chrome(profile.path())?).await?;
        driver.goto(daemon.url()).await?;
        Ok(Self {
            fake,
            proxy,
            env,
            data,
            _profile: profile,
            daemon: Some(daemon),
            driver,
            _chromedriver: chromedriver,
            git,
            started,
        })
    }

    /// The daemon's data directory.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        self.data.path()
    }

    /// The daemon's config file (DMN-009): `config.toml` in the data directory, absent until a journey writes it, so
    /// the user's own `~/.config/omfp/config.toml` never applies.
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        config_file(self.data.path())
    }

    /// The running daemon.
    ///
    /// # Panics
    /// After [`Journey::finish`].
    #[must_use]
    pub fn daemon(&self) -> &DaemonHandle {
        self.daemon.as_ref().expect("the daemon runs until finish")
    }

    /// Where the protocol client of the command line reaches the daemon.
    #[must_use]
    pub fn target(&self) -> Target {
        let daemon = self.daemon();
        Target {
            url: format!("ws://{}/api/ws", daemon.addr()),
            token: daemon.token().expose().to_owned(),
        }
    }

    /// A protocol connection of its own to the daemon, for what the UI does not offer (e.g. the approval mode).
    pub async fn connect(&self) -> anyhow::Result<Connection> {
        Connection::open(&self.target()).await
    }

    /// Polls `probe` until it returns `Some`, failing after [`TIMEOUT`] with `what` and the page's state
    /// ([`Journey::report`]).
    pub async fn until<T>(
        &self,
        what: &str,
        mut probe: impl AsyncFnMut(&Self) -> anyhow::Result<Option<T>>,
    ) -> anyhow::Result<T> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(value) = probe(self).await? {
                return Ok(value);
            }
            if Instant::now() > deadline {
                bail!("timed out waiting for {what}; {}", self.report().await);
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// What a failed wait says about the page: its text, the element that has the focus (the text alone hides it: a
    /// key typed into the wrong field leaves no trace) and the browser's console since the last report.
    pub async fn report(&self) -> String {
        let page = self.text("body").await.unwrap_or_default();
        let focus = match self.eval(FOCUSED, Vec::new()).await {
            Ok(Value::String(focus)) => focus,
            Ok(other) => other.to_string(),
            Err(error) => format!("unknown ({error})"),
        };
        let mut report =
            format!("the page shows:\n{page}\nthe focus is on {focus}\nthe browser's console:");
        match self.driver.browser_log().await {
            Ok(entries) if entries.is_empty() => report.push_str(" empty"),
            Ok(entries) => {
                for entry in entries {
                    let _ = write!(report, "\n{} {}", entry.level, entry.message);
                }
            }
            Err(error) => {
                let _ = write!(report, " unavailable ({error})");
            }
        }
        report
    }

    /// Evaluates `script` (a function body getting `arguments`) in the page.
    pub async fn eval(&self, script: &str, args: Vec<Value>) -> anyhow::Result<Value> {
        Ok(self.driver.execute(script, args).await?.json().clone())
    }

    /// The visible text of every element matching `css`, in document order.
    pub async fn texts(&self, css: &str) -> anyhow::Result<Vec<String>> {
        let value = self
            .eval(
                "return [...document.querySelectorAll(arguments[0])].map(e => e.innerText.trim());",
                vec![json!(css)],
            )
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// The visible text of the first element matching `css` (empty when there is none).
    pub async fn text(&self, css: &str) -> anyhow::Result<String> {
        Ok(self
            .texts(css)
            .await?
            .into_iter()
            .next()
            .unwrap_or_default())
    }

    /// How many elements match `css`.
    pub async fn count(&self, css: &str) -> anyhow::Result<usize> {
        let value = self
            .eval(
                "return document.querySelectorAll(arguments[0]).length;",
                vec![json!(css)],
            )
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Attribute `name` of every element matching `css`.
    pub async fn attrs(&self, css: &str, name: &str) -> anyhow::Result<Vec<Option<String>>> {
        let value = self
            .eval(
                "return [...document.querySelectorAll(arguments[0])].map(e => e.getAttribute(arguments[1]));",
                vec![json!(css), json!(name)],
            )
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Attribute `name` of the first element matching `css`.
    pub async fn attr(&self, css: &str, name: &str) -> anyhow::Result<Option<String>> {
        Ok(self.attrs(css, name).await?.into_iter().next().flatten())
    }

    /// The `data-testid` of the focused element.
    pub async fn focused(&self) -> anyhow::Result<Option<String>> {
        let value = self
            .eval(
                "const e = document.activeElement; return e && e.dataset ? e.dataset.testid || null : null;",
                Vec::new(),
            )
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Waits until the focused element has the test id `testid`.
    pub async fn until_focused(&self, testid: &str) -> anyhow::Result<()> {
        self.until(&format!("the focus on {testid}"), async |page| {
            Ok((page.focused().await?.as_deref() == Some(testid)).then_some(()))
        })
        .await
    }

    /// The first element matching `css`, once it is displayed.
    pub async fn element(&self, css: &str) -> anyhow::Result<WebElement> {
        Ok(self
            .driver
            .query(By::Css(css))
            .wait(TIMEOUT, POLL)
            .and_displayed()
            .first()
            .await?)
    }

    /// Waits until an element matching `css` is displayed, then until the first one is at rest: every finite animation
    /// on it, inside it or on an element around it has ended (infinite ones, spinners and glows, never do). A box
    /// measured before is a moving one: an overlay's entrance (`omfp-pop`) scales it from 98 %, so a 44 px control
    /// measures 43 px (TST-110).
    pub async fn settled(&self, css: &str) -> anyhow::Result<()> {
        self.element(css).await?;
        self.eval(
            "const e = document.querySelector(arguments[0]);
             const moving = document.getAnimations().filter((a) => {
               const target = a.effect?.target;
               return target && (target.contains(e) || e.contains(target))
                 && a.effect.getComputedTiming().endTime !== Infinity;
             });
             return Promise.allSettled(moving.map((a) => a.finished)).then(() => true);",
            vec![json!(css)],
        )
        .await?;
        Ok(())
    }

    /// Waits until nothing matches `css`.
    pub async fn gone(&self, css: &str) -> anyhow::Result<()> {
        self.until(&format!("{css} to go"), async |page| {
            Ok((page.count(css).await? == 0).then_some(()))
        })
        .await
    }

    /// Clicks the first displayed element matching `css`.
    pub async fn click(&self, css: &str) -> anyhow::Result<()> {
        self.element(css).await?.click().await?;
        Ok(())
    }

    /// Moves the mouse over the first displayed element matching `css`, as a user does before clicking what a hover
    /// shows or moves (a row's delete ×, UI-205; a section's actions, UI-415).
    pub async fn hover(&self, css: &str) -> anyhow::Result<()> {
        let element = self.element(css).await?;
        self.driver
            .action_chain()
            .move_to_element_center(&element)
            .perform()
            .await?;
        Ok(())
    }

    /// Types `text` into the first displayed element matching `css`.
    pub async fn type_into(&self, css: &str, text: &str) -> anyhow::Result<()> {
        self.element(css).await?.send_keys(text).await?;
        Ok(())
    }

    /// Presses `keys` on whatever has the focus.
    pub async fn keys(&self, keys: impl Into<TypingData>) -> anyhow::Result<()> {
        self.driver.action_chain().send_keys(keys).perform().await?;
        Ok(())
    }

    /// Waits until the page is connected, shows its view and has its index: the left column is no longer `aria-busy`
    /// (TST-111). Connected comes first; until the index arrives, the window can still move the focus (UI-023).
    pub async fn connected(&self) -> anyhow::Result<()> {
        self.element("[data-testid=view]").await?;
        self.gone("[data-testid=connection-banner]").await?;
        self.until("the index in the left column", async |page| {
            let loaded = page.count("[data-testid=sidebar][aria-busy=false]").await?;
            Ok((loaded == 1).then_some(()))
        })
        .await
    }

    /// The ids of the threads in the sidebar.
    pub async fn thread_ids(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .attrs("[data-testid=thread-row]", "data-thread-id")
            .await?
            .into_iter()
            .flatten()
            .collect())
    }

    /// The selected thread's id.
    pub async fn selected(&self) -> anyhow::Result<Option<String>> {
        self.attr(
            "[data-testid=thread-row][aria-current=true]",
            "data-thread-id",
        )
        .await
    }

    /// Creates a workspace holding a link to the repository at `path` from the "+ workspace" field (UI-023): types
    /// `typed` (a prefix of `path` starting with `/`), picks `path` among the daemon's completions (DMN-021), presses
    /// Enter and waits until the draft thread it opens has the composer focused.
    pub async fn new_linked_workspace(&self, typed: &str, path: &str) -> anyhow::Result<()> {
        let input = "[data-testid=new-workspace-input]";
        self.click(input).await?;
        self.pick_path(input, typed, path).await?;
        self.until("the picked path in the + workspace field", async |page| {
            let value = page
                .eval(
                    "return document.querySelector(arguments[0]).value;",
                    vec![json!(input)],
                )
                .await?;
            Ok((value.as_str() == Some(path)).then_some(()))
        })
        .await?;
        self.element(input).await?.send_keys(Key::Enter).await?;
        self.element("[data-testid=draft-thread]").await?;
        self.until_focused("composer-input").await
    }

    /// Sends `text` as the first message of the open draft thread, which creates the thread (UI-023); waits until it
    /// is selected and open and returns its id.
    pub async fn first_message(&self, text: &str) -> anyhow::Result<String> {
        let before = self.thread_ids().await?;
        self.send(text).await?;
        self.created(&before).await
    }

    /// Creates a thread in a new workspace named `workspace` through the protocol (no repository, no first message),
    /// then opens it from the left column; returns its id. Journeys about creating threads use
    /// [`Self::new_linked_workspace`].
    pub async fn create_thread(&self, workspace: &str) -> anyhow::Result<String> {
        self.create_thread_with(workspace, None).await
    }

    /// Waits until a thread not in `before` is selected and open; returns its id.
    pub async fn created(&self, before: &[String]) -> anyhow::Result<String> {
        let id = self
            .until("the new thread to be selected", async |page| {
                Ok(page.selected().await?.filter(|id| !before.contains(id)))
            })
            .await?;
        self.opened(&id).await?;
        Ok(id)
    }

    /// Creates a thread whose omp asks before every tool (`always-ask`; the UI keeps the `yolo` default, DMN-074)
    /// through the protocol, then opens it from the left column; returns its id.
    pub async fn create_asking_thread(&self, workspace: &str) -> anyhow::Result<String> {
        self.create_thread_with(workspace, Some(ApprovalMode::AlwaysAsk))
            .await
    }

    async fn create_thread_with(
        &self,
        workspace: &str,
        approval_mode: Option<ApprovalMode>,
    ) -> anyhow::Result<String> {
        let mut connection = self.connect().await?;
        let thread = connection
            .call::<ThreadsCreate>(CreateThreadParams {
                title: None,
                workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                    name: Some(workspace.to_owned()),
                    link: None,
                }),
                model: None,
                thinking_level: None,
                approval_mode,
                prompt: None,
                images: Vec::new(),
            })
            .await?;
        connection.close().await?;
        self.click(&row(&thread.id)).await?;
        self.opened(&thread.id).await?;
        Ok(thread.id)
    }

    /// Waits for a specific thread's fake omp, never a short-lived discovery process.
    pub async fn fake_process(
        &self,
        launcher: &FakeLauncher,
        thread: &str,
    ) -> anyhow::Result<FakeProcess> {
        self.until("the thread's omp", async |_| {
            Ok(fake_thread(launcher, thread))
        })
        .await
    }

    /// Types `typed` into the path field `input` (a CSS selector) and picks the offered option for `path`: a
    /// suggestion or a directory completion (DMN-021).
    pub async fn pick_path(&self, input: &str, typed: &str, path: &str) -> anyhow::Result<()> {
        self.type_into(input, typed).await?;
        self.click(&format!(
            "[data-testid=path-option][data-path={}]",
            json!(path)
        ))
        .await
    }

    /// Waits until thread `id` shows in the chat and its composer is ready.
    pub async fn opened(&self, id: &str) -> anyhow::Result<()> {
        let chat = format!("[data-testid=thread][data-thread-id=\"{id}\"]");
        self.until(&format!("thread {id} to open"), async |page| {
            let shown = page.count(&chat).await? == 1;
            let composer = page.count("[data-testid=composer-input]").await? == 1;
            Ok((shown && composer).then_some(()))
        })
        .await
    }

    /// Sends `text` from the composer with Enter.
    pub async fn send(&self, text: &str) -> anyhow::Result<()> {
        let input = self.element("[data-testid=composer-input]").await?;
        input.send_keys(text).await?;
        input.send_keys(Key::Enter).await?;
        Ok(())
    }

    /// Queues `text` from the composer as a follow-up with Ctrl+Enter (UI-061); while idle it sends like Enter.
    pub async fn queue(&self, text: &str) -> anyhow::Result<()> {
        let input = self.element("[data-testid=composer-input]").await?;
        input.send_keys(text).await?;
        self.driver
            .action_chain()
            .key_down(Key::Control)
            .send_keys(Key::Enter)
            .key_up(Key::Control)
            .perform()
            .await?;
        Ok(())
    }

    /// Waits until the assistant's messages include one containing `needle`.
    pub async fn reply(&self, needle: &str) -> anyhow::Result<()> {
        self.until(&format!("a reply containing {needle:?}"), async |page| {
            let replies = page
                .texts("[data-testid=message][data-role=assistant]")
                .await?;
            Ok(replies
                .iter()
                .any(|reply| reply.contains(needle))
                .then_some(()))
        })
        .await
    }

    /// Waits until the thread `thread_id` shows `status` in the thread list (`idle`, `running`, `awaiting`, …).
    pub async fn status(&self, thread_id: &str, status: &str) -> anyhow::Result<()> {
        let css = row(thread_id);
        self.until(
            &format!("thread {thread_id} to be {status}"),
            async |page| {
                let statuses = page.attrs(&css, "data-status").await?;
                Ok((statuses.first().cloned().flatten().as_deref() == Some(status)).then_some(()))
            },
        )
        .await
    }

    /// Waits until the composer shows the selected thread busy.
    pub async fn busy(&self) -> anyhow::Result<()> {
        self.until("the busy composer", async |page| {
            let busy = page.attrs("[data-testid=composer]", "data-busy").await?;
            Ok((busy == vec![Some("true".to_owned())]).then_some(()))
        })
        .await
    }

    /// Runs `text` on the `:` command line from normal mode and waits until the line closes.
    pub async fn command(&self, text: &str) -> anyhow::Result<()> {
        self.eval(
            "if (document.activeElement) { document.activeElement.blur(); }",
            Vec::new(),
        )
        .await?;
        self.keys(":").await?;
        self.until_focused("command-line-input").await?;
        self.keys(text).await?;
        self.keys(Key::Enter).await?;
        self.gone("[data-testid=command-line]").await
    }

    /// Switches the colour scheme with `:theme <scheme>` (`light`, `dark`, `system`) from normal mode and waits until
    /// the page applies it.
    pub async fn scheme(&self, scheme: &str) -> anyhow::Result<()> {
        self.command(&format!("theme {scheme}")).await?;
        self.until_scheme(scheme).await
    }

    /// The folder of thread `thread_id`'s workspace, asked from the daemon.
    pub async fn workspace_folder(&self, thread_id: &str) -> anyhow::Result<PathBuf> {
        let mut connection = self.connect().await?;
        let git = connection
            .call::<omfp_protocol::method::ThreadGit>(omfp_protocol::ThreadRef {
                thread_id: thread_id.to_owned(),
            })
            .await?;
        let workspace = connection
            .call::<omfp_protocol::method::WorkspacesRescan>(omfp_protocol::WorkspaceRef {
                workspace_id: git.workspace_id,
            })
            .await?;
        connection.close().await?;
        Ok(PathBuf::from(workspace.path))
    }

    /// Waits until `<html>` carries `data-color-scheme="<scheme>"`.
    pub async fn until_scheme(&self, scheme: &str) -> anyhow::Result<()> {
        self.until(&format!("the {scheme} colour scheme"), async |page| {
            let value = page.attr("html", "data-color-scheme").await?;
            Ok((value.as_deref() == Some(scheme)).then_some(()))
        })
        .await
    }

    /// The computed value of the custom property `name` on `<html>`.
    pub async fn token(&self, name: &str) -> anyhow::Result<String> {
        let value = self
            .eval(
                "return getComputedStyle(document.documentElement).getPropertyValue(arguments[0]).trim();",
                vec![json!(name)],
            )
            .await?;
        Ok(value.as_str().unwrap_or_default().to_owned())
    }

    /// Saves a screenshot of the window as `omfp-<name>.png` in [`screenshot_dir`], when there is one.
    pub async fn screenshot(&self, name: &str) -> anyhow::Result<()> {
        let Some(dir) = screenshot_dir() else {
            return Ok(());
        };
        let path = dir.join(format!("omfp-{name}.png"));
        self.driver.screenshot(&path).await?;
        eprintln!("screenshot: {}", path.display());
        Ok(())
    }

    /// Fails when the first element matching `css` scrolls sideways (`scrollWidth > clientWidth`), naming the
    /// descendants that stick out on the right.
    pub async fn fits(&self, css: &str) -> anyhow::Result<()> {
        let value = self
            .eval(
                "const e = document.querySelector(arguments[0]);
                 const right = e.getBoundingClientRect().left + e.clientLeft + e.clientWidth;
                 const out = [...e.querySelectorAll('*')]
                   .filter(d => d.getBoundingClientRect().right > right + 0.5)
                   .map(d => `${d.tagName.toLowerCase()}[${d.dataset.testid || d.className || ''}]`
                             + ` ${Math.round(d.getBoundingClientRect().width)}px`);
                 return [e.scrollWidth, e.clientWidth, out.slice(0, 8)];",
                vec![json!(css)],
            )
            .await?;
        let (scroll, client, out): (u64, u64, Vec<String>) = serde_json::from_value(value)?;
        if scroll > client {
            bail!(
                "{css} overflows sideways: scrollWidth {scroll} > clientWidth {client}: {out:#?}"
            );
        }
        Ok(())
    }

    /// Fails unless every composer box (`.omfp-compose`) under `css` starts its text where its bar's chips start
    /// (UI-337): the content edges of the text area and of the bar, half a pixel apart at most. Returns how many
    /// composers it checked.
    pub async fn composer_aligned(&self, css: &str) -> anyhow::Result<usize> {
        let value = self
            .eval(
                "const start = (e) => e.getBoundingClientRect().left + e.clientLeft
                   + parseFloat(getComputedStyle(e).paddingLeft);
                 return [...document.querySelectorAll(arguments[0])].map((box) => [
                   box.getBoundingClientRect().left,
                   start(box.querySelector('textarea')),
                   start(box.querySelector('.omfp-compose__bar')),
                 ]);",
                vec![json!(format!("{css} .omfp-compose"))],
            )
            .await?;
        let composers: Vec<(f64, f64, f64)> = serde_json::from_value(value)?;
        for (edge, text, chips) in &composers {
            if (text - chips).abs() > 0.5 {
                bail!(
                    "a composer under {css} (box at x {edge}) starts its text at x {text} and its bar's chips at x {chips}"
                );
            }
        }
        Ok(composers.len())
    }

    /// Stops the browser and the daemon, then checks that the page showed no error, nothing reached the network and
    /// the model got only scripted requests. Prints how long the journey took.
    pub async fn finish(mut self, name: &str) -> anyhow::Result<()> {
        let errors = self
            .texts("[data-testid=error], [data-testid=widget-error]")
            .await?;
        if !errors.is_empty() {
            bail!("the page shows errors: {errors:#?}");
        }
        self.driver.clone().quit().await?;
        if let Some(daemon) = self.daemon.take() {
            daemon.shutdown().await?;
        }
        let stray = self.proxy.unexpected_attempts(KNOWN_DISCOVERY_HOSTS);
        if !stray.is_empty() {
            bail!("stray network access: {stray:#?}");
        }
        if !self.fake.failures().is_empty() {
            bail!("unexpected model requests: {:#?}", self.fake.failures());
        }
        eprintln!("{name}: {:.1} s", self.started.elapsed().as_secs_f64());
        Ok(())
    }
}

/// Where journeys put their screenshots for a look by eye: `OMFP_SCREENSHOT_DIR`, a new directory per run under
/// `just test-e2e`; without it they take none (nothing compares pixels).
#[must_use]
pub fn screenshot_dir() -> Option<PathBuf> {
    std::env::var_os("OMFP_SCREENSHOT_DIR").map(PathBuf::from)
}

/// The system prompt of a recorded model request (its `system`/`developer` messages).
#[must_use]
pub fn system_prompt(request: &Value) -> String {
    request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"] == "system" || message["role"] == "developer")
        .map(|message| match &message["content"] {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .collect()
}

/// The `<workspace-roots>` list of a system prompt, one path per entry.
#[must_use]
pub fn workspace_roots(prompt: &str) -> Vec<String> {
    let Some(start) = prompt.find("<workspace-roots>") else {
        return Vec::new();
    };
    let end = prompt[start..]
        .find("</workspace-roots>")
        .map_or(prompt.len(), |end| end + start);
    prompt[start..end]
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(str::to_owned)
        .collect()
}

/// The in-process daemon's logs in the test's output, which nextest shows for a failed journey: `RUST_LOG`, else
/// warnings and errors. A second journey in one process keeps the subscriber of the first.
fn trace_daemon() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .try_init();
}

async fn start_daemon(
    env: &OmpEnv,
    data: &Path,
    bundle: &Path,
    launcher: Option<FakeLauncher>,
    git: Option<Arc<FakeGit>>,
) -> anyhow::Result<DaemonHandle> {
    let omp = omp_bin();
    let path = |path: &Path| path.to_string_lossy().into_owned();
    let cli = Cli::try_parse_from([
        "daemon".to_owned(),
        "--bind".to_owned(),
        "127.0.0.1:0".to_owned(),
        "--data-dir".to_owned(),
        path(data),
        "--workspaces-root".to_owned(),
        path(&data.join("workspaces")),
        "--omp-bin".to_owned(),
        omp,
        "--static-dir".to_owned(),
        path(bundle),
        "--no-title-generation".to_owned(),
        "--config".to_owned(),
        path(&config_file(data)),
    ])?;
    let config = Config::from_cli(cli, None, None).map_err(anyhow::Error::msg)?;
    let ports = if let Some(launcher) = launcher {
        Ports {
            launcher: Arc::new(launcher),
            git: git.unwrap_or_else(|| Arc::new(FakeGit::new())),
            omp_program: config.omp_bin.clone().unwrap_or_default(),
            omp_version: omfp_core::OMP_VERSION.to_owned(),
            omp_env: Vec::new(),
            warnings: Vec::new(),
            clock: Arc::new(omfp_daemon::jobs::declared::SystemClock),
        }
    } else {
        let mut ports = Ports::resolve(&config)?;
        ports.omp_env = env.prepare_omfp_profile(data)?;
        ports
    };
    start_with(config, ports).await
}

fn config_file(data: &Path) -> PathBuf {
    data.join("config.toml")
}

/// Headless Chromium with its own profile; every non-loopback request goes to a closed proxy port. The page's console
/// is kept for [`Journey::report`].
fn chrome(profile: &Path) -> anyhow::Result<ChromeCapabilities> {
    let mut caps = DesiredCapabilities::chrome();
    if let Some(binary) = std::env::var_os("CHROME_BIN") {
        caps.set_binary(&binary.to_string_lossy())?;
    }
    for arg in [
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--window-size=1400,1000",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-sync",
        "--disable-default-apps",
        "--proxy-server=http://127.0.0.1:9",
        // A mouse, not a touch screen: headless Chromium reports `hover: none`, which would give every journey the
        // touch rules (UI-205); the touch journeys emulate touch over CDP on top (TST-116).
        "--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4",
        // A fake microphone, granted without a prompt (live voice, E2E-14).
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
    ] {
        caps.add_arg(arg)?;
    }
    caps.add_arg(&format!("--user-data-dir={}", profile.display()))?;
    caps.set_browser_log_level(thirtyfour::LoggingPrefsLogLevel::All)?;
    Ok(caps)
}

#[cfg(test)]
mod tests {
    use super::ready_port;

    #[test]
    fn chromedriver_reports_the_port_it_picked() {
        assert_eq!(
            ready_port("ChromeDriver was started successfully on port 43063."),
            Some(43063)
        );
        assert_eq!(ready_port("Starting ChromeDriver 157.0 on port 0"), None);
        assert_eq!(ready_port("Only local connections are allowed."), None);
    }
}
