//! Daemon → client frames other than successful responses: errors, topic events and `resync_required`.

use std::path::Path;

use omfp_core::{ProcessState, ThreadEvent};
use omfp_protocol::{
    CreatedBy, ErrorBody, ErrorCode, EventFrame, IndexEvent, JobDefinition, JobEvent, JobRunStatus,
    JobStream, JobTrigger, JobView, JobViewStatus, JobsEvent, KeymapMode, MAX_FRAME_BYTES,
    MissedPolicy, PROTOCOL_VERSION, ReportWidgetsParams, Response, RestartPolicy, RunInfo,
    RunTrigger, ServerFrame, Topic, TopicEvent, TransientEvent, ViewsEvent, ViewsState, WidgetInfo,
    WidgetSource, WindowAnswer, WindowRequest, WorkspaceBinding, WorkspaceEvent,
};
use serde_json::{Value, json};

use crate::samples::{THREAD_ID, WORKSPACE_ID, repo_statuses, settings, theme, thread};

fn round_trip(frame: &ServerFrame) -> Value {
    let text = serde_json::to_string(frame).expect("frame serializes");
    assert_eq!(
        &serde_json::from_str::<ServerFrame>(&text).expect("frame decodes"),
        frame
    );
    serde_json::from_str(&text).expect("valid JSON")
}

// spec: PRT-011
#[test]
fn error_codes_are_the_closed_set_of_the_spec() {
    let wire: Vec<Value> = ErrorCode::ALL
        .iter()
        .map(|code| serde_json::to_value(code).unwrap())
        .collect();
    assert_eq!(
        wire,
        [
            "unauthorized",
            "bad_request",
            "not_found",
            "conflict",
            "already_resolved",
            "limit_reached",
            "version_mismatch",
            "thread_failed",
            "omp_error",
            "git_error",
            "internal",
        ]
        .map(Value::from)
    );
    for code in ErrorCode::ALL {
        let text = serde_json::to_string(&code).unwrap();
        assert_eq!(serde_json::from_str::<ErrorCode>(&text).unwrap(), code);
    }
    assert!(serde_json::from_value::<ErrorCode>(json!("teapot")).is_err());
}

// spec: PRT-011, PRT-010
#[test]
fn omp_errors_carry_omps_code() {
    let frame = ServerFrame::Response(Response::err(
        "c42",
        ErrorBody::new(ErrorCode::OmpError, "agent is busy")
            .with_omp_code("session_busy")
            .with_details(json!({ "command": "navigate_tree" })),
    ));
    let golden = round_trip(&frame);
    insta::assert_json_snapshot!("error_response_omp_error", golden);
}

// spec: PRT-004
#[test]
fn version_mismatch_lists_the_supported_versions() {
    let frame = ServerFrame::Response(Response::err(
        "1",
        ErrorBody::version_mismatch(&[PROTOCOL_VERSION]),
    ));
    let golden = round_trip(&frame);
    assert_eq!(
        golden["error"]["details"]["supported"],
        json!([PROTOCOL_VERSION])
    );
    insta::assert_json_snapshot!("error_response_version_mismatch", golden);
}

// spec: PRT-010
#[test]
fn a_response_is_either_a_result_or_an_error() {
    let ok: ServerFrame =
        serde_json::from_value(json!({ "type": "response", "id": "c1", "ok": true, "result": {} }))
            .unwrap();
    assert_eq!(
        ok,
        ServerFrame::Response(Response::ok("c1", &json!({})).unwrap())
    );
    for invalid in [
        json!({ "type": "response", "id": "c1", "ok": false, "result": {} }),
        json!({ "type": "response", "id": "c1", "ok": false, "result": {}, "error": { "code": "internal", "message": "x" } }),
        json!({ "type": "response", "id": "c1", "ok": true, "error": { "code": "internal", "message": "x" } }),
    ] {
        assert!(
            serde_json::from_value::<ServerFrame>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
}

// spec: PRT-022
#[test]
fn thread_event_frame() {
    let frame = ServerFrame::Event(EventFrame {
        topic: Topic::Thread(THREAD_ID.to_owned()),
        epoch: "e7".to_owned(),
        seq: 1289,
        event: TopicEvent::Thread(ThreadEvent::Process {
            state: ProcessState::Running,
        }),
    });
    let golden = round_trip(&frame);
    insta::assert_json_snapshot!("event_thread", golden);
}

#[test]
fn index_event_frame() {
    let frame = ServerFrame::Event(EventFrame {
        topic: Topic::Index,
        epoch: "e1".to_owned(),
        seq: 3,
        event: TopicEvent::Index(IndexEvent::ThreadUpserted { thread: thread() }),
    });
    let golden = round_trip(&frame);
    insta::assert_json_snapshot!("event_index", golden);

    let removed = ServerFrame::Event(EventFrame {
        topic: Topic::Index,
        epoch: "e1".to_owned(),
        seq: 4,
        event: TopicEvent::Index(IndexEvent::WorkspaceRemoved {
            workspace_id: WORKSPACE_ID.to_owned(),
        }),
    });
    assert_eq!(
        round_trip(&removed)["event"],
        json!({ "kind": "workspace_removed", "workspaceId": WORKSPACE_ID })
    );
}

// spec: UI-097
#[test]
fn the_index_topic_carries_settings_changes() {
    let changed = ServerFrame::Event(EventFrame {
        topic: Topic::Index,
        epoch: "e1".to_owned(),
        seq: 6,
        event: TopicEvent::Index(IndexEvent::SettingsChanged {
            settings: settings(),
        }),
    });
    let event = &round_trip(&changed)["event"];
    assert_eq!(event["kind"], "settings_changed");
    assert_eq!(event["settings"]["keymap"][0]["keys"], "Space t");
}

// spec: DMN-075
#[test]
fn the_index_topic_carries_theme_changes() {
    let changed = ServerFrame::Event(EventFrame {
        topic: Topic::Index,
        epoch: "e1".to_owned(),
        seq: 5,
        event: TopicEvent::Index(IndexEvent::ThemeChanged { theme: theme() }),
    });
    assert_eq!(
        round_trip(&changed)["event"],
        json!({ "kind": "theme_changed", "theme": { "css": ":root { --omfp-primary: #6750a4; }" } })
    );
}

#[test]
fn workspace_event_frame() {
    let frame = ServerFrame::Event(EventFrame {
        topic: Topic::Workspace(WORKSPACE_ID.to_owned()),
        epoch: "e1".to_owned(),
        seq: 1,
        event: TopicEvent::Workspace(WorkspaceEvent::Status {
            repos: repo_statuses(),
        }),
    });
    let golden = round_trip(&frame);
    insta::assert_json_snapshot!("event_workspace", golden);
}

fn event(topic: Topic, event: TopicEvent) -> ServerFrame {
    ServerFrame::Event(EventFrame {
        topic,
        epoch: "e1".to_owned(),
        seq: 1,
        event,
    })
}

// spec: PRT-040
#[test]
fn views_event_frames() {
    let topic = Topic::Views(WORKSPACE_ID.to_owned());
    let manifest = json!({ "views": { "work": { "layout": "omfp:chat" } } });
    let changed = event(
        topic.clone(),
        TopicEvent::Views(ViewsEvent::ViewsManifest {
            manifest: Some(manifest.clone()),
            error: None,
        }),
    );
    assert_eq!(
        round_trip(&changed)["event"],
        json!({ "kind": "views_manifest", "manifest": manifest, "error": null })
    );
    let invalid = event(
        topic.clone(),
        TopicEvent::Views(ViewsEvent::ViewsManifest {
            manifest: None,
            error: Some("views.work.layout: expected a widget name".to_owned()),
        }),
    );
    assert_eq!(
        round_trip(&invalid)["event"],
        json!({ "kind": "views_manifest", "manifest": null, "error": "views.work.layout: expected a widget name" })
    );
    let widgets = event(
        topic.clone(),
        TopicEvent::Views(ViewsEvent::ViewsWidgets {
            widgets: vec![WidgetInfo {
                source: WidgetSource::Workspace,
                name: "tests".to_owned(),
                version: "1f2e3d4c5b6a".to_owned(),
                entry: Some("index.tsx".to_owned()),
            }],
        }),
    );
    insta::assert_json_snapshot!("event_views_widgets", round_trip(&widgets));
    let stored = event(
        topic,
        TopicEvent::Views(ViewsEvent::WidgetStored {
            instance: "./widgets/tests".to_owned(),
            key: "runs".to_owned(),
            value: json!(3),
        }),
    );
    assert_eq!(
        round_trip(&stored)["event"],
        json!({ "kind": "widget_stored", "instance": "./widgets/tests", "key": "runs", "value": 3 })
    );
}

// spec: PRT-060
#[test]
fn views_keymap_event_frames_and_the_state_without_a_keymap() {
    let topic = Topic::Views(WORKSPACE_ID.to_owned());
    let keymap = event(
        topic,
        TopicEvent::Views(ViewsEvent::ViewsKeymap {
            keymap: vec![
                WorkspaceBinding {
                    mode: KeymapMode::Normal,
                    keys: "Space t".to_owned(),
                    command: "tests.run".to_owned(),
                    args: vec!["unit".to_owned()],
                },
                WorkspaceBinding {
                    mode: KeymapMode::Insert,
                    keys: "Ctrl-t".to_owned(),
                    command: String::new(),
                    args: Vec::new(),
                },
            ],
            error: Some("bindings[2].keys: expected a key sequence".to_owned()),
        }),
    );
    insta::assert_json_snapshot!("event_views_keymap", round_trip(&keymap));

    // A state without the keymap fields (an older daemon) reads as no workspace bindings.
    let state: ViewsState = serde_json::from_value(
        json!({ "manifest": null, "error": null, "widgets": [], "stored": {} }),
    )
    .unwrap();
    assert_eq!(state.keymap, Vec::new());
    assert_eq!(state.keymap_error, None);
}

// spec: PRT-095
#[test]
fn init_events_replace_the_init_versions_and_older_states_have_none() {
    let user = event(
        Topic::Index,
        TopicEvent::Index(IndexEvent::InitChanged {
            init: Some("0123456789abcdef".to_owned()),
        }),
    );
    assert_eq!(
        round_trip(&user)["event"],
        json!({ "kind": "init_changed", "init": "0123456789abcdef" })
    );
    let workspace = event(
        Topic::Views(WORKSPACE_ID.to_owned()),
        TopicEvent::Views(ViewsEvent::ViewsInit { init: None }),
    );
    assert_eq!(
        round_trip(&workspace)["event"],
        json!({ "kind": "views_init", "init": null })
    );

    // States without the field (an older daemon) read as no init file.
    let state: ViewsState = serde_json::from_value(
        json!({ "manifest": null, "error": null, "widgets": [], "stored": {} }),
    )
    .unwrap();
    assert_eq!(state.init, None);
    let index: omfp_protocol::IndexSnapshot = serde_json::from_value(json!({
        "workspaces": [],
        "threads": [],
        "theme": theme(),
        "settings": settings()
    }))
    .unwrap();
    assert_eq!(index.init, None);
}

// spec: PRT-141
#[test]
fn models_events_replace_the_usable_model_count_and_older_states_have_none() {
    let changed = event(
        Topic::Index,
        TopicEvent::Index(IndexEvent::ModelsChanged { models: 0 }),
    );
    assert_eq!(
        round_trip(&changed)["event"],
        json!({ "kind": "models_changed", "models": 0 })
    );
    // A snapshot without the field (an older daemon, or none checked yet) reads as unknown.
    let index: omfp_protocol::IndexSnapshot = serde_json::from_value(json!({
        "workspaces": [],
        "threads": [],
        "theme": theme(),
        "settings": settings()
    }))
    .unwrap();
    assert_eq!(index.models, None);
    let checked: omfp_protocol::IndexSnapshot = serde_json::from_value(json!({
        "workspaces": [],
        "threads": [],
        "theme": theme(),
        "settings": settings(),
        "models": 3
    }))
    .unwrap();
    assert_eq!(checked.models, Some(3));
}

// spec: PRT-096
#[test]
fn views_files_events_replace_the_view_files_version_and_older_states_have_none() {
    let changed = event(
        Topic::Views(WORKSPACE_ID.to_owned()),
        TopicEvent::Views(ViewsEvent::ViewsFiles {
            view_files: Some("0123456789abcdef".to_owned()),
        }),
    );
    assert_eq!(
        round_trip(&changed)["event"],
        json!({ "kind": "views_files", "viewFiles": "0123456789abcdef" })
    );
    let gone = event(
        Topic::Views(WORKSPACE_ID.to_owned()),
        TopicEvent::Views(ViewsEvent::ViewsFiles { view_files: None }),
    );
    assert_eq!(
        round_trip(&gone)["event"],
        json!({ "kind": "views_files", "viewFiles": null })
    );

    // States without the field (an older daemon) read as no `.omfp/views/` folder.
    let mut state: ViewsState = serde_json::from_value(
        json!({ "manifest": null, "error": null, "widgets": [], "stored": {} }),
    )
    .unwrap();
    assert_eq!(state.view_files, None);
    state.view_files = Some("0123456789abcdef".to_owned());
    assert_eq!(
        serde_json::to_value(&state).unwrap()["viewFiles"],
        json!("0123456789abcdef")
    );
}

// spec: PRT-041
#[test]
fn job_event_frames() {
    let topic = Topic::Job("01JOB".to_owned());
    let line = event(
        topic.clone(),
        TopicEvent::Job(JobEvent::JobOutput {
            stream: JobStream::Stderr,
            text: "PASS a".to_owned(),
        }),
    );
    assert_eq!(
        round_trip(&line)["event"],
        json!({ "kind": "job_output", "stream": "stderr", "text": "PASS a" })
    );
    let exited = event(
        topic,
        TopicEvent::Job(JobEvent::JobExited {
            code: None,
            signal: Some(15),
            ended_at: 1_700_000_000_000,
        }),
    );
    assert_eq!(
        round_trip(&exited)["event"],
        json!({ "kind": "job_exited", "code": null, "signal": 15, "endedAt": 1_700_000_000_000_i64 })
    );
}

// spec: PRT-200, PRT-201, PRT-209
#[test]
fn jobs_event_frames() {
    let topic = Topic::Jobs(WORKSPACE_ID.to_owned());
    let run = RunInfo {
        id: "01RUN".to_owned(),
        workspace_id: WORKSPACE_ID.to_owned(),
        job: Some("nightly".to_owned()),
        trigger: RunTrigger::Missed,
        started_by: Some(CreatedBy::Job {
            job: "nightly".to_owned(),
        }),
        thread_id: None,
        command: Some("cargo test".to_owned()),
        started_at: 1_700_000_000_000,
        ended_at: None,
        status: JobRunStatus::Running,
        reason: None,
        exit_code: None,
        signal: None,
    };
    let run_json = json!({
        "id": "01RUN", "workspaceId": WORKSPACE_ID, "job": "nightly", "trigger": "missed",
        "startedBy": { "kind": "job", "job": "nightly" }, "threadId": null, "command": "cargo test",
        "startedAt": 1_700_000_000_000_i64, "endedAt": null, "status": "running",
        "reason": null, "exitCode": null, "signal": null
    });
    let view = JobView {
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
        status: JobViewStatus::Stopped,
        run: None,
        last_run: None,
        next_at: None,
    };
    let view_json = json!({
        "name": "web",
        "job": { "trigger": "service", "missed": "skip", "command": "npm run dev",
                 "restart": "on-failure", "enabled": true },
        "status": "stopped", "run": null, "lastRun": null, "nextAt": null
    });
    let cases = [
        (
            JobsEvent::JobsSet {
                jobs: vec![view.clone()],
                error: Some("jobs.web.trigger: unknown".to_owned()),
            },
            json!({ "kind": "jobs_set", "jobs": [view_json], "error": "jobs.web.trigger: unknown" }),
        ),
        (
            JobsEvent::JobUpdated { job: view },
            json!({ "kind": "job_updated", "job": view_json }),
        ),
        (
            JobsEvent::RunStarted { run: run.clone() },
            json!({ "kind": "run_started", "run": run_json }),
        ),
        (
            JobsEvent::RunEnded { run },
            json!({ "kind": "run_ended", "run": run_json }),
        ),
    ];
    for (jobs_event, wire) in cases {
        let frame = event(topic.clone(), TopicEvent::Jobs(jobs_event));
        let value = round_trip(&frame);
        assert_eq!(value["topic"], json!(format!("jobs/{WORKSPACE_ID}")));
        assert_eq!(value["event"], wire);
    }
}

#[test]
fn event_must_match_its_topic() {
    let index_event_on_thread_topic = json!({
        "type": "event",
        "topic": format!("thread/{THREAD_ID}"),
        "epoch": "e1",
        "seq": 1,
        "event": { "kind": "thread_removed", "threadId": THREAD_ID }
    });
    assert!(serde_json::from_value::<ServerFrame>(index_event_on_thread_topic).is_err());
}

// spec: PRT-023
#[test]
fn resync_required_frame() {
    let frame = ServerFrame::ResyncRequired {
        topic: Topic::Thread(THREAD_ID.to_owned()),
    };
    let golden = round_trip(&frame);
    assert_eq!(
        golden,
        json!({ "type": "resync_required", "topic": format!("thread/{THREAD_ID}") })
    );
    insta::assert_json_snapshot!("resync_required", golden);
}

// spec: PRT-054, DMN-110
#[test]
fn transient_frame_carries_live_levels_without_a_position() {
    let frame = ServerFrame::Transient {
        topic: Topic::Thread(THREAD_ID.to_owned()),
        event: TransientEvent::LiveLevels {
            input: 0.25,
            output: 0.5,
        },
    };
    let golden = round_trip(&frame);
    assert_eq!(
        golden,
        json!({
            "type": "transient",
            "topic": format!("thread/{THREAD_ID}"),
            "event": { "kind": "live_levels", "input": 0.25, "output": 0.5 }
        })
    );
    insta::assert_json_snapshot!("transient_live_levels", golden);
}

// spec: PRT-124
#[test]
fn a_window_request_frame_carries_the_request_id_and_its_kind() {
    let run = ServerFrame::WindowRequest {
        id: "w1".to_owned(),
        request: WindowRequest::RunCommand {
            command: "view.switch".to_owned(),
            args: vec!["ops".to_owned()],
        },
    };
    assert_eq!(
        round_trip(&run),
        json!({
            "type": "window_request",
            "id": "w1",
            "request": { "kind": "run_command", "command": "view.switch", "args": ["ops"] }
        })
    );
    let check = ServerFrame::WindowRequest {
        id: "w2".to_owned(),
        request: WindowRequest::CheckWidget {
            reference: "./widgets/tests".to_owned(),
            version: Some("3f2a".to_owned()),
            timeout_ms: 3000,
        },
    };
    let golden = round_trip(&check);
    assert_eq!(
        golden,
        json!({
            "type": "window_request",
            "id": "w2",
            "request": {
                "kind": "check_widget",
                "reference": "./widgets/tests",
                "version": "3f2a",
                "timeoutMs": 3000
            }
        })
    );
    insta::assert_json_snapshot!("window_request_check_widget", golden);
}

// spec: PRT-131
#[test]
fn a_login_flow_reaches_the_window_as_url_progress_and_input_requests() {
    let url = ServerFrame::WindowRequest {
        id: "w3".to_owned(),
        request: WindowRequest::LoginUrl {
            provider_id: "anthropic".to_owned(),
            name: "Anthropic (Claude Pro/Max)".to_owned(),
            url: "https://claude.ai/oauth/authorize?code=true".to_owned(),
            launch_url: Some("http://localhost:54545/launch".to_owned()),
            instructions: None,
        },
    };
    let golden = round_trip(&url);
    assert_eq!(
        golden,
        json!({
            "type": "window_request",
            "id": "w3",
            "request": {
                "kind": "login_url",
                "providerId": "anthropic",
                "name": "Anthropic (Claude Pro/Max)",
                "url": "https://claude.ai/oauth/authorize?code=true",
                "launchUrl": "http://localhost:54545/launch",
                "instructions": null
            }
        })
    );
    insta::assert_json_snapshot!("window_request_login_url", golden);
    let progress = ServerFrame::WindowRequest {
        id: "w4".to_owned(),
        request: WindowRequest::LoginProgress {
            provider_id: "anthropic".to_owned(),
            name: "Anthropic (Claude Pro/Max)".to_owned(),
            message: "Waiting for the browser…".to_owned(),
        },
    };
    assert_eq!(
        round_trip(&progress)["request"],
        json!({
            "kind": "login_progress",
            "providerId": "anthropic",
            "name": "Anthropic (Claude Pro/Max)",
            "message": "Waiting for the browser…"
        })
    );
    let input = ServerFrame::WindowRequest {
        id: "w5".to_owned(),
        request: WindowRequest::LoginInput {
            provider_id: "kimi-code".to_owned(),
            name: "Kimi Code".to_owned(),
            message: "Paste the authorization code".to_owned(),
            placeholder: None,
        },
    };
    assert_eq!(
        round_trip(&input)["request"],
        json!({
            "kind": "login_input",
            "providerId": "kimi-code",
            "name": "Kimi Code",
            "message": "Paste the authorization code",
            "placeholder": null
        })
    );
    for (answer, wire) in [
        (
            WindowAnswer::LoginInput {
                value: Some("code#state".to_owned()),
            },
            json!({ "kind": "login_input", "value": "code#state" }),
        ),
        (
            WindowAnswer::LoginInput { value: None },
            json!({ "kind": "login_input", "value": null }),
        ),
    ] {
        assert_eq!(serde_json::to_value(&answer).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<WindowAnswer>(wire).unwrap(),
            answer
        );
    }
}

// spec: PRT-122
#[test]
fn a_report_without_window_state_still_decodes() {
    let report: ReportWidgetsParams =
        serde_json::from_value(json!({ "workspaceId": null, "views": null, "widgets": [] }))
            .unwrap();
    assert_eq!(report.window, None);
    let report: ReportWidgetsParams = serde_json::from_value(json!({
        "workspaceId": WORKSPACE_ID,
        "views": null,
        "widgets": [],
        "window": { "focused": true }
    }))
    .unwrap();
    let window = report.window.unwrap();
    assert!(window.focused);
    assert_eq!(window.views, Vec::new());
    assert_eq!(window.active_view, None);
}

// spec: TST-012
#[test]
fn every_recorded_thread_event_survives_an_event_frame() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/threads")
        .join(omfp_core::OMP_VERSION);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixture directory")
        .map(|entry| entry.expect("fixture entry").path())
        .collect();
    files.sort();
    assert_ne!(files, Vec::<std::path::PathBuf>::new());

    for file in files {
        let text = std::fs::read_to_string(&file).expect("fixture file");
        for (index, line) in text.lines().enumerate() {
            let event: ThreadEvent =
                serde_json::from_str(line).expect("fixture line is a ThreadEvent");
            let frame = ServerFrame::Event(EventFrame {
                topic: Topic::Thread(THREAD_ID.to_owned()),
                epoch: "e1".to_owned(),
                seq: u64::try_from(index).unwrap() + 1,
                event: TopicEvent::Thread(event),
            });
            // Compared as decoded values: omp-rpc re-encodes some integers as floats (`0` → `0.0`).
            round_trip(&frame);
        }
    }
}

// spec: PRT-013
#[test]
fn frames_are_capped_at_16_mib() {
    assert_eq!(MAX_FRAME_BYTES, 16 * 1024 * 1024);
}
