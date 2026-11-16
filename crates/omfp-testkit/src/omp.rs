use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use omp_rpc::{Client, ClientOptions, Event};
use regex::Regex;
use serde_json::Value;
use tempfile::TempDir;

use crate::DenyProxy;

/// Provider id of the fake LLM in the isolated `models.yml`.
pub const FAKE_PROVIDER: &str = "fake";
/// Model id served by the fake LLM.
pub const FAKE_MODEL: &str = "scripted";

/// omp executable under test: `OMFP_OMP_BIN`, else `omp` from `PATH`.
#[must_use]
pub fn omp_bin() -> String {
    std::env::var("OMFP_OMP_BIN").unwrap_or_else(|_| "omp".to_owned())
}

/// Version printed by `omp --version` (`omp/18.18.6` → `18.18.6`).
pub fn omp_version() -> anyhow::Result<String> {
    let output = Command::new(omp_bin())
        .arg("--version")
        .output()
        .with_context(|| {
            format!(
                "running `{} --version`; is the dev shell active?",
                omp_bin()
            )
        })?;
    let stdout = String::from_utf8(output.stdout)?;
    let line = stdout.lines().next().unwrap_or_default().trim();
    Ok(line.rsplit('/').next().unwrap_or(line).to_owned())
}

/// Fails fast when the omp under test is not the pinned release (TST-013).
pub fn require_pinned_omp() {
    let found =
        omp_version().unwrap_or_else(|error| panic!("cannot determine omp version: {error:#}"));
    assert_eq!(
        found,
        omfp_core::OMP_VERSION,
        "contract tests need omp {} (omfp_core::OMP_VERSION) but `{}` is {found}; enter the dev shell (`nix develop`)",
        omfp_core::OMP_VERSION,
        omp_bin(),
    );
}

/// An isolated omp installation: its own `HOME`, agent directory (`models.yml`, `config.yml`, sessions) and working
/// directory, with the fake LLM as the only model provider. Removed when dropped.
#[derive(Debug)]
pub struct OmpEnv {
    root: TempDir,
    proxy_url: Option<String>,
    profile_agent: OnceLock<PathBuf>,
}

impl OmpEnv {
    /// Creates the environment; `fake_base_url` is [`fake_llm::FakeLlm::base_url`]. With `proxy`, every non-loopback
    /// HTTP(S) request goes to the [`DenyProxy`] and is refused.
    pub fn new(fake_base_url: &str, proxy: Option<&DenyProxy>) -> anyhow::Result<Self> {
        let env = Self {
            root: tempfile::Builder::new().prefix("omfp-omp-").tempdir()?,
            proxy_url: proxy.map(DenyProxy::url),
            profile_agent: OnceLock::new(),
        };
        for dir in [env.agent_dir(), env.work_dir(), env.root().join("xdg")] {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            env.agent_dir().join("models.yml"),
            models_yml(fake_base_url),
        )?;
        std::fs::write(env.agent_dir().join("config.yml"), CONFIG_YML)?;
        Ok(env)
    }

    /// Rewrites `config.yml` before omp starts: `roles` extend `modelRoles` (e.g. `("web", "web/searxng")`) and
    /// `settings` (top-level YAML keys, e.g. `searxng:\n  endpoint: …\n`) follow the base settings.
    pub fn configure(&self, roles: &[(&str, &str)], settings: &str) -> anyhow::Result<()> {
        let mut config = CONFIG_YML.to_owned();
        for (role, model) in roles {
            writeln!(config, "  {role}: {model}")?;
        }
        config.push_str(settings);
        std::fs::write(self.agent_dir().join("config.yml"), config)?;
        Ok(())
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    #[must_use]
    pub fn home(&self) -> PathBuf {
        self.root().join("home")
    }

    #[must_use]
    pub fn agent_dir(&self) -> PathBuf {
        self.profile_agent
            .get()
            .cloned()
            .unwrap_or_else(|| self.home().join(".omp").join("agent"))
    }

    /// Places this test's synthetic fixtures in a daemon's isolated profile before startup. This is test setup,
    /// not application migration: production never moves or copies standalone credentials/configuration.
    pub fn prepare_omfp_profile(
        &self,
        data: &Path,
    ) -> anyhow::Result<Vec<(OsString, Option<OsString>)>> {
        let agent = data.join("omp-profile/agent");
        if let Some(current) = self.profile_agent.get() {
            anyhow::ensure!(
                current == &agent,
                "test environment already belongs to another daemon"
            );
        } else {
            std::fs::create_dir_all(data.join("omp-profile"))?;
            std::fs::rename(self.agent_dir(), &agent)?;
            self.profile_agent
                .set(agent)
                .map_err(|_| anyhow::anyhow!("test profile already prepared"))?;
        }
        Ok(self.env_overrides())
    }

    /// Default working directory for omp processes.
    #[must_use]
    pub fn work_dir(&self) -> PathBuf {
        self.root().join("work")
    }

    /// `omp <args>` with the isolated environment ([`OmpEnv::env_overrides`]) and cwd = [`OmpEnv::work_dir`].
    #[must_use]
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(omp_bin());
        command.args(args).current_dir(self.work_dir());
        for (key, value) in self.env_overrides() {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        command
    }

    /// Environment changes that isolate omp: own `HOME`, agent and XDG directories, no provider credentials
    /// (`*_API_KEY`, `*_TOKEN`, `AWS_*`, `OMP_*`, `PI_*` removed: `None`; EC2 instance-metadata credentials off,
    /// TST-022), and the deny proxy when requested.
    #[must_use]
    pub fn env_overrides(&self) -> Vec<(OsString, Option<OsString>)> {
        self.overrides_of(std::env::vars_os())
    }

    /// [`OmpEnv::env_overrides`] of a process whose environment is `inherited`.
    fn overrides_of(
        &self,
        inherited: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Vec<(OsString, Option<OsString>)> {
        let mut overrides: Vec<(OsString, Option<OsString>)> = inherited
            .into_iter()
            .filter(|(name, _)| {
                let name = name.to_string_lossy();
                name.ends_with("_API_KEY")
                    || name.ends_with("_TOKEN")
                    || name.starts_with("AWS_")
                    || name.starts_with("OMP_")
                    || name.starts_with("PI_")
            })
            .map(|(name, _)| (name, None))
            .collect();
        let xdg = self.root().join("xdg");
        let mut set = |key: &str, value: OsString| overrides.push((key.into(), Some(value)));
        set("HOME", self.home().into());
        set("PI_CODING_AGENT_DIR", self.agent_dir().into());
        set("XDG_CONFIG_HOME", xdg.join("config").into());
        set("XDG_DATA_HOME", xdg.join("data").into());
        set("XDG_STATE_HOME", xdg.join("state").into());
        set("XDG_CACHE_HOME", xdg.join("cache").into());
        set("NO_PROXY", "127.0.0.1,localhost".into());
        set("no_proxy", "127.0.0.1,localhost".into());
        // omp takes an EC2 host (its DMI vendor) for an instance-role AWS credential and then counts Amazon Bedrock's
        // models as usable: GitHub's AWS-hosted runners are such hosts (TST-022).
        set("AWS_EC2_METADATA_DISABLED", "true".into());
        if let Some(proxy) = &self.proxy_url {
            for name in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "http_proxy",
                "https_proxy",
                "ALL_PROXY",
                "all_proxy",
            ] {
                set(name, proxy.into());
            }
        }
        overrides
    }
}

fn models_yml(base_url: &str) -> String {
    format!(
        "providers:
  {FAKE_PROVIDER}:
    baseUrl: {base_url}
    auth: none
    api: openai-completions
    models:
      - id: {FAKE_MODEL}
        name: Scripted fake model
        reasoning: false
        input: [text]
        contextWindow: 128000
        maxTokens: 8192
"
    )
}

/// Settings of the isolated environment. omp still runs its background model-catalog discovery at startup (no setting
/// turns it off, S-02); the [`DenyProxy`] refuses those requests, see [`crate::KNOWN_DISCOVERY_HOSTS`].
const CONFIG_YML: &str = "modelRoles:
  default: fake/scripted
";

/// A live `omp --mode rpc-ui` process driven through `omp-rpc`, recording every raw stdout frame.
pub struct OmpSession {
    client: Option<Client>,
    events: Receiver<Event>,
    child: Child,
    frames: Arc<Mutex<Vec<u8>>>,
    stderr_path: PathBuf,
}

impl std::fmt::Debug for OmpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OmpSession")
            .field("pid", &self.child.id())
            .field("stderr_path", &self.stderr_path)
            .finish_non_exhaustive()
    }
}

impl OmpSession {
    /// Starts `omp --mode rpc-ui --model fake/scripted <extra_args>` in `env` and waits for the `ready` frame.
    pub fn start(env: &OmpEnv, extra_args: &[&str]) -> anyhow::Result<Self> {
        let model = format!("{FAKE_PROVIDER}/{FAKE_MODEL}");
        let mut args = vec!["--mode", "rpc-ui", "--model", model.as_str()];
        args.extend_from_slice(extra_args);
        let stderr_path = env.root().join("omp-stderr.log");
        let mut command = env.command(&args);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&stderr_path)?);
        let mut child = command.spawn().context("spawning omp")?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            bail!("omp started without piped stdio");
        };
        let frames = Arc::new(Mutex::new(Vec::new()));
        let reader = TeeReader {
            inner: stdout,
            copy: Arc::clone(&frames),
        };
        match Client::from_io(reader, stdin, ClientOptions::default()) {
            Ok((client, events)) => Ok(Self {
                client: Some(client),
                events,
                child,
                frames,
                stderr_path,
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
                bail!("omp did not become ready: {error}\nstderr:\n{stderr}")
            }
        }
    }

    #[must_use]
    pub fn client(&self) -> &Client {
        self.client.as_ref().expect("client is present until drop")
    }

    /// Frames not consumed by the client (notifications, unmatched responses).
    #[must_use]
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Waits until a frame of `kind` (its `type`) has been received, failing after `timeout`.
    pub fn wait_for_frame(&self, kind: &str, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        while !self.frames().iter().any(|frame| frame["type"] == kind) {
            if Instant::now() >= deadline {
                bail!(
                    "no `{kind}` frame within {timeout:?}\nstderr:\n{}",
                    self.stderr()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Every complete stdout line received so far, parsed as JSON.
    #[must_use]
    pub fn frames(&self) -> Vec<Value> {
        let bytes = self
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let text = String::from_utf8_lossy(&bytes);
        let complete = text.rsplit_once('\n').map_or("", |(complete, _)| complete);
        complete
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|_| Value::String(line.to_owned()))
            })
            .collect()
    }

    /// omp's stderr so far, for failure messages.
    #[must_use]
    pub fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr_path).unwrap_or_default()
    }

    /// With `OMFP_RECORD_FIXTURES` set, writes the recorded frames to `fixtures/omp/<OMP_VERSION>/<name>.jsonl`,
    /// normalized by [`normalize_frame`] so re-recording only shows real protocol changes (TST-012).
    pub fn record_fixture(&self, env: &OmpEnv, name: &str) -> anyhow::Result<Option<PathBuf>> {
        if std::env::var_os("OMFP_RECORD_FIXTURES").is_none() {
            return Ok(None);
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/omp")
            .join(omfp_core::OMP_VERSION);
        std::fs::create_dir_all(&dir)?;
        let root = env.root().to_string_lossy().into_owned();
        let mut out = String::new();
        for mut frame in self.frames() {
            normalize_frame(&mut frame, &root);
            out.push_str(&frame.to_string());
            out.push('\n');
        }
        let path = dir.join(format!("{name}.jsonl"));
        std::fs::write(&path, out)?;
        Ok(Some(path))
    }
}

/// Numeric fields that differ on every run (wall-clock times and latencies; `*Ms` of subagent progress), or between
/// environments: the fake LLM counts prompt tokens from the request size, which varies with the environment (e.g.
/// the length of the temporary paths in the prompt): `promptTokens` of a context snapshot, `contextTokens`/`tokens`
/// of subagent progress, and [`PROMPT_SIZED_USAGE`].
const VOLATILE_NUMBERS: &[&str] = &[
    "timestamp",
    "completedAt",
    "startedAt",
    "ttft",
    "duration",
    "t_ns",
    "dur_ns",
    "start_ns",
    "end_ns",
    "ns",
    "at",
    "wallTimeMs",
    "durationMs",
    "totalDurationMs",
    "currentToolStartMs",
    "endMs",
    "promptTokens",
    "contextTokens",
    "tokens",
];

/// Fields of a `usage` object that follow the request size (see [`VOLATILE_NUMBERS`]); `output` stays.
const PROMPT_SIZED_USAGE: &[&str] = &["input", "totalTokens"];

/// Makes a recorded frame stable across runs: volatile numbers (and the prompt-sized counts of `usage`) become `0`,
/// random 16-hex-digit request ids become `<id>`, and inside strings the temporary `root` becomes `<root>`, UUIDs
/// become `<uuid>`, session-file timestamps become `<time>` and the `duration="…"` attribute of a `<task-result>`
/// becomes `duration="<duration>"`.
pub fn normalize_frame(value: &mut Value, root: &str) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if VOLATILE_NUMBERS.contains(&key.as_str()) && field.is_number() {
                    *field = Value::from(0);
                } else if (key == "id" || key == "targetId")
                    && field.as_str().is_some_and(is_random_id)
                {
                    *field = Value::from("<id>");
                } else {
                    if key == "usage"
                        && let Value::Object(usage) = field
                    {
                        for name in PROMPT_SIZED_USAGE {
                            if let Some(count) =
                                usage.get_mut(*name).filter(|count| count.is_number())
                            {
                                *count = Value::from(0);
                            }
                        }
                    }
                    normalize_frame(field, root);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| normalize_frame(item, root)),
        Value::String(text) => {
            let mut normalized = if root.is_empty() {
                text.clone()
            } else {
                text.replace(root, "<root>")
            };
            normalized = UUID.replace_all(&normalized, "<uuid>").into_owned();
            normalized = SESSION_TIME.replace_all(&normalized, "<time>").into_owned();
            normalized = TASK_DURATION
                .replace_all(&normalized, r#"$1"<duration>""#)
                .into_owned();
            normalized = WALL_TIME
                .replace_all(&normalized, "Wall time: <duration>")
                .into_owned();
            *text = normalized;
        }
        _ => {}
    }
}

static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        .expect("valid regex")
});
/// `2026-11-10T11-02-28-652Z`, as used in omp session file names.
static SESSION_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z").expect("valid regex")
});
/// `<task-result id="…" agent="…" status="…" duration="63ms">`, the summary of a subagent run.
static TASK_DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(<task-result [^>]*duration=)"[^"]*""#).expect("valid regex"));
/// `Wall time: 1.10 seconds`, the footer of a `bash` result.
static WALL_TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Wall time: [0-9.]+ \w+").expect("valid regex"));

fn is_random_id(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl Drop for OmpSession {
    fn drop(&mut self) {
        // Closing the client closes omp's stdin; omp then drains and exits on its own.
        drop(self.client.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Passes stdout through to the client while keeping a copy of every byte.
struct TeeReader<R> {
    inner: R,
    copy: Arc<Mutex<Vec<u8>>>,
}

impl<R: Read> Read for TeeReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.copy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(&buffer[..read]);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_frame_blanks_volatile_values_only() {
        let mut frame = json!({
            "type": "message_end",
            "id": "15c88d181ed15103",
            "messageId": "msg-1",
            "message": {
                "timestamp": 1_794_306_420_199_u64,
                "usage": { "ttft": 3.7, "duration": 4.7, "input": 8503, "output": 7, "totalTokens": 8510 },
                "contextSnapshot": { "promptTokens": 8506, "toolSchemaTokens": 5099 },
                "text": "hi"
            },
            "cwd": "/tmp/omfp-omp-abc/work",
            "items": [ { "completedAt": 5, "id": "not-random" } ],
            "progress": { "currentToolStartMs": 1_794_356_027_239_u64, "contextTokens": 7177, "tokens": 7177, "recentTools": [ { "endMs": 1_794_356_027_241_u64 } ] },
            "input": 3,
            "text": "<task-result id=\"Counter\" agent=\"task\" status=\"completed\" duration=\"63ms\">\n<output>\nduration=\"5s\"\n</output>\n\nWall time: 1.10 seconds",
            "timestamp": "kept-because-not-a-number",
        });

        normalize_frame(&mut frame, "/tmp/omfp-omp-abc");

        assert_eq!(
            frame,
            json!({
                "type": "message_end",
                "id": "<id>",
                "messageId": "msg-1",
                "message": {
                    "timestamp": 0,
                    "usage": { "ttft": 0, "duration": 0, "input": 0, "output": 7, "totalTokens": 0 },
                    "contextSnapshot": { "promptTokens": 0, "toolSchemaTokens": 5099 },
                    "text": "hi"
                },
                "cwd": "<root>/work",
                "items": [ { "completedAt": 0, "id": "not-random" } ],
                "progress": { "currentToolStartMs": 0, "contextTokens": 0, "tokens": 0, "recentTools": [ { "endMs": 0 } ] },
                "input": 3,
                "text": "<task-result id=\"Counter\" agent=\"task\" status=\"completed\" duration=\"<duration>\">\n<output>\nduration=\"5s\"\n</output>\n\nWall time: <duration>",
                "timestamp": "kept-because-not-a-number",
            })
        );
    }

    #[test]
    fn isolated_env_points_omp_at_the_fake_provider_only() {
        let env = OmpEnv::new("http://127.0.0.1:9/v1", None).unwrap();

        let models = std::fs::read_to_string(env.agent_dir().join("models.yml")).unwrap();
        assert!(models.contains("baseUrl: http://127.0.0.1:9/v1"));
        assert!(models.contains("api: openai-completions"));

        let command = env.command(&["--version"]);
        let envs: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let get = |key: &str| {
            envs.iter()
                .find(|(k, _)| k == key)
                .and_then(|(_, v)| v.clone())
        };
        assert_eq!(get("HOME").unwrap(), env.home().to_string_lossy());
        assert_eq!(
            get("PI_CODING_AGENT_DIR").unwrap(),
            env.agent_dir().to_string_lossy()
        );
        assert_eq!(get("HTTPS_PROXY"), None, "no proxy unless requested");
        assert_eq!(command.get_current_dir(), Some(env.work_dir().as_path()));
    }

    // spec: TST-022
    #[test]
    fn isolated_env_hides_every_aws_credential_of_the_host() {
        let env = OmpEnv::new("http://127.0.0.1:9/v1", None).unwrap();
        let inherited = [
            ("AWS_ACCESS_KEY_ID", "AKIAHOST"),
            ("AWS_SECRET_ACCESS_KEY", "host-secret"),
            ("AWS_PROFILE", "host"),
            (
                "AWS_EC2_METADATA_SERVICE_ENDPOINT",
                "http://169.254.169.254",
            ),
            ("AWS_EC2_METADATA_DISABLED", "false"),
            ("PATH", "/bin"),
        ]
        .map(|(name, value)| (OsString::from(name), OsString::from(value)));

        // Applied in order, as `Command` and the daemon's launch spec do: the last change of a variable wins.
        let effective: std::collections::BTreeMap<_, _> =
            env.overrides_of(inherited).into_iter().collect();

        let value = |name: &str| effective.get(std::ffi::OsStr::new(name)).cloned();
        for name in [
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_PROFILE",
            "AWS_EC2_METADATA_SERVICE_ENDPOINT",
        ] {
            assert_eq!(value(name), Some(None), "{name} must be removed");
        }
        // On an EC2 host (GitHub's AWS-hosted runners) omp otherwise takes the instance role for a credential.
        assert_eq!(
            value("AWS_EC2_METADATA_DISABLED"),
            Some(Some("true".into()))
        );
        assert_eq!(value("PATH"), None, "other variables pass through");
    }

    #[tokio::test]
    async fn with_a_deny_proxy_every_proxy_variable_points_at_it() {
        let proxy = DenyProxy::start().await.unwrap();
        let env = OmpEnv::new("http://127.0.0.1:9/v1", Some(&proxy)).unwrap();

        let command = env.command(&[]);
        let value = |key: &str| {
            command
                .get_envs()
                .find(|(k, _)| k.to_string_lossy() == key)
                .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(value("HTTPS_PROXY").unwrap(), proxy.url());
        assert_eq!(value("HTTP_PROXY").unwrap(), proxy.url());
        assert_eq!(value("NO_PROXY").unwrap(), "127.0.0.1,localhost");
    }
}
