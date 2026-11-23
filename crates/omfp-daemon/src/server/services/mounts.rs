//! Mounts (AGT-060..063): `views.mount`, `views.mount_update` and `views.unmount` (PRT-080..082) edit the workspace's
//! `views.json` with [`crate::widgets::mounts`] and publish it as `views.set` does.

use std::path::Path;
use std::sync::Arc;

use omfp_protocol::{
    ErrorBody, ErrorCode, MountParams, MountPlace, MountUpdateParams, Mounted, UnmountParams,
    Unmounted,
};
use serde_json::Value;

use super::{Services, blocking};
use crate::paths::replace_file;
use crate::server::convert::{bad_request, internal};
use crate::widgets::folders::{ENTRIES, MOUNT_MARKER, checked_name};
use crate::widgets::layout::Place;
use crate::widgets::mounts::{self, Mount, MountError, Placed, Target};
use crate::widgets::{Widgets, manifest, manifest_path, workspace_widgets};

fn mount_error(error: MountError) -> ErrorBody {
    match error {
        MountError::Invalid(message) => bad_request(message),
        MountError::NotFound(message) => ErrorBody::new(ErrorCode::NotFound, message),
    }
}

/// `id` when it is a widget name (AGT-061).
fn mount_id(id: &str) -> Result<&str, ErrorBody> {
    checked_name("mount id", id.trim()).map_err(bad_request)
}

/// A widget reference as the agent wrote it: a bare name means `./widgets/<name>` (AGT-061).
fn reference(widget: &str) -> Result<String, ErrorBody> {
    let widget = widget.trim();
    if widget.is_empty() {
        return Err(bad_request("`widget` is empty"));
    }
    Ok(
        if ["./widgets/", "omfp:"]
            .iter()
            .any(|prefix| widget.starts_with(prefix))
        {
            widget.to_owned()
        } else {
            format!("./widgets/{widget}")
        },
    )
}

fn place(place: MountPlace) -> Place {
    match place {
        MountPlace::SideTab => Place::SideTab,
        MountPlace::MainTab => Place::MainTab,
        MountPlace::End => Place::End,
    }
}

fn mounted(id: String, placed: Placed, code_replaced: bool) -> Mounted {
    Mounted {
        id,
        reference: placed.reference,
        panel: placed.view.is_none(),
        view: placed.view,
        replaced: placed.replaced || code_replaced,
    }
}

/// Whether the code of mount `id` may go to `.omfp/widgets/<id>/` of the workspace at `workspace` (DMN-183): not when
/// `id` names a built-in (the folder would replace `omfp:<id>` in the whole workspace), nor when the folder exists
/// without this mount's marker or has another entry file (which `index.tsx` would shadow or that would shadow it).
/// `Ok(true)` when the mount's own folder exists: its code is replaced. `update` says the conflict as
/// `views.mount_update`'s, which cannot choose another id.
fn claim_folder(
    workspace: &Path,
    id: &str,
    builtins: &[String],
    update: bool,
) -> Result<bool, ErrorBody> {
    let conflict = |why: String| {
        let message = if update {
            format!(
                "cannot update the code of mount {id}: {why}; mount the code again under another id"
            )
        } else {
            format!("cannot mount code as {id}: {why}; choose another id")
        };
        ErrorBody::new(ErrorCode::Conflict, message)
    };
    if builtins.iter().any(|name| name == id) {
        return Err(conflict(format!(
            "a folder .omfp/widgets/{id}/ would replace the built-in omfp:{id} in this workspace"
        )));
    }
    let folder = workspace_widgets(workspace).join(id);
    match std::fs::symlink_metadata(&folder) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(internal(format!("{}: {error}", folder.display()))),
        Ok(metadata) if !metadata.is_dir() => Err(conflict(format!(
            ".omfp/widgets/{id} exists and is not a folder"
        ))),
        Ok(_) if !folder.join(MOUNT_MARKER).is_file() => Err(conflict(format!(
            ".omfp/widgets/{id}/ holds a widget no mount wrote"
        ))),
        Ok(_) => match ENTRIES
            .into_iter()
            .find(|entry| *entry != "index.tsx" && folder.join(entry).exists())
        {
            Some(entry) => Err(conflict(format!(".omfp/widgets/{id}/ also has {entry}"))),
            None => Ok(true),
        },
    }
}

/// Replaces `file` with `bytes`; an error names the file.
fn write(file: &Path, bytes: &[u8]) -> Result<(), ErrorBody> {
    replace_file(file, bytes, None)
        .map_err(|error| internal(format!("{}: {error}", file.display())))
}

/// Writes `.omfp/widgets/<id>/index.tsx` of the workspace at `workspace`, with the mount's marker.
fn write_code(workspace: &Path, id: &str, code: &str) -> Result<(), ErrorBody> {
    let folder = workspace_widgets(workspace).join(id);
    write(
        &folder.join(MOUNT_MARKER),
        format!("Written by views.mount `{id}`: its code (index.tsx) is replaced by the next mount with code.\n")
            .as_bytes(),
    )?;
    write(&folder.join("index.tsx"), code.as_bytes())
}

fn write_manifest(workspace: &Path, manifest: &Value) -> Result<(), ErrorBody> {
    write(
        &manifest_path(workspace),
        manifest::to_text(manifest).as_bytes(),
    )
}

impl Services {
    /// Runs `edit` on the folder and the valid manifest (`None` without a file) of workspace `workspace_id`, one edit
    /// of a manifest at a time, then publishes the workspace's views; an invalid file is a `bad_request` naming its
    /// error (PRT-080).
    async fn edit_manifest<T: Send + 'static>(
        &self,
        workspace_id: &str,
        edit: impl FnOnce(&Path, Option<Value>) -> Result<T, ErrorBody> + Send + 'static,
    ) -> Result<T, ErrorBody> {
        let _edit = self.manifest_edits.lock().await;
        let store = Arc::clone(&self.store);
        let id = workspace_id.to_owned();
        let edited = blocking(move || {
            let path = store.workspace(&id)?.path;
            let (manifest, error) = Widgets::manifest(&path);
            if let Some(error) = error {
                return Err(bad_request(format!(
                    ".omfp/views.json is invalid ({error}); fix it first (or ask the user to run `:view-reset`)"
                )));
            }
            edit(&path, manifest)
        })
        .await?;
        self.refresh_views(workspace_id).await;
        Ok(edited)
    }

    /// `views.mount` (PRT-080, AGT-061).
    pub async fn mount(&self, params: MountParams) -> Result<Mounted, ErrorBody> {
        let id = mount_id(&params.id)?.to_owned();
        let reference = match (&params.widget, &params.code) {
            (Some(widget), None) => reference(widget)?,
            (None, Some(_)) => format!("./widgets/{id}"),
            _ => return Err(bad_request("give exactly one of `widget` and `code`")),
        };
        if params.panel && (params.view.is_some() || params.place.is_some()) {
            return Err(bad_request(
                "a panel floats over every view: `view` and `place` do not apply",
            ));
        }
        let builtins = self.widgets.builtin_names();
        let workspace_id = params.workspace_id.clone();
        self.edit_manifest(&workspace_id, move |path, current| {
            let code_replaced = match params.code {
                Some(_) => claim_folder(path, &id, &builtins, false)?,
                None => false,
            };
            let view = params.view.as_deref().map(str::trim);
            let target = if params.panel {
                Target::Panel
            } else {
                Target::View {
                    view,
                    place: place(params.place.unwrap_or_default()),
                }
            };
            let placed = mounts::mount(
                current.as_ref(),
                &Mount {
                    id: &id,
                    reference: &reference,
                    props: params.props.unwrap_or_default(),
                    target,
                },
            )
            .map_err(mount_error)?;
            // The code first: a window importing the new reference finds it.
            if let Some(code) = &params.code {
                write_code(path, &id, code)?;
            }
            write_manifest(path, &placed.manifest)?;
            Ok(mounted(id, placed, code_replaced))
        })
        .await
    }

    /// `views.mount_update` (PRT-081, AGT-062).
    pub async fn mount_update(&self, params: MountUpdateParams) -> Result<Mounted, ErrorBody> {
        let id = mount_id(&params.id)?.to_owned();
        if params.props.is_none() && params.code.is_none() {
            return Err(bad_request("give `props`, `code` or both"));
        }
        let builtins = self.widgets.builtin_names();
        let workspace_id = params.workspace_id.clone();
        self.edit_manifest(&workspace_id, move |path, current| {
            let props = params.props.unwrap_or_default();
            let placed = mounts::update(current.as_ref(), &id, &props).map_err(mount_error)?;
            if let Some(code) = &params.code {
                let own = format!("./widgets/{id}");
                if placed.reference != own {
                    return Err(bad_request(format!(
                        "mount {id} shows {}, not inline code ({own}): mount it again with `code` instead",
                        placed.reference
                    )));
                }
                claim_folder(path, &id, &builtins, true)?;
                write_code(path, &id, code)?;
            }
            if current.as_ref() != Some(&placed.manifest) {
                write_manifest(path, &placed.manifest)?;
            }
            Ok(mounted(id, placed, true))
        })
        .await
    }

    /// `views.unmount` (PRT-082, AGT-063).
    pub async fn unmount(&self, params: UnmountParams) -> Result<Unmounted, ErrorBody> {
        let id = params.id.trim().to_owned();
        self.edit_manifest(&params.workspace_id, move |path, current| {
            let removed = mounts::unmount(current.as_ref(), &id).map_err(mount_error)?;
            write_manifest(path, &removed.manifest)?;
            Ok(Unmounted {
                views: removed.views,
                removed_views: removed.removed_views,
                panel: removed.panel,
            })
        })
        .await
    }
}
