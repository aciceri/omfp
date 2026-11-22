//! The `views/<workspaceId>` topics (DMN-082, PRT-040): each workspace's views manifest, keymap (DMN-121), init file
//! version (DMN-185), `.omfp/views/` version (DMN-186), widget folders and widget state, refreshed on every rescan and
//! poll round, plus the methods that change them (DMN-083, DMN-086, DMN-087).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use omfp_protocol::{
    CopyWidgetParams, ErrorBody, ErrorCode, SetViewsParams, SetWidgetStateParams, ViewsEvent,
    ViewsFile, ViewsState, WidgetCopied,
};
use tracing::warn;

use super::{LIVE_BUFFER, Services, blocking, lock};
use crate::event_log::EventLog;
use crate::paths::replace_file;
use crate::server::convert::{bad_request, internal};
use crate::store::{Store, new_id};
use crate::widgets::{
    CopyError, Widgets, init, keymap, manifest, manifest_path, view_files_version,
};

/// A `views/<workspaceId>` topic (PRT-040).
pub(crate) type ViewsLog = EventLog<ViewsEvent, ViewsState>;

const VIEWS_CAPACITY: usize = 64;

/// Largest JSON value a widget may store (DMN-086).
pub(crate) const MAX_STORED_BYTES: usize = 256 * 1024;

#[allow(clippy::trivially_copy_pass_by_ref)] // `EventLog` takes `fn(&mut S, &E)`.
fn apply_views(state: &mut ViewsState, event: &ViewsEvent) {
    match event {
        ViewsEvent::ViewsManifest { manifest, error } => {
            state.manifest.clone_from(manifest);
            state.error.clone_from(error);
        }
        ViewsEvent::ViewsKeymap { keymap, error } => {
            state.keymap.clone_from(keymap);
            state.keymap_error.clone_from(error);
        }
        ViewsEvent::ViewsInit { init } => state.init.clone_from(init),
        ViewsEvent::ViewsFiles { view_files } => state.view_files.clone_from(view_files),
        ViewsEvent::ViewsWidgets { widgets } => state.widgets.clone_from(widgets),
        ViewsEvent::WidgetStored {
            instance,
            key,
            value,
        } => {
            if value.is_null() {
                if let Some(keys) = state.stored.get_mut(instance) {
                    keys.remove(key);
                    if keys.is_empty() {
                        state.stored.remove(instance);
                    }
                }
            } else {
                state
                    .stored
                    .entry(instance.clone())
                    .or_default()
                    .insert(key.clone(), value.clone());
            }
        }
    }
}

/// The workspace's folder; `not_found` for an unknown workspace.
fn workspace_path(store: &Store, id: &str) -> Result<PathBuf, ErrorBody> {
    Ok(store.workspace(id)?.path)
}

/// The state of `views/<id>` as the workspace's files are now, without the widget state: its manifest, keymap (none
/// from an invalid file, DMN-121), init file version, `.omfp/views/` version and widget folders.
fn read_views(store: &Store, widgets: &Widgets, id: &str) -> Result<ViewsState, ErrorBody> {
    let path = workspace_path(store, id)?;
    let (manifest, error) = Widgets::manifest(&path);
    let (keymap, keymap_error) = match keymap::read(&keymap::path(&path)) {
        Ok(bindings) => (bindings.unwrap_or_default(), None),
        Err(error) => (Vec::new(), Some(error)),
    };
    Ok(ViewsState {
        manifest,
        error,
        widgets: widgets.widgets(&path),
        stored: BTreeMap::new(),
        keymap,
        keymap_error,
        init: init::read_version(&init::workspace_init(&path)),
        view_files: view_files_version(&path),
    })
}

/// The events turning `state` into `fresh`, the files read again; an invalid keymap file keeps the last bindings
/// (DMN-121).
fn changes(state: &ViewsState, mut fresh: ViewsState) -> Vec<ViewsEvent> {
    if fresh.keymap_error.is_some() {
        fresh.keymap.clone_from(&state.keymap);
    }
    let mut events = Vec::new();
    if state.manifest != fresh.manifest || state.error != fresh.error {
        events.push(ViewsEvent::ViewsManifest {
            manifest: fresh.manifest,
            error: fresh.error,
        });
    }
    if state.keymap != fresh.keymap || state.keymap_error != fresh.keymap_error {
        events.push(ViewsEvent::ViewsKeymap {
            keymap: fresh.keymap,
            error: fresh.keymap_error,
        });
    }
    if state.init != fresh.init {
        events.push(ViewsEvent::ViewsInit { init: fresh.init });
    }
    if state.view_files != fresh.view_files {
        events.push(ViewsEvent::ViewsFiles {
            view_files: fresh.view_files,
        });
    }
    if state.widgets != fresh.widgets {
        events.push(ViewsEvent::ViewsWidgets {
            widgets: fresh.widgets,
        });
    }
    events
}

pub(super) fn copy_error(error: CopyError) -> ErrorBody {
    let (code, message) = match error {
        CopyError::BadRequest(message) => (ErrorCode::BadRequest, message),
        CopyError::NotFound(message) => (ErrorCode::NotFound, message),
        CopyError::Conflict(message) => (ErrorCode::Conflict, message),
        CopyError::Io(message) => (ErrorCode::Internal, message),
    };
    ErrorBody::new(code, message)
}

impl Services {
    /// The log of `views/<id>` with its state read now (DMN-082); `not_found` for an unknown workspace.
    pub async fn views_log(&self, id: &str) -> Result<Arc<ViewsLog>, ErrorBody> {
        let _serial = self.views_refresh.lock().await;
        if let Some(log) = self.views_log_entry(id) {
            self.refresh_views_locked(id, &log).await?;
            return Ok(log);
        }
        let (store, widgets, workspace_id) = (
            Arc::clone(&self.store),
            Arc::clone(&self.widgets),
            id.to_owned(),
        );
        let state = blocking(move || {
            let mut state = read_views(&store, &widgets, &workspace_id)?;
            state.stored = store.widget_state(&workspace_id)?;
            Ok(state)
        })
        .await?;
        let log = Arc::new(EventLog::new(
            new_id(),
            state,
            apply_views,
            VIEWS_CAPACITY,
            LIVE_BUFFER,
        ));
        lock(&self.views_logs).insert(id.to_owned(), Arc::clone(&log));
        Ok(log)
    }

    fn views_log_entry(&self, id: &str) -> Option<Arc<ViewsLog>> {
        lock(&self.views_logs).get(id).cloned()
    }

    /// The position of `views/<id>` when a window follows it (DMN-063).
    pub(crate) fn views_log_position(&self, id: &str) -> Option<crate::event_log::Position> {
        self.views_log_entry(id).map(|log| log.position())
    }

    /// Reads the workspace's manifest, keymap, init file, `.omfp/views/` and widget folders again and publishes what
    /// changed on `views/<id>`, if the topic exists (DMN-082, DMN-121, DMN-185, DMN-186). A removed workspace's topic
    /// goes away.
    pub async fn refresh_views(&self, id: &str) {
        let _serial = self.views_refresh.lock().await;
        let Some(log) = self.views_log_entry(id) else {
            return;
        };
        match self.refresh_views_locked(id, &log).await {
            Ok(()) => {}
            Err(error) if error.code == ErrorCode::NotFound => {
                lock(&self.views_logs).remove(id);
            }
            Err(error) => warn!(workspace = %id, error = %error.message, "cannot read the views"),
        }
    }

    /// The refresh itself; the caller holds `views_refresh`, so events of one topic never interleave.
    async fn refresh_views_locked(&self, id: &str, log: &ViewsLog) -> Result<(), ErrorBody> {
        let (store, widgets, workspace_id) = (
            Arc::clone(&self.store),
            Arc::clone(&self.widgets),
            id.to_owned(),
        );
        let fresh = blocking(move || read_views(&store, &widgets, &workspace_id)).await?;
        for event in log.with_state(|state| changes(state, fresh)) {
            log.push(event);
        }
        Ok(())
    }

    /// `views.set` (DMN-083): checks the manifest, writes `.omfp/views.json` with sorted keys, then publishes it; with
    /// `expected`, only while the file still holds that manifest (DMN-364).
    pub async fn set_views(&self, params: SetViewsParams) -> Result<(), ErrorBody> {
        manifest::check(&params.manifest).map_err(bad_request)?;
        // Never between a mount's read and write (PRT-080..082), nor between the check of `expected` and the write.
        let _edit = self.manifest_edits.lock().await;
        let store = Arc::clone(&self.store);
        let id = params.workspace_id.clone();
        blocking(move || {
            let path = manifest_path(&workspace_path(&store, &id)?);
            if let Some(expected) = &params.expected {
                // What the file holds; `None` when it cannot be read as JSON, which no expectation matches.
                let holds = match std::fs::read_to_string(&path) {
                    Ok(text) => serde_json::from_str(&text).ok().map(ViewsFile::Manifest),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        Some(ViewsFile::Missing)
                    }
                    Err(_) => None,
                };
                let unchanged = match (&holds, expected) {
                    (Some(ViewsFile::Missing), ViewsFile::Missing) => true,
                    (Some(ViewsFile::Manifest(holds)), ViewsFile::Manifest(expected)) => {
                        manifest::same(holds, expected)
                    }
                    _ => false,
                };
                if !unchanged {
                    return Err(ErrorBody::new(
                        ErrorCode::Conflict,
                        ".omfp/views.json changed since: it no longer holds the expected manifest",
                    ));
                }
            }
            replace_file(&path, manifest::to_text(&params.manifest).as_bytes(), None)
                .map_err(|error| internal(format!("{}: {error}", path.display())))
        })
        .await?;
        self.refresh_views(&params.workspace_id).await;
        Ok(())
    }

    /// `widgets.copy` (DMN-087): copies a built-in or user widget into the workspace, then publishes its folders.
    pub async fn copy_widget(&self, params: CopyWidgetParams) -> Result<WidgetCopied, ErrorBody> {
        let (store, widgets) = (Arc::clone(&self.store), Arc::clone(&self.widgets));
        let id = params.workspace_id.clone();
        let name = blocking(move || {
            let path = workspace_path(&store, &id)?;
            widgets
                .copy(&path, &params.widget, params.name.as_deref())
                .map_err(copy_error)
        })
        .await?;
        self.refresh_views(&params.workspace_id).await;
        Ok(WidgetCopied { name })
    }

    /// `widgets.state_set` (DMN-086): stores one key of a widget instance (`null` deletes it) and publishes
    /// `widget_stored`.
    pub async fn set_widget_state(&self, params: SetWidgetStateParams) -> Result<(), ErrorBody> {
        let size = params.value.to_string().len();
        if size > MAX_STORED_BYTES {
            return Err(bad_request(format!(
                "a stored value is at most {MAX_STORED_BYTES} bytes of JSON; this one has {size}"
            )));
        }
        // Held while storing, so a topic created meanwhile reads the value or gets the event, never neither.
        let _serial = self.views_refresh.lock().await;
        let store = Arc::clone(&self.store);
        let stored = params.clone();
        blocking(move || {
            store.workspace(&stored.workspace_id)?;
            store
                .set_widget_state(
                    &stored.workspace_id,
                    &stored.instance,
                    &stored.key,
                    &stored.value,
                )
                .map_err(ErrorBody::from)
        })
        .await?;
        let SetWidgetStateParams {
            workspace_id,
            instance,
            key,
            value,
        } = params;
        if let Some(log) = self.views_log_entry(&workspace_id) {
            log.push(ViewsEvent::WidgetStored {
                instance,
                key,
                value,
            });
        }
        Ok(())
    }
}
