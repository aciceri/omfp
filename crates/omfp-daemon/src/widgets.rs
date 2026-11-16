//! Views and widgets (02-daemon "Views and widgets (M4)", 08 AGT-010..026): the views manifest of a workspace and its
//! default, the widget folders of the workspace and of the user library with their versions, serving widget files
//! (TypeScript and JSX transpiled with oxc), copying and checking widgets, and the read-only `omfp://` host URIs agents
//! learn widgets from.
//!
//! Everything here is blocking file work; the server runs it on blocking tasks.

pub mod compile;
pub mod folders;
pub mod init;
pub mod keymap;
pub mod layout;
pub mod library;
pub mod manifest;
pub mod mounts;
pub mod native;
pub mod shell;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use omfp_protocol::{WidgetInfo, WidgetSource};
use omp_rpc::{HostUriRead, HostUriResultContentType};
use serde_json::Value;
use tracing::warn;

use crate::host_tools::WidgetHost;
use crate::store::new_id;
use compile::{CompileError, Exports};
use folders::{files, is_widget_name};

/// The widget SDK and kit reference, served as [`DOCS_URL`] (AGT-025).
pub const DOCS: &str = include_str!("widgets/docs.md");
/// The JSON Schema of the views manifest, served as [`SCHEMA_URL`] (AGT-010, AGT-025).
pub const VIEWS_SCHEMA: &str = include_str!("widgets/views.schema.json");
/// The default manifest (AGT-010) as `views.set` writes it, written into every workspace (DMN-088).
pub const DEFAULT_MANIFEST: &str = include_str!("widgets/default-views.json");
/// Host URI of [`DOCS`] (DMN-062).
pub const DOCS_URL: &str = "omfp://docs/widgets";
/// Host URI of [`VIEWS_SCHEMA`] (DMN-062).
pub const SCHEMA_URL: &str = "omfp://schemas/views.json";
/// Host URI listing the built-in widgets; `<BUILTIN_URL>/<name>[/<path>]` reads their sources (DMN-062).
pub const BUILTIN_URL: &str = "omfp://widgets/builtin";
/// Host URI checking a widget: `<CHECK_URL>/<name>` reads the `omfp_widget_check` report (DMN-108).
pub const CHECK_URL: &str = "omfp://widgets/check";
/// The host URI scheme of the daemon (DMN-062).
pub const SCHEME: &str = "omfp";

/// Where a workspace keeps its views and widgets.
const OMFP_DIR: &str = ".omfp";

/// `<workspace>/.omfp/views.json` (AGT-010).
#[must_use]
pub fn manifest_path(workspace: &Path) -> PathBuf {
    workspace.join(OMFP_DIR).join("views.json")
}

/// `<workspace>/.omfp/widgets/`, the workspace's widget folders (AGT-012).
#[must_use]
pub fn workspace_widgets(workspace: &Path) -> PathBuf {
    workspace.join(OMFP_DIR).join("widgets")
}

/// `<workspace>/.omfp/views/`, the files of the TSX views (`"view": "./views/<name>.tsx"`, AGT-074).
#[must_use]
pub fn views_folder(workspace: &Path) -> PathBuf {
    workspace.join(OMFP_DIR).join("views")
}

/// The version of the workspace's [`views_folder`] (DMN-186, PRT-096): [`folders::version`] of it;
/// `None` when it is not a folder.
#[must_use]
pub fn view_files_version(workspace: &Path) -> Option<String> {
    let folder = views_folder(workspace);
    folder.is_dir().then(|| folders::version(&folder))
}

/// [`DEFAULT_MANIFEST`] as JSON (AGT-010).
///
/// # Panics
/// Never: the file is checked by a test.
#[must_use]
pub fn default_manifest() -> Value {
    serde_json::from_str(DEFAULT_MANIFEST).expect("the default manifest is JSON")
}

/// Writes the default manifest into `<workspace>/.omfp/views.json` unless a file is there (DMN-088): returns whether
/// it wrote one. A file created meanwhile is never replaced (the default is linked in place, not renamed over it).
pub fn write_default_manifest(workspace: &Path) -> std::io::Result<bool> {
    let path = manifest_path(workspace);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Ok(false);
    }
    let folder = workspace.join(OMFP_DIR);
    std::fs::create_dir_all(&folder)?;
    let staging = folder.join(format!(".views.json.{}.tmp", new_id()));
    std::fs::write(&staging, DEFAULT_MANIFEST)?;
    let linked = std::fs::hard_link(&staging, &path);
    let _ = std::fs::remove_file(&staging);
    match linked {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

fn markdown(content: String) -> HostUriRead {
    HostUriRead {
        content,
        content_type: Some(HostUriResultContentType::TextMarkdown),
        immutable: Some(true),
        ..HostUriRead::default()
    }
}

/// What an `omfp://` URL reads (DMN-062, DMN-122, DMN-198, DMN-199): the docs and their index, the schemas, the
/// default keymap (without a workspace's additions), the built-in widgets' sources from `builtin`
/// (`<static-dir>/builtin-widgets/`, `None` without a bundle), or an error naming what there is.
pub fn read_uri(url: &str, builtin: Option<&Path>) -> Result<HostUriRead, String> {
    let url = url.trim_end_matches('/');
    match url {
        shell::INDEX_URL => Ok(markdown(shell::index())),
        DOCS_URL => Ok(markdown(DOCS.to_owned())),
        shell::SHELL_URL => Ok(markdown(shell::SHELL.to_owned())),
        shell::KEYMAP_URL => Ok(markdown(shell::render_keymap(&shell::Additions::default()))),
        SCHEMA_URL => Ok(HostUriRead {
            content: VIEWS_SCHEMA.to_owned(),
            content_type: Some(HostUriResultContentType::ApplicationJson),
            immutable: Some(true),
            ..HostUriRead::default()
        }),
        keymap::SCHEMA_URL => Ok(HostUriRead {
            content: keymap::SCHEMA.to_owned(),
            content_type: Some(HostUriResultContentType::ApplicationJson),
            immutable: Some(true),
            ..HostUriRead::default()
        }),
        crate::jobs::file::SCHEMA_URL => Ok(HostUriRead {
            content: crate::jobs::file::SCHEMA.to_owned(),
            content_type: Some(HostUriResultContentType::ApplicationJson),
            immutable: Some(true),
            ..HostUriRead::default()
        }),
        BUILTIN_URL => Ok(markdown(list_builtins(builtin_folder(builtin)?))),
        _ => match url
            .strip_prefix(BUILTIN_URL)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            Some(rest) => read_builtin(builtin_folder(builtin)?, rest),
            None => Err(format!(
                "unknown omfp URL {url}: omfp serves {} (the index of these docs), {} (the protocol methods omfp_call \
                 accepts) and {}/<name> (one's schemas), {DOCS_URL} (the widget SDK and kit reference), {} (the \
                 window: left column, mode line, view tabs, `:` commands), {} (the default keys and commands), \
                 {SCHEMA_URL} (the views manifest schema), {} (the workspace keymap schema), {} (the workspace jobs \
                 schema), {BUILTIN_URL} (the built-in widgets) and {BUILTIN_URL}/<name>[/<path>] (their sources)",
                shell::INDEX_URL,
                crate::agent_tools::catalog::METHODS_URL,
                crate::agent_tools::catalog::METHODS_URL,
                shell::SHELL_URL,
                shell::KEYMAP_URL,
                keymap::SCHEMA_URL,
                crate::jobs::file::SCHEMA_URL
            )),
        },
    }
}

/// `omfp://keymap` read by a thread (DMN-199): the defaults with what the daemon's config and the thread's workspace
/// add, from `host`; `None` when `url` is not that URL or there is no host.
#[must_use]
pub fn read_keymap(
    url: &str,
    workspace_id: &str,
    host: Option<&dyn WidgetHost>,
) -> Option<Result<HostUriRead, String>> {
    if url.trim_end_matches('/') != shell::KEYMAP_URL {
        return None;
    }
    let additions = host?.keymap_additions(workspace_id);
    Some(Ok(HostUriRead {
        content: shell::render_keymap(&additions),
        content_type: Some(HostUriResultContentType::TextMarkdown),
        immutable: Some(false),
        ..HostUriRead::default()
    }))
}

/// `omfp://windows` read by a thread (AGT-081): the open windows showing the thread's workspace, as `omfp_windows`
/// describes them, read fresh every time; `None` when `url` is not that URL.
#[must_use]
pub fn read_windows(
    url: &str,
    workspace_id: &str,
    host: Option<&dyn WidgetHost>,
) -> Option<Result<HostUriRead, String>> {
    if url.trim_end_matches('/') != shell::WINDOWS_URL {
        return None;
    }
    let Some(host) = host else {
        return Some(Err(
            "this omfp daemon cannot describe its windows for this thread".to_owned(),
        ));
    };
    Some(Ok(HostUriRead {
        content: host.windows(workspace_id),
        content_type: Some(HostUriResultContentType::TextPlain),
        immutable: Some(false),
        ..HostUriRead::default()
    }))
}

fn builtin_folder(builtin: Option<&Path>) -> Result<&Path, String> {
    builtin.filter(|dir| dir.is_dir()).ok_or_else(|| {
        "no built-in widget sources: this omfp daemon serves no web bundle".to_owned()
    })
}

/// The built-in widget folders of `builtin`, by name.
fn builtin_names(builtin: &Path) -> Vec<String> {
    folders::scan(builtin, WidgetSource::Workspace)
        .into_iter()
        .map(|widget| widget.name)
        .collect()
}

/// `omfp://widgets/builtin`: every built-in with its files.
fn list_builtins(builtin: &Path) -> String {
    let mut text = format!(
        "# omfp built-in widgets\n\nEach is `omfp:<name>` in views.json. Read one's folder with {BUILTIN_URL}/<name> \
         and a file with {BUILTIN_URL}/<name>/<path>. A workspace folder `.omfp/widgets/<name>/` replaces \
         `omfp:<name>` in that workspace, so copy the files there to change one.\n\n"
    );
    for name in builtin_names(builtin) {
        let files: Vec<String> = files(&builtin.join(&name))
            .iter()
            .map(|(relative, _)| compile::display(relative))
            .collect();
        let _ = writeln!(text, "- `omfp:{name}`: {}", files.join(", "));
    }
    text
}

/// `omfp://widgets/builtin/<name>` (the folder: files and the entry's source) or `…/<name>/<path>` (one file).
fn read_builtin(builtin: &Path, rest: &str) -> Result<HostUriRead, String> {
    let (name, path) = rest
        .split_once('/')
        .map_or((rest, None), |(name, path)| (name, Some(path)));
    let names = builtin_names(builtin);
    if !names.iter().any(|known| known == name) {
        return Err(format!(
            "no built-in widget {name:?}; the built-ins are {}",
            names.join(", ")
        ));
    }
    let folder = std::fs::canonicalize(builtin.join(name)).map_err(|error| error.to_string())?;
    let read = |relative: &Path| {
        resolve(&folder, relative)
            .and_then(|file| std::fs::read_to_string(file).ok())
            .ok_or_else(|| {
                format!(
                    "omfp:{name} has no file {}; read {BUILTIN_URL}/{name} for its files",
                    compile::display(relative)
                )
            })
    };
    if let Some(path) = path {
        let relative = relative_path(path)
            .ok_or_else(|| format!("{path:?} is not a path inside the widget's folder"))?;
        return Ok(HostUriRead {
            content: read(&relative)?,
            content_type: Some(HostUriResultContentType::TextPlain),
            immutable: Some(true),
            ..HostUriRead::default()
        });
    }
    let mut text = format!(
        "# omfp:{name}\n\nFiles (read one with {BUILTIN_URL}/{name}/<path>; write them into \
         `.omfp/widgets/{name}/` to replace `omfp:{name}` in this workspace):\n\n"
    );
    for (relative, metadata) in files(&folder) {
        let lines = std::fs::read_to_string(folder.join(&relative))
            .map_or(0, |source| source.lines().count());
        let _ = writeln!(
            text,
            "- {} ({lines} lines, {} bytes)",
            compile::display(&relative),
            metadata.len()
        );
    }
    if let Some(entry) = folders::entry(&folder) {
        let source = read(Path::new(entry))?;
        let _ = write!(text, "\nEntry `{entry}`:\n\n```tsx\n{source}");
        if !source.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("```\n");
    }
    Ok(markdown(text))
}

/// `omfp://widgets/check/<name>` (DMN-108): the `omfp_widget_check` report of widget `name` of workspace
/// `workspace_id`, read fresh every time; `None` when `url` is no check URL.
pub fn read_check(
    url: &str,
    is_cancelled: impl Fn() -> bool + Sync,
    workspace_id: &str,
    host: Option<&dyn WidgetHost>,
) -> Option<Result<HostUriRead, String>> {
    let name = url
        .trim_end_matches('/')
        .strip_prefix(CHECK_URL)?
        .strip_prefix('/')?;
    let Some(host) = host else {
        return Some(Err(
            "this omfp daemon cannot check widgets, init files (`init`, `user:init`) or TSX views (`views/<file>`) \
             for this thread"
                .to_owned(),
        ));
    };
    if is_cancelled() {
        return Some(Err("cancelled".to_owned()));
    }
    let name = name.trim();
    if name.is_empty() {
        return Some(Err("`name` must be a non-empty string".to_owned()));
    }
    Some(
        host.check_widget(workspace_id, name, &is_cancelled)
            .map(|content| HostUriRead {
                content,
                content_type: Some(HostUriResultContentType::TextPlain),
                immutable: Some(false),
                ..HostUriRead::default()
            }),
    )
}

/// A file served from a widget folder (DMN-084).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Served {
    /// JavaScript made by the daemon: a transpiled file or a stand-in module (DMN-085).
    Module(String),
    /// A file served as it is; `path` gives its content type.
    File { path: PathBuf, bytes: Vec<u8> },
}

/// Why `widgets.copy` failed (DMN-087).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CopyError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Io(String),
}

/// A compiled file, valid while its size and modification time stay.
#[derive(Debug)]
struct Compiled {
    size: u64,
    modified: Option<SystemTime>,
    result: Result<String, CompileError>,
}

/// The widget libraries the daemon serves: the user library, the built-ins of the web bundle, and the compile cache.
#[derive(Debug)]
pub struct Widgets {
    user_library: PathBuf,
    builtin: Option<PathBuf>,
    exports: Option<Exports>,
    compiled: Mutex<HashMap<PathBuf, Compiled>>,
}

impl Widgets {
    /// The user library is `<data-dir>/widgets/` (AGT-023); with a web bundle (`static_dir`), the built-ins are its
    /// `builtin-widgets/` and the names the bundle exports are its `widget-exports.json` (DMN-085, DMN-087).
    #[must_use]
    pub fn new(user_library: PathBuf, static_dir: Option<&Path>) -> Self {
        let exports = static_dir.and_then(|dir| {
            let path = dir.join("widget-exports.json");
            match std::fs::read_to_string(&path) {
                Ok(text) => compile::parse_exports(&text)
                    .inspect_err(
                        |error| warn!(path = %path.display(), %error, "invalid widget exports"),
                    )
                    .ok(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    warn!(path = %path.display(), %error, "cannot read the widget exports");
                    None
                }
            }
        });
        Self {
            user_library,
            builtin: static_dir.map(|dir| dir.join("builtin-widgets")),
            exports,
            compiled: Mutex::new(HashMap::new()),
        }
    }

    /// `<data-dir>/widgets/` (AGT-023).
    #[must_use]
    pub fn user_library(&self) -> &Path {
        &self.user_library
    }

    /// `<static-dir>/builtin-widgets/` when the daemon serves a bundle that has it (AGT-021).
    #[must_use]
    pub fn builtin_dir(&self) -> Option<&Path> {
        self.builtin.as_deref().filter(|dir| dir.is_dir())
    }

    /// The names of the built-in widgets (`omfp:<name>`) of the bundle; none without one.
    #[must_use]
    pub fn builtin_names(&self) -> Vec<String> {
        self.builtin_dir().map(builtin_names).unwrap_or_default()
    }

    /// The manifest of the workspace at `workspace` and why it is refused (AGT-010, AGT-011, DMN-082).
    #[must_use]
    pub fn manifest(workspace: &Path) -> (Option<Value>, Option<String>) {
        manifest::read(&manifest_path(workspace))
    }

    /// The workspace's widget folders, then the user library's, each by name (DMN-082).
    #[must_use]
    pub fn widgets(&self, workspace: &Path) -> Vec<WidgetInfo> {
        let mut widgets = folders::scan(&workspace_widgets(workspace), WidgetSource::Workspace);
        widgets.extend(folders::scan(&self.user_library, WidgetSource::User));
        widgets
    }

    /// The file at `path` of widget folder `name` in `library` (DMN-084, DMN-085); `None` (404) when the folder or
    /// the file does not exist or `path` leaves the folder.
    #[must_use]
    pub fn serve(&self, library: &Path, name: &str, path: &str) -> Option<Served> {
        if !is_widget_name(name) {
            return None;
        }
        let folder = std::fs::canonicalize(library.join(name)).ok()?;
        if !folder.is_dir() {
            return None;
        }
        let file = resolve(&folder, &relative_path(path)?)?;
        let is_entry = folders::entry(&folder)
            .and_then(|entry| std::fs::canonicalize(folder.join(entry)).ok())
            .is_some_and(|entry| entry == file);
        if is_entry {
            // The entry stands for the whole folder: the first file that does not compile replaces it.
            let failure = files(&folder)
                .into_iter()
                .filter(|(relative, _)| compile::is_compiled(relative))
                .find_map(|(relative, _)| self.compile(&folder, &relative).err());
            if let Some(error) = failure {
                return Some(Served::Module(compile::stand_in(&error)));
            }
        }
        self.served(&folder, file)
    }

    /// The file at `path` of a workspace's views folder `folder` ([`views_folder`], DMN-186), compiled
    /// when it is TypeScript or JSX, else as it is; a file that does not compile is the stand-in module naming it
    /// relative to `folder`. `None` (404) when the folder or the file does not exist or `path` leaves the folder.
    #[must_use]
    pub fn serve_view(&self, folder: &Path, path: &str) -> Option<Served> {
        let folder = std::fs::canonicalize(folder).ok()?;
        if !folder.is_dir() {
            return None;
        }
        let file = resolve(&folder, &relative_path(path)?)?;
        self.served(&folder, file)
    }

    /// The file at `path` of a workspace's views folder `folder` resolved as [`Self::serve_view`]
    /// resolves it (DMN-197): its path relative to `folder` and, when it is TypeScript or JSX, its compile error;
    /// `None` when the folder or the file does not exist or `path` leaves the folder.
    #[must_use]
    pub fn check_view(&self, folder: &Path, path: &str) -> Option<(PathBuf, Option<CompileError>)> {
        let folder = std::fs::canonicalize(folder).ok()?;
        let file = resolve(&folder, &relative_path(path)?)?;
        let relative = file.strip_prefix(&folder).ok()?.to_owned();
        let error = compile::is_compiled(&relative)
            .then(|| self.compile(&folder, &relative).err())
            .flatten();
        Some((relative, error))
    }

    /// The resolved `file` of the canonical `folder`: compiled (or its stand-in) when it is TypeScript or JSX, else
    /// its bytes.
    fn served(&self, folder: &Path, file: PathBuf) -> Option<Served> {
        let relative = file.strip_prefix(folder).ok()?.to_owned();
        if compile::is_compiled(&relative) {
            return Some(Served::Module(
                self.compile(folder, &relative)
                    .unwrap_or_else(|error| compile::stand_in(&error)),
            ));
        }
        let bytes = std::fs::read(&file).ok()?;
        Some(Served::File { path: file, bytes })
    }

    /// The widget `name` of `workspace` (DMN-063): `./widgets/<name>`, `user:<name>`, or a bare name (the workspace's
    /// folder, else the user library's); its source, folder name and folder, or why there is none.
    pub fn locate(
        &self,
        workspace: &Path,
        name: &str,
    ) -> Result<(WidgetSource, String, PathBuf), String> {
        let name = name.trim();
        let workspace_widgets = workspace_widgets(workspace);
        let candidates: Vec<(WidgetSource, &str, &Path)> = if let Some(folder) =
            name.strip_prefix("./widgets/")
        {
            vec![(WidgetSource::Workspace, folder, &workspace_widgets)]
        } else if let Some(folder) = name.strip_prefix("user:") {
            vec![(WidgetSource::User, folder, &self.user_library)]
        } else if name.starts_with("omfp:") {
            return Err(format!(
                "{name} is a built-in widget; check the workspace's copy (`./widgets/<name>`) instead, or read \
                     its source at {BUILTIN_URL}/<name>"
            ));
        } else {
            vec![
                (WidgetSource::Workspace, name, &workspace_widgets),
                (WidgetSource::User, name, &self.user_library),
            ]
        };
        for (source, folder_name, library) in &candidates {
            if !is_widget_name(folder_name) {
                return Err(format!(
                    "invalid widget name {folder_name:?}: ASCII letters, digits, `_`, `-` and `.` (not first)"
                ));
            }
            let folder = library.join(folder_name);
            if folder.is_dir() {
                return Ok((*source, (*folder_name).to_owned(), folder));
            }
        }
        let places: Vec<String> = candidates
            .iter()
            .map(|(_, folder_name, library)| library.join(folder_name).display().to_string())
            .collect();
        Err(format!(
            "no widget {name}: no folder {}",
            places.join(" nor ")
        ))
    }

    /// Every compile error of the compiled files of `folder` (DMN-063, DMN-085), in path order.
    #[must_use]
    pub fn check(&self, folder: &Path) -> Vec<CompileError> {
        files(folder)
            .into_iter()
            .filter(|(relative, _)| compile::is_compiled(relative))
            .filter_map(|(relative, _)| self.compile(folder, &relative).err())
            .collect()
    }

    /// The compiled file `relative` of `folder`, from the cache while its size and modification time stay.
    fn compile(&self, folder: &Path, relative: &Path) -> Result<String, CompileError> {
        let path = folder.join(relative);
        let unreadable = |error: &std::io::Error| CompileError {
            file: compile::display(relative),
            line: None,
            column: None,
            message: format!("cannot read the file: {error}"),
        };
        let metadata = std::fs::metadata(&path).map_err(|error| unreadable(&error))?;
        let (size, modified) = (metadata.len(), metadata.modified().ok());
        {
            let cache = self.compiled.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(hit) = cache.get(&path)
                && hit.size == size
                && hit.modified == modified
            {
                return hit.result.clone();
            }
        }
        let source = std::fs::read_to_string(&path).map_err(|error| unreadable(&error))?;
        let result = compile::compile(relative, &source, self.exports.as_ref());
        self.compiled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                path,
                Compiled {
                    size,
                    modified,
                    result: result.clone(),
                },
            );
        result
    }

    /// Copies `omfp:<name>` (a built-in) or `user:<name>` into `<workspace>/.omfp/widgets/<new name>/`, the new name
    /// being `name` else the widget's own (DMN-087); returns the new name.
    pub fn copy(
        &self,
        workspace: &Path,
        widget: &str,
        name: Option<&str>,
    ) -> Result<String, CopyError> {
        let (library, source) = if let Some(source) = widget.strip_prefix("omfp:") {
            let Some(builtin) = self.builtin.as_deref() else {
                return Err(CopyError::NotFound(
                    "no built-in widgets: the daemon serves no web bundle".to_owned(),
                ));
            };
            (builtin, source)
        } else if let Some(source) = widget.strip_prefix("user:") {
            (self.user_library.as_path(), source)
        } else {
            return Err(CopyError::BadRequest(format!(
                "cannot copy {widget:?}: expected omfp:<name> or user:<name>"
            )));
        };
        let target = name.unwrap_or(source);
        for name in [source, target] {
            if !is_widget_name(name) {
                return Err(CopyError::BadRequest(format!(
                    "invalid widget name {name:?}: ASCII letters, digits, `_`, `-` and `.` (not first)"
                )));
            }
        }
        let from = library.join(source);
        if !from.is_dir() {
            return Err(CopyError::NotFound(format!("no widget {widget}")));
        }
        let widgets = workspace_widgets(workspace);
        let to = widgets.join(target);
        if std::fs::symlink_metadata(&to).is_ok() {
            return Err(CopyError::Conflict(format!(
                "the workspace already has a widget {target}"
            )));
        }
        // Copied under a hidden name (never listed nor served), then renamed: nobody sees half a widget.
        let staging = widgets.join(format!(".{target}.{}.tmp", new_id()));
        let io = |error: std::io::Error| CopyError::Io(format!("cannot copy {widget}: {error}"));
        let copied = std::fs::create_dir_all(&widgets)
            .and_then(|()| copy_dir(&from, &staging))
            .and_then(|()| std::fs::rename(&staging, &to));
        if let Err(error) = copied {
            let _ = std::fs::remove_dir_all(&staging);
            if std::fs::symlink_metadata(&to).is_ok() {
                return Err(CopyError::Conflict(format!(
                    "the workspace already has a widget {target}"
                )));
            }
            return Err(io(error));
        }
        Ok(target.to_owned())
    }
}

/// Copies the folder `from` to the new folder `to`: files (symlinks to files as their content) and real
/// subdirectories; symlinks to directories are left out.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_name() == folders::MOUNT_MARKER {
            continue;
        }
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type()?.is_dir() {
            copy_dir(&source, &target)?;
        } else if std::fs::metadata(&source)?.is_file() {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// The request path as a relative path without `..`; `None` when it is empty or tries to leave the folder.
fn relative_path(path: &str) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

/// The file `relative` names in `folder` (canonical `folder`): itself, else, without extension, the first of
/// `.tsx`, `.ts`, `.jsx`, `.js`; never outside the folder, even through a symlink (DMN-084).
fn resolve(folder: &Path, relative: &Path) -> Option<PathBuf> {
    let path = folder.join(relative);
    let candidates = if relative.extension().is_some() {
        vec![path]
    } else {
        std::iter::once(path.clone())
            .chain(["tsx", "ts", "jsx", "js"].into_iter().map(|extension| {
                let mut with = path.clone().into_os_string();
                with.push(".");
                with.push(extension);
                PathBuf::from(with)
            }))
            .collect()
    };
    candidates
        .into_iter()
        .filter(|candidate| candidate.is_file())
        .find_map(|candidate| std::fs::canonicalize(candidate).ok())
        .filter(|file| file.starts_with(folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-062, AGT-025, DMN-122, AGT-031
    #[test]
    fn omfp_urls_read_the_docs_and_the_schema_and_name_what_there_is_otherwise() {
        let docs = read_uri("omfp://docs/widgets", None).unwrap();
        assert_eq!(docs.content, DOCS);
        for section in [
            "## Keys",
            ".omfp/keymap.json",
            "omfp://schemas/keymap.json",
            "useCommand",
        ] {
            assert!(DOCS.contains(section), "{section:?} missing from the docs");
        }
        assert_eq!(
            docs.content_type,
            Some(HostUriResultContentType::TextMarkdown)
        );
        let schema = read_uri("omfp://schemas/views.json", None).unwrap();
        assert_eq!(schema.content, VIEWS_SCHEMA);
        let parsed: Value = serde_json::from_str(&schema.content).unwrap();
        assert_eq!(parsed["$id"], SCHEMA_URL);

        let error = read_uri("omfp://docs/nothing", None).unwrap_err();
        assert!(error.contains("omfp://docs/nothing"), "{error}");
        for url in [
            DOCS_URL,
            SCHEMA_URL,
            keymap::SCHEMA_URL,
            BUILTIN_URL,
            shell::INDEX_URL,
            shell::SHELL_URL,
            shell::KEYMAP_URL,
        ] {
            assert!(error.contains(url), "{url} missing: {error}");
        }
        assert!(
            !error.contains(CHECK_URL) && !error.contains(shell::WINDOWS_URL),
            "only what every reader (threads, MCP) gets: {error}"
        );
        let schema = read_uri("omfp://schemas/keymap.json", None).unwrap();
        assert_eq!(schema.content, keymap::SCHEMA);
        assert_eq!(
            schema.content_type,
            Some(HostUriResultContentType::ApplicationJson)
        );
    }

    // spec: DMN-198, DMN-199
    #[test]
    fn omfp_urls_read_the_docs_index_the_shell_reference_and_the_keymap() {
        for url in ["omfp://docs", "omfp://docs/"] {
            let index = read_uri(url, None).unwrap();
            assert_eq!(index.content, shell::index(), "{url}");
            assert_eq!(
                index.content_type,
                Some(HostUriResultContentType::TextMarkdown)
            );
        }
        let docs = read_uri("omfp://docs/shell", None).unwrap();
        assert_eq!(docs.content, shell::SHELL);
        assert_eq!(
            docs.content_type,
            Some(HostUriResultContentType::TextMarkdown)
        );
        let keys = read_uri("omfp://keymap", None).unwrap();
        assert_eq!(
            keys.content,
            shell::render_keymap(&shell::Additions::default())
        );

        assert_eq!(read_keymap("omfp://docs", "01WS", Some(&Checks)), None);
        assert_eq!(read_keymap("omfp://keymap", "01WS", None), None);
        let read = read_keymap("omfp://keymap/", "01WS", Some(&Checks))
            .unwrap()
            .unwrap();
        assert!(
            read.content
                .contains("  - normal mode: `g 01WS` → `tests.run`\n"),
            "the thread's workspace's bindings: {}",
            read.content
        );
        assert_eq!(read.immutable, Some(false), "the files change");
    }

    /// A bundle's `builtin-widgets/` with `header` (two files) and `chat`.
    fn builtin_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let header = dir.path().join("header");
        std::fs::create_dir_all(&header).unwrap();
        std::fs::write(
            header.join("index.ts"),
            "export { default } from \"./header.tsx\";\n",
        )
        .unwrap();
        std::fs::write(
            header.join("header.tsx"),
            "export default function Header() {\n  return <h1>omfp</h1>;\n}\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("chat")).unwrap();
        std::fs::write(dir.path().join("chat/index.ts"), "export {};\n").unwrap();
        dir
    }

    // spec: DMN-062
    #[test]
    fn omfp_urls_serve_the_built_in_widgets_sources() {
        let dir = builtin_dir();
        let builtin = Some(dir.path());

        let list = read_uri("omfp://widgets/builtin", builtin).unwrap();
        assert_eq!(
            list.content_type,
            Some(HostUriResultContentType::TextMarkdown)
        );
        assert!(
            list.content.contains("- `omfp:chat`: index.ts\n"),
            "{}",
            list.content
        );
        assert!(
            list.content
                .contains("- `omfp:header`: header.tsx, index.ts\n"),
            "{}",
            list.content
        );

        let folder = read_uri("omfp://widgets/builtin/header/", builtin).unwrap();
        assert!(
            folder.content.contains("- header.tsx (3 lines"),
            "{}",
            folder.content
        );
        assert!(
            folder.content.contains(
                "Entry `index.ts`:\n\n```tsx\nexport { default } from \"./header.tsx\";\n```"
            ),
            "{}",
            folder.content
        );
        assert!(
            folder.content.contains(".omfp/widgets/header/"),
            "{}",
            folder.content
        );

        let file = read_uri("omfp://widgets/builtin/header/header.tsx", builtin).unwrap();
        assert_eq!(file.content_type, Some(HostUriResultContentType::TextPlain));
        assert!(file.content.starts_with("export default function Header()"));
        let without_extension = read_uri("omfp://widgets/builtin/header/header", builtin).unwrap();
        assert_eq!(without_extension.content, file.content);

        for (url, needle) in [
            (
                "omfp://widgets/builtin/nothing",
                "the built-ins are chat, header",
            ),
            (
                "omfp://widgets/builtin/header/missing.ts",
                "has no file missing.ts",
            ),
            (
                "omfp://widgets/builtin/header/../chat/index.ts",
                "not a path inside",
            ),
        ] {
            let error = read_uri(url, builtin).unwrap_err();
            assert!(error.contains(needle), "{url}: {error}");
        }
        let error = read_uri("omfp://widgets/builtin/header", None).unwrap_err();
        assert!(error.contains("serves no web bundle"), "{error}");
    }

    /// A widget host answering checks with the workspace and name it was asked for, and binding `g <workspace>`.
    struct Checks;

    impl WidgetHost for Checks {
        fn check_widget(
            &self,
            workspace_id: &str,
            name: &str,
            _is_cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<String, String> {
            if name == "broken" {
                return Err("no widget broken".to_owned());
            }
            Ok(format!("report {workspace_id} {name}"))
        }

        fn keymap_additions(&self, workspace_id: &str) -> shell::Additions {
            shell::Additions {
                daemon: Vec::new(),
                workspace: Some(Ok(vec![omfp_protocol::WorkspaceBinding {
                    mode: omfp_protocol::KeymapMode::Normal,
                    keys: format!("g {workspace_id}"),
                    command: "tests.run".to_owned(),
                    args: Vec::new(),
                }])),
            }
        }

        fn windows(&self, workspace_id: &str) -> String {
            format!("windows of {workspace_id}")
        }
    }

    // spec: AGT-081
    #[test]
    fn the_windows_url_describes_the_windows_of_the_threads_workspace_fresh_every_time() {
        let read = read_windows("omfp://windows/", "01WS", Some(&Checks))
            .unwrap()
            .unwrap();
        assert_eq!(read.content, "windows of 01WS");
        assert_eq!(read.immutable, Some(false), "never cached: windows change");
        assert!(
            read_windows("omfp://windows", "01WS", None)
                .unwrap()
                .is_err()
        );
        assert_eq!(read_windows("omfp://keymap", "01WS", Some(&Checks)), None);
    }

    // spec: DMN-108
    #[test]
    fn the_check_url_reads_the_widget_check_report_fresh_every_time() {
        let check = |url: &str, cancelled: bool, host: Option<&dyn WidgetHost>| {
            read_check(url, || cancelled, "01WS", host)
        };
        let read = check("omfp://widgets/check/clock", false, Some(&Checks))
            .unwrap()
            .unwrap();
        assert_eq!(read.content, "report 01WS clock");
        assert_eq!(read.content_type, Some(HostUriResultContentType::TextPlain));
        assert_eq!(
            read.immutable,
            Some(false),
            "never cached: the report changes"
        );
        assert_eq!(
            check("omfp://widgets/check/user:clock/", false, Some(&Checks))
                .unwrap()
                .unwrap()
                .content,
            "report 01WS user:clock"
        );
        assert_eq!(
            check("omfp://widgets/check/broken", false, Some(&Checks)).unwrap(),
            Err("no widget broken".to_owned())
        );
        assert_eq!(
            check("omfp://widgets/check/clock", true, Some(&Checks)).unwrap(),
            Err("cancelled".to_owned())
        );
        assert_eq!(
            check("omfp://widgets/check/ ", false, Some(&Checks)).unwrap(),
            Err("`name` must be a non-empty string".to_owned())
        );
        assert!(
            check("omfp://widgets/check/clock", false, None)
                .unwrap()
                .is_err()
        );
        assert_eq!(check("omfp://docs/widgets", false, Some(&Checks)), None);
        assert!(
            read_uri("omfp://widgets/check", None)
                .unwrap_err()
                .starts_with("unknown omfp URL omfp://widgets/check: ")
        );
    }

    // spec: AGT-010, DMN-088
    #[test]
    fn the_default_manifest_is_valid_written_as_views_set_writes_and_has_the_main_and_side_slots() {
        let manifest = default_manifest();
        assert_eq!(manifest::check(&manifest), Ok(()));
        assert_eq!(manifest::to_text(&manifest), DEFAULT_MANIFEST);
        assert_eq!(manifest["default"], "default");
        assert_eq!(
            manifest["views"]["default"]["layout"],
            serde_json::json!([
                "row",
                { "sizes": [3, 1] },
                ["col", {}, "omfp:header", ["tabs", { "id": "main" }, "omfp:chat"]],
                ["tabs", { "id": "side" }, "omfp:workspace", "omfp:changes", "omfp:background"]
            ])
        );
    }

    // spec: DMN-088
    #[test]
    fn the_default_manifest_is_written_once_and_never_over_a_file() {
        let ws = tempfile::tempdir().unwrap();
        assert!(write_default_manifest(ws.path()).unwrap());
        let path = manifest_path(ws.path());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_MANIFEST);

        std::fs::write(&path, "{ \"mine\": true }").unwrap();
        assert!(!write_default_manifest(ws.path()).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ \"mine\": true }"
        );
        let left: Vec<_> = std::fs::read_dir(ws.path().join(".omfp"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["views.json"], "no staging file left");
    }

    // spec: DMN-063
    #[test]
    fn a_widget_is_located_by_reference_or_bare_name_and_checked_file_by_file() {
        let ws = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let widgets = Widgets::new(library.path().to_owned(), None);
        let tests = workspace_widgets(ws.path()).join("tests");
        std::fs::create_dir_all(&tests).unwrap();
        std::fs::write(
            tests.join("index.tsx"),
            "export default function T() { return <p>ok</p>; }\n",
        )
        .unwrap();
        std::fs::create_dir_all(library.path().join("clock")).unwrap();

        let (source, name, folder) = widgets.locate(ws.path(), "tests").unwrap();
        assert_eq!((source, name.as_str()), (WidgetSource::Workspace, "tests"));
        assert_eq!(folder, tests);
        assert_eq!(
            widgets.locate(ws.path(), "./widgets/tests").unwrap().2,
            tests
        );
        let (source, _, _) = widgets.locate(ws.path(), "clock").unwrap();
        assert_eq!(source, WidgetSource::User);
        assert_eq!(widgets.locate(ws.path(), "user:clock").unwrap().1, "clock");
        for (name, needle) in [
            ("nothing", "no widget nothing"),
            ("../x", "invalid widget name"),
            ("omfp:chat", "built-in"),
            ("user:tests", "no widget user:tests"),
        ] {
            let error = widgets.locate(ws.path(), name).unwrap_err();
            assert!(error.contains(needle), "{name}: {error}");
        }

        assert_eq!(widgets.check(&tests), Vec::new());
        std::fs::write(tests.join("lib.ts"), "export const = 1;\n").unwrap();
        std::fs::write(
            tests.join("other.tsx"),
            "import x from \"lodash\";\nexport default x;\n",
        )
        .unwrap();
        let errors = widgets.check(&tests);
        let files: Vec<&str> = errors.iter().map(|error| error.file.as_str()).collect();
        assert_eq!(files, ["lib.ts", "other.tsx"]);
        assert_eq!(errors[0].line, Some(1));
        assert!(errors[1].message.contains("lodash"), "{:?}", errors[1]);
    }

    // spec: DMN-186
    #[test]
    fn view_files_are_served_compiled_from_the_views_folder_and_never_outside_it() {
        let ws = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let widgets = Widgets::new(library.path().to_owned(), None);
        let folder = views_folder(ws.path());
        assert_eq!(folder, ws.path().join(".omfp/views"));
        assert_eq!(view_files_version(ws.path()), None, "no folder");
        assert_eq!(widgets.serve_view(&folder, "ops.tsx"), None, "no folder");

        std::fs::create_dir_all(folder.join("lib")).unwrap();
        std::fs::write(
            folder.join("ops.tsx"),
            "import { label } from \"./lib/label\";\nconst n: number = 1;\nexport default function Ops() {\n  return <p>{label(n)}</p>;\n}\n",
        )
        .unwrap();
        std::fs::write(
            folder.join("lib/label.ts"),
            "export const label = (n: number): string => `#${n}`;\n",
        )
        .unwrap();
        std::fs::write(folder.join("notes.txt"), "plain").unwrap();
        std::fs::write(ws.path().join(".omfp/secret.ts"), "export {};\n").unwrap();
        let version = view_files_version(ws.path()).unwrap();
        assert_eq!(version, folders::version(&folder));

        let Some(Served::Module(code)) = widgets.serve_view(&folder, "ops.tsx") else {
            panic!("ops.tsx is a module");
        };
        assert!(code.contains("./lib/label"), "{code}");
        assert!(!code.contains(": number"), "types are stripped: {code}");
        assert!(!code.contains("compileError"), "{code}");
        // A relative import resolves to its sibling by URL, extension or not.
        for path in ["lib/label", "lib/label.ts", "./lib/label.ts"] {
            let Some(Served::Module(code)) = widgets.serve_view(&folder, path) else {
                panic!("{path} is a module");
            };
            assert!(code.contains("export const label"), "{code}");
            assert!(!code.contains(": string"), "{code}");
        }
        assert_eq!(
            widgets.serve_view(&folder, "notes.txt"),
            Some(Served::File {
                path: std::fs::canonicalize(folder.join("notes.txt")).unwrap(),
                bytes: b"plain".to_vec(),
            })
        );
        for missing in [
            "nope.tsx",
            "../secret.ts",
            "lib/../../secret.ts",
            "",
            "/etc/passwd",
        ] {
            assert_eq!(widgets.serve_view(&folder, missing), None, "{missing:?}");
        }

        std::fs::write(
            folder.join("ops.tsx"),
            "export default function Ops() {\n  return <p>;\n}\n",
        )
        .unwrap();
        let Some(Served::Module(stand_in)) = widgets.serve_view(&folder, "ops.tsx") else {
            panic!("the stand-in is a module");
        };
        assert!(
            stand_in.contains("export const compileError = {\"file\":\"ops.tsx\",\"line\":3,"),
            "{stand_in}"
        );
        assert_ne!(view_files_version(ws.path()), Some(version), "a save");
    }

    // spec: DMN-197
    #[test]
    fn a_view_file_is_checked_as_it_is_served() {
        let ws = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let widgets = Widgets::new(library.path().to_owned(), None);
        let folder = views_folder(ws.path());
        assert_eq!(widgets.check_view(&folder, "ops.tsx"), None, "no folder");
        std::fs::create_dir_all(folder.join("lib")).unwrap();
        std::fs::write(
            folder.join("ops.tsx"),
            "import { label } from \"./lib/label\";\nexport default function Ops() {\n  return <p>{label}</p>;\n}\n",
        )
        .unwrap();
        std::fs::write(folder.join("lib/label.js"), "export const label = 1;\n").unwrap();
        std::fs::write(ws.path().join(".omfp/secret.ts"), "export {};\n").unwrap();

        assert_eq!(
            widgets.check_view(&folder, "ops.tsx"),
            Some((PathBuf::from("ops.tsx"), None))
        );
        assert_eq!(
            widgets.check_view(&folder, "ops"),
            Some((PathBuf::from("ops.tsx"), None)),
            "resolved like a request"
        );
        assert_eq!(
            widgets.check_view(&folder, "lib/label.js"),
            Some((PathBuf::from("lib/label.js"), None)),
            "served as it is"
        );
        for missing in ["nope.tsx", "../secret.ts", ""] {
            assert_eq!(widgets.check_view(&folder, missing), None, "{missing:?}");
        }
        std::fs::write(
            folder.join("ops.tsx"),
            "export default function Ops() {\n  return <p>;\n}\n",
        )
        .unwrap();
        let (relative, error) = widgets.check_view(&folder, "ops.tsx").unwrap();
        assert_eq!(relative, PathBuf::from("ops.tsx"));
        let error = error.unwrap();
        assert_eq!((error.file.as_str(), error.line), ("ops.tsx", Some(3)));
    }

    #[test]
    fn request_paths_stay_relative() {
        assert_eq!(
            relative_path("lib/chart.ts"),
            Some(PathBuf::from("lib/chart.ts"))
        );
        assert_eq!(
            relative_path("./index.tsx"),
            Some(PathBuf::from("index.tsx"))
        );
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a/.."] {
            assert_eq!(relative_path(bad), None, "{bad:?}");
        }
    }
}
