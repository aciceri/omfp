//! Params and results of logging omp in to a provider (PRT-129..PRT-133, PRT-140): `auth.status`, `auth.providers`,
//! `auth.login`, `auth.login_cancel` and `auth.logout`. The flow of a login reaches a window as `login_*` window
//! requests ([`crate::WindowRequest`]).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Discovery result shared by `auth.models` and `thread.models`. The wire result remains omp's full model record;
/// this schema describes its stable selection fields and permits upstream extension fields.
#[derive(Debug, JsonSchema)]
pub struct AvailableModels {
    pub models: Vec<AvailableModel>,
}

/// Selection fields of an omp model descriptor; other upstream model capabilities may also be present.
#[derive(Debug, JsonSchema)]
pub struct AvailableModel {
    /// Provider identifier. Create with `threads.create { model: "<provider>/<id>", ... }`.
    pub provider: String,
    /// Model identifier. Change with `thread.set_model { provider: "<provider>", modelId: "<id>", ... }`.
    pub id: String,
    /// Human-readable model name, not a selector.
    pub name: String,
}

/// `auth.status` result (PRT-140): whether omp has a model it can use, as the daemon last checked it (DMN-235).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    /// How many models omp can use: of a provider it holds a credential for (a login, a key from the environment or
    /// omp's config) or of a keyless one. `0` on a fresh install, where every thread start fails until a login.
    pub models: u32,
}

/// `auth.providers` result (PRT-129): the providers omp can log in to with OAuth, in omp's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthProviders {
    pub providers: Vec<AuthProvider>,
}

/// One provider of `auth.providers` (PRT-129), as omp's `get_login_providers` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthProvider {
    /// omp's id, e.g. `anthropic`, `openai-codex`: what `auth.login` takes.
    pub id: String,
    /// What the user reads, e.g. `Anthropic (Claude Pro/Max)`.
    pub name: String,
    /// omp can run its login here.
    pub available: bool,
    /// omp holds a credential for it (a login, or a key from the environment or omp's config).
    pub authenticated: bool,
}

/// `auth.login` and `auth.logout` params: the provider's omp id (PRT-130, PRT-133).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRef {
    pub provider_id: String,
}

/// `auth.login` result (PRT-130): omp stored the provider's credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedIn {
    pub provider_id: String,
    /// The provider's name, as `auth.providers` gives it.
    pub name: String,
}

/// `auth.login_cancel` result (PRT-132).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginCancelled {
    /// The provider whose login was cancelled; `null` when none ran.
    pub provider_id: Option<String>,
}

/// The login running now, as every window sees it on the `index` topic (DMN-290): which provider, and the latest of
/// its flow, so a window that did not start it can show it (an agent's `auth.login`, another window's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginActivity {
    pub provider_id: String,
    /// The provider's name, as `auth.providers` gives it.
    pub name: String,
    /// The provider's page omp opened (its `login_url`); `null` until omp opened one.
    pub url: Option<String>,
    /// What the provider's page asks of the user, as omp words it; `null` when omp says nothing.
    pub instructions: Option<String>,
    /// omp's latest progress message (its `login_progress`); `null` before the first.
    pub progress: Option<String>,
}

/// `auth.logout` result (PRT-133).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedOut {
    pub provider_id: String,
    /// How many stored credentials (logins and stored keys) of the provider omp removed.
    pub removed: u32,
    /// What still authenticates the provider once they are gone, as omp names it (e.g. an environment variable);
    /// `null` when nothing does.
    pub remaining_source: Option<String>,
}
