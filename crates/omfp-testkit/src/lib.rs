//! Test helpers shared by the workspace (docs/specs/06-testing.md).
//!
//! - [`TempRepo`]: a throwaway git repository with an initial commit.
//! - [`DenyProxy`]: an HTTP(S) proxy that refuses and records every connection, to catch stray network access.
//! - [`OmpEnv`]: an isolated omp home and agent directory wired to a [`fake_llm::FakeLlm`].
//! - [`OmpSession`]: a live `omp --mode rpc-ui` process with every raw stdout frame recorded.
//! - [`absent_standalone_omp`], [`standalone_omp_env`]: the user's own omp a test daemon looks at (`Ports::user_omp_env`).

use std::ffi::OsString;
use std::path::Path;

mod deny_proxy;
mod omp;
mod repo;

pub use deny_proxy::DenyProxy;
pub use omp::{
    FAKE_MODEL, FAKE_PROVIDER, OmpEnv, OmpSession, normalize_frame, omp_bin, omp_version,
    require_pinned_omp,
};
pub use repo::TempRepo;

/// Hosts the pinned omp contacts on its own at startup for background model-catalog discovery (S-02): the shared
/// models.dev mirror and the model lists of keyless providers. No setting disables it, so contract tests run omp
/// behind a [`DenyProxy`] that refuses them, and fail on any host outside this list (TST-021). Review this list on
/// every omp upgrade.
pub const KNOWN_DISCOVERY_HOSTS: &[&str] = &[
    "catalog.stencil.so",
    "coding-intl.dashscope.aliyuncs.com",
    "hyper.charm.land",
    "api.commandcode.ai",
    "api.kilo.ai",
    "api.venice.ai",
    "zenmux.ai",
];

/// The environment variables the daemon reads to find standalone omp's credential store (DMN-229).
const STANDALONE_OMP_VARIABLES: &[&str] = &[
    "PI_CONFIG_DIR",
    "OMP_PROFILE",
    "PI_PROFILE",
    "PI_CODING_AGENT_DIR",
    "XDG_DATA_HOME",
];

/// `Ports::user_omp_env` of a standalone omp at `home`: `HOME` set to it and every other variable the daemon reads to
/// find standalone omp's store unset, so nothing inherited points at the developer's own omp (TST-003).
#[must_use]
pub fn standalone_omp_env(home: &Path) -> Vec<(OsString, Option<OsString>)> {
    std::iter::once(("HOME".into(), Some(home.as_os_str().to_owned())))
        .chain(
            STANDALONE_OMP_VARIABLES
                .iter()
                .map(|key| (OsString::from(key), None)),
        )
        .collect()
}

/// `Ports::user_omp_env` of a test daemon without a standalone omp: [`standalone_omp_env`] at `<dir>/standalone-home`,
/// which a test may fill with a store (`.omp/agent/agent.db`) or leave absent.
#[must_use]
pub fn absent_standalone_omp(dir: impl AsRef<Path>) -> Vec<(OsString, Option<OsString>)> {
    standalone_omp_env(&dir.as_ref().join("standalone-home"))
}
