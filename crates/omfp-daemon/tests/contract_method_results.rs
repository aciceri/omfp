//! L3: the real daemon with the pinned omp and the fake LLM answers omp-forwarded methods with their declared result
//! types (`Method::Result`, TST-117): omp's response data, not `omp-rpc`'s unwrapped outputs.

mod client;
mod common;

use clap::Parser;
use client::{Client, Follower};
use common::Harness;
use fake_llm::{Reply, Step};
use omfp_core::wire::QueuedMessageQueue;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::{DaemonHandle, Ports, start_with};
use omfp_protocol::method::{
    AuthModels, ThreadAbort, ThreadCommands, ThreadFollowUp, ThreadModels, ThreadPrompt,
    ThreadQueuePromote, ThreadQueueRemove, ThreadSetModel, ThreadSetThinking, ThreadState,
    ThreadThinkingLevels, ThreadsCreate,
};
use omfp_protocol::{
    CreateThreadParams, CreateWorkspaceParams, Empty, MessageParams, PromptParams,
    QueueMessageParams, SetModelParams, SetThinkingParams, ThreadRef, ThreadWorkspace,
};
use omfp_testkit::{FAKE_MODEL, FAKE_PROVIDER, omp_bin};
use tempfile::TempDir;

async fn start(harness: &Harness) -> (DaemonHandle, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let omp = omp_bin();
    let workspaces = data.path().join("workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.path().to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--omp-bin",
        &omp,
        // The fake LLM scripts every model call; a title run would take a step (DMN-073).
        "--no-title-generation",
    ])
    .unwrap();
    let config = Config::from_cli(cli, None, None).unwrap();
    let mut ports = Ports::resolve(&config).unwrap();
    ports.omp_env = harness.env.prepare_omfp_profile(data.path()).unwrap();
    (start_with(config, ports).await.unwrap(), data)
}

fn texts(messages: &[omfp_core::wire::RestoredQueuedMessage]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message.text.as_str())
        .collect()
}

/// Queues `text` as a follow-up and waits until omp reports it.
async fn follow_up(client: &mut Client, view: &mut Follower, thread: &str, text: &str) {
    client
        .typed::<ThreadFollowUp>(MessageParams {
            thread_id: thread.into(),
            text: text.into(),
            images: Vec::new(),
        })
        .await;
    view.until(client, |state| {
        state.queue.follow_up.iter().any(|queued| queued == text)
    })
    .await;
}

// spec: TST-117, PRT-140, DMN-235
#[tokio::test(flavor = "multi_thread")]
async fn omp_forwarded_methods_return_their_declared_results() {
    let harness = Harness::new(vec![Step::reply(Reply::text("slow answer")).gated()]).await;
    let (daemon, _data) = start(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    let models = client.typed::<AuthModels>(Empty {}).await.models;
    assert!(
        models
            .iter()
            .any(|model| model.id == FAKE_MODEL && model.provider == FAKE_PROVIDER),
        "{models:?}"
    );
    let thread = client
        .typed::<ThreadsCreate>(CreateThreadParams {
            title: Some("typed results".into()),
            workspace: ThreadWorkspace::New(CreateWorkspaceParams::default()),
            model: Some(format!("{FAKE_PROVIDER}/{FAKE_MODEL}")),
            thinking_level: None,
            approval_mode: None,
            prompt: None,
        })
        .await
        .id;
    let this = || ThreadRef {
        thread_id: thread.clone(),
    };

    let state = client.typed::<ThreadState>(this()).await;
    assert_eq!(
        state.model.map(|model| model.id).as_deref(),
        Some(FAKE_MODEL)
    );
    let models = client.typed::<ThreadModels>(this()).await.models;
    assert!(
        models.iter().any(|model| model.id == FAKE_MODEL),
        "{models:?}"
    );
    let levels = client.typed::<ThreadThinkingLevels>(this()).await.levels;
    client.typed::<ThreadCommands>(this()).await;
    client
        .typed::<ThreadSetModel>(SetModelParams {
            thread_id: thread.clone(),
            provider: FAKE_PROVIDER.into(),
            model_id: FAKE_MODEL.into(),
        })
        .await;
    client
        .typed::<ThreadSetThinking>(SetThinkingParams {
            thread_id: thread.clone(),
            level: levels[0],
        })
        .await;

    // Keep the turn busy (gated reply) so messages stay queued.
    let mut view = Follower::snapshot(&mut client, &thread, None).await;
    client
        .typed::<ThreadPrompt>(PromptParams {
            thread_id: thread.clone(),
            text: "go".into(),
            images: Vec::new(),
            behavior: None,
        })
        .await;
    harness.fake.wait_for_gate(0).await;
    follow_up(&mut client, &mut view, &thread, "drop me").await;
    follow_up(&mut client, &mut view, &thread, "steer me").await;
    let queued = |message: &str, queue| QueueMessageParams {
        thread_id: thread.clone(),
        message: message.into(),
        queue,
    };
    let promoted = client
        .typed::<ThreadQueuePromote>(queued("steer me", None))
        .await;
    assert!(promoted.promoted);
    let removed = client
        .typed::<ThreadQueueRemove>(queued("drop me", Some(QueuedMessageQueue::FollowUp)))
        .await;
    assert!(removed.removed);
    let restored = client.typed::<ThreadAbort>(this()).await;
    harness.fake.release_gate(0);

    assert_eq!(
        (texts(&restored.steering), texts(&restored.follow_up)),
        (vec!["steer me"], Vec::new())
    );
    daemon.shutdown().await.unwrap();
}
