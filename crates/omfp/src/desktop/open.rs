//! Opening a page in the browser without a click (UI-380, Linux). `WebKitGTK` opens a new window from script only
//! within a click, so `window.open` of a login's page, which reaches the window seconds after the click that started
//! the login, never gets to the window's new-window handler. An initialization script gives the daemon's top-level page
//! `window.omfpOpenExternal(url)`, which posts the address to a script message handler of the window; the window opens
//! it in the browser as its new-window handler would, only pages of other origins (DMN-182). Only the window's own
//! script knows the handler's name, so a frame of another origin cannot use it.

use tauri::{Url, WebviewWindow};
use webkit2gtk::UserContentManagerExt as _;
use webkit2gtk::WebViewExt as _;

/// The script message handler of the window of `nonce`.
fn handler(nonce: &str) -> String {
    format!("omfpOpen-{nonce}")
}

/// `window.omfpOpenExternal(url)` for the top-level page of the daemon's `origin` only (DMN-182).
#[must_use]
pub fn open_script(origin: &str, nonce: &str) -> String {
    let quoted = |text: &str| serde_json::Value::from(text).to_string();
    format!(
        "(() => {{\n  const ORIGIN = {};\n  const HANDLER = {};\n{OPEN_BODY}",
        quoted(origin),
        quoted(&handler(nonce))
    )
}

const OPEN_BODY: &str = r#"  if (window.top !== window || location.origin !== ORIGIN) return;
  const post = window.webkit?.messageHandlers?.[HANDLER];
  if (post === undefined) return;
  Object.defineProperty(window, "omfpOpenExternal", { value: (url) => post.postMessage(String(url)) });
})();"#;

/// Registers the handler of [`open_script`] with `nonce`: each address it gets that the window's new-window handler
/// would send to the browser (a page of another origin than the daemon's `origin`) goes there.
pub fn follow_open(window: &WebviewWindow, nonce: &str, origin: String) {
    let name = handler(nonce);
    let followed = window.with_webview(move |webview| {
        let view = webview.inner();
        let Some(manager) = view.user_content_manager() else {
            eprintln!(
                "omfp desktop: pages open only on a click, the web view has no user content manager"
            );
            return;
        };
        if !manager.register_script_message_handler(&name) {
            eprintln!(
                "omfp desktop: pages open only on a click, cannot register its message handler"
            );
            return;
        }
        manager.connect_script_message_received(Some(&name), move |_, message| {
            let Some(value) = message.js_value() else {
                return;
            };
            if let Ok(url) = Url::parse(&value.to_string())
                && super::opens_externally(&url, &origin)
            {
                crate::tray::open_in_browser(url.as_str());
            }
        });
    });
    if let Err(error) = followed {
        eprintln!("omfp desktop: pages open only on a click: {error}");
    }
}
