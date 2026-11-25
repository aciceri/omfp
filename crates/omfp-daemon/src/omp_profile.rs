//! The mandatory omp user-state boundary (ARC-021). HOME and project discovery stay unchanged; native
//! config lookup and mutable runtime directories are pinned to the daemon's own data directory.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::config::DATA_DIR_VARIABLE;
use crate::paths::DataDir;

type Environment = Vec<(OsString, Option<OsString>)>;

/// Environment for every daemon-owned omp, including the version probe, auth and print-mode runs. Call after
/// caller-supplied overrides: neither an inherited named profile nor an agent/mutable-XDG override may select
/// standalone omp state. Explicit provider API keys still work; host XDG configuration remains available to tools.
pub fn environment(data: &DataDir, mut env: Environment) -> std::io::Result<Environment> {
    let inherited_home = std::env::var_os("HOME");
    let home = crate::conventions::omp_home(&env, inherited_home.as_deref())
        .filter(|home| home.is_absolute())
        .ok_or_else(|| std::io::Error::other("omp profile requires an absolute HOME"))?;
    let root = std::path::absolute(data.omp_profile())?;
    // omp 19.2.0 joins PI_CONFIG_DIR to HOME even when it is absolute. Use a home-relative spelling so its
    // generic config helpers resolve the same root as getAgentDir(); changing only PI_CODING_AGENT_DIR is unsafe.
    let mut config = PathBuf::new();
    let depth = home
        .components()
        .fold(0_usize, |depth, component| match component {
            Component::Normal(_) => depth + 1,
            Component::ParentDir => depth.saturating_sub(1),
            _ => depth,
        });
    for _ in 0..depth {
        config.push("..");
    }
    config.push(
        root.strip_prefix(Path::new("/"))
            .map_err(std::io::Error::other)?,
    );
    for key in [
        "PI_CONFIG_FILES",
        "OMP_CONFIG_FILES",
        "PI_CODING_AGENT_SESSION_DIR",
        "OMP_CODING_AGENT_SESSION_DIR",
        "OMP_WORKTREE_DIR",
        "OMP_GITHUB_CACHE_DB",
        "OMP_COMMIT_CACHE_DB",
        "OMP_JUDGMENT_CACHE_DB",
        "OMP_AUTH_BROKER_SNAPSHOT_CACHE",
    ] {
        env.push((key.into(), None));
    }
    let mut set = |key: &str, value: OsString| env.push((key.into(), Some(value)));
    set("HOME", home.into_os_string());
    for key in ["OMP_PROFILE", "PI_PROFILE"] {
        set(key, "default".into());
    }
    for key in ["PI_CONFIG_DIR", "OMP_CONFIG_DIR"] {
        set(key, config.clone().into_os_string());
    }
    for key in ["PI_CODING_AGENT_DIR", "OMP_CODING_AGENT_DIR"] {
        set(key, root.join("agent").into_os_string());
    }
    // The embedded native loader predates the config-root resolver and otherwise hard-codes ~/.omp/natives.
    for key in ["PI_NATIVES_DIR", "OMP_NATIVES_DIR"] {
        set(key, root.join("natives").into_os_string());
    }
    // Shell tools inherit omp's environment. Keep the host configuration root so CLIs such as gh remain usable;
    // PI_CONFIG_DIR and PI_CODING_AGENT_DIR already isolate omp's own user configuration.
    for (key, category) in [
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        set(key, root.join("xdg").join(category).into_os_string());
    }
    // A thread's shell finds its daemon by this (`omfp daemon restart`), not by the profile's XDG_DATA_HOME (DMN-393).
    set(
        DATA_DIR_VARIABLE,
        std::path::absolute(data.root())?.into_os_string(),
    );
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: ARC-021, DMN-393
    #[test]
    fn inherited_profile_and_directory_overrides_cannot_escape_the_profile() {
        let data = DataDir::new("/data/omfp");
        let inherited = vec![
            ("HOME".into(), Some("/home/user".into())),
            ("PI_CODING_AGENT_DIR".into(), Some("/standalone".into())),
            ("XDG_CONFIG_HOME".into(), Some("/home/user/.config".into())),
            ("OMP_PROFILE".into(), Some("work".into())),
            (
                "PI_CONFIG_FILES".into(),
                Some("/standalone/config.yml".into()),
            ),
            (
                "OPENAI_API_KEY".into(),
                Some("explicit-environment-key".into()),
            ),
        ];
        let env = environment(&data, inherited).unwrap();
        let value = |key: &str| {
            env.iter()
                .rev()
                .find(|(name, _)| name == key)
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(value("HOME"), Some("/home/user".into()));
        assert_eq!(
            value("PI_CONFIG_DIR"),
            Some("../../data/omfp/omp-profile".into())
        );
        assert_eq!(
            value("PI_CODING_AGENT_DIR"),
            Some("/data/omfp/omp-profile/agent".into())
        );
        assert_eq!(value("OMP_PROFILE"), Some("default".into()));
        assert_eq!(value("PI_CONFIG_FILES"), None);
        assert_eq!(
            value("XDG_DATA_HOME"),
            Some("/data/omfp/omp-profile/xdg/data".into())
        );
        assert_eq!(value("XDG_CONFIG_HOME"), Some("/home/user/.config".into()));
        assert_eq!(
            value("OPENAI_API_KEY"),
            Some("explicit-environment-key".into())
        );
        assert_eq!(value("OMFP_DATA_DIR"), Some("/data/omfp".into()));
    }

    // spec: ARC-021
    #[test]
    fn relative_home_is_rejected_instead_of_falling_back_to_standalone() {
        let env = vec![("HOME".into(), Some("relative".into()))];
        assert!(environment(&DataDir::new("/data"), env).is_err());
    }
}
