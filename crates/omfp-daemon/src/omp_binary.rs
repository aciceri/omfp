//! Which omp the daemon runs (DMN-006) and whether it is the pinned release (DMN-001).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Binary name looked up next to the daemon and on `PATH`.
const OMP: &str = "omp";

/// Picks the omp executable: `explicit` (`--omp-bin`), else the omp bundled next to the daemon executable
/// (`exe_dir/omp`), else the first `omp` on `path` (the `PATH` value).
#[must_use]
pub fn resolve(
    explicit: Option<&Path>,
    exe_dir: Option<&Path>,
    path: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        return Some(explicit.to_path_buf());
    }
    let bundled = exe_dir.map(|dir| dir.join(OMP));
    if let Some(bundled) = bundled.filter(|candidate| is_executable(candidate)) {
        return Some(bundled);
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(OMP))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// The version in `omp --version` output (`omp/18.22.0` → `18.22.0`).
#[must_use]
pub fn parse_version(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim();
    let version = line.rsplit('/').next()?.trim();
    let parts: Vec<&str> = version.split('.').collect();
    let numeric = parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    numeric.then(|| version.to_owned())
}

/// Why the daemon refuses an omp binary.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VersionError {
    #[error("cannot run `{program} --version`: {message}")]
    Unrunnable { program: String, message: String },
    #[error("`{program} --version` printed no version: {output:?}")]
    Unparsable { program: String, output: String },
    #[error(
        "omp {found} at {program} does not match the pinned {expected}; use the bundled omp or pass --allow-omp-mismatch"
    )]
    Mismatch {
        program: String,
        found: String,
        expected: String,
    },
}

/// Decides whether `found` may be used: equal versions pass silently; a mismatch fails, or with
/// `allow_mismatch` passes with a warning that the UI shows (DMN-001).
pub fn accept(
    program: &str,
    found: &str,
    expected: &str,
    allow_mismatch: bool,
) -> Result<Option<String>, VersionError> {
    if found == expected {
        return Ok(None);
    }
    let mismatch = VersionError::Mismatch {
        program: program.to_owned(),
        found: found.to_owned(),
        expected: expected.to_owned(),
    };
    if allow_mismatch {
        Ok(Some(format!(
            "running anyway (--allow-omp-mismatch): {mismatch}"
        )))
    } else {
        Err(mismatch)
    }
}

/// Runs `program --version` and returns the version it prints.
pub fn version(
    program: &Path,
    env: &[(std::ffi::OsString, Option<std::ffi::OsString>)],
) -> Result<String, VersionError> {
    let name = program.display().to_string();
    let mut command = Command::new(program);
    command.arg("--version");
    for (key, value) in env {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    // On the host when the daemon runs in Flatpak (ENV-064).
    let output =
        crate::flatpak::on_host(command)
            .output()
            .map_err(|error| VersionError::Unrunnable {
                program: name.clone(),
                message: error.to_string(),
            })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    parse_version(&stdout).ok_or(VersionError::Unparsable {
        program: name,
        output: stdout,
    })
}

/// Runs `program --version` and applies [`accept`] against `omfp_core::OMP_VERSION`.
pub fn check(program: &Path, allow_mismatch: bool) -> Result<Option<String>, VersionError> {
    let found = version(program, &[])?;
    accept(
        &program.display().to_string(),
        &found,
        omfp_core::OMP_VERSION,
        allow_mismatch,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho omp/{}\n", omfp_core::OMP_VERSION),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    // spec: DMN-006
    #[test]
    fn explicit_binary_wins_over_bundled_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = executable(dir.path(), OMP);
        let explicit = dir.path().join("custom-omp");

        let chosen = resolve(
            Some(&explicit),
            Some(dir.path()),
            Some(dir.path().as_os_str()),
        );

        assert_eq!(chosen, Some(explicit));
        assert_ne!(chosen, Some(bundled));
    }

    // spec: DMN-006
    #[test]
    fn bundled_binary_wins_over_path() {
        let exe_dir = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        let bundled = executable(exe_dir.path(), OMP);
        executable(path_dir.path(), OMP);

        let chosen = resolve(
            None,
            Some(exe_dir.path()),
            Some(path_dir.path().as_os_str()),
        );

        assert_eq!(chosen, Some(bundled));
    }

    // spec: DMN-006
    #[test]
    fn falls_back_to_the_first_executable_omp_on_path() {
        let empty = tempfile::tempdir().unwrap();
        let not_executable = tempfile::tempdir().unwrap();
        std::fs::write(not_executable.path().join(OMP), "").unwrap();
        let good = tempfile::tempdir().unwrap();
        let expected = executable(good.path(), OMP);
        let path =
            std::env::join_paths([empty.path(), not_executable.path(), good.path()]).unwrap();

        let chosen = resolve(None, Some(empty.path()), Some(&path));

        assert_eq!(chosen, Some(expected));
        assert_eq!(resolve(None, None, None), None);
    }

    // spec: DMN-001
    #[test]
    fn parses_only_well_formed_versions() {
        assert_eq!(parse_version("omp/18.18.6\n").as_deref(), Some("18.18.6"));
        assert_eq!(parse_version("18.18.6").as_deref(), Some("18.18.6"));
        assert_eq!(parse_version("omp/18.18"), None);
        assert_eq!(parse_version("omp/v18.18.6"), None);
        assert_eq!(parse_version(""), None);
    }

    // spec: DMN-001
    #[test]
    fn a_mismatch_is_refused_unless_explicitly_allowed_with_a_warning() {
        assert_eq!(accept("omp", "18.18.6", "18.18.6", false), Ok(None));
        assert!(matches!(
            accept("omp", "18.18.5", "18.18.6", false),
            Err(VersionError::Mismatch { .. })
        ));
        let warning = accept("omp", "18.18.5", "18.18.6", true).unwrap().unwrap();
        assert!(
            warning.contains("18.18.5") && warning.contains("18.18.6"),
            "{warning}"
        );
    }

    // spec: DMN-001
    #[test]
    fn check_runs_the_binary_and_compares_with_the_pinned_version() {
        let dir = tempfile::tempdir().unwrap();
        let fake = executable(dir.path(), OMP);

        assert_eq!(check(&fake, false), Ok(None));
        assert!(matches!(
            check(&dir.path().join("missing"), false),
            Err(VersionError::Unrunnable { .. })
        ));
    }
}
