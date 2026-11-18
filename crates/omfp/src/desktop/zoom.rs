//! The page zoom of the Linux window (UI-283). `WebKitGTK` zooms on no key and no wheel, and the window has no menu
//! (macOS's zoom items, UI-281) and no IPC, so an initialization script in the daemon's top-level page turns Ctrl with
//! `+`, `=`, `-`, `0` and Ctrl with the wheel into messages to the window's `WebKit` script message handlers, which
//! step the web view's zoom level through [`zoomed`]'s levels.
//!
//! The script, not the GTK events, decides because only the page knows whether it handled the event: `WebKitGTK`
//! re-emits to GTK the key and wheel events the page left alone, but a wheel event that scrolled anything (Ctrl with the
//! wheel over the transcript) counts as handled and never comes back.

use tauri::WebviewWindow;
use webkit2gtk::glib::prelude::ObjectExt as _;
use webkit2gtk::{UserContentManagerExt as _, WebViewExt as _};

use super::{Zoom, zoomed};

/// The zoom steps by the names the script posts them under.
const STEPS: [(&str, Zoom); 3] = [("in", Zoom::In), ("out", Zoom::Out), ("reset", Zoom::Reset)];

/// The script message handler of the step `name` in the window of `nonce`: only the window's own script knows it, so a
/// frame of another origin cannot zoom the window (DMN-182).
fn handler(name: &str, nonce: &str) -> String {
    format!("omfpZoom-{name}-{nonce}")
}

/// The page's zoom keys and wheel (UI-283), for the top-level page of the daemon's `origin` only (DMN-182). Its
/// listeners are on the window, so the page's own listeners (on elements and the document) run first: an event they
/// prevented is theirs (an image viewer zooming its image, UI-550); one they left is prevented here, so Ctrl with the
/// wheel never scrolls. A wheel notch is 120 of `wheelDeltaY` (`WebKitGTK`'s, whatever the pixels it scrolls); a
/// touchpad's shares add up to a step, and a turn the other way starts again.
#[must_use]
pub fn zoom_script(origin: &str, nonce: &str) -> String {
    let handlers: serde_json::Map<String, serde_json::Value> = STEPS
        .iter()
        .map(|(name, _)| ((*name).to_owned(), handler(name, nonce).into()))
        .collect();
    format!(
        "(() => {{\n  const ORIGIN = {};\n  const HANDLERS = {};\n{ZOOM_BODY}",
        serde_json::Value::from(origin),
        serde_json::Value::Object(handlers)
    )
}

const ZOOM_BODY: &str = r#"  if (window.top !== window || location.origin !== ORIGIN) return;
  const zoom = (step) => window.webkit?.messageHandlers?.[HANDLERS[step]]?.postMessage(step);
  const KEYS = { "+": "in", "=": "in", "-": "out", "0": "reset" };
  window.addEventListener("keydown", (event) => {
    const step = KEYS[event.key];
    if (step === undefined || !event.ctrlKey || event.altKey || event.metaKey || event.isComposing) return;
    if (event.defaultPrevented) return;
    event.preventDefault();
    zoom(step);
  });
  const NOTCH = 120;
  let ticks = 0;
  window.addEventListener(
    "wheel",
    (event) => {
      if (!event.ctrlKey || event.defaultPrevented) return;
      event.preventDefault();
      const delta = event.wheelDeltaY;
      ticks = Math.sign(delta) === -Math.sign(ticks) ? delta : ticks + delta;
      while (Math.abs(ticks) >= NOTCH) {
        zoom(ticks > 0 ? "in" : "out");
        ticks -= Math.sign(ticks) * NOTCH;
      }
    },
    { passive: false },
  );
})();"#;

/// Registers the window's zoom handlers of [`zoom_script`] with `nonce`: each message steps the web view's zoom level.
pub fn follow_zoom(window: &WebviewWindow, nonce: &str) -> tauri::Result<()> {
    let handlers = STEPS.map(|(name, step)| (handler(name, nonce), step));
    window.with_webview(move |webview| {
        let view = webview.inner();
        let Some(manager) = view.user_content_manager() else {
            eprintln!("omfp desktop: no page zoom, the web view has no user content manager");
            return;
        };
        for (name, step) in handlers {
            if !manager.register_script_message_handler(&name) {
                eprintln!("omfp desktop: no page zoom, cannot register its message handler");
                return;
            }
            // The view keeps its manager, which keeps this handler: a weak reference, no cycle.
            let view = view.downgrade();
            manager.connect_script_message_received(Some(&name), move |_, _| {
                if let Some(view) = view.upgrade() {
                    view.set_zoom_level(zoomed(view.zoom_level(), step));
                }
            });
        }
    })
}
