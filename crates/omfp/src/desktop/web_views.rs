//! Web pages as native views over the Linux desktop window's page (UI-682..UI-684; spikes S-16 and S-17). A web frame
//! (`WebFrame`, UI-670) is an `<iframe>`, and sites that refuse framing (`X-Frame-Options`, CSP `frame-ancestors`:
//! Google's sign-in) stay blank there whatever the engine. In this window the frame is a placeholder instead, and the
//! window draws a `WebKit` view of its own over it: a top-level page, so framing rules do not apply.
//!
//! The page and the window talk as the zoom does (UI-283): an initialization script of the daemon's top-level page
//! ([`script`]) reports every placeholder each time one changes — the part of it that shows, cut by the viewport and
//! every ancestor clipping it, and whether anything of the page lies on top of it — to a script message handler whose
//! name only that script knows (DMN-182). The window moves the main web view from Tauri's `GtkBox` into a
//! `gtk::Overlay` and keeps one overlay child per placeholder ([`follow`]): Tauri's own child webviews would not do,
//! on Linux they are packed into that box and ignore their position (S-17).
//!
//! A native view is always on top of the page, so omfp's own UI (the `:` line, dialogs, another panel) would show under
//! it. While anything covers a placeholder the window captures its view, the page shows the capture in the
//! placeholder and the view hides; uncovered, the view shows again (S-16's freeze).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use tauri::{Url, WebviewWindow};
use webkit2gtk::{
    CookieManagerExt as _, CookiePersistentStorage, SnapshotOptions, SnapshotRegion,
    URIRequestExt as _, UserContentManagerExt as _, WebContext, WebView, WebViewExt as _,
    WebsiteDataManager, WebsiteDataManagerExt as _,
};

use super::{Navigation, navigation};

/// A placeholder as the page reports it, in the page's CSS pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    /// The placeholder's id: one view per id, a new id (a reload) is a new view.
    pub id: String,
    /// The page it shows.
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

/// Where a view goes in the window, in GTK's logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The placeholders of a report of [`script`]: a JSON array; an entry that is not a placeholder is left out.
#[must_use]
pub fn parse_slots(json: &str) -> Vec<Slot> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(json) else {
        return Vec::new();
    };
    entries
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
        .collect()
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

/// The script message handler of the window of `nonce`: only the window's own script knows it (DMN-182).
fn handler(nonce: &str) -> String {
    format!("omfpWebViews-{nonce}")
}

/// The page's side (UI-682), for the top-level page of the daemon's `origin` only (DMN-182): reports the placeholders
/// of web frames (`[data-omfp-web-view]`) at every animation frame where they changed, and defines
/// `window.__omfpWebViews`, which tells `WebFrame` to draw a placeholder and lets the window show and drop a view's
/// capture.
#[must_use]
pub fn script(origin: &str, nonce: &str) -> String {
    format!(
        "(() => {{\n  const ORIGIN = {};\n  const HANDLER = {};\n{BRIDGE}",
        serde_json::Value::from(origin),
        serde_json::Value::from(handler(nonce)),
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
    for (const element of document.querySelectorAll(SELECTOR)) {
      const rect = shown(element);
      const visible =
        rect.width > 2 * INSET &&
        rect.height > 2 * INSET &&
        element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
      slots.push({
        id: element.dataset.omfpWebView,
        url: element.dataset.url ?? "about:blank",
        x: rect.left,
        y: rect.top,
        w: rect.width,
        h: rect.height,
        visible,
        covered: visible && covered(element, rect),
      });
    }
    const json = JSON.stringify(slots);
    if (json !== last) {
      last = json;
      window.webkit?.messageHandlers?.[HANDLER]?.postMessage(json);
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

/// One placeholder's view.
struct Native {
    view: WebView,
    url: String,
    /// Whether the page shows the view's capture and the view is hidden (or about to be, its capture still coming).
    frozen: Rc<Cell<bool>>,
}

/// Puts the window's page into a `gtk::Overlay` and draws a native view over every placeholder [`script`] with `nonce`
/// reports, for pages [`shows`] lets through. The views keep their web data (cookies: a sign-in) in `data`, apart from
/// the daemon page's storage.
pub fn follow(window: &WebviewWindow, origin: String, nonce: &str, data: &Path) {
    let name = handler(nonce);
    let data = data.to_path_buf();
    let owner = window.clone();
    let followed = window.with_webview(move |webview| {
        let page = webview.inner();
        let Some(parent) = page
            .parent()
            .and_then(|parent| parent.downcast::<gtk::Box>().ok())
        else {
            eprintln!("omfp desktop: no web pages in web frames, the web view is not in a box");
            return;
        };
        let Some(manager) = page.user_content_manager() else {
            eprintln!(
                "omfp desktop: no web pages in web frames, the web view has no user content manager"
            );
            return;
        };
        if !manager.register_script_message_handler(&name) {
            eprintln!(
                "omfp desktop: no web pages in web frames, cannot register their message handler"
            );
            return;
        }
        let overlay = gtk::Overlay::new();
        parent.remove(&page);
        overlay.add(&page);
        parent.pack_start(&overlay, true, true, 0);
        overlay.show_all();
        let context = web_context(&data);
        let views = RefCell::new(HashMap::<String, Native>::new());
        // The view keeps its manager, which keeps this handler: a weak reference, no cycle.
        let page = page.downgrade();
        manager.connect_script_message_received(Some(&name), move |_, message| {
            let (Some(page), Some(json)) = (page.upgrade(), message.js_value()) else {
                return;
            };
            let slots = parse_slots(&json.to_string());
            sync(
                &overlay,
                &context,
                &mut views.borrow_mut(),
                &owner,
                &origin,
                &slots,
                page.zoom_level(),
            );
        });
    });
    if let Err(error) = followed {
        eprintln!("omfp desktop: no web pages in web frames: {error}");
    }
}

/// The web context of the views: web data and cookies kept in `data`.
fn web_context(data: &Path) -> WebContext {
    let directory = data.to_string_lossy();
    let manager = WebsiteDataManager::builder()
        .base_cache_directory(directory.as_ref())
        .base_data_directory(directory.as_ref())
        .build();
    if let Some(cookies) = manager.cookie_manager() {
        cookies.set_persistent_storage(
            &data.join("cookies").to_string_lossy(),
            CookiePersistentStorage::Text,
        );
    }
    WebContext::builder().website_data_manager(&manager).build()
}

/// Brings the views in line with the page's placeholders `slots`.
fn sync(
    overlay: &gtk::Overlay,
    context: &WebContext,
    views: &mut HashMap<String, Native>,
    window: &WebviewWindow,
    origin: &str,
    slots: &[Slot],
    zoom: f64,
) {
    views.retain(|id, native| {
        let kept = slots.iter().any(|slot| &slot.id == id);
        if !kept {
            overlay.remove(&native.view);
        }
        kept
    });
    for slot in slots {
        if !Url::parse(&slot.url).is_ok_and(|url| shows(&url, origin)) {
            continue;
        }
        let native = views
            .entry(slot.id.clone())
            .or_insert_with(|| open(overlay, context, &slot.url));
        if native.url != slot.url {
            native.view.load_uri(&slot.url);
            native.url.clone_from(&slot.url);
        }
        let Some(place) = placement(slot, zoom) else {
            native.view.hide();
            if native.frozen.replace(false) {
                thaw(window, &slot.id);
            }
            continue;
        };
        native.view.set_margin_start(place.x);
        native.view.set_margin_top(place.y);
        native.view.set_size_request(place.width, place.height);
        if slot.covered {
            if !native.frozen.replace(true) {
                freeze(native, window, &slot.id);
            }
        } else {
            native.view.show();
            if native.frozen.replace(false) {
                thaw(window, &slot.id);
            }
        }
    }
}

/// A new view on `url` over the page: a link to a new window opens in the view itself (UI-684); it gets no script
/// and no permission (`WebKitGTK` denies the requests nobody handles).
fn open(overlay: &gtk::Overlay, context: &WebContext, url: &str) -> Native {
    let view = WebView::with_context(context);
    view.set_halign(gtk::Align::Start);
    view.set_valign(gtk::Align::Start);
    view.connect_create(|view, action| {
        let target = action.clone().request().and_then(|request| request.uri());
        if let Some(target) = target.filter(|target| {
            Url::parse(target).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
        }) {
            view.load_uri(&target);
        }
        None
    });
    overlay.add_overlay(&view);
    view.load_uri(url);
    Native {
        view,
        url: url.to_owned(),
        frozen: Rc::new(Cell::new(false)),
    }
}

/// Captures the view of placeholder `id`, has the page show the capture and hides the view, unless the placeholder
/// was uncovered meanwhile. Without a capture the view hides all the same: omfp's UI must stay on top.
fn freeze(native: &Native, window: &WebviewWindow, id: &str) {
    let (view, frozen, window, id) = (
        native.view.clone(),
        Rc::clone(&native.frozen),
        window.clone(),
        id.to_owned(),
    );
    native.view.snapshot(
        SnapshotRegion::Visible,
        SnapshotOptions::NONE,
        None::<&gtk::gio::Cancellable>,
        move |surface| {
            if !frozen.get() {
                return;
            }
            if let Some(src) = surface.ok().and_then(|surface| capture_url(&surface)) {
                let call = format!(
                    "window.__omfpWebViews?.freeze({}, {})",
                    serde_json::Value::from(id),
                    serde_json::Value::from(src)
                );
                if let Err(error) = window.eval(call) {
                    eprintln!("omfp desktop: cannot show a web page's capture: {error}");
                }
            }
            view.hide();
        },
    );
}

/// Has the page drop the capture of placeholder `id`.
fn thaw(window: &WebviewWindow, id: &str) {
    if let Err(error) = window.eval(format!(
        "window.__omfpWebViews?.thaw({})",
        serde_json::Value::from(id)
    )) {
        eprintln!("omfp desktop: cannot drop a web page's capture: {error}");
    }
}

/// A capture as a `data:` URL of a PNG image.
fn capture_url(surface: &gtk::cairo::Surface) -> Option<String> {
    let image = gtk::cairo::ImageSurface::try_from(surface.clone()).ok()?;
    let pixbuf = gtk::gdk::pixbuf_get_from_surface(&image, 0, 0, image.width(), image.height())?;
    let png = pixbuf.save_to_bufferv("png", &[]).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        glib::base64_encode(&png)
    ))
}

#[cfg(test)]
mod tests {
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

    // spec: UI-683
    #[test]
    fn the_page_reports_its_placeholders_as_json() {
        let report = r#"[
            {"id":"r1","url":"https://accounts.google.com/","x":320,"y":80.5,"w":900,"h":740,"visible":true,"covered":true},
            {"id":"broken","url":"https://example.com/"},
            "nothing"
        ]"#;
        assert_eq!(
            parse_slots(report),
            vec![Slot {
                id: "r1".to_owned(),
                url: "https://accounts.google.com/".to_owned(),
                x: 320.0,
                y: 80.5,
                width: 900.0,
                height: 740.0,
                visible: true,
                covered: true,
            }]
        );
        assert_eq!(parse_slots("not json"), Vec::new());
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
}
