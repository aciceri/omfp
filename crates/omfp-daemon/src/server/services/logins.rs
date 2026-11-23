//! `auth.*` (PRT-129..PRT-133, PRT-136, PRT-137, PRT-140, DMN-219..DMN-224, DMN-229, DMN-230, DMN-235, DMN-236):
//! omp's OAuth logins, run by the short-lived login omp of the supervisor (`supervisor/login.rs`), standalone omp's
//! logins copied on request (`supervisor/import.rs`), and the count of models omp can use
//! (`supervisor/models.rs`). A login's flow — the URL to open, progress, a code to paste — reaches one window as
//! `login_*` window requests (PRT-131); the window answers the code with `windows.answer`. Every window sees the
//! running login, its page and its progress on the `index` topic (DMN-290).

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omfp_protocol::{
    AuthProvider, AuthProviders, AuthStatus, ErrorBody, ErrorCode, IndexEvent, LoggedIn, LoggedOut,
    LoginActivity, OmpLogins, WindowAnswer, WindowRequest,
};
use omp_rpc::{
    CancelUiResponse, Event, ExtensionUiRequest, ExtensionUiResponse, GetLoginProvidersResult,
    GetLogoutAccountsCommand, GetLogoutAccountsResult, LoginCommand, LogoutCommand, LogoutResult,
    RpcNotification, ValueUiResponse,
};
use serde::de::DeserializeOwned;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tracing::warn;

use super::super::convert::{bad_request, blocking, internal};
use super::windows::Windows;
use super::{IndexLog, Services, lock};
use crate::agent::{AgentProcess, OmpCall};
use crate::supervisor::SupervisorError;

/// How long the window may take to answer a `login_input`: omp's own wait for the pasted code.
const INPUT_TIMEOUT: Duration = Duration::from_mins(10);

/// How the window is named in the errors of its requests.
const WINDOW: &str = "login";

/// Runs `call` on the login omp and decodes its response data.
async fn run<T: DeserializeOwned + Send + 'static>(
    process: &Arc<dyn AgentProcess>,
    call: OmpCall,
) -> Result<T, ErrorBody> {
    let process = Arc::clone(process);
    blocking(move || {
        let value = process
            .call(call)
            .map_err(|error| ErrorBody::from(SupervisorError::from(error)))?;
        serde_json::from_value(value).map_err(internal)
    })
    .await
}

/// Stops the login omp off the runtime: dropping the last handle waits until omp exited (DMN-219).
async fn close(process: Arc<dyn AgentProcess>) {
    process.kill();
    let _ = tokio::task::spawn_blocking(move || drop(process)).await;
}

async fn providers(process: &Arc<dyn AgentProcess>) -> Result<Vec<AuthProvider>, ErrorBody> {
    let listed: GetLoginProvidersResult = run(process, OmpCall::LoginProviders).await?;
    Ok(listed
        .providers
        .into_iter()
        .map(|provider| AuthProvider {
            id: provider.id,
            name: provider.name,
            available: provider.available,
            authenticated: provider.authenticated,
        })
        .collect())
}

/// The provider `id` of `providers`; an unknown or unavailable one is `bad_request`, an unknown one naming the known
/// ones (DMN-220).
fn provider(providers: Vec<AuthProvider>, id: &str) -> Result<AuthProvider, ErrorBody> {
    let known: Vec<String> = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect();
    let provider = providers
        .into_iter()
        .find(|provider| provider.id == id)
        .ok_or_else(|| {
            bad_request(format!(
                "omp knows no login provider `{id}`; auth.providers lists them: {}",
                known.join(", ")
            ))
        })?;
    if !provider.available {
        return Err(bad_request(format!(
            "omp cannot log in to {} ({id}) here",
            provider.name
        )));
    }
    Ok(provider)
}

/// The running login as the `index` topic shows it (DMN-290): its number ([`crate::supervisor::Supervisor::begin_login`])
/// and its activity. Every change and its `login_changed` event happen under one lock, so a login that a newer one
/// replaced can neither overwrite nor clear the newer one's.
#[derive(Default)]
pub(super) struct LoginBoard(Mutex<Option<(u64, LoginActivity)>>);

impl LoginBoard {
    /// Shows login `id` of `provider`, nothing of its flow yet.
    fn begin(&self, index: &IndexLog, id: u64, provider: &AuthProvider) {
        let activity = LoginActivity {
            provider_id: provider.id.clone(),
            name: provider.name.clone(),
            url: None,
            instructions: None,
            progress: None,
        };
        let mut shown = lock(&self.0);
        *shown = Some((id, activity.clone()));
        index.push(IndexEvent::LoginChanged {
            login: Some(activity),
        });
    }

    /// Applies `change` to login `id`'s activity, unless another login replaced it.
    fn update(&self, index: &IndexLog, id: u64, change: impl FnOnce(&mut LoginActivity)) {
        let mut shown = lock(&self.0);
        if let Some((shown_id, activity)) = shown.as_mut()
            && *shown_id == id
        {
            change(activity);
            index.push(IndexEvent::LoginChanged {
                login: Some(activity.clone()),
            });
        }
    }

    /// Login `id` ended: the index shows none, unless another login replaced it.
    fn end(&self, index: &IndexLog, id: u64) {
        let mut shown = lock(&self.0);
        if shown.as_ref().is_some_and(|(shown_id, _)| *shown_id == id) {
            *shown = None;
            index.push(IndexEvent::LoginChanged { login: None });
        }
    }
}

/// Where a login's flow goes: the window of `connection`, the provider, the login omp to answer, and the board every
/// window reads (login `login`).
struct Flow {
    windows: Arc<Windows>,
    connection: u64,
    provider: AuthProvider,
    process: Arc<dyn AgentProcess>,
    runtime: Handle,
    login: u64,
    board: Arc<LoginBoard>,
    index: Arc<IndexLog>,
    supervisor: crate::supervisor::Supervisor,
}

impl Flow {
    fn notify(&self, request: WindowRequest) {
        if let Err(error) = self.windows.notify(self.connection, request) {
            warn!(window = self.connection, message = %error.message, "cannot show the login");
        }
    }

    /// Asks the window for the text omp's `input` wants and answers omp: the text, or a cancel when the window
    /// cancels, closes or does not answer in time (DMN-221).
    fn ask(&self, id: String, message: String, placeholder: Option<String>) -> JoinHandle<()> {
        let request = WindowRequest::LoginInput {
            provider_id: self.provider.id.clone(),
            name: self.provider.name.clone(),
            message,
            placeholder,
        };
        let windows = Arc::clone(&self.windows);
        let process = Arc::clone(&self.process);
        let connection = self.connection;
        self.runtime.spawn(async move {
            let answer = windows
                .request_to(connection, WINDOW, request, INPUT_TIMEOUT)
                .await;
            let response = match answer {
                Ok(WindowAnswer::LoginInput { value: Some(value) }) => {
                    ExtensionUiResponse::ValueUiResponse(ValueUiResponse { id, value })
                }
                _ => cancel(id),
            };
            if let Err(error) = process.respond(response) {
                warn!(%error, "cannot answer the login omp");
            }
        })
    }

    fn refuse(&self, id: String) {
        if let Err(error) = self.process.respond(cancel(id)) {
            warn!(%error, "cannot answer the login omp");
        }
    }

    /// Hands the login's frames to the window until the login omp ends (DMN-221): `open_url` becomes `login_url`,
    /// `notify` `login_progress`, `input` a `login_input` the window answers; other dialogs are cancelled (omp's
    /// logins ask nothing else).
    fn pump(self, events: &Receiver<Event>) {
        let mut asks = Vec::new();
        for event in events {
            let Event::Notification(RpcNotification::ExtensionUiRequest(request)) = event else {
                continue;
            };
            match request {
                ExtensionUiRequest::OpenUrl(open) => {
                    // Its loopback callback may be relayed by `oauth.callback` (DMN-340).
                    self.supervisor.login_opened_url(self.login, &open.url);
                    self.board.update(&self.index, self.login, |activity| {
                        activity.url = Some(open.url.clone());
                        activity.instructions.clone_from(&open.instructions);
                    });
                    self.notify(WindowRequest::LoginUrl {
                        provider_id: self.provider.id.clone(),
                        name: self.provider.name.clone(),
                        url: open.url,
                        launch_url: open.launch_url,
                        instructions: open.instructions,
                    });
                }
                ExtensionUiRequest::Notify(notify) => {
                    self.board.update(&self.index, self.login, |activity| {
                        activity.progress = Some(notify.message.clone());
                    });
                    self.notify(WindowRequest::LoginProgress {
                        provider_id: self.provider.id.clone(),
                        name: self.provider.name.clone(),
                        message: notify.message,
                    });
                }
                ExtensionUiRequest::Input(input) => {
                    asks.push(self.ask(input.id, input.title, input.placeholder));
                }
                ExtensionUiRequest::Select(dialog) => self.refuse(dialog.id),
                ExtensionUiRequest::Confirm(dialog) => self.refuse(dialog.id),
                ExtensionUiRequest::Editor(dialog) => self.refuse(dialog.id),
                ExtensionUiRequest::Ask(dialog) => self.refuse(dialog.id),
                ExtensionUiRequest::Form(dialog) => self.refuse(dialog.id),
                ExtensionUiRequest::Cancel(_)
                | ExtensionUiRequest::SetStatus(_)
                | ExtensionUiRequest::SetWidget(_)
                | ExtensionUiRequest::SetTitle(_)
                | ExtensionUiRequest::SetEditorText(_) => {}
            }
        }
        // The login omp is gone: a question still open has nobody left to answer, and its OAuth listeners died with
        // it, so no pasted callback may be relayed to whatever takes their ports (DMN-340).
        self.supervisor.login_omp_ended(self.login);
        for ask in asks {
            ask.abort();
        }
    }
}

fn cancel(id: String) -> ExtensionUiResponse {
    ExtensionUiResponse::CancelUiResponse(CancelUiResponse {
        id,
        cancelled: omfp_core::wire::LitTrue,
        timed_out: None,
    })
}

impl Services {
    /// `auth.status` (PRT-140, DMN-235): how many models omp can use, checked by a login omp the first time.
    pub async fn auth_status(&self) -> Result<AuthStatus, ErrorBody> {
        let models = self.supervisor.usable_models().await?;
        Ok(AuthStatus { models })
    }

    /// Lists usable models without creating or starting a thread (UI-023, UI-030).
    pub async fn auth_models(&self) -> Result<omp_rpc::GetAvailableModelsResult, ErrorBody> {
        let omp = self.supervisor.auth_omp().await?;
        let listed = run(&omp.process, OmpCall::AvailableModels).await;
        close(omp.process).await;
        listed
    }

    /// `auth.providers` (PRT-129, DMN-219): omp's OAuth providers, from a login omp started for the question.
    pub async fn auth_providers(&self) -> Result<AuthProviders, ErrorBody> {
        let omp = self.supervisor.auth_omp().await?;
        let listed = providers(&omp.process).await;
        close(omp.process).await;
        Ok(AuthProviders { providers: listed? })
    }

    /// `auth.login` (PRT-130, DMN-220..DMN-224): runs omp's login of `provider_id` on a login omp, its flow shown by
    /// the window of connection `caller` (an agent's goes to the most recently focused window); once omp stored the
    /// credential, the running threads restart when free and the usable models are checked again (DMN-236).
    pub async fn auth_login(
        &self,
        caller: Option<u64>,
        provider_id: String,
    ) -> Result<LoggedIn, ErrorBody> {
        let connection = caller.or_else(|| self.windows.focused()).ok_or_else(|| {
            ErrorBody::new(
                ErrorCode::Conflict,
                "no omfp window is open to show the login: open omfp and log in there",
            )
        })?;
        let omp = self.supervisor.auth_omp().await?;
        let process = omp.process;
        let provider = match providers(&process)
            .await
            .and_then(|all| provider(all, &provider_id))
        {
            Ok(provider) => provider,
            Err(error) => {
                close(process).await;
                return Err(error);
            }
        };
        // A login begins only when no import runs (DMN-230): omp's import would replace the account it stores.
        let writes = self.supervisor.credential_writes().await;
        let login = self
            .supervisor
            .begin_login(&provider.id, connection, &process);
        drop(writes);
        self.login_board.begin(&self.index, login, &provider);
        let flow = Flow {
            windows: Arc::clone(&self.windows),
            connection,
            provider: provider.clone(),
            process: Arc::clone(&process),
            runtime: Handle::current(),
            login,
            board: Arc::clone(&self.login_board),
            index: Arc::clone(&self.index),
            supervisor: self.supervisor.clone(),
        };
        let events = omp.events;
        let pump = tokio::task::spawn_blocking(move || flow.pump(&events));
        let command = LoginCommand {
            provider_id: provider.id.clone(),
        };
        let logging_in = Arc::clone(&process);
        let result = tokio::task::spawn_blocking(move || logging_in.call(OmpCall::Login(command)))
            .await
            .map_err(internal);
        let ours = self.supervisor.end_login(login);
        // The login omp has done its work; stopping it ends the pump too.
        close(process).await;
        if let Err(error) = pump.await {
            warn!(%error, "the login's frames stopped");
        }
        self.login_board.end(&self.index, login);
        match result? {
            Ok(_) => {
                self.supervisor.restart_after_login();
                self.supervisor.recheck_usable_models();
                Ok(LoggedIn {
                    provider_id: provider.id,
                    name: provider.name,
                })
            }
            Err(_) if !ours => Err(ErrorBody::new(
                ErrorCode::Conflict,
                format!("the login to {} was cancelled", provider.name),
            )),
            Err(error) => Err(SupervisorError::from(error).into()),
        }
    }

    /// `auth.logout` (PRT-133, DMN-223): removes every credential omp stored for `provider_id`, one `logout` per
    /// account `get_logout_accounts` lists; then the usable models are checked again (DMN-236). Holds the credential
    /// writes (DMN-230) until every `logout` answered and the login omp is closed, in a task of its own that outlives
    /// its caller: a caller that goes away (an MCP stream closed) cannot let a copy write omp's store meanwhile.
    pub async fn auth_logout(&self, provider_id: String) -> Result<LoggedOut, ErrorBody> {
        let supervisor = self.supervisor.clone();
        tokio::spawn(async move {
            let _writes = supervisor.credential_writes().await;
            let omp = supervisor.auth_omp().await?;
            let result = logout(&omp.process, provider_id).await;
            close(omp.process).await;
            if result.is_ok() {
                supervisor.recheck_usable_models();
            }
            result
        })
        .await
        .map_err(internal)?
    }

    /// `auth.importable` (PRT-136, DMN-229): the provider logins standalone omp holds that omfp could copy.
    pub async fn auth_importable(&self) -> Result<OmpLogins, ErrorBody> {
        self.supervisor.omp_logins().await.map_err(ErrorBody::from)
    }

    /// `auth.import` (PRT-137, DMN-230): copies standalone omp's provider logins into omfp's profile.
    pub async fn auth_import(&self) -> Result<OmpLogins, ErrorBody> {
        self.supervisor
            .import_omp_logins()
            .await
            .map_err(ErrorBody::from)
    }
}

async fn logout(
    process: &Arc<dyn AgentProcess>,
    provider_id: String,
) -> Result<LoggedOut, ErrorBody> {
    let listed: GetLogoutAccountsResult = run(
        process,
        OmpCall::LogoutAccounts(GetLogoutAccountsCommand {
            provider_id: provider_id.clone(),
        }),
    )
    .await?;
    let mut removed = 0;
    let mut remaining_source = None;
    for account in listed.accounts {
        let result: LogoutResult = run(
            process,
            OmpCall::Logout(LogoutCommand {
                provider_id: provider_id.clone(),
                credential_id: account.credential_id,
            }),
        )
        .await?;
        removed += 1;
        remaining_source = result.remaining_source;
    }
    Ok(LoggedOut {
        provider_id,
        removed,
        remaining_source,
    })
}

#[cfg(test)]
mod tests {
    use futures::FutureExt as _;
    use omfp_protocol::{IndexSnapshot, Settings, Theme};

    use super::*;
    use crate::event_log::EventLog;

    fn provider(id: &str) -> AuthProvider {
        AuthProvider {
            id: id.to_owned(),
            name: id.to_uppercase(),
            available: true,
            authenticated: false,
        }
    }

    fn shown(index: &IndexLog) -> Option<(String, Option<String>)> {
        index.with_state(|state| {
            state
                .login
                .as_ref()
                .map(|login| (login.provider_id.clone(), login.progress.clone()))
        })
    }

    // spec: DMN-290
    #[test]
    fn a_replaced_login_neither_updates_nor_clears_the_login_that_replaced_it() {
        let index = EventLog::new(
            "e",
            IndexSnapshot {
                workspaces: Vec::new(),
                threads: Vec::new(),
                theme: Theme::default(),
                settings: Settings::default(),
                init: None,
                models: None,
                login: None,
            },
            super::super::apply_index,
            16,
            16,
        );
        let board = LoginBoard::default();
        board.begin(&index, 1, &provider("a"));
        board.begin(&index, 2, &provider("b"));

        board.update(&index, 1, |login| login.progress = Some("old".into()));
        board.end(&index, 1);
        assert_eq!(shown(&index), Some(("b".into(), None)));

        board.update(&index, 2, |login| login.progress = Some("new".into()));
        assert_eq!(shown(&index), Some(("b".into(), Some("new".into()))));
        board.end(&index, 2);
        assert_eq!(shown(&index), None);
    }

    /// Services on a fake launcher whose user environment holds a standalone omp store under `dir`.
    async fn services(dir: &std::path::Path, launcher: &crate::agent::FakeLauncher) -> Services {
        use crate::git::{FakeGit, Git};
        use crate::paths::DataDir;
        use crate::store::Store;
        use crate::supervisor::{Supervisor, SupervisorConfig};
        use crate::threads::Threads;
        use crate::widgets::Widgets;
        use crate::workspaces::Workspaces;

        let home = dir.join("standalone-home");
        let agent = home.join(".omp").join("agent");
        std::fs::create_dir_all(&agent).unwrap();
        rusqlite::Connection::open(agent.join("agent.db"))
            .unwrap()
            .execute_batch("CREATE TABLE standalone (n INTEGER);")
            .unwrap();
        let data = DataDir::new(dir.join("data"));
        std::fs::create_dir_all(data.root()).unwrap();
        let root = dir.join("workspaces");
        let store = Arc::new(Store::open(&data.db()).unwrap());
        let threads = Arc::new(Threads::new(Arc::clone(&store), data.clone()));
        let workspaces = Arc::new(Workspaces::new(
            Arc::clone(&store),
            Arc::new(FakeGit::new()) as Arc<dyn Git>,
            root,
        ));
        let widgets = Arc::new(Widgets::new(None));
        let mut config = SupervisorConfig::new("omp");
        config.user_omp_env = omfp_testkit::standalone_omp_env(&home);
        let supervisor = Supervisor::new(
            Arc::new(launcher.clone()),
            Arc::clone(&threads),
            Arc::clone(&workspaces),
            data,
            config,
        );
        Services::new(
            store,
            threads,
            workspaces,
            supervisor,
            widgets,
            Arc::new(crate::jobs::declared::SystemClock),
        )
        .await
        .unwrap()
    }

    /// Opens its gates when dropped, also when the test panics.
    struct OpenOnDrop(Vec<crate::agent::Gate>);

    impl Drop for OpenOnDrop {
        fn drop(&mut self) {
            for gate in &self.0 {
                gate.open();
            }
        }
    }

    // spec: DMN-230, DMN-223
    #[tokio::test(flavor = "multi_thread")]
    async fn a_logout_whose_caller_went_away_holds_the_credential_writes_until_its_omp_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = crate::agent::FakeLauncher::new();
        let services = services(dir.path(), &launcher).await;
        launcher.reply(
            "get_logout_accounts",
            crate::agent::FakeReply::Ok(serde_json::json!({ "accounts": [
                { "credentialId": 3, "provider": "anthropic", "label": "a@example.com", "detail": "oauth #3",
                  "type": "oauth", "active": true }
            ]})),
        );
        launcher.set_export(Ok(
            r#"{"version":1,"credentials":[{"provider":"openai","credential":{"type":"api_key","key":"k"}}]}"#,
        ));
        let logout = launcher.gate("logout");
        let closes = launcher.gate_closes();
        // A failed assertion must not leave the logout's blocking call held at a gate: the runtime would wait for it.
        let _open = OpenOnDrop(vec![logout.clone(), closes.clone()]);
        let caller = tokio::spawn({
            let services = services.clone();
            async move { services.auth_logout("anthropic".into()).await }
        });
        logout.arrived(1).await;

        // The caller goes away (an MCP stream closed) while omp logs out.
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert!(
            services
                .supervisor
                .credential_writes()
                .now_or_never()
                .is_none(),
            "the logout still holds the credential writes"
        );

        // The logout's RPCs end; while its omp is being closed the lock is still held.
        logout.open();
        closes.arrived(1).await;
        assert!(
            services
                .supervisor
                .credential_writes()
                .now_or_never()
                .is_none(),
            "the logout holds the credential writes until its omp is closed"
        );
        // A copy asked for now runs once the close is done.
        let import = tokio::spawn({
            let services = services.clone();
            async move { services.auth_import().await }
        });
        closes.open();
        let copied = import.await.unwrap().unwrap();
        assert_eq!(copied.providers[0].provider_id, "openai");
        let process = launcher.process(0);
        assert!(process.is_closed());
        assert_eq!(
            process
                .calls()
                .into_iter()
                .filter(|call| matches!(call, crate::agent::FakeCall::Call(OmpCall::Logout(_))))
                .count(),
            1,
            "the logout ran to its end"
        );
        assert_eq!(launcher.exports().len(), 1);
    }
}
