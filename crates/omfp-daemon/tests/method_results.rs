//! Integration tests (L2): every method answers with its declared result type (`Method::Result`, TST-117), checked
//! by decoding the daemon's response over a real WebSocket. The daemon runs on the fake launcher and fake git.

mod client;

use std::path::Path;

use client::{Client, Follower, TestDaemon};
use omfp_core::wire::{ExtensionUiRequest, ThinkingLevel};
use omfp_daemon::agent::FakeRun;
use omfp_protocol::method::ActivityList;
use omfp_protocol::method::{
    FsComplete, InteractionRespond, ReposSuggest, Subscribe, SystemHello, ThemeGet, ThemeSet,
    ThreadFollowUp, ThreadGit, ThreadLabel, ThreadPrompt, ThreadRestart, ThreadSetModel,
    ThreadSetThinking, ThreadSteer, ThreadStop, ThreadSubagentCancel, ThreadSubagentSteer,
    ThreadsArchive, ThreadsCreate, ThreadsDelete, ThreadsSearch, ThreadsUnarchive, ThreadsUpdate,
    Unsubscribe, WorkspacesAddRepo, WorkspacesCreate, WorkspacesDiff, WorkspacesFindFile,
    WorkspacesReadFile, WorkspacesRemove, WorkspacesRemoveRepo, WorkspacesRescan, WorkspacesStatus,
    WorkspacesUpdate, WorkspacesWriteFile,
};
use omfp_protocol::{
    AddRepoParams, ApprovalMode, CompletePathParams, CreateThreadParams, CreateWorkspaceParams,
    Empty, FileMatch, FindFileParams, HelloParams, InteractionResponse, LabelParams, MessageParams,
    PromptParams, ReadFileParams, RemoveRepoParams, RemoveWorkspaceParams, RepoRef, RepoSource,
    RespondParams, SearchThreadsParams, SetModelParams, SetThinkingParams, SubagentRef,
    SubagentSteerParams, SubscribeParams, SubscribeResult, SuggestReposParams, Theme,
    ThreadCloseParams, ThreadRef, ThreadWorkspace, Topic, TopicParams, UpdateThreadParams,
    UpdateWorkspaceParams, WorkspaceDiffParams, WorkspaceRef, WriteFileParams,
};
use omfp_protocol::{ListActivityParams, StopThreadParams};

impl TestDaemon {
    /// A new directory under the test's temporary directory, outside the data directory's entries.
    fn folder(&self, name: &str) -> String {
        let path = self.dir.path().join("folders").join(name);
        std::fs::create_dir_all(&path).unwrap();
        path.to_str().unwrap().to_owned()
    }
}

fn fixture(name: &str) -> FakeRun {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/threads")
        .join(omfp_core::OMP_VERSION)
        .join(format!("{name}.jsonl"));
    FakeRun::from_fixture(&path).unwrap()
}

fn new_thread(title: &str) -> CreateThreadParams {
    CreateThreadParams {
        title: Some(title.into()),
        workspace: ThreadWorkspace::New(CreateWorkspaceParams::default()),
        model: None,
        thinking_level: None,
        approval_mode: None,
        prompt: None,
        images: Vec::new(),
    }
}

fn thread_ref(thread_id: &str) -> ThreadRef {
    ThreadRef {
        thread_id: thread_id.into(),
    }
}

fn thread_close(thread_id: &str) -> ThreadCloseParams {
    ThreadCloseParams {
        thread_id: thread_id.into(),
        jobs: None,
    }
}

fn message(thread_id: &str, text: &str) -> MessageParams {
    MessageParams {
        thread_id: thread_id.into(),
        text: text.into(),
        images: Vec::new(),
    }
}

// spec: TST-117
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // One method after the other.
async fn connection_repository_and_workspace_methods_return_their_declared_results() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = Client::connect(daemon.handle.addr()).await;
    let hello = client
        .typed::<SystemHello>(HelloParams {
            token: daemon.handle.token().expose().to_owned(),
            protocol: omfp_protocol::PROTOCOL_VERSION,
            client: "test".into(),
        })
        .await;
    assert_eq!(hello.protocol, omfp_protocol::PROTOCOL_VERSION);
    let subscribed = client
        .typed::<Subscribe>(SubscribeParams {
            topic: Topic::Index,
            since: None,
            watch: None,
        })
        .await;
    assert!(matches!(subscribed, SubscribeResult::Snapshot { .. }));
    client
        .typed::<Unsubscribe>(TopicParams {
            topic: Topic::Index,
        })
        .await;

    let root = daemon.folder("repo");
    daemon.git.add_repo(Path::new(&root), "main");
    let completed = client
        .typed::<FsComplete>(CompletePathParams {
            path: format!("{}/folders/re", daemon.dir.path().display()),
        })
        .await;
    assert_eq!(
        completed
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["repo"]
    );
    let workspace = client
        .typed::<WorkspacesCreate>(CreateWorkspaceParams {
            name: Some("checkout".into()),
            link: None,
        })
        .await;
    let workspace_ref = || WorkspaceRef {
        workspace_id: workspace.id.clone(),
    };
    let renamed = client
        .typed::<WorkspacesUpdate>(UpdateWorkspaceParams {
            workspace_id: workspace.id.clone(),
            name: "checkout-v2".into(),
        })
        .await;
    assert_eq!(renamed.name, "checkout-v2");
    let added = client
        .typed::<WorkspacesAddRepo>(AddRepoParams {
            workspace_id: workspace.id.clone(),
            name: None,
            source: RepoSource::Worktree {
                repo: root.clone(),
                branch: Some("feature".into()),
                base_ref: Some("main".into()),
            },
        })
        .await;
    assert_eq!(added.repos.len(), 1);
    let rescanned = client.typed::<WorkspacesRescan>(workspace_ref()).await;
    assert_eq!(rescanned.repos, added.repos);
    let suggested = client
        .typed::<ReposSuggest>(SuggestReposParams { query: None })
        .await;
    assert_eq!(
        suggested
            .repos
            .iter()
            .map(|repo| repo.name.as_str())
            .collect::<Vec<_>>(),
        ["repo"]
    );
    let repo = RepoRef {
        workspace_id: workspace.id.clone(),
        repo: "repo".into(),
    };
    let status = client.typed::<WorkspacesStatus>(repo).await;
    assert_eq!(status.branch.as_deref(), Some("feature"));
    client
        .typed::<WorkspacesDiff>(WorkspaceDiffParams {
            workspace_id: workspace.id.clone(),
            repo: "repo".into(),
            path: None,
            cursor: None,
        })
        .await;
    let written = client
        .typed::<WorkspacesWriteFile>(WriteFileParams {
            workspace_id: workspace.id.clone(),
            path: "repo/notes.md".into(),
            content: "hi\n".into(),
            expected_hash: None,
        })
        .await;
    let read = client
        .typed::<WorkspacesReadFile>(ReadFileParams {
            workspace_id: workspace.id.clone(),
            path: "notes.md".into(),
            known_hash: None,
        })
        .await;
    assert_eq!(
        (read.path.as_str(), read.hash, read.content.as_deref()),
        ("repo/notes.md", written.hash, Some("hi\n"))
    );
    let emptied = client
        .typed::<WorkspacesRemoveRepo>(RemoveRepoParams {
            workspace_id: workspace.id.clone(),
            name: "repo".into(),
            force: true,
            delete_branch: true,
        })
        .await;
    assert_eq!(emptied.repos, Vec::new());

    client
        .typed::<WorkspacesRemove>(RemoveWorkspaceParams {
            workspace_id: workspace.id.clone(),
            force: false,
        })
        .await;
}

// spec: TST-117
#[tokio::test(flavor = "multi_thread")]
async fn thread_management_methods_return_their_declared_results() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let thread = client.typed::<ThreadsCreate>(new_thread("typed")).await;

    let updated = client
        .typed::<ThreadsUpdate>(UpdateThreadParams {
            thread_id: thread.id.clone(),
            title: Some("renamed".into()),
            approval_mode: Some(ApprovalMode::AlwaysAsk),
        })
        .await;
    assert_eq!(updated.title, "renamed");
    let git = client.typed::<ThreadGit>(thread_ref(&thread.id)).await;
    assert_eq!(git.workspace_id, thread.workspace_id);
    assert_eq!(git.repos, Vec::new());
    client
        .typed::<WorkspacesWriteFile>(WriteFileParams {
            workspace_id: thread.workspace_id.clone(),
            path: "plan.md".into(),
            content: "plan\n".into(),
            expected_hash: None,
        })
        .await;
    let candidates = client
        .typed::<WorkspacesFindFile>(FindFileParams {
            workspace_id: thread.workspace_id.clone(),
            query: "docs/plan.md".into(),
            thread_id: Some(thread.id.clone()),
            fuzzy: false,
        })
        .await;
    assert_eq!(
        candidates
            .candidates
            .iter()
            .map(|candidate| (candidate.path.as_str(), candidate.matched))
            .collect::<Vec<_>>(),
        [("plan.md", FileMatch::Basename)]
    );

    client
        .typed::<ThreadsArchive>(thread_close(&thread.id))
        .await;
    client
        .typed::<ThreadsUnarchive>(thread_ref(&thread.id))
        .await;
    let doomed = client.typed::<ThreadsCreate>(new_thread("doomed")).await;
    client
        .typed::<ThreadsDelete>(thread_close(&doomed.id))
        .await;
    let found = client
        .typed::<ThreadsSearch>(SearchThreadsParams {
            query: "nothing was said".into(),
        })
        .await;
    assert_eq!(found.results, Vec::new());
}

// spec: TST-117, DMN-075
#[tokio::test(flavor = "multi_thread")]
async fn theme_methods_return_their_declared_results() {
    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    assert_eq!(
        client.typed::<ThemeGet>(Empty {}).await,
        Theme { css: None }
    );

    let theme = Theme {
        css: Some(":root { --omfp-primary: #6750a4; }".into()),
    };
    assert_eq!(client.typed::<ThemeSet>(theme.clone()).await, Empty {});
    assert_eq!(client.typed::<ThemeGet>(Empty {}).await, theme);
}

// spec: TST-117, PRT-202, PRT-203, PRT-204, PRT-205, PRT-206, PRT-207
#[tokio::test(flavor = "multi_thread")]
async fn declared_job_methods_return_their_declared_results() {
    use omfp_protocol::method::{JobsDefine, JobsEnable, JobsList, JobsRemove, JobsRun, JobsRuns};
    use omfp_protocol::{
        DefineJobParams, EnableJobParams, JobNameParams, JobRemoveParams, JobRunsParams,
        JobViewStatus,
    };

    let daemon = TestDaemon::start(&[]).await;
    let mut client = daemon.client("test").await;
    let ws = client
        .typed::<WorkspacesCreate>(CreateWorkspaceParams {
            name: None,
            link: None,
        })
        .await
        .id;

    let view = client
        .typed::<JobsDefine>(DefineJobParams {
            workspace_id: ws.clone(),
            name: "check".into(),
            job: serde_json::json!({ "command": "true" }),
            replace: None,
        })
        .await;
    assert_eq!(view.name, "check");
    let state = client
        .typed::<JobsList>(WorkspaceRef {
            workspace_id: ws.clone(),
        })
        .await;
    assert_eq!(state.jobs, [view]);
    let run = client
        .typed::<JobsRun>(JobNameParams {
            workspace_id: ws.clone(),
            job: "check".into(),
        })
        .await;
    assert_eq!(run.job.as_deref(), Some("check"));
    let runs = client
        .typed::<JobsRuns>(JobRunsParams {
            workspace_id: ws.clone(),
            job: "check".into(),
            before: None,
            limit: Some(1),
        })
        .await;
    assert_eq!(runs.runs[0].id, run.id);
    let view = client
        .typed::<JobsEnable>(EnableJobParams {
            workspace_id: ws.clone(),
            name: "check".into(),
            enabled: false,
        })
        .await;
    assert!(!view.job.enabled);
    assert!(matches!(
        view.status,
        JobViewStatus::Disabled | JobViewStatus::Running
    ));
    assert_eq!(
        client
            .typed::<JobsRemove>(JobRemoveParams {
                workspace_id: ws,
                name: "check".into(),
            })
            .await,
        Empty {}
    );
}

// spec: TST-117
#[tokio::test(flavor = "multi_thread")]
async fn agent_methods_the_daemon_answers_itself_return_their_declared_results() {
    let daemon = TestDaemon::start(&[]).await;
    daemon.launcher.push_run(fixture("approval_granted"));
    let mut client = daemon.client("test").await;
    let thread = client.typed::<ThreadsCreate>(new_thread("agent")).await.id;
    let mut view = Follower::snapshot(&mut client, &thread, None).await;

    let admitted = client
        .typed::<ThreadPrompt>(PromptParams {
            thread_id: thread.clone(),
            text: "run it".into(),
            images: Vec::new(),
            behavior: None,
        })
        .await;
    assert!(admitted.agent_invoked);
    view.until(&mut client, |state| !state.interactions.is_empty())
        .await;
    let Some(ExtensionUiRequest::Select(select)) = view.state.interactions.first().cloned() else {
        panic!(
            "expected the approval select: {:?}",
            view.state.interactions
        );
    };
    client
        .typed::<InteractionRespond>(RespondParams {
            thread_id: thread.clone(),
            interaction_id: select.id,
            response: InteractionResponse::Value(select.options[0].clone()),
        })
        .await;

    client.typed::<ThreadSteer>(message(&thread, "steer")).await;
    client
        .typed::<ThreadFollowUp>(message(&thread, "later"))
        .await;
    client
        .typed::<ThreadSetModel>(SetModelParams {
            thread_id: thread.clone(),
            provider: "fake".into(),
            model_id: "scripted".into(),
        })
        .await;
    client
        .typed::<ThreadSetThinking>(SetThinkingParams {
            thread_id: thread.clone(),
            level: ThinkingLevel::Low,
        })
        .await;
    client
        .typed::<ThreadLabel>(LabelParams {
            thread_id: thread.clone(),
            entry_id: "e1".into(),
            label: Some("label".into()),
        })
        .await;
    let subagent = SubagentRef {
        thread_id: thread.clone(),
        subagent_id: "s1".into(),
    };
    client
        .typed::<ThreadSubagentSteer>(SubagentSteerParams {
            thread_id: thread.clone(),
            subagent_id: subagent.subagent_id.clone(),
            text: "focus".into(),
        })
        .await;
    client.typed::<ThreadSubagentCancel>(subagent).await;
    client.typed::<ThreadRestart>(thread_ref(&thread)).await;
    let stopped = client
        .typed::<ThreadStop>(StopThreadParams {
            thread_id: thread.clone(),
            cascade: true,
        })
        .await;
    assert_eq!(stopped.thread_ids, vec![thread.clone()]);
    client
        .typed::<ActivityList>(ListActivityParams::default())
        .await;
}

// spec: TST-117, TST-119, PRT-229, PRT-230, DMN-370, DMN-372
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_a_daemon_restart_interrupted_shows_in_the_summary_and_resume_continues_it() {
    use omfp_protocol::method::{ThreadResume, ThreadsList};
    use omfp_protocol::{ErrorCode, ListThreadsParams};

    let daemon = TestDaemon::start(&[]).await;
    daemon.launcher.push_run(fixture("approval_granted"));
    let mut client = daemon.client("test").await;
    let busy = client.typed::<ThreadsCreate>(new_thread("busy")).await.id;
    let idle = client.typed::<ThreadsCreate>(new_thread("idle")).await.id;
    let mut view = Follower::snapshot(&mut client, &busy, None).await;
    client
        .typed::<ThreadPrompt>(PromptParams {
            thread_id: busy.clone(),
            text: "run it".into(),
            images: Vec::new(),
            behavior: None,
        })
        .await;
    // The turn waits on the approval: not settled when the daemon stops.
    view.until(&mut client, |state| !state.interactions.is_empty())
        .await;
    drop(client);

    let daemon = daemon.restart().await;
    let mut client = daemon.client("test").await;
    let listed = client
        .typed::<ThreadsList>(ListThreadsParams::default())
        .await;
    let interrupted = |list: &omfp_protocol::ThreadList, id: &str| {
        list.threads
            .iter()
            .find(|thread| thread.id == id)
            .and_then(|thread| thread.interrupted_at)
    };
    assert!(interrupted(&listed, &busy).is_some(), "{listed:?}");
    assert_eq!(interrupted(&listed, &idle), None);

    let resumed = client.typed::<ThreadResume>(thread_ref(&busy)).await;

    assert!(resumed.agent_invoked);
    let prompts: Vec<String> = daemon
        .launcher
        .process(0)
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            omfp_daemon::agent::FakeCall::Prompt(prompt) => Some(prompt.message),
            _ => None,
        })
        .collect();
    assert_eq!(prompts, [omfp_daemon::supervisor::RESUME_NOTICE]);
    let listed = client
        .typed::<ThreadsList>(ListThreadsParams::default())
        .await;
    assert_eq!(interrupted(&listed, &busy), None);
    let again = client
        .call("thread.resume", serde_json::json!({ "threadId": busy }))
        .await
        .unwrap_err();
    assert_eq!(
        again["code"],
        serde_json::to_value(ErrorCode::Conflict).unwrap()
    );
}
