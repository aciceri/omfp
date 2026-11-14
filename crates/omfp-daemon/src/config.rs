//! Daemon configuration (02-daemon, "Startup and configuration"): the command line over the `[daemon]` table of the
//! config file over the defaults (DMN-009).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;

use crate::config_file::{ConfigFile, DaemonOptions};
use crate::server::parse_origin;

/// Default `--bind` (DMN-002).
pub const DEFAULT_BIND: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 7420);

/// Command line of `omfp daemon`. Options left out come from the config file's `[daemon]` table, else their
/// defaults (DMN-009).
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "daemon",
    about = "Runs the omfp daemon: owns threads, workspaces and omp processes, serves the UI"
)]
pub struct Cli {
    /// Config file (DMN-009; default: `$XDG_CONFIG_HOME/omfp/config.toml`, else `~/.config/omfp/config.toml`). It need
    /// not exist.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Address and port to listen on (DMN-002; default: `127.0.0.1:7420`). Exposure beyond the machine is the
    /// deployer's job (D-13).
    #[arg(long)]
    pub bind: Option<SocketAddr>,
    /// Data directory (default: `$XDG_DATA_HOME/omfp`, else `~/.local/share/omfp`; macOS: `~/Library/Application
    /// Support/omfp`).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Where new workspace folders are made (DMN-020; default: `~/.omfp`).
    #[arg(long)]
    pub workspaces_root: Option<PathBuf>,
    /// omp executable; default: the bundled one, else `omp` on PATH (DMN-006).
    #[arg(long)]
    pub omp_bin: Option<PathBuf>,
    /// Directory with the web UI bundle to serve; default: the `web/` bundle next to the executable (DMN-008).
    #[arg(long)]
    pub static_dir: Option<PathBuf>,
    /// Run even if omp is not the pinned release (DMN-001); the UI shows a warning.
    #[arg(long)]
    pub allow_omp_mismatch: bool,
    /// Minutes before an idle, unwatched thread's omp is stopped (DMN-034; default: 15).
    #[arg(long)]
    pub idle_timeout_minutes: Option<u64>,
    /// Maximum number of omp processes running at once (DMN-038; default: 8).
    #[arg(long)]
    pub max_processes: Option<usize>,
    /// Events kept in memory per thread so reconnecting clients can catch up (DMN-041; default: 2000).
    #[arg(long)]
    pub event_buffer: Option<usize>,
    /// An `Origin` allowed to open the WebSocket besides the daemon's own (PRT-003, DMN-140), exactly
    /// `http(s)://host[:port]`, e.g. the UI's URL behind a tunnel or reverse proxy; repeat for more. Replaces the
    /// config file's `allowed_origins`; `*` is refused.
    #[arg(long = "allowed-origin", value_name = "ORIGIN")]
    pub allowed_origins: Vec<String>,
    /// Theme file followed by the daemon (DMN-075; default: `$XDG_CONFIG_HOME/omfp/theme.css`, else
    /// `~/.config/omfp/theme.css`).
    #[arg(long)]
    pub theme_file: Option<PathBuf>,
    /// Do not generate thread titles with an extra omp print-mode run (DMN-073); threads keep the placeholder from
    /// their first message until renamed. In the config file: `title_generation = false`.
    #[arg(long)]
    pub no_title_generation: bool,
}

/// Resolved daemon settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub workspaces_root: PathBuf,
    pub omp_bin: Option<PathBuf>,
    pub static_dir: Option<PathBuf>,
    pub allow_omp_mismatch: bool,
    pub idle_timeout: Duration,
    pub max_processes: usize,
    pub event_buffer: usize,
    /// Origins allowed to upgrade besides the daemon's own (DMN-140), normalized as a browser sends them (DMN-141).
    pub allowed_origins: Vec<String>,
    /// The theme file to follow (DMN-075); `None` follows none. [`Config::from_cli`] takes `--theme-file` or the
    /// config file's `theme_file`; `omfp daemon` ([`crate::run`]) adds the default location.
    pub theme_file: Option<PathBuf>,
    /// The user's init file to follow and serve (DMN-185); `None` follows none. [`Config::from_cli`]
    /// leaves it `None`; `omfp daemon` ([`crate::run`]) sets the default location.
    pub init_file: Option<PathBuf>,
    /// Generate and periodically refresh automatic titles after meaningful new conversation (DMN-073).
    pub title_generation: bool,
    /// The config file the daemon follows for `[ui]` / `[keymap]` and writes `settings.set` into (DMN-009); `None`
    /// keeps UI settings in memory only. [`Config::from_cli`] takes `--config`; `omfp daemon` adds the default.
    pub config_file: Option<PathBuf>,
    /// `[daemon]` as the file held at startup: a reload that changes it needs a restart.
    pub file_options: DaemonOptions,
}

/// `$XDG_CONFIG_HOME/omfp` when absolute, else `$HOME/.config/omfp`: where the config and theme files live.
#[must_use]
pub fn config_dir(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg_config_home
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))
        .map(|base| base.join("omfp"))
}

/// Where the default data directory lives under `HOME` without `XDG_DATA_HOME` (DMN-170).
#[cfg(target_os = "macos")]
pub const HOME_DATA: &str = "Library/Application Support";
/// Where the default data directory lives under `HOME` without `XDG_DATA_HOME`.
#[cfg(not(target_os = "macos"))]
pub const HOME_DATA: &str = ".local/share";

/// The default data directory: `$XDG_DATA_HOME/omfp` when absolute, else `$HOME/`[`HOME_DATA`]`/omfp`
/// (DMN-170: `~/Library/Application Support/omfp` on macOS, `~/.local/share/omfp` elsewhere).
#[must_use]
pub fn default_data_dir(xdg_data_home: Option<PathBuf>, home: Option<&Path>) -> Option<PathBuf> {
    xdg_data_home
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(HOME_DATA)))
        .map(|base| base.join("omfp"))
}

/// The config file's default location (DMN-009).
#[must_use]
pub fn default_config_file(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    config_dir(xdg_config_home, home).map(|dir| dir.join("config.toml"))
}

impl Config {
    /// Merges the command line over `--config` (when given and present) over the defaults, which depend on the
    /// environment (`XDG_DATA_HOME`, `HOME`; `~` in file paths is `HOME`). An invalid config file is an error.
    pub fn from_cli(
        cli: Cli,
        xdg_data_home: Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> Result<Self, String> {
        let file = match &cli.config {
            Some(path) => ConfigFile::read(path)?.unwrap_or_default(),
            None => ConfigFile::default(),
        };
        let file_options = file.daemon.clone();
        let options = file.daemon.expanded(home.as_deref());
        let data_dir = match cli.data_dir.or(options.data_dir) {
            Some(dir) => dir,
            None => default_data_dir(xdg_data_home, home.as_deref()).ok_or(
                "cannot determine a data directory: set --data-dir, XDG_DATA_HOME or HOME",
            )?,
        };
        let workspaces_root = match cli.workspaces_root.or(options.workspaces_root) {
            Some(dir) => dir,
            None => home
                .map(|home| home.join(".omfp"))
                .ok_or("cannot determine the workspaces root: set --workspaces-root or HOME")?,
        };
        let max_processes = cli.max_processes.or(options.max_processes).unwrap_or(8);
        if max_processes == 0 {
            return Err("--max-processes must be at least 1".into());
        }
        let idle_minutes = cli
            .idle_timeout_minutes
            .or(options.idle_timeout_minutes)
            .unwrap_or(15);
        let allowed_origins = if cli.allowed_origins.is_empty() {
            let file = cli
                .config
                .as_deref()
                .unwrap_or(Path::new("the config file"));
            origins(
                options.allowed_origins.as_deref().unwrap_or_default(),
                &format!("`allowed_origins` in {}", file.display()),
            )?
        } else {
            origins(&cli.allowed_origins, "--allowed-origin")?
        };
        Ok(Self {
            bind: cli.bind.or(options.bind).unwrap_or(DEFAULT_BIND),
            data_dir,
            workspaces_root,
            omp_bin: cli.omp_bin.or(options.omp_bin),
            static_dir: cli.static_dir.or(options.static_dir),
            allow_omp_mismatch: cli.allow_omp_mismatch
                || options.allow_omp_mismatch.unwrap_or(false),
            idle_timeout: Duration::from_secs(idle_minutes.saturating_mul(60)),
            max_processes,
            event_buffer: cli.event_buffer.or(options.event_buffer).unwrap_or(2000),
            allowed_origins,
            theme_file: cli.theme_file.or(options.theme_file),
            init_file: None,
            title_generation: !cli.no_title_generation && options.title_generation.unwrap_or(true),
            config_file: cli.config,
            file_options,
        })
    }
}

/// The allow-list entries normalized (DMN-141); the first invalid one fails startup, named with its `source`.
fn origins(entries: &[String], source: &str) -> Result<Vec<String>, String> {
    entries
        .iter()
        .map(|entry| parse_origin(entry).map_err(|error| format!("{source}: {error}")))
        .collect()
}

/// Name of the web bundle directory shipped next to the `omfp` executable (packaging, DMN-008).
pub const BUNDLED_WEB_DIR: &str = "web";

/// The web UI to serve (DMN-008): `--static-dir`, else the bundle next to the executable, else the one in the
/// `Resources/` directory beside the executable's directory (the release `omfp.app`, whose `Contents/MacOS` holds code
/// only, ENV-063), the first that holds an `index.html`; `None` serves only the API.
#[must_use]
pub fn web_bundle(explicit: Option<&Path>, exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        return Some(explicit.to_path_buf());
    }
    let exe_dir = exe_dir?;
    let resources = exe_dir
        .parent()
        .map(|contents| contents.join("Resources").join(BUNDLED_WEB_DIR));
    std::iter::once(exe_dir.join(BUNDLED_WEB_DIR))
        .chain(resources)
        .find(|bundle| bundle.join("index.html").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("daemon").chain(args.iter().copied())).unwrap()
    }

    fn config(args: &[&str]) -> Result<Config, String> {
        Config::from_cli(parse(args), None, Some("/home/u".into()))
    }

    // spec: DMN-002
    #[test]
    fn listens_on_loopback_by_default_and_accepts_any_address() {
        assert_eq!(config(&[]).unwrap().bind, "127.0.0.1:7420".parse().unwrap());
        assert_eq!(
            config(&["--bind", "0.0.0.0:9000"]).unwrap().bind,
            "0.0.0.0:9000".parse().unwrap()
        );
    }

    /// A config file with `text`, and `--config` pointing at it.
    fn with_file(text: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let path = path.to_str().unwrap().to_owned();
        (dir, path)
    }

    // spec: DMN-009
    #[test]
    fn flags_win_over_the_config_file_which_wins_over_the_defaults() {
        let (_dir, file) = with_file(
            r#"
# Comments are fine.
[daemon]
bind = "0.0.0.0:9000"
data_dir = "~/omfp-data"
workspaces_root = "/srv/ws"
idle_timeout_minutes = 30
max_processes = 3
allowed_origins = ["https://omfp.example.com"]
theme_file = "~/themes/mine.css"
title_generation = false
allow_omp_mismatch = true
"#,
        );
        let from_file = config(&["--config", &file]).unwrap();
        assert_eq!(from_file.bind, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(from_file.data_dir, PathBuf::from("/home/u/omfp-data"));
        assert_eq!(from_file.workspaces_root, PathBuf::from("/srv/ws"));
        assert_eq!(from_file.idle_timeout, Duration::from_mins(30));
        assert_eq!(from_file.max_processes, 3);
        assert_eq!(from_file.event_buffer, 2000, "not in the file: the default");
        assert_eq!(from_file.allowed_origins, ["https://omfp.example.com"]);
        assert_eq!(
            from_file.theme_file,
            Some(PathBuf::from("/home/u/themes/mine.css"))
        );
        assert!(!from_file.title_generation);
        assert!(from_file.allow_omp_mismatch);
        assert_eq!(from_file.config_file, Some(PathBuf::from(&file)));

        let flags = config(&[
            "--config",
            &file,
            "--bind",
            "127.0.0.1:1",
            "--max-processes",
            "5",
            "--allowed-origin",
            "https://other.example",
            "--data-dir",
            "/d",
        ])
        .unwrap();
        assert_eq!(flags.bind, "127.0.0.1:1".parse().unwrap());
        assert_eq!(flags.max_processes, 5);
        assert_eq!(flags.allowed_origins, ["https://other.example"]);
        assert_eq!(flags.data_dir, PathBuf::from("/d"));
        assert_eq!(flags.idle_timeout, Duration::from_mins(30));
    }

    // spec: DMN-009
    #[test]
    fn an_invalid_config_file_is_an_error_naming_the_key_and_a_missing_one_is_empty() {
        let (_dir, file) = with_file("[daemon]\nbnid = \"0.0.0.0:1\"\n");
        let error = config(&["--config", &file]).unwrap_err();
        assert!(error.contains("bnid"), "{error}");
        assert!(error.contains(&file), "{error}");
        let (_dir, file) = with_file("[daemon]\nmax_processes = 0\n");
        assert!(config(&["--config", &file]).is_err());

        let missing = config(&["--config", "/nonexistent/omfp/config.toml"]).unwrap();
        assert_eq!(missing.bind, DEFAULT_BIND);
        assert_eq!(
            missing.config_file,
            Some(PathBuf::from("/nonexistent/omfp/config.toml"))
        );
        assert_eq!(
            default_config_file(None, Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/omfp/config.toml"))
        );
    }

    // spec: DMN-140, DMN-009
    #[test]
    fn allowed_origins_come_from_the_repeatable_flag_else_the_config_file() {
        assert_eq!(config(&[]).unwrap().allowed_origins, Vec::<String>::new());
        let (_dir, file) = with_file(
            "[daemon]\nallowed_origins = [\"https://omfp.example.ts.net/\", \"HTTP://100.64.0.7:7420\"]\n",
        );
        assert_eq!(
            config(&["--config", &file]).unwrap().allowed_origins,
            ["https://omfp.example.ts.net", "http://100.64.0.7:7420"],
            "normalized as a browser sends them"
        );
        assert_eq!(
            config(&[
                "--config",
                &file,
                "--allowed-origin",
                "https://a.example",
                "--allowed-origin",
                "https://b.example:8443",
            ])
            .unwrap()
            .allowed_origins,
            ["https://a.example", "https://b.example:8443"],
            "the flags replace the file's list"
        );
    }

    // spec: DMN-141
    #[test]
    fn an_invalid_allowed_origin_fails_startup_naming_the_entry_and_where_it_is() {
        let error = config(&[
            "--allowed-origin",
            "https://ok.example",
            "--allowed-origin",
            "*",
        ])
        .unwrap_err();
        assert!(error.starts_with("--allowed-origin: `*`"), "{error}");
        assert!(error.contains("wildcards are not allowed"), "{error}");

        let (_dir, file) = with_file("[daemon]\nallowed_origins = [\"omfp.example.ts.net\"]\n");
        let error = config(&["--config", &file]).unwrap_err();
        assert!(
            error.contains(&format!("`allowed_origins` in {file}")),
            "{error}"
        );
        assert!(error.contains("`omfp.example.ts.net`"), "{error}");
        assert!(error.contains("http:// or https://"), "{error}");
        assert!(
            config(&["--config", &file, "--allowed-origin", "https://ok.example"]).is_ok(),
            "a list replaced by the flags is not checked"
        );

        let (_dir, file) = with_file("[daemon]\nallow_origin = [\"https://omfp.example.com\"]\n");
        let error = config(&["--config", &file]).unwrap_err();
        assert!(
            error.contains("allow_origin"),
            "the old key is unknown: {error}"
        );
    }

    // spec: DMN-170
    #[test]
    fn data_dir_defaults_to_xdg_then_home() {
        let config = |xdg: Option<&str>, home: Option<&str>| {
            Config::from_cli(parse(&[]), xdg.map(PathBuf::from), home.map(PathBuf::from))
        };
        assert_eq!(
            config(Some("/xdg"), Some("/home/u")).unwrap().data_dir,
            PathBuf::from("/xdg/omfp")
        );
        let home_data = if cfg!(target_os = "macos") {
            "/home/u/Library/Application Support/omfp"
        } else {
            "/home/u/.local/share/omfp"
        };
        assert_eq!(
            config(Some("relative"), Some("/home/u")).unwrap().data_dir,
            PathBuf::from(home_data)
        );
        assert_eq!(
            config(None, Some("/home/u")).unwrap().data_dir,
            PathBuf::from(home_data)
        );
        assert!(config(None, None).is_err());
        let explicit = Config::from_cli(
            parse(&["--data-dir", "/d", "--workspaces-root", "/w"]),
            None,
            None,
        )
        .unwrap();
        assert_eq!(explicit.data_dir, PathBuf::from("/d"));
        assert_eq!(explicit.workspaces_root, PathBuf::from("/w"));
    }

    // spec: DMN-020
    #[test]
    fn workspace_folders_go_under_a_hidden_home_folder_by_default() {
        let config = Config::from_cli(parse(&[]), None, Some("/home/u".into())).unwrap();
        assert_eq!(config.workspaces_root, PathBuf::from("/home/u/.omfp"));
        assert!(Config::from_cli(parse(&["--data-dir", "/d"]), None, None).is_err());
    }

    // spec: DMN-034, DMN-038, DMN-041
    #[test]
    fn idle_timeout_process_limit_and_event_buffer_have_spec_defaults() {
        let config = Config::from_cli(parse(&[]), None, Some("/h".into())).unwrap();
        assert_eq!(config.idle_timeout, Duration::from_mins(15));
        assert_eq!(config.max_processes, 8);
        assert_eq!(config.event_buffer, 2000);
        assert!(
            Config::from_cli(parse(&["--max-processes", "0"]), None, Some("/h".into())).is_err()
        );
    }

    // spec: DMN-073, DMN-075
    #[test]
    fn titles_are_generated_and_only_an_explicit_theme_file_is_followed_by_default() {
        let config = Config::from_cli(parse(&[]), None, Some("/h".into())).unwrap();
        assert!(config.title_generation);
        assert_eq!(config.theme_file, None);
        let config = Config::from_cli(
            parse(&["--no-title-generation", "--theme-file", "/t.css"]),
            None,
            Some("/h".into()),
        )
        .unwrap();
        assert!(!config.title_generation);
        assert_eq!(config.theme_file, Some(PathBuf::from("/t.css")));
    }

    // spec: DMN-008
    #[test]
    fn the_web_bundle_is_the_static_dir_else_the_one_next_to_the_executable() {
        let exe_dir = tempfile::tempdir().unwrap();
        let explicit = Path::new("/srv/omfp-web");
        assert_eq!(
            web_bundle(Some(explicit), Some(exe_dir.path())),
            Some(explicit.to_path_buf())
        );
        // No bundle (or an empty directory): the API only.
        assert_eq!(web_bundle(None, Some(exe_dir.path())), None);
        let bundle = exe_dir.path().join(BUNDLED_WEB_DIR);
        std::fs::create_dir(&bundle).unwrap();
        assert_eq!(web_bundle(None, Some(exe_dir.path())), None);
        std::fs::write(bundle.join("index.html"), "<!doctype html>").unwrap();
        assert_eq!(web_bundle(None, Some(exe_dir.path())), Some(bundle));
        assert_eq!(web_bundle(None, None), None);
    }

    // spec: DMN-008, ENV-063
    #[test]
    fn the_web_bundle_of_an_app_bundle_is_in_its_resources() {
        let app = tempfile::tempdir().unwrap();
        let macos = app.path().join("Contents/MacOS");
        let resources = app.path().join("Contents/Resources/web");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        assert_eq!(web_bundle(None, Some(&macos)), None);
        std::fs::write(resources.join("index.html"), "<!doctype html>").unwrap();
        assert_eq!(web_bundle(None, Some(&macos)), Some(resources));
        // `web/` next to the executable comes first.
        let next_to = macos.join(BUNDLED_WEB_DIR);
        std::fs::create_dir(&next_to).unwrap();
        std::fs::write(next_to.join("index.html"), "<!doctype html>").unwrap();
        assert_eq!(web_bundle(None, Some(&macos)), Some(next_to));
    }
}
