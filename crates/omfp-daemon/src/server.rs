//! The WebSocket server (03-protocol): `/api/ws` for clients, the MCP server at `/mcp` for agents outside omfp
//! (DMN-155), the user's theme at `/theme.css` (DMN-075), widget modules under the secret `widgetBase` (DMN-084),
//! workspace files under the secret `fileBase` (DMN-250), and the web UI's static files at `/` when `--static-dir` is
//! set.

mod activity;
mod auth;
mod connection;
pub(crate) mod convert;
pub(crate) mod dispatch;
mod mcp;
mod prompts;
mod raw_files;
mod services;

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{self, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use omfp_protocol::{BuiltinTheme, UiSettings};
use subtle::ConstantTimeEq;
use tokio::sync::{Semaphore, watch};

pub use auth::{Permission, Principal, authorize, origin_allowed, parse_origin, permission};
#[cfg(test)]
pub(crate) use connection::Outbound;
pub(crate) use mcp::Sessions as McpSessions;
pub(crate) use raw_files::PREFIX as FILES_PREFIX;
pub(crate) use services::{CHECK_WAIT, Services};

use crate::token::Token;
use crate::widgets::{Served, init, views_folder, workspace_widgets};

/// Protocol versions the daemon accepts in `system.hello`: the current one and, during a transition, the previous
/// one (PRT-032). There is no previous version yet.
pub const SUPPORTED_PROTOCOLS: &[u32] = &[omfp_protocol::PROTOCOL_VERSION];

/// Frames a client may have queued before its topics are dropped with `resync_required` (PRT-023).
pub(crate) const OUTBOUND_QUEUE: usize = 1024;
/// Connections that may wait for their `system.hello` at once; further upgrades get `503` (DMN-184).
pub const UNAUTHENTICATED_CONNECTIONS: usize = 64;

/// Everything a connection needs.
pub(crate) struct AppState {
    pub token: Token,
    /// omp version reported in `system.hello` (PRT-005).
    pub omp_version: String,
    /// Startup warnings reported in `system.hello`, e.g. `--allow-omp-mismatch` (DMN-001).
    pub warnings: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub static_dir: Option<PathBuf>,
    /// `/widgets/<key>`: where authenticated clients load widget modules, the key random per start (DMN-084).
    pub widget_base: String,
    /// `/files/<key>`: where authenticated clients load the raw bytes of workspace files, the key random per start
    /// and distinct from the widget key (DMN-250).
    pub file_base: String,
    pub services: Services,
    /// The MCP server's legacy sessions (DMN-157).
    pub mcp_sessions: McpSessions,
    /// Becomes `true` when the daemon shuts down; connections then close.
    pub shutdown: watch::Receiver<bool>,
    /// One permit per connection still waiting for its `system.hello` (DMN-184).
    pub unauthenticated: Arc<Semaphore>,
}

pub(crate) fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/ws", get(upgrade))
        .route(mcp::PATH, post(mcp::post).delete(mcp::delete))
        .route("/theme.css", get(theme_css))
        .route("/widgets/{*rest}", get(widget_file))
        .route("/files/{*rest}", get(raw_files::serve))
        .fallback(static_file)
        .with_state(state)
}

/// The user's theme CSS (DMN-075): served only while `[ui] theme` is `matugen` (DMN-240), so a window loading under
/// another theme never shows it; empty otherwise or when none is set, never cached (a change must show on reload), and
/// served without the token like the bundle.
async fn theme_css(State(state): State<Arc<AppState>>) -> Response {
    let css = if state.services.settings().ui.theme == Some(BuiltinTheme::Matugen) {
        state.services.theme().css.unwrap_or_default()
    } else {
        String::new()
    };
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        css,
    )
        .into_response()
}

/// `GET /widgets/<key>/<source>/<name>/<version>/<path>` (DMN-084, DMN-085): a file of a workspace's widget folder
/// (`<source>` = its id) or of the user library (`user`), compiled when it is TypeScript or JSX. A wrong key, an
/// unknown source, folder or file, or a path leaving the folder is `404`; nothing is cached.
///
/// `GET /widgets/<key>/init/<scope>/<version>/init.ts` (DMN-185, PRT-095): the user's init file
/// (`<scope>` = `user`) or a workspace's `.omfp/init.ts` (`<scope>` = its id), compiled; the version only busts caches.
///
/// `GET /widgets/<key>/views/<workspaceId>/<version>/<path>` (DMN-186, PRT-096): the file `<path>` of
/// the workspace's `.omfp/views/` (subfolders too, so relative imports resolve to siblings by URL), compiled when it is
/// TypeScript or JSX, the stand-in module when it does not compile; `<version>` (`viewFiles`) only busts caches.
async fn widget_file(
    State(state): State<Arc<AppState>>,
    extract::Path(rest): extract::Path<String>,
) -> Response {
    const NO_STORE: (header::HeaderName, &str) = (header::CACHE_CONTROL, "no-store");
    let not_found = || ([NO_STORE], StatusCode::NOT_FOUND).into_response();
    let mut parts = rest.splitn(5, '/');
    let (Some(key), Some(source), Some(name), Some(_version), Some(path)) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return not_found();
    };
    let expected = state
        .widget_base
        .strip_prefix("/widgets/")
        .unwrap_or_default();
    if !bool::from(expected.as_bytes().ct_eq(key.as_bytes())) {
        return not_found();
    }
    let services = state.services.clone();
    let (source, name, path) = (source.to_owned(), name.to_owned(), path.to_owned());
    let served = tokio::task::spawn_blocking(move || {
        if source == "init" {
            if path != init::FILE {
                return None;
            }
            let file = if name == "user" {
                services.init_file()?
            } else {
                init::workspace_init(&services.store.workspace(&name).ok()?.path)
            };
            return services.widgets.serve_init(&file).map(Served::Module);
        }
        if source == "views" {
            let workspace = services.store.workspace(&name).ok()?.path;
            return services
                .widgets
                .serve_view(&views_folder(&workspace), &path);
        }
        let library = if source == "user" {
            services.widgets.user_library().to_path_buf()
        } else {
            workspace_widgets(&services.store.workspace(&source).ok()?.path)
        };
        services.widgets.serve(&library, &name, &path)
    })
    .await;
    match served {
        Ok(Some(Served::Module(code))) => (
            [
                (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                NO_STORE,
            ],
            code,
        )
            .into_response(),
        Ok(Some(Served::File { path, bytes })) => (
            [(header::CONTENT_TYPE, content_type(&path)), NO_STORE],
            bytes,
        )
            .into_response(),
        _ => not_found(),
    }
}

async fn upgrade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let (origin, host) = (header(header::ORIGIN), header(header::HOST));
    if !origin_allowed(origin, host, &state.allowed_origins) {
        tracing::warn!(
            origin,
            host,
            "refused a WebSocket upgrade from a foreign origin; if the UI is reached through a tunnel or a \
             reverse proxy, add that origin to `allowed_origins` (DMN-140)"
        );
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    // A connection holds its permit until `system.hello` succeeds or it closes (DMN-184).
    let Ok(permit) = Arc::clone(&state.unauthenticated).try_acquire_owned() else {
        tracing::warn!(
            "refused a WebSocket upgrade: {UNAUTHENTICATED_CONNECTIONS} connections are waiting for system.hello"
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "too many connections waiting for system.hello",
        )
            .into_response();
    };
    upgrade
        .max_message_size(omfp_protocol::MAX_FRAME_BYTES)
        .max_frame_size(omfp_protocol::MAX_FRAME_BYTES)
        .on_upgrade(move |socket| connection::serve(socket, state, permit))
}

/// Serves `--static-dir`; unknown paths get `index.html` so the UI can route them. `index.html` carries the daemon's
/// appearance on `<html>` (`html_with_appearance`) and is never cached, so a reload shows the current one.
async fn static_file(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let Some(root) = state.static_dir.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(relative) = safe_relative(request.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The `/kit` gallery takes its whole appearance from its query (UI-173).
    let gallery = matches!(request.uri().path(), "/kit" | "/kit/");
    let read = tokio::task::spawn_blocking(move || {
        let index = root.join("index.html");
        let path = root.join(&relative);
        if path.is_file() && path != index {
            return std::fs::read(&path).map(|bytes| (bytes, content_type(&path), false));
        }
        let bytes = std::fs::read(&index)?;
        let bytes = if gallery {
            bytes
        } else {
            html_with_appearance(bytes, &state.services.settings().ui)
        };
        Ok((bytes, content_type(&index), true))
    })
    .await;
    match read {
        Ok(Ok((bytes, content_type, false))) => {
            ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
        }
        Ok(Ok((bytes, content_type, true))) => (
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `html` with the daemon's `[ui]` theme, skin, density, motion and font size on its `<html>` tag, as the window puts
/// them (UI-091, UI-510, UI-794), so the first paint already shows them instead of the defaults until the window's
/// first `index` snapshot. Only set values are added: an absent one is the stylesheets' default. `matugen` (Synth under
/// the user theme, which `/theme.css` brings) and `accent` (Synth unless the host gives a system accent, UI-512) add no
/// theme, Synth being the stylesheets' default. Without an `<html` tag, `html` is unchanged.
fn html_with_appearance(html: Vec<u8>, ui: &UiSettings) -> Vec<u8> {
    const TAG: &[u8] = b"<html";
    let theme = ui
        .theme
        .filter(|theme| !matches!(theme, BuiltinTheme::Matugen | BuiltinTheme::Accent));
    let mut attributes = String::new();
    for (name, value) in [
        ("data-theme", theme.and_then(setting_name)),
        ("data-skin", ui.skin.and_then(setting_name)),
        ("data-density", ui.density.and_then(setting_name)),
        ("data-motion", ui.motion.and_then(setting_name)),
    ] {
        if let Some(value) = value {
            let _ = write!(attributes, " {name}=\"{value}\"");
        }
    }
    if let Some(size) = ui.font_size {
        let _ = write!(attributes, " style=\"font-size:{size}px\"");
    }
    if attributes.is_empty() {
        return html;
    }
    let Some(at) = html
        .windows(TAG.len())
        .position(|window| window == TAG)
        .map(|at| at + TAG.len())
        .filter(|&at| {
            html.get(at)
                .is_some_and(|&next| next == b'>' || next.is_ascii_whitespace())
        })
    else {
        return html;
    };
    let mut with = Vec::with_capacity(html.len() + attributes.len());
    with.extend_from_slice(&html[..at]);
    with.extend_from_slice(attributes.as_bytes());
    with.extend_from_slice(&html[at..]);
    with
}

/// The protocol's name of a setting's value (`tokyo-night`, `boring`, …): a string for every `[ui]` enum.
fn setting_name(value: impl serde::Serialize) -> Option<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Some(name),
        _ => None,
    }
}

/// The request path as a relative path without `..`; `None` when it tries to leave the root.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in Path::new(path.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(relative)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_paths_cannot_leave_the_root() {
        assert_eq!(safe_relative("/"), Some(PathBuf::new()));
        assert_eq!(
            safe_relative("/assets/app.js"),
            Some(PathBuf::from("assets/app.js"))
        );
        assert_eq!(safe_relative("/../etc/passwd"), None);
        assert_eq!(safe_relative("/a/../../b"), None);
    }

    // spec: UI-141
    #[test]
    fn the_bundled_fonts_and_their_licences_are_served_with_their_types() {
        assert_eq!(
            content_type(Path::new("commit-mono-latin-400-normal-a1b2.woff2")),
            "font/woff2"
        );
        assert_eq!(
            content_type(Path::new("commit-mono-OFL.txt")),
            "text/plain; charset=utf-8"
        );
    }

    // spec: UI-425
    #[test]
    fn the_web_app_manifest_is_served_as_a_manifest() {
        assert_eq!(
            content_type(Path::new("manifest.webmanifest")),
            "application/manifest+json"
        );
    }
}
