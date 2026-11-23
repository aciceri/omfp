//! Finding or starting the local daemon of a data directory (ARC-003, D-16): the daemon's own command line and config
//! file decide its data directory and bind address; a daemon started here runs detached in the background, in the tray.

use std::ffi::OsString;
use std::net::{SocketAddr, TcpListener};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use clap::Parser as _;
use omfp_daemon::config::{Cli, Config, default_config_file};
use omfp_daemon::paths::{DataDir, create_private_dir, open_private_log};

use super::Env;

/// How long a started daemon gets to write `daemon.json`.
const START_DEADLINE: Duration = Duration::from_secs(30);

/// How to find or start the daemon of the data directory (ARC-003): the daemon's own command line and config file
/// decide its data directory and bind address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDaemon {
    /// Absolute data directory.
    pub data_dir: PathBuf,
    /// The configured or default address (DMN-002).
    pub bind: SocketAddr,
    /// `--bind` was passed after `--`: never replaced.
    pub explicit_bind: bool,
    /// Arguments for `omfp daemon` (without the subcommand).
    pub daemon_args: Vec<OsString>,
}

impl LocalDaemon {
    /// Resolves the daemon's settings like `omfp daemon` would; `data_dir` is `omfp desktop --data-dir`.
    pub fn plan(
        data_dir: &Option<PathBuf>,
        daemon_args: &[String],
        env: &Env,
    ) -> anyhow::Result<Self> {
        let mut cli = Cli::try_parse_from(
            std::iter::once("daemon").chain(daemon_args.iter().map(String::as_str)),
        )
        .map_err(|error| anyhow::anyhow!("daemon arguments after `--`: {error}"))?;
        if data_dir.is_some() && cli.data_dir.is_some() {
            bail!("--data-dir is given twice: pass it before `--` only");
        }
        let mut args: Vec<OsString> = Vec::new();
        if let Some(dir) = data_dir {
            cli.data_dir = Some(dir.clone());
        }
        let explicit_bind = cli.bind.is_some();
        cli.config = cli
            .config
            .or_else(|| default_config_file(env.xdg_config_home.clone(), env.home.clone()));
        let config = Config::from_cli(cli, env.xdg_data_home.clone(), env.home.clone())
            .map_err(anyhow::Error::msg)?;
        let data_dir = std::path::absolute(&config.data_dir)?;
        args.extend(["--data-dir".into(), data_dir.clone().into_os_string()]);
        args.extend(
            daemon_args
                .iter()
                .scan(false, |skip, arg| {
                    // The data directory goes first, resolved: drop its `--data-dir` from the passed arguments.
                    let keep = !*skip && arg != "--data-dir" && !arg.starts_with("--data-dir=");
                    *skip = arg == "--data-dir";
                    Some(keep.then(|| OsString::from(arg)))
                })
                .flatten(),
        );
        Ok(Self {
            data_dir,
            bind: config.bind,
            explicit_bind,
            daemon_args: args,
        })
    }

    /// The config of the smoke test's in-process daemon (TST-033): on a random port unless `--bind` was passed.
    pub(super) fn in_process_config(&self) -> anyhow::Result<Config> {
        let mut cli = Cli::try_parse_from(
            std::iter::once(OsString::from("daemon")).chain(self.daemon_args.iter().cloned()),
        )?;
        if !self.explicit_bind {
            cli.bind = Some(SocketAddr::new(self.bind.ip(), 0));
        }
        let env = Env::current();
        Config::from_cli(cli, env.xdg_data_home, env.home).map_err(anyhow::Error::msg)
    }

    /// The command line of the background daemon: in the tray (DMN-128).
    #[must_use]
    pub fn command_line(&self, bind: Option<SocketAddr>) -> Vec<OsString> {
        let mut line = vec![OsString::from("daemon"), OsString::from("--tray")];
        if let Some(bind) = bind {
            line.extend(["--bind".into(), bind.to_string().into()]);
        }
        line.extend(self.daemon_args.iter().cloned());
        line
    }

    /// The address of the running daemon, after starting one in the background when none runs: on the configured
    /// address first, so the window's origin (and its `localStorage`) stays the same across restarts, else on a
    /// random port (ARC-003). A daemon restarting in place is waited for, never replaced (DMN-392).
    pub fn find_or_start(&self) -> anyhow::Result<SocketAddr> {
        if let Some(info) =
            omfp_daemon::discover_through_restart(&self.data_dir, omfp_daemon::RESTART_DEADLINE)?
        {
            note!(
                "omfp desktop: found the daemon (pid {}) on {}",
                info.pid,
                info.addr
            );
            return Ok(info.addr);
        }
        let mut bind = None;
        loop {
            match self.start(bind)? {
                Started::Ready(addr) => return Ok(addr),
                Started::Exited(status) => {
                    if bind.is_none()
                        && let Some(fallback) =
                            fallback_bind(self.bind, self.explicit_bind, port_in_use(self.bind))
                    {
                        note!(
                            "omfp desktop: {} is taken, starting the daemon on {fallback}",
                            self.bind
                        );
                        bind = Some(fallback);
                        continue;
                    }
                    bail!(
                        "omfp daemon exited ({status}), see {}",
                        self.log().display()
                    );
                }
            }
        }
    }

    fn log(&self) -> PathBuf {
        DataDir::new(&self.data_dir).daemon_log()
    }

    /// Starts `omfp daemon` detached (D-16): its own process group (no terminal signals), stdin `/dev/null`, output
    /// appended to `<data-dir>/daemon.log`, reaped by a thread of this process while it lives. Waits until a daemon
    /// runs for the data directory (this one, or another that won a start race).
    fn start(&self, bind: Option<SocketAddr>) -> anyhow::Result<Started> {
        if let Some(mode) = create_private_dir(&self.data_dir)
            .with_context(|| format!("cannot create {}", self.data_dir.display()))?
        {
            note!(
                "omfp desktop: {} was open to other users (mode {mode:o}): its mode is now 0700",
                self.data_dir.display()
            );
        }
        let path = self.log();
        let log =
            open_private_log(&path).with_context(|| format!("cannot open {}", path.display()))?;
        let mut child = Command::new(std::env::current_exe()?)
            .args(self.command_line(bind))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0)
            .spawn()
            .context("cannot start omfp daemon")?;
        note!(
            "omfp desktop: started the daemon (pid {}), log in {}",
            child.id(),
            path.display()
        );
        let started = self.wait_ready(&mut child);
        std::thread::spawn(move || child.wait());
        started
    }

    fn wait_ready(&self, child: &mut Child) -> anyhow::Result<Started> {
        let deadline = Instant::now() + START_DEADLINE;
        loop {
            if let Some(info) = omfp_daemon::discover(&self.data_dir)? {
                return Ok(Started::Ready(info.addr));
            }
            // A child that lost a start race exits while the winner, which holds the lock, is still starting.
            if let Some(status) = child.try_wait()?
                && !omfp_daemon::lock_held(&self.data_dir)?
            {
                return Ok(Started::Exited(status));
            }
            if Instant::now() > deadline {
                bail!(
                    "omfp daemon did not start in {} s, see {}",
                    START_DEADLINE.as_secs(),
                    self.log().display()
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

enum Started {
    Ready(SocketAddr),
    Exited(ExitStatus),
}

/// Where to retry a background daemon that exited without starting (ARC-003): on a random port of the same
/// interface when the configured port is taken; never when `--bind` was given or the port was already random.
#[must_use]
pub fn fallback_bind(bind: SocketAddr, explicit: bool, in_use: bool) -> Option<SocketAddr> {
    (!explicit && bind.port() != 0 && in_use).then(|| SocketAddr::new(bind.ip(), 0))
}

fn port_in_use(bind: SocketAddr) -> bool {
    TcpListener::bind(bind).is_err_and(|error| error.kind() == std::io::ErrorKind::AddrInUse)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: ARC-003
    #[test]
    fn a_taken_configured_port_falls_back_to_a_random_one() {
        let default = omfp_daemon::config::DEFAULT_BIND;
        assert_eq!(
            fallback_bind(default, false, true),
            Some("127.0.0.1:0".parse().unwrap())
        );
        assert_eq!(
            fallback_bind("0.0.0.0:7420".parse().unwrap(), false, true),
            Some("0.0.0.0:0".parse().unwrap())
        );
        // Another reason to exit, an explicit `--bind`, or a random port already: no retry.
        assert_eq!(fallback_bind(default, false, false), None);
        assert_eq!(fallback_bind(default, true, true), None);
        assert_eq!(
            fallback_bind("127.0.0.1:0".parse().unwrap(), false, true),
            None
        );

        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(port_in_use(taken.local_addr().unwrap()));
        let free = taken.local_addr().unwrap();
        drop(taken);
        assert!(!port_in_use(free));
    }
}
