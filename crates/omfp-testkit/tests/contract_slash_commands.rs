//! Contract test (L3): the builtin `/` commands the pinned omp announces over RPC (`get_available_commands`, the same
//! list as `available_commands_update`): the ones it runs without its TUI. A TUI-only builtin is not announced and,
//! sent as a prompt, reaches the model as plain text. The window routes or refuses those from its own table
//! (`ui/packages/app/src/vm/slash.ts`, UI-975): when this test fails after an omp upgrade, check that table against
//! omp's builtin registry (`packages/coding-agent/src/slash-commands/builtin-*.ts`) and update both.
//!
//! Run with `just test-contract` inside the dev shell.

use std::time::Duration;

use fake_llm::{FakeLlm, Reply, Script, Step};
use omfp_testkit::{DenyProxy, FAKE_MODEL, OmpEnv, OmpSession, require_pinned_omp};
use omp_rpc::{GetAvailableCommandsCommand, PromptCommand, SlashCommandSource};

/// omp 19.2.0's builtins with a text-mode handler, in its order, each with its aliases.
const RPC_BUILTINS: &[(&str, &[&str])] = &[
    ("security", &[]),
    ("model", &["models"]),
    ("switch", &[]),
    ("fast", &[]),
    ("slow", &[]),
    ("skillful", &[]),
    ("extended-context", &[]),
    ("computer", &[]),
    ("ratchet", &[]),
    ("prewalk", &[]),
    ("modelpreset", &[]),
    ("effort", &[]),
    ("advisor", &[]),
    ("export", &[]),
    ("trace", &[]),
    ("dump", &[]),
    ("share", &[]),
    ("browser", &[]),
    ("todo", &[]),
    ("session", &[]),
    ("jobs", &[]),
    ("usage", &[]),
    ("stats", &[]),
    ("changelog", &[]),
    ("tools", &[]),
    ("context", &[]),
    ("mcp", &[]),
    ("commit", &[]),
    ("ssh", &[]),
    ("fresh", &[]),
    ("compact", &[]),
    ("shake", &[]),
    ("handoff", &[]),
    ("pin", &[]),
    ("unpin", &[]),
    ("retry", &[]),
    ("memory", &[]),
    ("rename", &[]),
    ("move", &[]),
    ("wt", &["worktree"]),
    ("add-dir", &[]),
    ("remove-dir", &[]),
    ("dirs", &[]),
    ("marketplace", &[]),
    ("plugins", &["plugin"]),
    ("reload-plugins", &[]),
    ("force", &["force:"]),
    ("forget-approvals", &[]),
];

// spec: UI-975
#[tokio::test(flavor = "multi_thread")]
async fn omp_announces_the_pinned_builtin_commands_and_prompts_with_a_tui_only_one() {
    require_pinned_omp();
    let fake = FakeLlm::start(Script::new(
        FAKE_MODEL,
        vec![Step::reply(Reply::text("no settings here"))],
    ))
    .await
    .unwrap();
    let proxy = DenyProxy::start().await.unwrap();
    let env = OmpEnv::new(&fake.base_url(), Some(&proxy)).unwrap();

    let commands = tokio::task::spawn_blocking(move || {
        let session = OmpSession::start(&env, &["--no-session"]).unwrap();
        let commands = session
            .client()
            .call(&GetAvailableCommandsCommand {})
            .unwrap_or_else(|error| {
                panic!(
                    "get_available_commands failed: {error}\nstderr:\n{}",
                    session.stderr()
                )
            });
        let prompt = PromptCommand {
            message: "/settings".into(),
            images: None,
            streaming_behavior: None,
        };
        session
            .client()
            .prompt_and_wait(&prompt, Duration::from_secs(60))
            .unwrap_or_else(|error| {
                panic!("prompt failed: {error}\nstderr:\n{}", session.stderr())
            });
        commands
    })
    .await
    .unwrap();

    // `/settings` has only a TUI handler: omp found no command and sent it to the model.
    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].to_string().contains("/settings"));

    let builtins: Vec<(String, Vec<String>)> = commands
        .into_iter()
        .filter(|command| command.source == SlashCommandSource::Builtin)
        .map(|command| (command.name, command.aliases.unwrap_or_default()))
        .collect();
    let pinned: Vec<(String, Vec<String>)> = RPC_BUILTINS
        .iter()
        .map(|(name, aliases)| {
            (
                (*name).to_owned(),
                aliases.iter().map(|alias| (*alias).to_owned()).collect(),
            )
        })
        .collect();
    assert_eq!(builtins, pinned);
}
