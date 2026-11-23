//! `omfp daemon restart` (DMN-391): restarts the running daemon of a data directory in place onto this `omfp`
//! executable, from any shell, a shell of one of that daemon's threads included (DMN-393).

use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser as _;
use omfp_daemon::config::{Cli, Config, DATA_DIR_VARIABLE, default_config_file};

use crate::remote::reachable;

/// `omfp daemon restart`.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// Data directory of the daemon [default: `$OMFP_DATA_DIR`, set in the shells of omfp's threads; else the
    /// daemon's: `data_dir` of the config file, `$XDG_DATA_HOME/omfp`, else `~/.local/share/omfp`, on macOS
    /// `~/Library/Application Support/omfp`].
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Config file whose `[daemon] data_dir` names the data directory (DMN-009) [default:
    /// `$XDG_CONFIG_HOME/omfp/config.toml`, else `~/.config/omfp/config.toml`].
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Runs `omfp daemon restart`; returns what to print on success.
pub fn run(args: &Args) -> anyhow::Result<String> {
    let data_dir = data_dir(args, |key| std::env::var_os(key).map(PathBuf::from))?;
    let program = program(std::env::current_exe().context("cannot find this omfp executable")?);
    let info = omfp_daemon::restart(&data_dir, &program, omfp_daemon::RESTART_DEADLINE)?;
    Ok(format!(
        "omfp daemon restarted as {} ({}): pid {}, listening on http://{}/",
        program.display(),
        omfp_daemon::version_string(),
        info.pid,
        reachable(info.addr)
    ))
}

/// What the restarted daemon runs (DMN-391): this executable, or the Nix wrapper around it (`bin/omfp` beside
/// `bin/.omfp-wrapped`), which puts its package's environment (its omp first on `PATH`, ENV-032) over the one the
/// daemon inherited from its own package.
fn program(exe: PathBuf) -> PathBuf {
    let wrapper = exe
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix('.')?.strip_suffix("-wrapped"))
        .map(|name| exe.with_file_name(name));
    wrapper.filter(|wrapper| wrapper.is_file()).unwrap_or(exe)
}

/// The data directory of the daemon to restart (DMN-393): `--data-dir`, else `$OMFP_DATA_DIR` (the profile of a
/// thread's omp moves `XDG_DATA_HOME` away from the daemon's), else as `omfp daemon` resolves it (DMN-009, DMN-170).
fn data_dir(args: &Args, var: impl Fn(&str) -> Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let mut cli = Cli::try_parse_from(["daemon"])?;
    cli.data_dir = args
        .data_dir
        .clone()
        .or_else(|| var(DATA_DIR_VARIABLE).filter(|dir| dir.is_absolute()));
    let home = var("HOME");
    cli.config = args
        .config
        .clone()
        .or_else(|| default_config_file(var("XDG_CONFIG_HOME"), home.clone()));
    let config = Config::from_cli(cli, var("XDG_DATA_HOME"), home).map_err(anyhow::Error::msg)?;
    Ok(std::path::absolute(config.data_dir)?)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    // spec: DMN-393
    #[test]
    fn the_data_directory_of_a_threads_daemon_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let profile = dir.path().join("data/omp-profile/xdg/data");
        let mut env = HashMap::from([
            ("HOME", home.clone()),
            // A thread's shell: the omp profile's XDG_DATA_HOME, and the daemon's data directory.
            ("XDG_DATA_HOME", profile.clone()),
            (DATA_DIR_VARIABLE, dir.path().join("data")),
        ]);
        let resolve = |args: &Args, env: &HashMap<&str, PathBuf>| {
            data_dir(args, |key| env.get(key).cloned()).unwrap()
        };
        assert_eq!(resolve(&Args::default(), &env), dir.path().join("data"));
        let explicit = Args {
            data_dir: Some(dir.path().join("other")),
            config: None,
        };
        assert_eq!(resolve(&explicit, &env), dir.path().join("other"));

        // Outside a thread: the config file's `data_dir`, else `$XDG_DATA_HOME/omfp`.
        env.remove(DATA_DIR_VARIABLE);
        assert_eq!(resolve(&Args::default(), &env), profile.join("omfp"));
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "[daemon]\ndata_dir = \"~/omfp-data\"\n").unwrap();
        let with_config = Args {
            data_dir: None,
            config: Some(config),
        };
        assert_eq!(resolve(&with_config, &env), home.join("omfp-data"));
    }

    // spec: DMN-391
    #[test]
    fn a_nix_package_restarts_through_its_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let wrapped = dir.path().join(".omfp-wrapped");
        std::fs::write(&wrapped, "").unwrap();
        assert_eq!(program(wrapped.clone()), wrapped);
        std::fs::write(dir.path().join("omfp"), "").unwrap();
        assert_eq!(program(wrapped), dir.path().join("omfp"));
        let plain = dir.path().join("omfp");
        assert_eq!(program(plain.clone()), plain);
    }
}
