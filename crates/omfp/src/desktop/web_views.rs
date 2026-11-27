//! Web pages as native views over the desktop window's page (UI-682..UI-686; spikes S-16 and S-17). The kit's
//! `WebView` (UI-670) is an `<iframe>` elsewhere, and sites that refuse framing (`X-Frame-Options`, CSP
//! `frame-ancestors`: Google's sign-in) stay blank there whatever the engine. In this window it is a placeholder
//! instead, and the window draws a web view of its own over it: a top-level page, so framing rules do not apply.
//!
//! An initialization script of the daemon's top-level page ([`script`]) reports every placeholder each time one
//! changes — the part of it that shows, cut by the viewport and every ancestor clipping it, and whether anything of the
//! page lies on top of it — and posts the commands of a widget's `WebView` handle (reload, back, forward). It posts to
//! a channel only that script knows (DMN-182): a script message handler named after the window's nonce on Linux (as
//! the zoom does, UI-283), a navigation of a hidden frame to `omfp-web-views:?n=<nonce>&m=…` on macOS (as the
//! notifications do, UI-252). [`Views`] keeps one view per placeholder through a platform [`Engine`]: `WebKitGTK` views
//! in a `gtk::Overlay` on Linux (`gtk`; Tauri's child webviews are packed into a `GtkBox` there and ignore their
//! position, S-17), Tauri child webviews on macOS (`macos`). Each view tells the page where it went
//! (`__omfpWebViews.navigated`) and when it loaded, which `WebView` hands to the widget.
//!
//! A native view is always on top of the page, so omfp's own UI (the `:` line, dialogs, another panel) would show under
//! it. While anything covers a placeholder the view hides, behind a capture of it in the placeholder where the engine
//! can take one (Linux); uncovered, the view shows again (S-16's freeze).

use std::collections::HashMap;
use std::collections::hash_map::Entry as Slotted;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::Url;

use super::{Navigation, navigation};

#[cfg(target_os = "linux")]
mod gtk;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use gtk::follow;
#[cfg(target_os = "macos")]
pub use macos::{SharedViews, on_navigation, start};

/// A placeholder as the page reports it, in the page's CSS pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    /// The placeholder's id: one view per id.
    pub id: String,
    /// The page it asks for.
    pub url: String,
    /// The part of the placeholder that shows, in the viewport.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Whether the placeholder shows at all (not in a hidden tab, not cut away entirely).
    pub visible: bool,
    /// Whether anything of the page lies on top of it.
    pub covered: bool,
}

/// Where a view goes in the window, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// What a widget's `WebView` handle asks of its page (UI-685).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Reload,
    Back,
    Forward,
}

/// What a widget's `WebView` gives its page beyond the URL (UI-687): scripts run in the page at document start, and
/// whether the page gets `window.omfp.post` to send the widget messages. A change makes a new view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Setup {
    pub scripts: Vec<String>,
    pub channel: bool,
}

/// What the page posts.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Every placeholder, the setups of those whose setup is new or changed, and the viewport's width in CSS pixels
    /// (macOS derives the page zoom from it).
    Slots {
        slots: Vec<Slot>,
        setups: Vec<(String, Setup)>,
        viewport_width: f64,
    },
    /// A command for the view of placeholder `id`.
    Command { id: String, command: Command },
}

/// A message of [`script`]: `{"slots": […], "setups": {id: {"scripts": […], "channel": …}}, "vw": …}` or
/// `{"command": "back", "id": …}`; `None` for anything else. A slot or setup entry of the wrong shape is left out.
#[must_use]
pub fn parse_message(json: &str) -> Option<Message> {
    let message: serde_json::Value = serde_json::from_str(json).ok()?;
    if let Some(command) = message.get("command") {
        let command = match command.as_str()? {
            "reload" => Command::Reload,
            "back" => Command::Back,
            "forward" => Command::Forward,
            _ => return None,
        };
        let id = message.get("id")?.as_str()?.to_owned();
        return Some(Message::Command { id, command });
    }
    let slots = message
        .get("slots")?
        .as_array()?
        .iter()
        .filter_map(|entry| {
            let number = |key: &str| entry.get(key)?.as_f64().filter(|value| value.is_finite());
            let flag = |key: &str| entry.get(key)?.as_bool();
            Some(Slot {
                id: entry.get("id")?.as_str()?.to_owned(),
                url: entry.get("url")?.as_str()?.to_owned(),
                x: number("x")?,
                y: number("y")?,
                width: number("w")?,
                height: number("h")?,
                visible: flag("visible")?,
                covered: flag("covered")?,
            })
        })
        .collect();
    let setups = message
        .get("setups")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(id, setup)| {
            let scripts = setup
                .get("scripts")?
                .as_array()?
                .iter()
                .map(|script| script.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?;
            let channel = setup.get("channel")?.as_bool()?;
            Some((id.clone(), Setup { scripts, channel }))
        })
        .collect();
    let viewport_width = message
        .get("vw")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    Some(Message::Slots {
        slots,
        setups,
        viewport_width,
    })
}

/// The largest message a page sends its widget (UI-687), in bytes of JSON: a bigger one is dropped.
pub const MAX_PAGE_MESSAGE: usize = 1 << 20;

/// The JavaScript that hands the widget of placeholder `id` the message `json` its page posted (UI-687).
#[must_use]
pub fn message_call(id: &str, json: &str) -> String {
    format!(
        "window.__omfpWebViews?.message({}, {})",
        serde_json::Value::from(id),
        serde_json::Value::from(json)
    )
}

/// The script message handler a view's page posts its widget's messages to on Linux (UI-687).
pub const PAGE_HANDLER: &str = "omfp";

/// The scheme of a page's messages to its widget on macOS (UI-687): navigations of hidden frames.
pub const MESSAGE_SCHEME: &str = "omfp-web-view-message";

/// The script giving a view's page `window.omfp.post(data)` (UI-687): `data` as JSON to its widget's `onMessage`,
/// through [`PAGE_HANDLER`] or navigations to [`MESSAGE_SCHEME`]; data without a JSON form throws.
#[must_use]
pub fn channel_script(transport: Transport) -> String {
    let send = match transport {
        Transport::Handler => format!(
            "(json) => window.webkit.messageHandlers[{}].postMessage(json)",
            serde_json::Value::from(PAGE_HANDLER)
        ),
        Transport::Frames => format!(
            "(json) => {{ const frame = document.createElement(\"iframe\"); frame.hidden = true; \
             frame.src = `{MESSAGE_SCHEME}:?${{new URLSearchParams({{ m: json }})}}`; \
             (document.body ?? document.documentElement).append(frame); setTimeout(() => frame.remove(), 1000); }}"
        ),
    };
    format!(
        "(() => {{\n  const send = {send};\n  const post = (data) => {{\n    const json = JSON.stringify(data);\n    \
         if (json === undefined) throw new TypeError(\"omfp.post: the data has no JSON form\");\n    send(json);\n  \
         }};\n  Object.defineProperty(window, \"omfp\", {{ value: Object.freeze({{ post }}) }});\n}})();"
    )
}

/// The message a navigation to [`MESSAGE_SCHEME`] carries (UI-687); `None` for any other URL.
#[cfg(any(target_os = "macos", test))]
#[must_use]
pub fn page_message(url: &Url) -> Option<String> {
    if url.scheme() != MESSAGE_SCHEME {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key == "m")
        .map(|(_, json)| json.into_owned())
}

/// The scheme of the page's messages on macOS (UI-686).
pub const SCHEME: &str = "omfp-web-views";

/// The message of a navigation to [`SCHEME`] signed with the window's `nonce` (DMN-182); `None` for any other URL or
/// a message without the nonce.
#[cfg(any(target_os = "macos", test))]
#[must_use]
pub fn posted(url: &Url, nonce: &str) -> Option<Message> {
    if url.scheme() != SCHEME {
        return None;
    }
    let mut signed = false;
    let mut message = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "n" => signed = value == nonce,
            "m" => message = Some(value.into_owned()),
            _ => {}
        }
    }
    signed.then(|| parse_message(&message?)).flatten()
}

/// Where the view of `slot` goes when the page is zoomed by `zoom` (UI-683): the page's CSS pixels are `zoom` logical
/// pixels each. `None` while the placeholder does not show: the view hides.
#[must_use]
pub fn placement(slot: &Slot, zoom: f64) -> Option<Placement> {
    if !slot.visible || slot.width < 1.0 || slot.height < 1.0 || zoom.is_nan() || zoom <= 0.0 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a window's coordinates are far within i32"
    )]
    let pixel = |value: f64| (value * zoom).round() as i32;
    let (left, top) = (pixel(slot.x), pixel(slot.y));
    Some(Placement {
        x: left,
        y: top,
        width: pixel(slot.x + slot.width) - left,
        height: pixel(slot.y + slot.height) - top,
    })
}

/// Whether a native view shows `url` in the window on the daemon at `origin` (UI-684): a web page of another origin;
/// never the daemon's own pages (the token's), never another scheme.
#[must_use]
pub fn shows(url: &Url, origin: &str) -> bool {
    matches!(url.scheme(), "http" | "https") && navigation(url, origin) == Navigation::External
}

/// How the page's script posts to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// A `WebKit` script message handler (Linux).
    Handler,
    /// Navigations of hidden frames to [`SCHEME`] (macOS: Tauri registers no handler of ours).
    Frames,
}

/// The script message handler of the window of `nonce`: only the window's own script knows it (DMN-182).
fn handler(nonce: &str) -> String {
    format!("omfpWebViews-{nonce}")
}

/// The page's side (UI-682), for the top-level page of the daemon's `origin` only (DMN-182): reports the placeholders
/// of `WebView`s (`[data-omfp-web-view]`) at every animation frame where they changed, and defines
/// `window.__omfpWebViews`, which tells `WebView` to draw a placeholder, carries its handle's commands and lets the
/// window show and drop a view's capture and report where the view went.
#[must_use]
pub fn script(origin: &str, nonce: &str, transport: Transport) -> String {
    let value = |text: &str| serde_json::Value::from(text).to_string();
    format!(
        "(() => {{\n  const ORIGIN = {};\n  const NONCE = {};\n  const HANDLER = {};\n  const SCHEME = {};\n  \
         const FRAMES = {};\n{BRIDGE}",
        value(origin),
        value(nonce),
        value(&handler(nonce)),
        value(SCHEME),
        transport == Transport::Frames,
    )
}

const BRIDGE: &str = r#"  if (window.top !== window || location.origin !== ORIGIN || window.__omfpWebViews) return;
  const SELECTOR = "[data-omfp-web-view]";
  // Points sampled inside a placeholder to find whether another element is on top of it: a 5 × 5 grid, inset by 2 px
  // so that a neighbour's border does not count.
  const GRID = 5;
  const INSET = 2;
  // The frame around a widget: its own controls (a floating panel's bar and resize corner) do not cover it.
  const FRAME = ".omfp-floating-widget, .omfp-floating-file";
  // Not covering either: a splitter's grab area reaches a few pixels over the cells beside it (UI-683).
  const EDGE = ".omfp-splitter";
  let last = "";
  // The setup (`data-setup`: `{ scripts, channel }`) the window last got for each placeholder id (UI-687).
  let known = new Map();

  function send(json) {
    if (!FRAMES) {
      window.webkit?.messageHandlers?.[HANDLER]?.postMessage(json);
      return;
    }
    const frame = document.createElement("iframe");
    frame.hidden = true;
    frame.src = `${SCHEME}:?${new URLSearchParams({ n: NONCE, m: json })}`;
    (document.body ?? document.documentElement).append(frame);
    setTimeout(() => frame.remove(), 1000);
  }

  // The part of `element` that can show: its box cut by the viewport and by every ancestor clipping its overflow.
  function shown(element) {
    const box = element.getBoundingClientRect();
    let left = Math.max(box.left, 0);
    let top = Math.max(box.top, 0);
    let right = Math.min(box.right, window.innerWidth);
    let bottom = Math.min(box.bottom, window.innerHeight);
    for (let parent = element.parentElement; parent !== null; parent = parent.parentElement) {
      const style = getComputedStyle(parent);
      if (style.overflowX === "visible" && style.overflowY === "visible") continue;
      const clip = parent.getBoundingClientRect();
      const clipLeft = clip.left + parent.clientLeft;
      const clipTop = clip.top + parent.clientTop;
      left = Math.max(left, clipLeft);
      top = Math.max(top, clipTop);
      right = Math.min(right, clipLeft + parent.clientWidth);
      bottom = Math.min(bottom, clipTop + parent.clientHeight);
    }
    return { left, top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
  }

  function covered(element, rect) {
    const frame = element.closest(FRAME) ?? element;
    for (let row = 0; row < GRID; row += 1) {
      for (let column = 0; column < GRID; column += 1) {
        const x = rect.left + INSET + ((rect.width - 2 * INSET) * column) / (GRID - 1);
        const y = rect.top + INSET + ((rect.height - 2 * INSET) * row) / (GRID - 1);
        const top = document.elementFromPoint(x, y);
        if (top !== null && !frame.contains(top) && top.closest(EDGE) === null) return true;
      }
    }
    return false;
  }

  function report() {
    const slots = [];
    const setups = {};
    const present = new Set();
    for (const element of document.querySelectorAll(SELECTOR)) {
      const rect = shown(element);
      const visible =
        rect.width > 2 * INSET &&
        rect.height > 2 * INSET &&
        element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
      const id = element.dataset.omfpWebView;
      slots.push({
        id,
        url: element.dataset.url ?? "about:blank",
        x: rect.left,
        y: rect.top,
        w: rect.width,
        h: rect.height,
        visible,
        covered: visible && covered(element, rect),
      });
      present.add(id);
      const setup = element.dataset.setup ?? '{"scripts":[],"channel":false}';
      if (known.get(id) !== setup) {
        try {
          setups[id] = JSON.parse(setup);
          known.set(id, setup);
        } catch {}
      }
    }
    // A placeholder that comes back gets its setup sent again.
    if (known.size > present.size) {
      for (const id of known.keys()) if (!present.has(id)) known.delete(id);
    }
    const json = JSON.stringify({ slots, vw: window.innerWidth });
    if (json !== last || Object.keys(setups).length > 0) {
      last = json;
      send(JSON.stringify({ slots, setups, vw: window.innerWidth }));
    }
    requestAnimationFrame(report);
  }

  function placeholder(id) {
    for (const element of document.querySelectorAll(SELECTOR)) {
      if (element.dataset.omfpWebView === id) return element;
    }
    return null;
  }

  window.__omfpWebViews = {
    // A widget's command for the view of placeholder `id`: "reload", "back" or "forward".
    command(id, command) {
      send(JSON.stringify({ command, id }));
    },
    // The window's report: the view of `id` shows `url` (`canGoBack`, `canGoForward`: booleans, or null unknown).
    navigated(id, url, canGoBack, canGoForward) {
      placeholder(id)?.dispatchEvent(
        new CustomEvent("omfp-web-view-navigate", { detail: { url, canGoBack, canGoForward } }),
      );
    },
    // The window's report: the view of `id` finished loading its page.
    loaded(id) {
      placeholder(id)?.dispatchEvent(new CustomEvent("omfp-web-view-load"));
    },
    // The window's report: the page of `id` posted `json` with `window.omfp.post` (UI-687).
    message(id, json) {
      let data;
      try {
        data = JSON.parse(json);
      } catch {
        return;
      }
      placeholder(id)?.dispatchEvent(new CustomEvent("omfp-web-view-message", { detail: { data } }));
    },
    // Shows the capture `src` (a data: URL) where the view was: the shown part of the placeholder `id`.
    freeze(id, src) {
      const element = placeholder(id);
      if (element === null) return;
      let capture = element.querySelector(":scope > img[data-omfp-web-view-capture]");
      if (capture === null) {
        capture = document.createElement("img");
        capture.dataset.omfpWebViewCapture = "";
        capture.alt = "";
        element.append(capture);
      }
      const box = element.getBoundingClientRect();
      const rect = shown(element);
      // `cover` from the top left: a capture of another size (taken while the view was still being laid out) is cut,
      // never stretched.
      capture.style.cssText = `position:absolute;left:${rect.left - box.left}px;top:${rect.top - box.top}px;width:${rect.width}px;height:${rect.height}px;object-fit:cover;object-position:left top;pointer-events:none`;
      capture.src = src;
    },
    thaw(id) {
      placeholder(id)?.querySelector(":scope > img[data-omfp-web-view-capture]")?.remove();
    },
  };
  requestAnimationFrame(report);
})();"#;

/// The JavaScript that tells the page the view of placeholder `id` shows `url` (UI-685); `None` when unknown.
#[must_use]
pub fn navigated_call(
    id: &str,
    url: &str,
    can_go_back: Option<bool>,
    can_go_forward: Option<bool>,
) -> String {
    format!(
        "window.__omfpWebViews?.navigated({}, {}, {}, {})",
        serde_json::Value::from(id),
        serde_json::Value::from(url),
        serde_json::Value::from(can_go_back),
        serde_json::Value::from(can_go_forward),
    )
}

/// The JavaScript that tells the page the view of placeholder `id` finished loading (UI-685).
#[must_use]
pub fn loaded_call(id: &str) -> String {
    format!(
        "window.__omfpWebViews?.loaded({})",
        serde_json::Value::from(id)
    )
}

/// The JavaScript that drops the capture of placeholder `id`.
#[must_use]
pub fn thaw_call(id: &str) -> String {
    format!(
        "window.__omfpWebViews?.thaw({})",
        serde_json::Value::from(id)
    )
}

/// The native views of one platform.
pub trait Engine {
    type View;
    /// A new view of placeholder `id` loading `url` with `setup` (UI-687), hidden until placed; `None` when none can
    /// be made.
    fn open(&mut self, id: &str, url: &str, setup: &Setup) -> Option<Self::View>;
    /// Loads `url` in `view`.
    fn load(&self, view: &Self::View, url: &str);
    /// The page `view` shows now.
    fn current(&self, view: &Self::View) -> Option<String>;
    fn place(&self, view: &Self::View, at: Placement);
    fn show(&self, view: &Self::View);
    fn hide(&self, view: &Self::View);
    /// Hides `view` behind a capture of it in placeholder `id` where the engine can take one, unless `frozen` was
    /// cleared meanwhile.
    fn freeze(&self, view: &Self::View, id: &str, frozen: &Arc<AtomicBool>);
    /// Drops the capture of placeholder `id`.
    fn thaw(&self, id: &str);
    fn act(&self, view: &Self::View, command: Command);
    fn close(&self, view: Self::View);
}

/// One placeholder's view.
#[derive(Debug)]
struct Native<V> {
    view: V,
    /// The page the placeholder asked for last.
    url: String,
    /// The setup the view was made with.
    setup: Setup,
    /// Whether the view is hidden behind its capture (or about to be, its capture still coming).
    frozen: Arc<AtomicBool>,
}

/// The window's native views, one per placeholder the page reports.
pub struct Views<E: Engine> {
    engine: E,
    /// The daemon's origin: its own pages never show in a view (UI-684).
    origin: String,
    natives: HashMap<String, Native<E::View>>,
    /// Each placeholder's setup, as the page last sent it (UI-687).
    setups: HashMap<String, Setup>,
}

impl<E: Engine> std::fmt::Debug for Views<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Views")
            .field("origin", &self.origin)
            .field("placeholders", &self.natives.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl<E: Engine> Views<E> {
    #[must_use]
    pub fn new(engine: E, origin: String) -> Self {
        Self {
            engine,
            origin,
            natives: HashMap::new(),
            setups: HashMap::new(),
        }
    }

    #[cfg(any(target_os = "macos", test))]
    #[must_use]
    pub fn engine(&self) -> &E {
        &self.engine
    }

    /// Brings the views in line with the page's placeholders `slots` and their new `setups` at the page zoom `zoom`
    /// (UI-683): a view per placeholder of a page [`shows`] lets through, closed with its placeholder and made anew
    /// when its setup changes (UI-687); placed over the part that shows, hidden while none does, frozen while covered.
    /// A placeholder asking for the page its view already went to (a widget following `onNavigate`) loads nothing.
    pub fn sync(&mut self, slots: &[Slot], setups: Vec<(String, Setup)>, zoom: f64) {
        self.setups.extend(setups);
        self.setups
            .retain(|id, _| slots.iter().any(|slot| &slot.id == id));
        let gone: Vec<String> = self
            .natives
            .iter()
            .filter(|(id, native)| {
                self.setups
                    .get(*id)
                    .is_some_and(|setup| *setup != native.setup)
                    || !slots.iter().any(|slot| {
                        &slot.id == *id
                            && Url::parse(&slot.url).is_ok_and(|url| shows(&url, &self.origin))
                    })
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            if let Some(native) = self.natives.remove(&id) {
                if native.frozen.swap(false, Ordering::SeqCst) {
                    self.engine.thaw(&id);
                }
                self.engine.close(native.view);
            }
        }
        for slot in slots {
            if !Url::parse(&slot.url).is_ok_and(|url| shows(&url, &self.origin)) {
                continue;
            }
            let native = match self.natives.entry(slot.id.clone()) {
                Slotted::Occupied(entry) => entry.into_mut(),
                Slotted::Vacant(entry) => {
                    let setup = self.setups.get(&slot.id).cloned().unwrap_or_default();
                    let Some(view) = self.engine.open(&slot.id, &slot.url, &setup) else {
                        continue;
                    };
                    entry.insert(Native {
                        view,
                        url: slot.url.clone(),
                        setup,
                        frozen: Arc::default(),
                    })
                }
            };
            if native.url != slot.url {
                if self.engine.current(&native.view).as_deref() != Some(slot.url.as_str()) {
                    self.engine.load(&native.view, &slot.url);
                }
                native.url.clone_from(&slot.url);
            }
            let Some(place) = placement(slot, zoom) else {
                self.engine.hide(&native.view);
                if native.frozen.swap(false, Ordering::SeqCst) {
                    self.engine.thaw(&slot.id);
                }
                continue;
            };
            self.engine.place(&native.view, place);
            if slot.covered {
                if !native.frozen.swap(true, Ordering::SeqCst) {
                    self.engine.freeze(&native.view, &slot.id, &native.frozen);
                }
            } else {
                self.engine.show(&native.view);
                if native.frozen.swap(false, Ordering::SeqCst) {
                    self.engine.thaw(&slot.id);
                }
            }
        }
    }

    /// Runs a widget's `command` on the view of placeholder `id` (UI-685); nothing for an unknown placeholder.
    pub fn command(&self, id: &str, command: Command) {
        if let Some(native) = self.natives.get(id) {
            self.engine.act(&native.view, command);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn slot(x: f64, y: f64, width: f64, height: f64) -> Slot {
        Slot {
            id: "a".to_owned(),
            url: "https://example.com/".to_owned(),
            x,
            y,
            width,
            height,
            visible: true,
            covered: false,
        }
    }

    /// An engine that records what the views ask of it; a view is its number and its page.
    #[derive(Debug, Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        opened: usize,
        /// The page every view went to by itself (a link followed), by view number.
        current: RefCell<HashMap<usize, String>>,
    }

    impl Fake {
        fn take(&self) -> Vec<String> {
            self.calls.take()
        }
        fn log(&self, call: String) {
            self.calls.borrow_mut().push(call);
        }
    }

    impl Engine for Fake {
        type View = usize;
        fn open(&mut self, id: &str, url: &str, setup: &Setup) -> Option<usize> {
            self.opened += 1;
            self.log(format!(
                "open {} {id} {url} {:?} {}",
                self.opened, setup.scripts, setup.channel
            ));
            Some(self.opened)
        }
        fn load(&self, view: &usize, url: &str) {
            self.log(format!("load {view} {url}"));
        }
        fn current(&self, view: &usize) -> Option<String> {
            self.current.borrow().get(view).cloned()
        }
        fn place(&self, view: &usize, at: Placement) {
            self.log(format!(
                "place {view} {},{} {}x{}",
                at.x, at.y, at.width, at.height
            ));
        }
        fn show(&self, view: &usize) {
            self.log(format!("show {view}"));
        }
        fn hide(&self, view: &usize) {
            self.log(format!("hide {view}"));
        }
        fn freeze(&self, view: &usize, id: &str, _: &Arc<AtomicBool>) {
            self.log(format!("freeze {view} {id}"));
        }
        fn thaw(&self, id: &str) {
            self.log(format!("thaw {id}"));
        }
        fn act(&self, view: &usize, command: Command) {
            self.log(format!("act {view} {command:?}"));
        }
        fn close(&self, view: usize) {
            self.log(format!("close {view}"));
        }
    }

    fn views() -> Views<Fake> {
        Views::new(Fake::default(), "http://127.0.0.1:7420".to_owned())
    }

    // spec: UI-683
    #[test]
    fn a_view_covers_the_shown_part_of_its_placeholder_at_the_page_zoom() {
        assert_eq!(
            placement(&slot(320.0, 80.0, 900.0, 740.0), 1.0),
            Some(Placement {
                x: 320,
                y: 80,
                width: 900,
                height: 740
            })
        );
        // Zoomed to 125 %: a CSS pixel is 1.25 logical pixels, and the edges round alike so neighbours never overlap.
        assert_eq!(
            placement(&slot(10.4, 20.2, 100.3, 50.5), 1.25),
            Some(Placement {
                x: 13,
                y: 25,
                width: 125,
                height: 63
            })
        );
    }

    // spec: UI-683
    #[test]
    fn a_view_hides_while_its_placeholder_does_not_show() {
        let hidden = Slot {
            visible: false,
            ..slot(0.0, 0.0, 400.0, 300.0)
        };
        assert_eq!(placement(&hidden, 1.0), None);
        assert_eq!(placement(&slot(0.0, 0.0, 0.0, 300.0), 1.0), None);
        assert_eq!(placement(&slot(0.0, 0.0, 400.0, 300.0), 0.0), None);
    }

    // spec: UI-683, UI-685, UI-687
    #[test]
    fn the_page_posts_its_placeholders_setups_and_commands_as_json() {
        let report = r#"{"slots":[
            {"id":"r1","url":"https://accounts.google.com/","x":320,"y":80.5,"w":900,"h":740,"visible":true,"covered":true},
            {"id":"broken","url":"https://example.com/"},
            "nothing"
        ],"setups":{"r1":{"scripts":["window.omfp.post(1)"],"channel":true},"bad":{"scripts":[3],"channel":true}},
        "vw":1280}"#;
        assert_eq!(
            parse_message(report),
            Some(Message::Slots {
                slots: vec![Slot {
                    id: "r1".to_owned(),
                    url: "https://accounts.google.com/".to_owned(),
                    x: 320.0,
                    y: 80.5,
                    width: 900.0,
                    height: 740.0,
                    visible: true,
                    covered: true,
                }],
                setups: vec![(
                    "r1".to_owned(),
                    Setup {
                        scripts: vec!["window.omfp.post(1)".to_owned()],
                        channel: true
                    }
                )],
                viewport_width: 1280.0,
            })
        );
        assert_eq!(
            parse_message(r#"{"command":"back","id":"r1"}"#),
            Some(Message::Command {
                id: "r1".to_owned(),
                command: Command::Back
            })
        );
        assert_eq!(parse_message(r#"{"command":"print","id":"r1"}"#), None);
        assert_eq!(parse_message("not json"), None);
    }

    // spec: UI-686
    #[test]
    fn on_macos_the_page_posts_through_frames_signed_with_the_windows_nonce() {
        let message = r#"{"command":"reload","id":"r1"}"#;
        let url = |nonce: &str| {
            Url::parse_with_params(&format!("{SCHEME}:"), [("n", nonce), ("m", message)])
                .expect("a URL")
        };
        assert_eq!(
            posted(&url("secret"), "secret"),
            Some(Message::Command {
                id: "r1".to_owned(),
                command: Command::Reload
            })
        );
        assert_eq!(posted(&url("guess"), "secret"), None);
        assert_eq!(
            posted(
                &Url::parse("https://example.com/?n=secret").expect("a URL"),
                "secret"
            ),
            None
        );
    }

    // spec: UI-684
    #[test]
    fn a_view_shows_only_web_pages_off_the_daemon() {
        let origin = "http://127.0.0.1:7420";
        let shown = |url: &str| shows(&Url::parse(url).expect("a URL"), origin);
        assert!(shown("https://accounts.google.com/"));
        assert!(shown("http://localhost:5173/"));
        assert!(!shown("http://127.0.0.1:7420/#token=secret"));
        assert!(!shown("file:///etc/passwd"));
        assert!(!shown("mailto:someone@example.com"));
        assert!(!shown("about:blank"));
    }

    // spec: UI-682, UI-683
    #[test]
    fn a_placeholder_gets_one_view_placed_frozen_while_covered_and_closed_with_it() {
        let mut views = views();
        let mut a = slot(10.0, 20.0, 300.0, 200.0);
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(
            views.engine().take(),
            [
                "open 1 a https://example.com/ [] false",
                "place 1 10,20 300x200",
                "show 1"
            ]
        );
        a.covered = true;
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(
            views.engine().take(),
            [
                "place 1 10,20 300x200",
                "freeze 1 a",
                "place 1 10,20 300x200"
            ]
        );
        a.covered = false;
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(
            views.engine().take(),
            ["place 1 10,20 300x200", "show 1", "thaw a"]
        );
        a.visible = false;
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(views.engine().take(), ["hide 1"]);
        views.sync(&[], Vec::new(), 1.0);
        assert_eq!(views.engine().take(), ["close 1"]);
    }

    // spec: UI-687
    #[test]
    fn a_view_opens_with_its_setup_and_is_made_anew_when_the_setup_changes() {
        let mut views = views();
        let a = slot(0.0, 0.0, 100.0, 100.0);
        let setup = |script: &str| {
            vec![(
                "a".to_owned(),
                Setup {
                    scripts: vec![script.to_owned()],
                    channel: true,
                },
            )]
        };
        views.sync(std::slice::from_ref(&a), setup("one()"), 1.0);
        assert_eq!(
            views.engine().take()[0],
            r#"open 1 a https://example.com/ ["one()"] true"#
        );
        // The page sends a setup only when it changed: nothing new keeps the view.
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(views.engine().take(), ["place 1 0,0 100x100", "show 1"]);
        views.sync(std::slice::from_ref(&a), setup("two()"), 1.0);
        assert_eq!(
            views.engine().take()[..2],
            ["close 1", r#"open 2 a https://example.com/ ["two()"] true"#]
        );
        // Gone and back: the page sends its setup again, and the old one is forgotten meanwhile.
        views.sync(&[], Vec::new(), 1.0);
        views.engine().take();
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(
            views.engine().take()[0],
            "open 3 a https://example.com/ [] false"
        );
    }

    // spec: UI-684, UI-685
    #[test]
    fn a_new_url_loads_unless_the_view_already_went_there_and_the_daemons_pages_never_show() {
        let mut views = views();
        let mut a = slot(0.0, 0.0, 100.0, 100.0);
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        views.engine().take();
        a.url = "https://example.com/next".to_owned();
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(views.engine().take()[0], "load 1 https://example.com/next");
        // The page followed a link, and the widget passes the address it heard back as its url.
        views
            .engine()
            .current
            .borrow_mut()
            .insert(1, "https://example.com/followed".to_owned());
        a.url = "https://example.com/followed".to_owned();
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(views.engine().take(), ["place 1 0,0 100x100", "show 1"]);
        a.url = "http://127.0.0.1:7420/#token=secret".to_owned();
        views.sync(std::slice::from_ref(&a), Vec::new(), 1.0);
        assert_eq!(views.engine().take(), ["close 1"]);
    }

    // spec: UI-685
    #[test]
    fn a_widgets_command_reaches_its_view() {
        let mut views = views();
        views.sync(&[slot(0.0, 0.0, 100.0, 100.0)], Vec::new(), 1.0);
        views.engine().take();
        views.command("a", Command::Back);
        views.command("unknown", Command::Reload);
        assert_eq!(views.engine().take(), ["act 1 Back"]);
    }

    // spec: UI-687
    #[test]
    fn on_macos_a_pages_message_is_the_json_of_its_frame_navigation() {
        let json = r#"{"text":"a & b = c?"}"#;
        let url =
            Url::parse_with_params(&format!("{MESSAGE_SCHEME}:"), [("m", json)]).expect("a URL");
        assert_eq!(page_message(&url).as_deref(), Some(json));
        let other = Url::parse_with_params("https://example.com/", [("m", json)]).expect("a URL");
        assert_eq!(page_message(&other), None);
    }
}
