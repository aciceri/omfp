//! Golden JSON of every protocol method: one example request and its response per method, as insta snapshots, so
//! every change to the wire format shows up in review.

use omfp_core::ProcessState;
use omfp_core::wire::{AskAnswer, GoalOp, ImageContent, QueuedMessageQueue, ThinkingLevel};
use omfp_protocol::{
    ActivityEntry, ActivityPage, ListActivityParams, PromptHistory, PromptHistoryEntry,
    PromptHistoryParams, StopThreadParams, ThreadsStopped,
};
use omfp_protocol::{
    AddRepoParams, ApprovalMode, BashParams, BtwParams, Call, CompactParams, CompletePathParams,
    CopyWidgetParams, CreateThreadParams, CreateWorkspaceParams, DiffPage, Empty, FileCandidate,
    FileCandidates, FileContent, FileDiff, FileKind, FileMatch, FileWritten, FindFileParams,
    ForkParams, GoalParams, HelloParams, HelloResult, IndexSnapshot, InteractionResponse, JobInfo,
    JobRef, JobStatus, LabelParams, LiveConnectParams, LiveConnected, LiveMuteParams, LiveMuted,
    LiveStarted, METHODS, MessageParams, Method, NavigateParams, PROTOCOL_VERSION, PathCompletions,
    PathEntry, Position, PromptBehavior, PromptParams, PromptResult, QueueMessageParams,
    ReadFileParams, RemoveRepoParams, RemoveWorkspaceParams, RepoRef, RepoSource, RepoSuggestions,
    ReportWidgetsParams, Request, RespondParams, Response, SearchThreadsParams, SeenRepository,
    ServerFrame, SetModelParams, SetSettingsParams, SetThinkingParams, SetViewsParams,
    SetWidgetStateParams, StartJobParams, StartTerminalParams, SubagentMessagesParams, SubagentRef,
    SubagentSteerParams, SubscribeParams, SubscribeResult, SuggestReposParams, TerminalCursor,
    TerminalInfo, TerminalInputParams, TerminalRef, TerminalResizeParams, TerminalScreen,
    TerminalScreenParams, TerminalStatus, Theme, ThreadGit, ThreadRef, ThreadSearchResult,
    ThreadSearchResults, ThreadWorkspace, Topic, TopicParams, UpdateThreadParams,
    UpdateWorkspaceParams, WidgetCopied, WidgetJobReport, WidgetReport, WidgetReportState,
    WidgetSource, WorkspaceDiffParams, WorkspaceRef, WriteFileParams, decode_request, method,
};
use omfp_protocol::{
    AnswerWindowParams, CheckWidgetParams, CommandSource, KeymapMode, ListWindowsParams,
    RanInWindow, RunFailure, RunInWindowParams, ViewSource, WidgetChecked, WindowAnswer,
    WindowBinding, WindowCommand, WindowList, WindowRef, WindowReport, WindowState, WindowSummary,
    WindowView,
};
use omfp_protocol::{
    AuthProvider, AuthProviders, AuthStatus, LoggedIn, LoggedOut, LoginCancelled,
    OAuthCallbackParams, OAuthCallbackRelayed, ProviderRef,
};
use omfp_protocol::{
    CloseJobs, DefineJobParams, EnableJobParams, JobDefinition, JobNameParams, JobPrompt,
    JobRemoveParams, JobRunStatus, JobRuns, JobRunsParams, JobTrigger, JobView, JobViewStatus,
    JobsState, MissedPolicy, PromptThread, RestartPolicy, RunInfo, RunTrigger, ThreadCloseParams,
};
use omfp_protocol::{ContextUsage, SessionTokens, SessionUsage, ThreadSession};
use omfp_protocol::{
    CreatedBy, ForkOrigin, ListThreadsParams, Provenance, RestartReason, ThreadList, ThreadSummary,
    TitleSource, WorkspaceList,
};
use omfp_protocol::{FileErrorKind, FileErrorReport, ViewErrors, WindowFileErrors};
use omfp_protocol::{
    LibraryComponent, MountParams, MountPlace, MountUpdateParams, Mounted, SaveWidgetParams,
    UnmountParams, Unmounted, WidgetLibrary, WidgetSaved,
};
use omfp_protocol::{
    McpReconnectParams, McpScope, McpServer, McpServers, McpSource, McpState, McpTransport,
};
use omfp_protocol::{ReorderThreadParams, ReorderWorkspaceParams};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::samples::{
    OTHER_WORKSPACE_ID, THREAD_ID, WORKSPACE_ID, git_status, repo_statuses, root_workspace,
    settings, theme, thread, workspace,
};

const REQUEST_ID: &str = "c42";

/// Checks that the example request and response survive a round trip through their JSON text, then returns the golden
/// document for the method.
fn example<M: Method>(params: M::Params, result: &M::Result) -> (&'static str, Value) {
    let request = Request::new(REQUEST_ID, M::call(params));
    assert_eq!(request.call.method(), M::NAME);
    let text = serde_json::to_string(&request).expect("request serializes");
    assert_eq!(
        decode_request(&text).expect("example request decodes"),
        request
    );

    let response =
        ServerFrame::Response(Response::ok(REQUEST_ID, result).expect("result serializes"));
    let text = serde_json::to_string(&response).expect("response serializes");
    let decoded: ServerFrame = serde_json::from_str(&text).expect("example response decodes");
    assert_eq!(decoded, response);
    let ServerFrame::Response(Response {
        result: Ok(value), ..
    }) = decoded
    else {
        panic!("expected a successful response, got {decoded:?}");
    };
    let typed: M::Result = serde_json::from_value(value).expect("result decodes");
    assert_eq!(&typed, result);

    (M::NAME, json!({ "request": request, "response": response }))
}

/// Builds an omp-defined value from its JSON form.
fn wire<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("valid omp wire value")
}

fn thread_ref() -> ThreadRef {
    ThreadRef {
        thread_id: THREAD_ID.to_owned(),
    }
}

fn png() -> ImageContent {
    wire(json!({ "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" }))
}

fn model_info() -> Value {
    json!({
        "id": "scripted",
        "name": "Scripted",
        "api": "openai-completions",
        "provider": "fake",
        "baseUrl": "http://127.0.0.1:4000/v1",
        "reasoning": false,
        "input": ["text"],
        "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0 },
        "contextWindow": 128_000,
        "maxTokens": 8192
    })
}

fn system_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::SystemHello>(
            HelloParams {
                token: "3f9c1e…".to_owned(),
                protocol: PROTOCOL_VERSION,
                client: "omfp/web".to_owned(),
            },
            &HelloResult {
                protocol: PROTOCOL_VERSION,
                daemon: "0.1.0".to_owned(),
                omp: "18.18.6".to_owned(),
                warnings: vec!["omp version mismatch allowed (--allow-omp-mismatch)".to_owned()],
                widget_base: "/widgets/8c1f…".to_owned(),
                file_base: "/files/a07e…".to_owned(),
                connection: 3,
            },
        ),
        example::<method::Subscribe>(
            SubscribeParams {
                topic: Topic::Thread(THREAD_ID.to_owned()),
                since: Some(Position {
                    epoch: "e7".to_owned(),
                    seq: 1200,
                }),
            },
            &SubscribeResult::Snapshot {
                epoch: "e9".to_owned(),
                seq: 40,
                state: json!({ "transcript": [], "queue": { "steering": [], "followUp": [] } }),
            },
        ),
        example::<method::Unsubscribe>(
            TopicParams {
                topic: Topic::Index,
            },
            &Empty {},
        ),
    ]
}

fn repository_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ReposSuggest>(
            SuggestReposParams {
                query: Some("shop".to_owned()),
            },
            &RepoSuggestions {
                repos: vec![SeenRepository {
                    path: "/home/dev/src/shop-frontend".to_owned(),
                    name: "shop-frontend".to_owned(),
                    last_seen_at: 1_794_268_800_000,
                }],
            },
        ),
        example::<method::FsComplete>(
            CompletePathParams {
                path: "~/src/sh".to_owned(),
            },
            &PathCompletions {
                entries: vec![
                    PathEntry {
                        name: "shop-backend".to_owned(),
                        path: "/home/dev/src/shop-backend".to_owned(),
                        is_git_root: true,
                    },
                    PathEntry {
                        name: "shared".to_owned(),
                        path: "/home/dev/src/shared".to_owned(),
                        is_git_root: false,
                    },
                ],
            },
        ),
    ]
}

fn workspace_ref() -> WorkspaceRef {
    WorkspaceRef {
        workspace_id: WORKSPACE_ID.to_owned(),
    }
}

fn workspace_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::WorkspacesCreate>(
            CreateWorkspaceParams {
                name: None,
                link: Some("/home/dev/src/shop-backend".to_owned()),
            },
            &workspace(),
        ),
        example::<method::WorkspacesUpdate>(
            UpdateWorkspaceParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                name: "checkout-v2".to_owned(),
            },
            &workspace(),
        ),
        example::<method::WorkspacesRemove>(
            RemoveWorkspaceParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                force: false,
            },
            &Empty {},
        ),
        example::<method::WorkspacesRescan>(workspace_ref(), &workspace()),
        example::<method::WorkspacesAddRepo>(
            AddRepoParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                name: None,
                source: RepoSource::Worktree {
                    repo: "/home/dev/src/shop-frontend".to_owned(),
                    branch: None,
                    base_ref: Some("origin/main".to_owned()),
                },
            },
            &workspace(),
        ),
        example::<method::WorkspacesRemoveRepo>(
            RemoveRepoParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                name: "backend".to_owned(),
                force: false,
                delete_branch: false,
            },
            &workspace(),
        ),
        example::<method::WorkspacesStatus>(
            RepoRef {
                workspace_id: WORKSPACE_ID.to_owned(),
                repo: "shop-frontend".to_owned(),
            },
            &git_status(),
        ),
        example::<method::WorkspacesDiff>(
            WorkspaceDiffParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                repo: "shop-frontend".to_owned(),
                path: None,
                cursor: Some("p2".to_owned()),
            },
            &DiffPage {
                files: vec![FileDiff {
                    path: "src/checkout.rs".to_owned(),
                    patch: "@@ -1,3 +1,4 @@\n use std::fmt;\n+use crate::cart;\n".to_owned(),
                    truncated: false,
                }],
                next_cursor: Some("p3".to_owned()),
            },
        ),
    ]
}

/// The files of a workspace (DMN-089, DMN-090, DMN-092).
fn file_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::WorkspacesReadFile>(
            ReadFileParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                path: "src/checkout.rs".to_owned(),
                known_hash: None,
            },
            &FileContent {
                path: "shop-frontend/src/checkout.rs".to_owned(),
                kind: FileKind::Text,
                size: 31,
                hash: "3f786850e387550fdab836ed7e6dc881de23001b".to_owned(),
                unchanged: false,
                content: Some("use std::fmt;\nuse crate::cart;\n".to_owned()),
                truncated: false,
                mime: None,
                repo: Some("shop-frontend".to_owned()),
                head: Some("use std::fmt;\n".to_owned()),
                head_hash: None,
                outside: false,
            },
        ),
        example::<method::WorkspacesFindFile>(
            FindFileParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                query: "checkout.rs".to_owned(),
                thread_id: Some(THREAD_ID.to_owned()),
                fuzzy: true,
            },
            &FileCandidates {
                candidates: vec![
                    FileCandidate {
                        path: "shop-frontend/src/checkout.rs".to_owned(),
                        repo: Some("shop-frontend".to_owned()),
                        matched: FileMatch::Basename,
                        mentioned: true,
                    },
                    FileCandidate {
                        path: "shop-backend/src/checkout.rs".to_owned(),
                        repo: Some("shop-backend".to_owned()),
                        matched: FileMatch::Basename,
                        mentioned: false,
                    },
                    FileCandidate {
                        path: "shop-frontend/tests/check-out.rs".to_owned(),
                        repo: Some("shop-frontend".to_owned()),
                        matched: FileMatch::Fuzzy,
                        mentioned: false,
                    },
                ],
                truncated: false,
            },
        ),
        example::<method::WorkspacesWriteFile>(
            WriteFileParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                path: "shop-frontend/src/checkout.rs".to_owned(),
                content: "use std::fmt;\n".to_owned(),
                expected_hash: Some("3f786850e387550fdab836ed7e6dc881de23001b".to_owned()),
            },
            &FileWritten {
                path: "shop-frontend/src/checkout.rs".to_owned(),
                hash: "89e6c98d92887913cadf06b2adb97f26cde4849b".to_owned(),
                size: 14,
            },
        ),
        example::<method::WorkspacesReorder>(
            ReorderWorkspaceParams {
                workspace_id: OTHER_WORKSPACE_ID.to_owned(),
                before_id: Some(WORKSPACE_ID.to_owned()),
            },
            &Empty {},
        ),
    ]
}

fn threads_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadsCreate>(
            CreateThreadParams {
                title: None,
                workspace: ThreadWorkspace::New(CreateWorkspaceParams {
                    name: Some("checkout-v2".to_owned()),
                    link: None,
                }),
                model: Some("anthropic/claude-sonnet-4-5".to_owned()),
                thinking_level: Some(ThinkingLevel::High),
                approval_mode: Some(ApprovalMode::AlwaysAsk),
                prompt: Some("Implement checkout v2 as described in docs/checkout.md".to_owned()),
                images: vec![png()],
            },
            &thread(),
        ),
        example::<method::ThreadsUpdate>(
            UpdateThreadParams {
                thread_id: THREAD_ID.to_owned(),
                title: Some("Checkout v2".to_owned()),
                approval_mode: Some(ApprovalMode::AlwaysAsk),
            },
            &thread(),
        ),
        example::<method::ThreadsReorder>(
            ReorderThreadParams {
                thread_id: THREAD_ID.to_owned(),
                workspace_id: Some(OTHER_WORKSPACE_ID.to_owned()),
                before_id: None,
            },
            &Empty {},
        ),
        example::<method::ThreadsArchive>(
            ThreadCloseParams {
                thread_id: THREAD_ID.to_owned(),
                jobs: Some(CloseJobs::Kill),
            },
            &Empty {},
        ),
        example::<method::ThreadsUnarchive>(thread_ref(), &Empty {}),
        example::<method::ThreadsDelete>(
            ThreadCloseParams {
                thread_id: THREAD_ID.to_owned(),
                jobs: Some(CloseJobs::Keep),
            },
            &Empty {},
        ),
        example::<method::ThreadsFork>(
            ForkParams {
                thread_id: THREAD_ID.to_owned(),
                entry_id: Some("e1".to_owned()),
            },
            &forked_thread(),
        ),
        example::<method::ThreadsSearch>(
            SearchThreadsParams {
                query: "flaky".to_owned(),
            },
            &ThreadSearchResults {
                results: vec![ThreadSearchResult {
                    thread_id: THREAD_ID.to_owned(),
                    snippet: "…why is the checkout test flaky on CI?…".to_owned(),
                }],
            },
        ),
    ]
}

fn conversation_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadPrompt>(
            PromptParams {
                thread_id: THREAD_ID.to_owned(),
                text: "Add a test for the cart total".to_owned(),
                images: vec![png()],
                behavior: Some(PromptBehavior::FollowUp),
            },
            &PromptResult {
                prompt_id: "p17".to_owned(),
                agent_invoked: true,
            },
        ),
        example::<method::ThreadSteer>(
            MessageParams {
                thread_id: THREAD_ID.to_owned(),
                text: "Use the existing Money type".to_owned(),
                images: Vec::new(),
            },
            &Empty {},
        ),
        example::<method::ThreadFollowUp>(
            MessageParams {
                thread_id: THREAD_ID.to_owned(),
                text: "Then run the tests".to_owned(),
                images: Vec::new(),
            },
            &Empty {},
        ),
        example::<method::ThreadAbort>(
            thread_ref(),
            &wire(json!({
                "steering": [{ "text": "Use the existing Money type" }],
                "followUp": [{ "text": "Then run the tests" }]
            })),
        ),
        example::<method::ThreadQueueRemove>(
            QueueMessageParams {
                thread_id: THREAD_ID.to_owned(),
                message: "Then run the tests".to_owned(),
                queue: Some(QueuedMessageQueue::FollowUp),
            },
            &wire(json!({ "removed": true })),
        ),
        example::<method::ThreadQueuePromote>(
            QueueMessageParams {
                thread_id: THREAD_ID.to_owned(),
                message: "Then run the tests".to_owned(),
                queue: None,
            },
            &wire(json!({ "promoted": true })),
        ),
    ]
}

fn settings_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadSetModel>(
            SetModelParams {
                thread_id: THREAD_ID.to_owned(),
                provider: "fake".to_owned(),
                model_id: "scripted".to_owned(),
            },
            &Empty {},
        ),
        example::<method::ThreadSetThinking>(
            SetThinkingParams {
                thread_id: THREAD_ID.to_owned(),
                level: ThinkingLevel::Medium,
            },
            &Empty {},
        ),
        example::<method::ThreadModels>(thread_ref(), &wire(json!({ "models": [model_info()] }))),
        example::<method::ThreadThinkingLevels>(
            thread_ref(),
            &wire(json!({ "levels": ["off", "low", "medium", "high"] })),
        ),
        example::<method::ThreadCommands>(
            thread_ref(),
            &wire(json!({
                "commands": [{
                    "name": "compact",
                    "source": "builtin",
                    "description": "Compact the conversation",
                    "input": { "hint": "[instructions]" }
                }]
            })),
        ),
        example::<method::ThreadCompact>(
            CompactParams {
                thread_id: THREAD_ID.to_owned(),
                instructions: Some("Keep the API decisions".to_owned()),
            },
            &wire(json!({
                "summary": "Implemented the cart total; tests pass.",
                "firstKeptEntryId": "e12",
                "tokensBefore": 48_210
            })),
        ),
    ]
}

fn tree_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadTree>(
            thread_ref(),
            &wire(json!({
                "tree": [{ "id": "e1", "type": "message", "children": [] }],
                "leafId": "e1"
            })),
        ),
        example::<method::ThreadNavigate>(
            NavigateParams {
                thread_id: THREAD_ID.to_owned(),
                entry_id: "e1".to_owned(),
                summarize: Some(true),
                instructions: None,
            },
            &wire(json!({ "cancelled": false, "editorText": "Add a test for the cart total" })),
        ),
        example::<method::ThreadFork>(
            ForkParams {
                thread_id: THREAD_ID.to_owned(),
                entry_id: Some("e1".to_owned()),
            },
            &wire(json!({ "cancelled": false })),
        ),
        example::<method::ThreadLabel>(
            LabelParams {
                thread_id: THREAD_ID.to_owned(),
                entry_id: "e1".to_owned(),
                label: Some("before refactor".to_owned()),
            },
            &Empty {},
        ),
    ]
}

fn subagent_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadSubagents>(
            thread_ref(),
            &wire(json!({
                "subagents": [{
                    "id": "sa1",
                    "index": 0,
                    "agent": "explore",
                    "agentSource": "bundled",
                    "status": "running",
                    "lastUpdate": 1_794_268_800_000_i64,
                    "description": "Find the cart code"
                }]
            })),
        ),
        example::<method::ThreadSubagentMessages>(
            SubagentMessagesParams {
                thread_id: THREAD_ID.to_owned(),
                subagent_id: "sa1".to_owned(),
                from_byte: Some(0),
            },
            &wire(json!({
                "sessionFile": "/home/dev/.local/share/omfp/threads/t/sessions/sa1.jsonl",
                "fromByte": 0,
                "nextByte": 0,
                "reset": false,
                "entries": [],
                "messages": []
            })),
        ),
        example::<method::ThreadSubagentSteer>(
            SubagentSteerParams {
                thread_id: THREAD_ID.to_owned(),
                subagent_id: "sa1".to_owned(),
                text: "Only look under src/cart".to_owned(),
            },
            &Empty {},
        ),
        example::<method::ThreadSubagentCancel>(
            SubagentRef {
                thread_id: THREAD_ID.to_owned(),
                subagent_id: "sa1".to_owned(),
            },
            &Empty {},
        ),
    ]
}

fn live_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadLiveConnect>(
            LiveConnectParams {
                thread_id: THREAD_ID.to_owned(),
                sdp_offer: "v=0\r\no=- 46117 2 IN IP4 127.0.0.1\r\n…".to_owned(),
            },
            &LiveConnected {
                sdp_answer: "v=0\r\no=- 7215 1 IN IP4 0.0.0.0\r\n…".to_owned(),
                voice: "sol".to_owned(),
            },
        ),
        example::<method::ThreadLiveStart>(
            thread_ref(),
            &LiveStarted {
                voice: "sol".to_owned(),
            },
        ),
        example::<method::ThreadLiveStop>(thread_ref(), &Empty {}),
        example::<method::ThreadLiveMute>(
            LiveMuteParams {
                thread_id: THREAD_ID.to_owned(),
                muted: true,
            },
            &LiveMuted { muted: true },
        ),
        example::<method::InteractionRespond>(
            RespondParams {
                thread_id: THREAD_ID.to_owned(),
                interaction_id: "ui-7".to_owned(),
                response: InteractionResponse::Confirmed(true),
            },
            &Empty {},
        ),
    ]
}

/// A running thread's session facts (`thread.session`, PRT-125).
fn thread_session() -> ThreadSession {
    ThreadSession {
        thread_id: THREAD_ID.to_owned(),
        workspace_id: WORKSPACE_ID.to_owned(),
        cwd: "/home/ada/omfp/workspaces/checkout-v2".to_owned(),
        roots: vec!["/home/ada/omfp/workspaces/checkout-v2/shop".to_owned()],
        session_dir: format!("/home/ada/.local/share/omfp/threads/{THREAD_ID}/sessions"),
        session_id: Some("0f8e2c4a-5b1d-4e7f-9a3c-2d6b8e1f4a7c".to_owned()),
        session_file: Some(format!(
            "/home/ada/.local/share/omfp/threads/{THREAD_ID}/sessions/\
             2026-11-13T09-12-44-120Z_0f8e2c4a-5b1d-4e7f-9a3c-2d6b8e1f4a7c.jsonl"
        )),
        model: Some("anthropic/claude-sonnet-4-5".to_owned()),
        // A running thread: only a failed start reads the saved model from the session file.
        session_model: None,
        thinking_level: Some("high".to_owned()),
        approval_mode: Some(ApprovalMode::Yolo),
        process: ProcessState::Running,
        settled: false,
        needs_attention: false,
        message_count: Some(14),
        usage: Some(SessionUsage {
            cost: 0.4213,
            tokens: SessionTokens {
                input: 48_210,
                output: 6_532,
                cache_read: 210_400,
                cache_write: 12_800,
                total: 277_942,
            },
            context: Some(ContextUsage {
                tokens: 61_250,
                context_window: 200_000,
                percent: 30.625,
            }),
        }),
    }
}

fn tool_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadGoal>(
            GoalParams {
                thread_id: THREAD_ID.to_owned(),
                op: GoalOp::Create,
                objective: Some("Ship checkout v2".to_owned()),
                token_budget: Some(200_000),
            },
            &wire(json!({
                "goal": {
                    "id": "g1",
                    "objective": "Ship checkout v2",
                    "status": "active",
                    "tokensUsed": 0,
                    "timeUsedSeconds": 0.0,
                    "createdAt": 1_794_268_800_000_i64,
                    "updatedAt": 1_794_268_800_000_i64,
                    "tokenBudget": 200_000
                },
                "state": null
            })),
        ),
        example::<method::ThreadBtw>(
            BtwParams {
                thread_id: THREAD_ID.to_owned(),
                question: "Which test runner does this repo use?".to_owned(),
                record_id: None,
            },
            &wire(json!({
                "record": {
                    "id": "b1",
                    "question": "Which test runner does this repo use?",
                    "answer": "cargo-nextest",
                    "status": "complete",
                    "createdAt": 1_794_268_800_000_i64,
                    "updatedAt": 1_794_268_801_000_i64,
                    "leafId": null
                }
            })),
        ),
        example::<method::ThreadBash>(
            BashParams {
                thread_id: THREAD_ID.to_owned(),
                command: "git log --oneline -1".to_owned(),
                exclude_from_context: Some(true),
            },
            &wire(json!({
                "output": "f6ab326 feat(daemon): add omp process adapter\n",
                "cancelled": false,
                "truncated": false,
                "totalLines": 1,
                "totalBytes": 45,
                "outputLines": 1,
                "outputBytes": 45,
                "exitCode": 0
            })),
        ),
        example::<method::ThreadGit>(
            thread_ref(),
            &ThreadGit {
                workspace_id: WORKSPACE_ID.to_owned(),
                repos: repo_statuses(),
            },
        ),
        example::<method::ThreadState>(
            thread_ref(),
            &wire(
                json!({ "sessionId": "0f8e2c4a-5b1d-4e7f-9a3c-2d6b8e1f4a7c", "model": model_info() }),
            ),
        ),
        example::<method::ThreadSession>(thread_ref(), &thread_session()),
        example::<method::ThreadRestart>(thread_ref(), &Empty {}),
        example::<method::ThreadStop>(
            StopThreadParams {
                thread_id: THREAD_ID.to_owned(),
                cascade: true,
            },
            &ThreadsStopped {
                thread_ids: vec![
                    THREAD_ID.to_owned(),
                    "01JD8X3Q4T9V2W6Y5Z7A1B3C4E".to_owned(),
                ],
            },
        ),
    ]
}

fn theme_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThemeGet>(Empty {}, &theme()),
        example::<method::ThemeSet>(theme(), &Empty {}),
    ]
}

fn ui_settings_examples() -> Vec<(&'static str, Value)> {
    let set = settings();
    vec![
        example::<method::SettingsGet>(Empty {}, &set),
        example::<method::SettingsSet>(
            SetSettingsParams {
                ui: Some(set.ui.clone()),
                keymap: Some(set.keymap.clone()),
            },
            &set,
        ),
    ]
}

fn widget_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ViewsSet>(
            SetViewsParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                manifest: json!({
                    "default": "work",
                    "views": { "work": { "title": "Work", "layout": ["row", {}, "omfp:chat", "./widgets/tests"] } }
                }),
            },
            &Empty {},
        ),
        example::<method::WidgetsCopy>(
            CopyWidgetParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                widget: "omfp:workspace".to_owned(),
                name: None,
            },
            &WidgetCopied {
                name: "workspace".to_owned(),
            },
        ),
        example::<method::WidgetsStateSet>(
            SetWidgetStateParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                instance: "./widgets/tests".to_owned(),
                key: "job:cargo nextest run".to_owned(),
                value: json!("01JOB"),
            },
            &Empty {},
        ),
        example::<method::WidgetsReport>(
            ReportWidgetsParams {
                workspace_id: Some(WORKSPACE_ID.to_owned()),
                views: Some(Position {
                    epoch: "01EPOCH".to_owned(),
                    seq: 4,
                }),
                widgets: vec![WidgetReport {
                    reference: "./widgets/tests".to_owned(),
                    instance: "./widgets/tests".to_owned(),
                    source: WidgetSource::Workspace,
                    name: "tests".to_owned(),
                    version: Some("3f2a".to_owned()),
                    state: WidgetReportState::Rendered,
                    errors: Vec::new(),
                    console: vec!["TypeError: x is undefined".to_owned()],
                    text: "Tests 1/3".to_owned(),
                    jobs: vec![WidgetJobReport {
                        command: "make test".to_owned(),
                        status: "running".to_owned(),
                        exit_code: None,
                        signal: None,
                        runs: 1,
                        started_at: Some(1_700_000_000_000),
                        tail: vec!["PASS 1".to_owned()],
                    }],
                }],
                file_errors: vec![FileErrorReport {
                    kind: FileErrorKind::Init,
                    file: ".omfp/init.ts".to_owned(),
                    message: ".omfp/init.ts:3:7: Unexpected token".to_owned(),
                }],
                window: Some(window_report()),
            },
            &Empty {},
        ),
        example::<method::JobsStart>(
            StartJobParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                command: "cargo nextest run".to_owned(),
                replaces: Some("01JOBOLD".to_owned()),
            },
            &JobInfo {
                id: "01JOB".to_owned(),
                workspace_id: WORKSPACE_ID.to_owned(),
                command: "cargo nextest run".to_owned(),
                started_at: 1_700_000_000_000,
                status: JobStatus::Running,
            },
        ),
        example::<method::JobsKill>(
            JobRef {
                job_id: "01JOB".to_owned(),
            },
            &Empty {},
        ),
    ]
}

fn terminal_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::TerminalsStart>(
            StartTerminalParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                command: "read -s password".to_owned(),
                cwd: Some("app".to_owned()),
                cols: 100,
                rows: 30,
            },
            &TerminalInfo {
                id: "01TERMINAL".to_owned(),
                workspace_id: WORKSPACE_ID.to_owned(),
                command: "read -s password".to_owned(),
                cwd: "/work/app".to_owned(),
                cols: 100,
                rows: 30,
                started_at: 1_700_000_000_000,
                status: TerminalStatus::Running,
            },
        ),
        example::<method::TerminalsScreen>(
            TerminalScreenParams {
                terminal_id: "01TERMINAL".to_owned(),
                after_revision: Some(7),
                wait_ms: Some(10_000),
            },
            &TerminalScreen {
                revision: 8,
                cols: 100,
                rows: 30,
                contents: "Password:".to_owned(),
                cursor: TerminalCursor {
                    row: 0,
                    column: 9,
                    visible: true,
                },
                alternate_screen: false,
                status: TerminalStatus::Running,
            },
        ),
        example::<method::TerminalsInput>(
            TerminalInputParams {
                terminal_id: "01TERMINAL".to_owned(),
                data: "c2VjcmV0DQ==".to_owned(),
            },
            &Empty {},
        ),
        example::<method::TerminalsResize>(
            TerminalResizeParams {
                terminal_id: "01TERMINAL".to_owned(),
                cols: 120,
                rows: 40,
            },
            &Empty {},
        ),
        example::<method::TerminalsKill>(
            TerminalRef {
                terminal_id: "01TERMINAL".to_owned(),
            },
            &Empty {},
        ),
    ]
}

fn nightly_job() -> JobDefinition {
    JobDefinition {
        description: Some("Nightly test run".to_owned()),
        trigger: JobTrigger::Schedule,
        schedule: Some("0 3 * * *".to_owned()),
        missed: MissedPolicy::Run,
        command: Some("cargo test".to_owned()),
        prompt: None,
        cwd: Some("app".to_owned()),
        restart: RestartPolicy::Always,
        enabled: true,
    }
}

fn nightly_run(status: JobRunStatus, ended_at: Option<i64>, exit_code: Option<i32>) -> RunInfo {
    RunInfo {
        id: "01RUN".to_owned(),
        workspace_id: WORKSPACE_ID.to_owned(),
        job: Some("nightly".to_owned()),
        trigger: RunTrigger::Manual,
        started_by: Some(CreatedBy::User),
        thread_id: None,
        command: Some("cargo test".to_owned()),
        started_at: 1_700_000_000_000,
        ended_at,
        status,
        reason: None,
        exit_code,
        signal: None,
    }
}

fn nightly_view(enabled: bool) -> JobView {
    JobView {
        name: "nightly".to_owned(),
        job: JobDefinition {
            enabled,
            ..nightly_job()
        },
        status: if enabled {
            JobViewStatus::Idle
        } else {
            JobViewStatus::Disabled
        },
        run: None,
        last_run: Some(nightly_run(
            JobRunStatus::Succeeded,
            Some(1_700_000_060_000),
            Some(0),
        )),
        next_at: enabled.then_some(1_700_010_000_000),
    }
}

fn triage_view() -> JobView {
    JobView {
        name: "triage".to_owned(),
        job: JobDefinition {
            description: None,
            trigger: JobTrigger::Manual,
            schedule: None,
            missed: MissedPolicy::Skip,
            command: None,
            prompt: Some(JobPrompt {
                text: "Triage the new issues".to_owned(),
                thread: PromptThread::Reuse,
                model: Some("fake/scripted".to_owned()),
                thinking_level: None,
                approval_mode: None,
            }),
            cwd: None,
            restart: RestartPolicy::Always,
            enabled: true,
        },
        status: JobViewStatus::Running,
        run: Some(RunInfo {
            id: "01RUNPROMPT".to_owned(),
            job: Some("triage".to_owned()),
            thread_id: Some(THREAD_ID.to_owned()),
            command: None,
            ..nightly_run(JobRunStatus::Running, None, None)
        }),
        last_run: None,
        next_at: None,
    }
}

fn web_view() -> JobView {
    JobView {
        name: "web".to_owned(),
        job: JobDefinition {
            description: None,
            trigger: JobTrigger::Service,
            schedule: None,
            missed: MissedPolicy::Skip,
            command: Some("npm run dev".to_owned()),
            prompt: None,
            cwd: None,
            restart: RestartPolicy::OnFailure,
            enabled: true,
        },
        status: JobViewStatus::Waiting,
        run: None,
        last_run: Some(RunInfo {
            id: "01RUNWEB".to_owned(),
            job: Some("web".to_owned()),
            trigger: RunTrigger::Service,
            command: Some("npm run dev".to_owned()),
            started_by: None,
            signal: Some(9),
            ..nightly_run(JobRunStatus::Failed, Some(1_700_000_005_000), None)
        }),
        next_at: Some(1_700_000_007_000),
    }
}

fn jobs_examples() -> Vec<(&'static str, Value)> {
    let prompt_job = triage_view();
    let service_job = web_view();
    vec![
        example::<method::JobsList>(
            workspace_ref(),
            &JobsState {
                jobs: vec![nightly_view(true), prompt_job, service_job],
                error: None,
            },
        ),
        example::<method::JobsRuns>(
            JobRunsParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                job: "nightly".to_owned(),
                before: Some("01RUNOLDER".to_owned()),
                limit: Some(2),
            },
            &JobRuns {
                runs: vec![
                    nightly_run(JobRunStatus::Succeeded, Some(1_700_000_060_000), Some(0)),
                    RunInfo {
                        id: "01RUNSKIPPED".to_owned(),
                        trigger: RunTrigger::Schedule,
                        started_by: None,
                        reason: Some("the previous run still runs".to_owned()),
                        started_at: 1_699_999_000_000,
                        ..nightly_run(JobRunStatus::Skipped, Some(1_699_999_000_000), None)
                    },
                ],
                more: true,
            },
        ),
        example::<method::JobsRun>(
            JobNameParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                job: "nightly".to_owned(),
            },
            &nightly_run(JobRunStatus::Running, None, None),
        ),
        example::<method::JobsDefine>(
            DefineJobParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                name: "nightly".to_owned(),
                job: json!({
                    "description": "Nightly test run",
                    "trigger": "schedule",
                    "schedule": "0 3 * * *",
                    "missed": "run",
                    "command": "cargo test",
                    "cwd": "app"
                }),
                replace: Some(true),
            },
            &nightly_view(true),
        ),
        example::<method::JobsRemove>(
            JobRemoveParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                name: "nightly".to_owned(),
            },
            &Empty {},
        ),
        example::<method::JobsEnable>(
            EnableJobParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                name: "nightly".to_owned(),
                enabled: false,
            },
            &nightly_view(false),
        ),
    ]
}

fn view_error_examples() -> Vec<(&'static str, Value)> {
    vec![example::<method::ViewsErrors>(
        workspace_ref(),
        &ViewErrors {
            windows: vec![
                WindowFileErrors {
                    client: "web".to_owned(),
                    errors: vec![
                        FileErrorReport {
                            kind: FileErrorKind::Init,
                            file: "user".to_owned(),
                            message: "TypeError: x is undefined".to_owned(),
                        },
                        FileErrorReport {
                            kind: FileErrorKind::View,
                            file: ".omfp/views/ops.tsx".to_owned(),
                            message: "Row: unknown widget ./widgets/nope".to_owned(),
                        },
                    ],
                },
                WindowFileErrors {
                    client: "desktop".to_owned(),
                    errors: Vec::new(),
                },
            ],
        },
    )]
}

/// What a window reports besides its widgets (PRT-122).
fn window_report() -> WindowReport {
    WindowReport {
        thread_id: Some(THREAD_ID.to_owned()),
        focused: true,
        active_view: Some("ops".to_owned()),
        default_view: Some("default".to_owned()),
        views: vec![
            WindowView {
                id: "default".to_owned(),
                title: "Default".to_owned(),
                source: ViewSource::ViewsJson,
                file: None,
                widgets: Some(vec![
                    "omfp:header".to_owned(),
                    "omfp:chat".to_owned(),
                    "./widgets/tests".to_owned(),
                ]),
            },
            WindowView {
                id: "ops".to_owned(),
                title: "Ops".to_owned(),
                source: ViewSource::WorkspaceInit,
                file: Some("./views/ops.tsx".to_owned()),
                widgets: None,
            },
        ],
        commands: vec![WindowCommand {
            id: "demo.solo".to_owned(),
            title: "Show the init view".to_owned(),
            aliases: vec!["solo".to_owned()],
            source: CommandSource::WorkspaceInit,
            widget: None,
            enabled: true,
            bindings: vec![WindowBinding {
                mode: KeymapMode::Normal,
                keys: "g h".to_owned(),
                args: Vec::new(),
            }],
        }],
    }
}

fn widget_report(state: WidgetReportState, errors: Vec<String>) -> WidgetReport {
    WidgetReport {
        reference: "./widgets/tests".to_owned(),
        instance: "./widgets/tests@check".to_owned(),
        source: WidgetSource::Workspace,
        name: "tests".to_owned(),
        version: Some("3f2a".to_owned()),
        state,
        errors,
        console: Vec::new(),
        text: "Tests 1/3".to_owned(),
        jobs: Vec::new(),
    }
}

/// The windows as agents see and drive them (PRT-123, PRT-124).
fn window_examples() -> Vec<(&'static str, Value)> {
    let report = window_report();
    vec![
        example::<method::WindowsList>(
            ListWindowsParams {
                workspace_id: Some(WORKSPACE_ID.to_owned()),
            },
            &WindowList {
                windows: vec![WindowSummary {
                    window_id: 3,
                    client: "web".to_owned(),
                    workspace_id: Some(WORKSPACE_ID.to_owned()),
                    thread_id: Some(THREAD_ID.to_owned()),
                    focused: true,
                    active_view: Some("ops".to_owned()),
                    errors: 1,
                }],
            },
        ),
        example::<method::WindowsState>(
            WindowRef { window_id: 3 },
            &WindowState {
                window_id: 3,
                client: "web".to_owned(),
                workspace_id: Some(WORKSPACE_ID.to_owned()),
                thread_id: report.thread_id,
                focused: report.focused,
                active_view: report.active_view,
                default_view: report.default_view,
                views: report.views,
                commands: report.commands,
                file_errors: vec![FileErrorReport {
                    kind: FileErrorKind::View,
                    file: ".omfp/views/ops.tsx".to_owned(),
                    message: "Row: unknown widget ./widgets/nope".to_owned(),
                }],
                widgets: vec![widget_report(WidgetReportState::Rendered, Vec::new())],
            },
        ),
        example::<method::WindowsRun>(
            RunInWindowParams {
                window_id: None,
                workspace_id: Some(WORKSPACE_ID.to_owned()),
                command: "view".to_owned(),
                args: vec!["ops".to_owned()],
            },
            &RanInWindow {
                window_id: 3,
                command: "view.switch".to_owned(),
            },
        ),
        example::<method::WindowsCheckWidget>(
            CheckWidgetParams {
                window_id: None,
                workspace_id: WORKSPACE_ID.to_owned(),
                widget: "./widgets/tests".to_owned(),
            },
            &WidgetChecked {
                window_id: 3,
                report: widget_report(
                    WidgetReportState::RenderError,
                    vec!["x is not defined".to_owned()],
                ),
                settled: true,
            },
        ),
        example::<method::WindowsAnswer>(
            AnswerWindowParams {
                request_id: "w7".to_owned(),
                answer: WindowAnswer::NotRun {
                    reason: RunFailure::Disabled,
                    message: "view.next is disabled".to_owned(),
                },
            },
            &Empty {},
        ),
    ]
}

fn mount_examples() -> Vec<(&'static str, Value)> {
    let props = json!({ "title": "Checkout", "threads": ["01T1", "01T2"] });
    vec![
        example::<method::ViewsMount>(
            MountParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                id: "checkout".to_owned(),
                widget: None,
                code: Some("export default function Checkout() { return <p>v2</p>; }\n".to_owned()),
                props: props.as_object().cloned(),
                view: Some("dashboard".to_owned()),
                place: Some(MountPlace::End),
                panel: false,
            },
            &Mounted {
                id: "checkout".to_owned(),
                reference: "./widgets/checkout".to_owned(),
                view: Some("dashboard".to_owned()),
                panel: false,
                replaced: false,
            },
        ),
        example::<method::ViewsMountUpdate>(
            MountUpdateParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                id: "checkout".to_owned(),
                props: json!({ "progress": 0.5, "title": null })
                    .as_object()
                    .cloned(),
                code: None,
            },
            &Mounted {
                id: "checkout".to_owned(),
                reference: "./widgets/checkout".to_owned(),
                view: Some("dashboard".to_owned()),
                panel: false,
                replaced: true,
            },
        ),
        example::<method::ViewsUnmount>(
            UnmountParams {
                workspace_id: WORKSPACE_ID.to_owned(),
                id: "checkout".to_owned(),
            },
            &Unmounted {
                views: Vec::new(),
                removed_views: vec!["dashboard".to_owned()],
                panel: false,
            },
        ),
        example::<method::WidgetsSave>(
            SaveWidgetParams {
                name: "checkout".to_owned(),
                workspace_id: Some(WORKSPACE_ID.to_owned()),
                widget: Some("./widgets/checkout".to_owned()),
                code: None,
                replace: true,
            },
            &WidgetSaved {
                name: "checkout".to_owned(),
                reference: "user:checkout".to_owned(),
            },
        ),
        example::<method::WidgetsLibrary>(
            Empty {},
            &WidgetLibrary {
                components: vec![LibraryComponent {
                    name: "checkout".to_owned(),
                    reference: "user:checkout".to_owned(),
                    entry: Some("index.tsx".to_owned()),
                    files: vec!["index.tsx".to_owned()],
                    version: "3f2a".to_owned(),
                    description: "Threads of a feature with their live state; props: threads."
                        .to_owned(),
                }],
            },
        ),
    ]
}

/// The activity log of agent-originated actions (PRT-086, AGT-072).
fn activity_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ActivityList>(
            ListActivityParams {
                thread_id: Some(THREAD_ID.to_owned()),
                before: Some(42),
                after: None,
                limit: Some(1),
            },
            &ActivityPage {
                entries: vec![ActivityEntry {
                    seq: 41,
                    at: 1_731_000_000_000,
                    thread_id: Some(THREAD_ID.to_owned()),
                    client: "omfp-agent".to_owned(),
                    method: "threads.create".to_owned(),
                    params: json!({ "workspace": { "id": WORKSPACE_ID }, "prompt": "Fix the flaky test" }),
                    error: None,
                }],
                more: true,
            },
        ),
        example::<method::PromptsHistory>(
            PromptHistoryParams {
                thread_id: Some(THREAD_ID.to_owned()),
                workspace_id: Some(WORKSPACE_ID.to_owned()),
                query: Some("flaky".to_owned()),
                limit: Some(2),
            },
            &PromptHistory {
                prompts: vec![PromptHistoryEntry {
                    text: "Fix the flaky test".to_owned(),
                    thread_id: THREAD_ID.to_owned(),
                    workspace_id: WORKSPACE_ID.to_owned(),
                    at: 1_731_000_000_000,
                }],
            },
        ),
    ]
}

/// The `index` topic's entities without a subscription (PRT-090, PRT-091).
fn list_examples() -> Vec<(&'static str, Value)> {
    vec![
        example::<method::ThreadsList>(
            ListThreadsParams {
                workspace_id: Some(WORKSPACE_ID.to_owned()),
                include_archived: true,
            },
            &ThreadList {
                threads: vec![thread()],
            },
        ),
        example::<method::WorkspacesList>(
            Empty {},
            &WorkspaceList {
                workspaces: vec![workspace()],
            },
        ),
    ]
}

/// The MCP servers of a workspace and a thread (PRT-110..PRT-114).
fn mcp_examples() -> Vec<(&'static str, Value)> {
    let project = McpSource {
        path: "/work/checkout/.omp/mcp.json".to_owned(),
        workspace_path: Some(".omp/mcp.json".to_owned()),
        scope: McpScope::Project,
        tool: "omp".to_owned(),
    };
    let docs = McpServer {
        name: "docs".to_owned(),
        transport: McpTransport::Http,
        target: "https://docs.example/mcp".to_owned(),
        source: project.clone(),
        state: McpState::Configured,
        checkable: true,
        tools: None,
        error: None,
    };
    let fs = McpServer {
        name: "fs".to_owned(),
        transport: McpTransport::Stdio,
        target: "npx -y fs-server".to_owned(),
        source: McpSource {
            path: "/home/u/.cursor/mcp.json".to_owned(),
            workspace_path: None,
            scope: McpScope::User,
            tool: "cursor".to_owned(),
        },
        state: McpState::Configured,
        checkable: false,
        tools: None,
        error: None,
    };
    let listing = |thread_id: Option<&str>, servers: Vec<McpServer>| McpServers {
        workspace_id: WORKSPACE_ID.to_owned(),
        thread_id: thread_id.map(str::to_owned),
        running: thread_id.is_some(),
        project_config: "/work/checkout/.omp/mcp.json".to_owned(),
        user_config: Some("/home/u/.omp/agent/mcp.json".to_owned()),
        servers,
    };
    let connected = McpServer {
        state: McpState::Connected,
        tools: Some(2),
        ..docs.clone()
    };
    let failed = McpServer {
        name: "broken".to_owned(),
        transport: McpTransport::Stdio,
        target: "/nonexistent/mcp-server".to_owned(),
        source: project,
        state: McpState::Failed,
        checkable: true,
        tools: None,
        error: Some(
            "ENOENT: no such file or directory, posix_spawn '/nonexistent/mcp-server'".to_owned(),
        ),
    };
    vec![
        example::<method::WorkspacesMcpServers>(
            WorkspaceRef {
                workspace_id: WORKSPACE_ID.to_owned(),
            },
            &listing(None, vec![docs, fs.clone()]),
        ),
        example::<method::ThreadMcpServers>(
            ThreadRef {
                thread_id: THREAD_ID.to_owned(),
            },
            &listing(Some(THREAD_ID), vec![connected.clone(), failed, fs.clone()]),
        ),
        example::<method::ThreadMcpReconnect>(
            McpReconnectParams {
                thread_id: THREAD_ID.to_owned(),
                name: Some("docs".to_owned()),
            },
            &listing(Some(THREAD_ID), vec![connected, fs]),
        ),
    ]
}

fn auth_examples() -> Vec<(&'static str, Value)> {
    let anthropic = || ProviderRef {
        provider_id: "anthropic".to_owned(),
    };
    vec![
        example::<method::AuthStatus>(Empty {}, &AuthStatus { models: 0 }),
        example::<method::AuthModels>(Empty {}, &wire(json!({ "models": [model_info()] }))),
        example::<method::AuthProviders>(
            Empty {},
            &AuthProviders {
                providers: vec![
                    AuthProvider {
                        id: "openai-codex".to_owned(),
                        name: "ChatGPT Plus/Pro (Codex Subscription)".to_owned(),
                        available: true,
                        authenticated: true,
                    },
                    AuthProvider {
                        id: "anthropic".to_owned(),
                        name: "Anthropic (Claude Pro/Max)".to_owned(),
                        available: true,
                        authenticated: false,
                    },
                ],
            },
        ),
        example::<method::AuthLogin>(
            anthropic(),
            &LoggedIn {
                provider_id: "anthropic".to_owned(),
                name: "Anthropic (Claude Pro/Max)".to_owned(),
            },
        ),
        example::<method::AuthLoginCancel>(
            Empty {},
            &LoginCancelled {
                provider_id: Some("anthropic".to_owned()),
            },
        ),
        example::<method::AuthLogout>(
            anthropic(),
            &LoggedOut {
                provider_id: "anthropic".to_owned(),
                removed: 1,
                remaining_source: Some("ANTHROPIC_API_KEY".to_owned()),
            },
        ),
        example::<method::OauthCallback>(
            OAuthCallbackParams {
                thread_id: Some(THREAD_ID.to_owned()),
                url: "http://localhost:1455/auth/callback?code=c0de&state=5tate".to_owned(),
            },
            &OAuthCallbackRelayed { status: 200 },
        ),
    ]
}

// spec: PRT-212
#[test]
fn an_oauth_callback_without_a_thread_is_the_running_logins() {
    let login: OAuthCallbackParams =
        serde_json::from_value(json!({ "url": "http://localhost:1455/auth/callback?code=c" }))
            .expect("decodes");
    assert_eq!(login.thread_id, None);
    assert_eq!(
        serde_json::to_value(&login).unwrap(),
        json!({ "url": "http://localhost:1455/auth/callback?code=c" })
    );
}

// spec: PRT-085
#[test]
fn stop_cascades_only_when_asked() {
    let plain: StopThreadParams =
        serde_json::from_value(json!({ "threadId": THREAD_ID })).expect("decodes");
    assert!(!plain.cascade);
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        json!({ "threadId": THREAD_ID })
    );
}

// spec: PRT-086
#[test]
fn activity_entries_carry_their_outcome() {
    let failed = ActivityEntry {
        seq: 7,
        at: 1,
        thread_id: Some(THREAD_ID.to_owned()),
        client: "omfp-agent".to_owned(),
        method: "thread.prompt".to_owned(),
        params: json!({ "threadId": THREAD_ID, "text": "hi" }),
        error: Some("thread 01J not found".to_owned()),
    };
    let value = serde_json::to_value(&failed).unwrap();
    assert_eq!(value["error"], json!("thread 01J not found"));
    let succeeded = ActivityEntry {
        error: None,
        ..failed
    };
    assert_eq!(
        serde_json::to_value(&succeeded).unwrap()["error"],
        Value::Null
    );
    let params: ListActivityParams = serde_json::from_value(json!({})).expect("decodes");
    assert_eq!(params, ListActivityParams::default());
}

// spec: PRT-094, AGT-053
#[test]
fn an_mcp_entry_has_a_null_thread_and_names_its_client() {
    let entry = ActivityEntry {
        seq: 8,
        at: 1,
        thread_id: None,
        client: "omp".to_owned(),
        method: "threads.create".to_owned(),
        params: json!({}),
        error: None,
    };
    let value = serde_json::to_value(&entry).unwrap();
    assert_eq!(value["threadId"], Value::Null);
    assert_eq!(value["client"], json!("omp"));
    assert_eq!(
        serde_json::from_value::<ActivityEntry>(value).unwrap(),
        entry
    );
}

// spec: PRT-093, AGT-051
#[test]
fn what_an_mcp_client_created_says_so() {
    let provenance = Provenance {
        created_by: CreatedBy::Mcp,
        client: Some("claude-code".to_owned()),
    };
    let value = serde_json::to_value(&provenance).unwrap();
    assert_eq!(
        value,
        json!({ "createdBy": { "kind": "mcp" }, "client": "claude-code" })
    );
    assert_eq!(
        serde_json::from_value::<Provenance>(value).unwrap(),
        provenance
    );
}

// spec: PRT-115, PRT-116
#[test]
fn a_report_without_file_errors_has_none_and_view_errors_name_kind_file_and_message() {
    let older: ReportWidgetsParams = serde_json::from_value(
        json!({ "workspaceId": WORKSPACE_ID, "views": null, "widgets": [] }),
    )
    .expect("a report of an older window decodes");
    assert_eq!(older.file_errors, Vec::new());
    let report: ReportWidgetsParams = serde_json::from_value(json!({
        "workspaceId": WORKSPACE_ID,
        "views": null,
        "widgets": [],
        "fileErrors": [
            { "kind": "init", "file": "user", "message": "boom" },
            { "kind": "view", "file": ".omfp/views/ops.tsx", "message": "ops.tsx:2:3: x" },
        ],
    }))
    .expect("decodes");
    assert_eq!(
        report.file_errors,
        [
            FileErrorReport {
                kind: FileErrorKind::Init,
                file: "user".to_owned(),
                message: "boom".to_owned(),
            },
            FileErrorReport {
                kind: FileErrorKind::View,
                file: ".omfp/views/ops.tsx".to_owned(),
                message: "ops.tsx:2:3: x".to_owned(),
            },
        ]
    );
    assert!(
        serde_json::from_value::<FileErrorReport>(
            json!({ "kind": "widget", "file": "x", "message": "y" })
        )
        .is_err(),
        "only init and view files"
    );
    assert_eq!(
        serde_json::to_value(ViewErrors {
            windows: vec![WindowFileErrors {
                client: "web".to_owned(),
                errors: Vec::new(),
            }],
        })
        .unwrap(),
        json!({ "windows": [{ "client": "web", "errors": [] }] })
    );
}

// spec: PRT-042
#[test]
fn hello_names_where_widgets_are_served() {
    let result = HelloResult {
        protocol: PROTOCOL_VERSION,
        daemon: "0.1.0".to_owned(),
        omp: "18.18.6".to_owned(),
        warnings: Vec::new(),
        widget_base: "/widgets/abc".to_owned(),
        file_base: "/files/def".to_owned(),
        connection: 1,
    };
    assert_eq!(
        serde_json::to_value(&result).unwrap()["widgetBase"],
        json!("/widgets/abc")
    );
}

// spec: PRT-150
#[test]
fn hello_names_where_workspace_files_are_served() {
    let result = HelloResult {
        protocol: PROTOCOL_VERSION,
        daemon: "0.1.0".to_owned(),
        omp: "18.18.6".to_owned(),
        warnings: Vec::new(),
        widget_base: "/widgets/abc".to_owned(),
        file_base: "/files/def".to_owned(),
        connection: 1,
    };
    assert_eq!(
        serde_json::to_value(&result).unwrap()["fileBase"],
        json!("/files/def")
    );
}

// spec: PRT-051
#[test]
fn hello_names_the_connection() {
    let result = HelloResult {
        protocol: PROTOCOL_VERSION,
        daemon: "0.1.0".to_owned(),
        omp: "18.18.6".to_owned(),
        warnings: Vec::new(),
        widget_base: "/widgets/abc".to_owned(),
        file_base: "/files/def".to_owned(),
        connection: 42,
    };
    assert_eq!(
        serde_json::to_value(&result).unwrap()["connection"],
        json!(42)
    );
}

// spec: PRT-050
#[test]
fn live_methods_have_the_shapes_of_the_spec() {
    let connect = Request::new(
        "c1",
        method::ThreadLiveConnect::call(LiveConnectParams {
            thread_id: THREAD_ID.to_owned(),
            sdp_offer: "v=0".to_owned(),
        }),
    );
    assert_eq!(
        serde_json::to_value(&connect).unwrap()["params"],
        json!({ "threadId": THREAD_ID, "sdpOffer": "v=0" })
    );
    let connected = LiveConnected {
        sdp_answer: "v=0".to_owned(),
        voice: "sol".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(&connected).unwrap(),
        json!({ "sdpAnswer": "v=0", "voice": "sol" })
    );
    let mute = Request::new(
        "c2",
        method::ThreadLiveMute::call(LiveMuteParams {
            thread_id: THREAD_ID.to_owned(),
            muted: false,
        }),
    );
    assert_eq!(
        serde_json::to_value(&mute).unwrap()["params"],
        json!({ "threadId": THREAD_ID, "muted": false })
    );
    assert_eq!(
        serde_json::to_value(LiveMuted { muted: false }).unwrap(),
        json!({ "muted": false })
    );
    assert_eq!(method::ThreadLiveStop::NAME, "thread.live_stop");
}

// spec: PRT-053
#[test]
fn live_start_has_the_shape_of_the_spec() {
    let start = Request::new(
        "c1",
        method::ThreadLiveStart::call(ThreadRef {
            thread_id: THREAD_ID.to_owned(),
        }),
    );
    assert_eq!(
        serde_json::to_value(&start).unwrap(),
        json!({ "type": "request", "id": "c1", "method": "thread.live_start", "params": { "threadId": THREAD_ID } })
    );
    assert_eq!(
        serde_json::to_value(LiveStarted {
            voice: "sol".to_owned()
        })
        .unwrap(),
        json!({ "voice": "sol" })
    );
}

#[test]
fn a_finished_job_carries_its_exit() {
    let status = JobStatus::Exited {
        code: Some(1),
        signal: None,
        ended_at: 5,
    };
    assert_eq!(
        serde_json::to_value(&status).unwrap(),
        json!({ "state": "exited", "code": 1, "signal": null, "endedAt": 5 })
    );
    assert_eq!(
        serde_json::to_value(JobStatus::Running).unwrap(),
        json!({ "state": "running" })
    );
}

// spec: PRT-031, PRT-010
#[test]
fn every_method_has_a_golden_request_and_response() {
    let examples: Vec<(&str, Value)> = [
        system_examples(),
        repository_examples(),
        workspace_examples(),
        file_examples(),
        threads_examples(),
        conversation_examples(),
        settings_examples(),
        tree_examples(),
        subagent_examples(),
        tool_examples(),
        live_examples(),
        theme_examples(),
        ui_settings_examples(),
        widget_examples(),
        terminal_examples(),
        jobs_examples(),
        view_error_examples(),
        window_examples(),
        mount_examples(),
        activity_examples(),
        list_examples(),
        mcp_examples(),
        auth_examples(),
    ]
    .concat();

    let names: Vec<&str> = examples.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, METHODS);
    for (name, golden) in examples {
        insta::assert_json_snapshot!(name, golden);
    }
}

// spec: PRT-020
#[test]
fn subscribe_answers_with_a_replay_or_a_snapshot() {
    let replay = SubscribeResult::Replay {
        epoch: "e7".to_owned(),
        from_seq: 1201,
    };
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        json!({ "mode": "replay", "epoch": "e7", "fromSeq": 1201 })
    );
    let snapshot = SubscribeResult::Snapshot {
        epoch: "e9".to_owned(),
        seq: 40,
        state: json!({}),
    };
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap(),
        json!({ "mode": "snapshot", "epoch": "e9", "seq": 40, "state": {} })
    );
    for result in [replay, snapshot] {
        let text = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serde_json::from_str::<SubscribeResult>(&text).unwrap(),
            result
        );
    }
}

// spec: PRT-020
#[test]
fn subscribe_without_since_omits_it() {
    let call = Call::Subscribe(SubscribeParams {
        topic: Topic::Index,
        since: None,
    });
    let request = serde_json::to_value(Request::new("c7", call)).unwrap();
    assert_eq!(
        request,
        json!({ "type": "request", "id": "c7", "method": "subscribe", "params": { "topic": "index" } })
    );
}

#[test]
fn interaction_responses_carry_exactly_one_answer_key() {
    let answer: AskAnswer = wire(json!({ "id": "q1", "selectedOptions": ["Postgres"] }));
    let cases = [
        (
            InteractionResponse::Value("main".to_owned()),
            json!({ "value": "main" }),
        ),
        (
            InteractionResponse::Confirmed(false),
            json!({ "confirmed": false }),
        ),
        (InteractionResponse::Cancelled, json!({ "cancelled": true })),
        (
            InteractionResponse::Answers(vec![answer]),
            json!({ "answers": [{ "id": "q1", "selectedOptions": ["Postgres"] }] }),
        ),
    ];
    for (response, expected) in cases {
        assert_eq!(serde_json::to_value(&response).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<InteractionResponse>(expected).unwrap(),
            response
        );
    }
    assert!(serde_json::from_value::<InteractionResponse>(json!({ "cancelled": false })).is_err());
}

#[test]
fn index_snapshot_lists_workspaces_threads_the_theme_and_settings() {
    let snapshot = IndexSnapshot {
        workspaces: vec![workspace(), root_workspace()],
        threads: vec![thread()],
        theme: theme(),
        settings: settings(),
        init: None,
        models: Some(12),
        login: None,
    };
    let text = serde_json::to_string(&snapshot).unwrap();
    assert_eq!(
        serde_json::from_str::<IndexSnapshot>(&text).unwrap(),
        snapshot
    );
    insta::assert_json_snapshot!("index_snapshot", snapshot);
}

// spec: DMN-027
#[test]
fn a_discovered_repository_carries_its_kind_inline() {
    let repos = serde_json::to_value(&workspace().repos).unwrap();
    assert_eq!(repos[0]["kind"], "link");
    assert_eq!(repos[0]["target"], "/home/dev/src/shop-backend");
    assert_eq!(repos[0]["sharedWith"], json!([root_workspace().id]));
    assert_eq!(repos[1]["kind"], "worktree");
    assert_eq!(repos[1]["main"], "/home/dev/src/shop-frontend");
    let root = serde_json::to_value(&root_workspace().repos[0]).unwrap();
    assert_eq!(
        (root["kind"].clone(), root["isRoot"].clone()),
        (json!("clone"), json!(true))
    );
    let link = serde_json::to_value(&root_workspace().repos[1]).unwrap();
    assert_eq!(link["sharedWith"], json!([workspace().id]));
}

// spec: UI-097, PRT-143
#[test]
fn settings_omit_defaults_and_null_the_error() {
    assert_eq!(
        serde_json::to_value(omfp_protocol::Settings::default()).unwrap(),
        json!({ "ui": {}, "keymap": [], "error": null })
    );
    let wire = serde_json::to_value(settings()).unwrap();
    assert_eq!(wire["ui"]["density"], "normal");
    assert_eq!(wire["ui"]["fontSize"], 18);
    assert_eq!(wire["ui"]["motion"], "reduced");
    assert_eq!(wire["ui"]["notifications"], "always");
    assert_eq!(wire["ui"]["theme"], "rose-pine");
    assert_eq!(
        wire["ui"]["fonts"],
        json!({ "prose": "Literata, serif", "code": "\"JetBrains Mono\", monospace" })
    );
    assert_eq!(
        wire["keymap"][1],
        json!({ "mode": "insert", "keys": "Ctrl-j", "command": "" })
    );
}

// spec: PRT-143, UI-510
#[test]
fn built_in_themes_are_named_in_kebab_case_and_unknown_names_are_refused() {
    use omfp_protocol::BuiltinTheme;
    let names = [
        (BuiltinTheme::Synth, "synth"),
        (BuiltinTheme::Catppuccin, "catppuccin"),
        (BuiltinTheme::Gruvbox, "gruvbox"),
        (BuiltinTheme::TokyoNight, "tokyo-night"),
        (BuiltinTheme::Nord, "nord"),
        (BuiltinTheme::Solarized, "solarized"),
        (BuiltinTheme::RosePine, "rose-pine"),
        (BuiltinTheme::Accent, "accent"),
        (BuiltinTheme::Matugen, "matugen"),
    ];
    for (theme, name) in names {
        assert_eq!(serde_json::to_value(theme).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<BuiltinTheme>(json!(name)).unwrap(),
            theme
        );
    }
    assert!(serde_json::from_value::<BuiltinTheme>(json!("dracula")).is_err());
    assert!(serde_json::from_value::<BuiltinTheme>(json!("tokyo_night")).is_err());
}

// spec: DMN-024
#[test]
fn repository_sources_are_tagged_by_kind() {
    let cases = [
        (
            RepoSource::Link {
                path: "/src/app".to_owned(),
            },
            json!({ "kind": "link", "path": "/src/app" }),
        ),
        (
            RepoSource::Worktree {
                repo: "/src/app".to_owned(),
                branch: Some("fix".to_owned()),
                base_ref: None,
            },
            json!({ "kind": "worktree", "repo": "/src/app", "branch": "fix" }),
        ),
        (
            RepoSource::Clone {
                url: "https://example.com/app.git".to_owned(),
            },
            json!({ "kind": "clone", "url": "https://example.com/app.git" }),
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(serde_json::to_value(&source).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<RepoSource>(expected).unwrap(),
            source
        );
    }
}

// spec: DMN-020
#[test]
fn a_new_thread_requires_an_explicit_workspace() {
    let decode = |params: Value| serde_json::from_value::<CreateThreadParams>(params).unwrap();
    assert!(serde_json::from_value::<CreateThreadParams>(json!({})).is_err());
    assert_eq!(
        decode(json!({ "workspace": { "new": {} } })).workspace,
        ThreadWorkspace::New(CreateWorkspaceParams::default())
    );
    assert_eq!(
        decode(json!({ "workspace": { "id": WORKSPACE_ID } })).workspace,
        ThreadWorkspace::Id(WORKSPACE_ID.to_owned())
    );
    assert_eq!(
        decode(json!({ "workspace": { "new": { "link": "/src/app" } } })).workspace,
        ThreadWorkspace::New(CreateWorkspaceParams {
            name: None,
            link: Some("/src/app".to_owned()),
        })
    );
}

// spec: DMN-073
#[test]
fn a_new_thread_takes_its_title_only_from_scripts_and_agents() {
    let decode = |params: Value| serde_json::from_value::<CreateThreadParams>(params).unwrap();
    assert_eq!(
        decode(json!({ "workspace": { "new": {} }, "prompt": "Fix the login" })).title,
        None
    );
    assert_eq!(
        decode(json!({ "workspace": { "new": {} }, "title": "Fix" }))
            .title
            .as_deref(),
        Some("Fix")
    );
    let summary = serde_json::to_value(thread()).unwrap();
    assert_eq!(summary["titleSource"], json!("generated"));
}

// spec: DMN-211
#[test]
fn a_first_message_may_be_images_only_and_no_images_are_left_out() {
    let decode = |params: Value| serde_json::from_value::<CreateThreadParams>(params).unwrap();
    let png_json = json!({ "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" });
    let images_only = decode(json!({ "workspace": { "new": {} }, "images": [png_json] }));
    assert_eq!(
        (images_only.prompt, images_only.images),
        (None, vec![png()])
    );
    let text_only = decode(json!({ "workspace": { "new": {} }, "prompt": "Fix the login" }));
    assert_eq!(text_only.images, Vec::new());
    assert_eq!(
        serde_json::to_value(&text_only).unwrap().get("images"),
        None
    );
}

// spec: PRT-121, PRT-134
#[test]
fn a_held_back_restart_carries_its_reasons_and_none_is_absent() {
    let summary = serde_json::to_value(thread()).unwrap();
    assert_eq!(summary.get("restartPending"), None);
    let held = ThreadSummary {
        restart_pending: vec![
            RestartReason::McpConfig,
            RestartReason::McpReauth,
            RestartReason::Login,
        ],
        ..thread()
    };
    let summary = serde_json::to_value(&held).unwrap();
    assert_eq!(
        summary["restartPending"],
        json!(["mcp_config", "mcp_reauth", "login"])
    );
    assert_eq!(
        serde_json::from_value::<ThreadSummary>(summary).unwrap(),
        held
    );
}

/// A thread forked from [`thread`] at its first message (PRT-181).
fn forked_thread() -> ThreadSummary {
    ThreadSummary {
        id: "01JD8X3Q4T9V2W6Y5Z7A1B3C4F".to_owned(),
        title: "Checkout v2 (fork)".to_owned(),
        title_source: TitleSource::Placeholder,
        forked_from: Some(ForkOrigin {
            thread_id: THREAD_ID.to_owned(),
            title: "Checkout v2".to_owned(),
            entry_id: Some("e1".to_owned()),
            message: Some("Implement checkout v2 as described in docs/checkout.md".to_owned()),
        }),
        ..thread()
    }
}

// spec: PRT-181
#[test]
fn a_fork_names_its_source_and_message_and_other_threads_have_no_origin() {
    let summary = serde_json::to_value(thread()).unwrap();
    assert_eq!(summary.get("forkedFrom"), None);
    let forked = forked_thread();
    let summary = serde_json::to_value(&forked).unwrap();
    assert_eq!(
        summary["forkedFrom"],
        json!({
            "threadId": THREAD_ID,
            "title": "Checkout v2",
            "entryId": "e1",
            "message": "Implement checkout v2 as described in docs/checkout.md"
        })
    );
    assert_eq!(
        serde_json::from_value::<ThreadSummary>(summary).unwrap(),
        forked
    );
    let whole = ThreadSummary {
        forked_from: Some(ForkOrigin {
            thread_id: THREAD_ID.to_owned(),
            title: "Checkout v2".to_owned(),
            entry_id: None,
            message: None,
        }),
        ..thread()
    };
    assert_eq!(
        serde_json::to_value(&whole).unwrap()["forkedFrom"],
        json!({ "threadId": THREAD_ID, "title": "Checkout v2" })
    );
}

// spec: PRT-180
#[test]
fn threads_fork_takes_a_thread_and_an_optional_entry_and_answers_the_new_thread() {
    let decode = |params: Value| {
        decode_request(
            &json!({ "type": "request", "id": "c1", "method": "threads.fork", "params": params })
                .to_string(),
        )
        .unwrap()
        .call
    };
    assert_eq!(
        decode(json!({ "threadId": THREAD_ID })),
        Call::ThreadsFork(ForkParams {
            thread_id: THREAD_ID.to_owned(),
            entry_id: None,
        })
    );
    assert_eq!(
        decode(json!({ "threadId": THREAD_ID, "entryId": "e1" })),
        Call::ThreadsFork(ForkParams {
            thread_id: THREAD_ID.to_owned(),
            entry_id: Some("e1".to_owned()),
        })
    );
    assert!(METHODS.contains(&"threads.fork"));
}

// spec: DMN-075
#[test]
fn a_theme_without_css_is_null_on_the_wire() {
    assert_eq!(
        serde_json::to_value(Theme::default()).unwrap(),
        json!({ "css": null })
    );
    assert_eq!(
        serde_json::from_value::<Theme>(json!({ "css": null })).unwrap(),
        Theme::default()
    );
}

// spec: PRT-170, PRT-171
#[test]
fn a_reorder_names_what_it_goes_before_or_nothing_for_last() {
    let params =
        |call: Call| serde_json::to_value(Request::new("c1", call)).unwrap()["params"].take();
    assert_eq!(
        params(Call::WorkspacesReorder(ReorderWorkspaceParams {
            workspace_id: WORKSPACE_ID.to_owned(),
            before_id: None,
        })),
        json!({ "workspaceId": WORKSPACE_ID })
    );
    assert_eq!(
        params(Call::ThreadsReorder(ReorderThreadParams {
            thread_id: THREAD_ID.to_owned(),
            workspace_id: None,
            before_id: Some("01JD8X3Q4T9V2W6Y5Z7A1B3C9Z".to_owned()),
        })),
        json!({ "threadId": THREAD_ID, "beforeId": "01JD8X3Q4T9V2W6Y5Z7A1B3C9Z" })
    );
    for method in ["workspaces.reorder", "threads.reorder"] {
        assert!(METHODS.contains(&method), "{method} is in the method table");
    }
}

// spec: PRT-172
#[test]
fn workspaces_and_thread_summaries_carry_their_place_in_the_column() {
    let workspace = serde_json::to_value(root_workspace()).unwrap();
    assert_eq!(workspace["position"], json!(1));
    let summary = serde_json::to_value(ThreadSummary {
        position: 3,
        ..thread()
    })
    .unwrap();
    assert_eq!(summary["position"], json!(3));
    assert_eq!(
        serde_json::from_value::<ThreadSummary>(summary)
            .unwrap()
            .position,
        3
    );
}

// spec: PRT-195
#[test]
fn a_file_outside_the_workspace_says_so_and_an_older_daemon_s_file_is_inside() {
    let file = json!({
        "path": "/home/me/notes.md",
        "kind": "text",
        "size": 3,
        "hash": "a9993e364706816aba3e25717850c26c9cd0d89d",
        "content": "abc",
        "outside": true
    });
    let outside: FileContent = serde_json::from_value(file.clone()).unwrap();
    assert!(outside.outside);
    assert_eq!(serde_json::to_value(&outside).unwrap()["outside"], true);
    let mut older = file;
    older.as_object_mut().unwrap().remove("outside");
    assert!(
        !serde_json::from_value::<FileContent>(older)
            .unwrap()
            .outside
    );
}
