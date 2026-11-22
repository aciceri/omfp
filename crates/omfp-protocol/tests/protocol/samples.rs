//! Example values shared by the golden tests. Ids are fixed ULID-shaped strings so snapshots stay stable.

use omfp_core::ProcessState;
use omfp_protocol::{
    BuiltinTheme, CreatedBy, Density, FileChange, FileStatus, FontSettings, GitStatus, JobCounts,
    KeymapEntry, KeymapMode, LiveVoice, Motion, NotificationMode, Provenance, RecentThreads,
    RepoKind, RepoStatus, Settings, Skin, Theme, ThreadSummary, TitleSource, UiSettings, Workspace,
    WorkspaceRepo,
};

pub const WORKSPACE_ID: &str = "01JD8X3Q4T9V2W6Y5Z7A1B3C5E";
pub const OTHER_WORKSPACE_ID: &str = "01JD8X3Q4T9V2W6Y5Z7A1B3C7G";
pub const THREAD_ID: &str = "01JD8X3Q4T9V2W6Y5Z7A1B3C4D";
/// 2026-11-10T00:00:00Z in Unix milliseconds.
pub const CREATED_AT: i64 = 1_794_268_800_000;

pub fn user_provenance() -> Provenance {
    Provenance {
        created_by: CreatedBy::User,
        client: Some("omfp/web".to_owned()),
    }
}

pub fn workspace() -> Workspace {
    Workspace {
        id: WORKSPACE_ID.to_owned(),
        name: "checkout-v2".to_owned(),
        path: "/home/dev/omfp/workspaces/checkout-v2".to_owned(),
        repos: vec![
            WorkspaceRepo {
                name: "backend".to_owned(),
                path: "/home/dev/omfp/workspaces/checkout-v2/backend".to_owned(),
                kind: RepoKind::Link {
                    target: "/home/dev/src/shop-backend".to_owned(),
                },
                branch: Some("main".to_owned()),
                head: Some("f6ab326c0e3d4b2a1f9e8d7c6b5a4f3e2d1c0b9a".to_owned()),
                is_root: false,
                canonical_path: "/home/dev/src/shop-backend".to_owned(),
                shared_with: vec![OTHER_WORKSPACE_ID.to_owned()],
            },
            WorkspaceRepo {
                name: "shop-frontend".to_owned(),
                path: "/home/dev/omfp/workspaces/checkout-v2/shop-frontend".to_owned(),
                kind: RepoKind::Worktree {
                    main: "/home/dev/src/shop-frontend".to_owned(),
                },
                branch: Some("omfp/checkout-v2-1".to_owned()),
                head: Some("0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d".to_owned()),
                is_root: false,
                canonical_path: "/home/dev/omfp/workspaces/checkout-v2/shop-frontend".to_owned(),
                shared_with: Vec::new(),
            },
        ],
        thread_ids: vec![THREAD_ID.to_owned()],
        provenance: user_provenance(),
        created_at: CREATED_AT,
        position: 0,
        jobs: Some(JobCounts {
            total: 3,
            manual: 1,
            schedule: 1,
            service: 1,
            running: 1,
            failing: 1,
            error: false,
        }),
    }
}

/// A workspace an agent made a repository itself (`git init`), holding a link to a repository of [`workspace`].
pub fn root_workspace() -> Workspace {
    Workspace {
        id: OTHER_WORKSPACE_ID.to_owned(),
        name: "scratch".to_owned(),
        path: "/home/dev/omfp/workspaces/scratch".to_owned(),
        repos: vec![
            WorkspaceRepo {
                name: "scratch".to_owned(),
                path: "/home/dev/omfp/workspaces/scratch".to_owned(),
                kind: RepoKind::Clone,
                branch: Some("main".to_owned()),
                head: Some("f6ab326c0e3d4b2a1f9e8d7c6b5a4f3e2d1c0b9a".to_owned()),
                is_root: true,
                canonical_path: "/home/dev/omfp/workspaces/scratch".to_owned(),
                shared_with: Vec::new(),
            },
            WorkspaceRepo {
                name: "shop-backend".to_owned(),
                path: "/home/dev/omfp/workspaces/scratch/shop-backend".to_owned(),
                kind: RepoKind::Link {
                    target: "/home/dev/src/shop-backend".to_owned(),
                },
                branch: Some("main".to_owned()),
                head: Some("f6ab326c0e3d4b2a1f9e8d7c6b5a4f3e2d1c0b9a".to_owned()),
                is_root: false,
                canonical_path: "/home/dev/src/shop-backend".to_owned(),
                shared_with: vec![WORKSPACE_ID.to_owned()],
            },
        ],
        thread_ids: Vec::new(),
        provenance: Provenance {
            created_by: CreatedBy::Thread {
                thread_id: THREAD_ID.to_owned(),
            },
            client: Some("omfp-agent".to_owned()),
        },
        created_at: CREATED_AT,
        position: 1,
        jobs: None,
    }
}

pub fn thread() -> ThreadSummary {
    ThreadSummary {
        id: THREAD_ID.to_owned(),
        title: "Checkout v2".to_owned(),
        title_source: TitleSource::Generated,
        workspace_id: WORKSPACE_ID.to_owned(),
        process: ProcessState::Stopped,
        settled: true,
        needs_attention: false,
        restart_pending: Vec::new(),
        provenance: user_provenance(),
        created_at: CREATED_AT,
        updated_at: CREATED_AT + 60_000,
        archived_at: None,
        position: 0,
        forked_from: None,
    }
}

/// A user theme (DMN-075), as `theme.set` takes it and the `index` topic carries it.
pub fn theme() -> Theme {
    Theme {
        css: Some(":root { --omfp-primary: #6750a4; }".to_owned()),
    }
}

/// UI settings of a config file (DMN-009, UI-097): every `[ui]` value and one binding per mode.
pub fn settings() -> Settings {
    Settings {
        ui: UiSettings {
            density: Some(Density::Normal),
            font_size: Some(18),
            motion: Some(Motion::Reduced),
            notifications: Some(NotificationMode::Always),
            fonts: Some(FontSettings {
                prose: Some("Literata, serif".to_owned()),
                ui: None,
                code: Some("\"JetBrains Mono\", monospace".to_owned()),
            }),
            theme: Some(BuiltinTheme::RosePine),
            show_xdev_notices: Some(true),
            skin: Some(Skin::Boring),
            recent_threads: Some(RecentThreads::Count(30)),
            live_voice: Some(LiveVoice::Maple),
        },
        keymap: vec![
            KeymapEntry {
                mode: KeymapMode::Normal,
                keys: "Space t".to_owned(),
                command: "theme.set".to_owned(),
            },
            KeymapEntry {
                mode: KeymapMode::Insert,
                keys: "Ctrl-j".to_owned(),
                command: String::new(),
            },
        ],
        error: None,
    }
}

pub fn repo_statuses() -> Vec<RepoStatus> {
    vec![
        RepoStatus {
            name: "backend".to_owned(),
            status: GitStatus {
                branch: Some("main".to_owned()),
                base_ref: None,
                ahead: 0,
                behind: 0,
                dirty: false,
                files: Vec::new(),
            },
        },
        RepoStatus {
            name: "shop-frontend".to_owned(),
            status: git_status(),
        },
    ]
}

pub fn git_status() -> GitStatus {
    GitStatus {
        branch: Some("omfp/checkout-v2-1".to_owned()),
        base_ref: Some("origin/main".to_owned()),
        ahead: 2,
        behind: 1,
        dirty: true,
        files: vec![
            FileChange {
                path: "src/checkout.rs".to_owned(),
                old_path: None,
                status: FileStatus::Modified,
                additions: 40,
                deletions: 3,
            },
            FileChange {
                path: "src/cart/total.rs".to_owned(),
                old_path: Some("src/total.rs".to_owned()),
                status: FileStatus::Renamed,
                additions: 1,
                deletions: 1,
            },
            FileChange {
                path: "notes.txt".to_owned(),
                old_path: None,
                status: FileStatus::Untracked,
                additions: 5,
                deletions: 0,
            },
        ],
    }
}
