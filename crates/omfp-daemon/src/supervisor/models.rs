//! What the daemon knows of the models omp can use (DMN-235..DMN-237, DMN-914): `auth.status` and the `index` topic's
//! `models`. omp exits at startup when it has none (`No default model selected`), so the count comes from a
//! short-lived omp started like the login omp (DMN-219). The first `auth.status` checks; later ones answer the known
//! count, which a login, a logout, a thread start contradicting it and `auth.status { recheck }` (an API key added to
//! the profile's `.env`) check again.

use std::path::PathBuf;

use tokio::sync::watch;
use tracing::warn;

use super::{Result, Supervisor};
use crate::omp::LaunchSpec;

/// omp's words when it refuses to start without a usable model (DMN-212).
const NO_MODEL: &str = "No default model selected";

/// The known count and the check that sets it.
pub(super) struct UsableModels {
    /// `None` until a check finished.
    known: watch::Sender<Option<u32>>,
    /// Held while a check runs: one at a time, and a caller that waited for it reads its count.
    check: tokio::sync::Mutex<()>,
}

impl Default for UsableModels {
    fn default() -> Self {
        Self {
            known: watch::channel(None).0,
            check: tokio::sync::Mutex::new(()),
        }
    }
}

impl Supervisor {
    /// The count of usable models as it changes; `None` until a check finished (DMN-235).
    #[must_use]
    pub fn subscribe_usable_models(&self) -> watch::Receiver<Option<u32>> {
        self.inner.models.known.subscribe()
    }

    /// `auth.status` (DMN-235): the known count of usable models, else the count a check finds; callers arriving
    /// during a check wait for it instead of starting another.
    pub async fn usable_models(&self) -> Result<u32> {
        if let Some(models) = *self.inner.models.known.borrow() {
            return Ok(models);
        }
        let _check = self.inner.models.check.lock().await;
        if let Some(models) = *self.inner.models.known.borrow() {
            return Ok(models);
        }
        self.check_usable_models().await
    }

    /// `auth.status { recheck }` (DMN-914): checks the usable models again and waits for the count; a caller arriving
    /// during a check waits for it and answers its count.
    pub async fn recheck_usable_models_now(&self) -> Result<u32> {
        let mut known = self.inner.models.known.subscribe();
        known.borrow_and_update();
        let _check = self.inner.models.check.lock().await;
        if known.has_changed().unwrap_or(false)
            && let Some(models) = *known.borrow_and_update()
        {
            return Ok(models);
        }
        self.check_usable_models().await
    }

    /// The `.env` file of omfp's omp profile, which every omp the daemon starts reads for API keys (DMN-914).
    #[must_use]
    pub fn keys_file(&self) -> PathBuf {
        let profile = self.inner.data.omp_profile();
        std::path::absolute(&profile)
            .unwrap_or(profile)
            .join("agent")
            .join(".env")
    }

    /// Checks the usable models again in the background (DMN-236, DMN-237), unless nobody asked yet: with no count
    /// known and none being checked, the next `auth.status` checks anyway.
    pub fn recheck_usable_models(&self) {
        let models = &self.inner.models;
        if models.known.borrow().is_none() && models.check.try_lock().is_ok() {
            return;
        }
        let supervisor = self.clone();
        tokio::spawn(async move {
            let _check = supervisor.inner.models.check.lock().await;
            if let Err(error) = supervisor.check_usable_models().await {
                warn!(%error, "cannot check the usable models");
            }
        });
    }

    /// A thread's omp started from `spec` (DMN-237): without a model of its own, omp found a usable one, which
    /// contradicts a known count of none.
    pub(super) fn started_with(&self, spec: &LaunchSpec) {
        if spec.model.is_none() {
            self.contradicts_usable_models(true);
        }
    }

    /// A thread's omp did not start (DMN-237): omp's refusal without a usable model contradicts a known count of
    /// some.
    pub(super) fn failed_with(&self, stderr_tail: &str) {
        if stderr_tail.contains(NO_MODEL) {
            self.contradicts_usable_models(false);
        }
    }

    fn contradicts_usable_models(&self, usable: bool) {
        let known = *self.inner.models.known.borrow();
        if known.is_some_and(|models| (models > 0) != usable) {
            self.recheck_usable_models();
        }
    }

    /// Runs the check (the caller holds `check`) and publishes its count.
    async fn check_usable_models(&self) -> Result<u32> {
        let models = self
            .on_login_omp(|launcher, spec| launcher.usable_models(spec))
            .await?;
        self.inner.models.known.send_replace(Some(models));
        Ok(models)
    }
}
