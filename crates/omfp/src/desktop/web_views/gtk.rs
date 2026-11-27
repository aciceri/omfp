//! The Linux engine of the native web views (UI-682..UI-685, UI-687): `WebKitGTK` views in a `gtk::Overlay` holding
//! the window's page. The page posts through a script message handler (UI-283's channel); each view reports where it
//! went as its `uri` changes and when it finished loading, and its capture (a snapshot of its visible part) stands in
//! while omfp's UI covers it. Each view has a user content manager of its own: its widget's scripts, and the channel
//! its page posts its widget messages through (a script message handler of that view only).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::glib;
use gtk::prelude::*;
use tauri::{Url, WebviewWindow};
use webkit2gtk::{
    CookieManagerExt as _, CookiePersistentStorage, LoadEvent, NavigationPolicyDecision,
    NavigationPolicyDecisionExt, PolicyDecisionExt as _, PolicyDecisionType, SnapshotOptions,
    SnapshotRegion, URIRequestExt as _, UserContentInjectedFrames, UserContentManager,
    UserContentManagerExt as _, UserScript, UserScriptInjectionTime, WebContext, WebView,
    WebViewExt as _, WebsiteDataManager, WebsiteDataManagerExt as _,
};

use super::{
    Command, Engine, MAX_PAGE_MESSAGE, Message, PAGE_HANDLER, Placement, Setup, Transport, Views,
    channel_script, handler, loaded_call, message_call, navigated_call, parse_message, shows,
    thaw_call, view_loads,
};

/// The views of the window: GTK widgets over its page.
struct Gtk {
    overlay: gtk::Overlay,
    context: WebContext,
    /// The window, to tell its page where a view went.
    window: WebviewWindow,
    /// The daemon's origin: a view never loads its pages (UI-684).
    origin: String,
}

/// Puts the window's page into a `gtk::Overlay` and draws a native view over every placeholder the page's [`script`]
/// with `nonce` reports. The views keep their web data (cookies: a sign-in) in `data`, apart from the daemon page's
/// storage. Failures are reported and leave `WebView`s blank.
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
            eprintln!("omfp desktop: no web views, the window's web view is not in a box");
            return;
        };
        let Some(manager) = page.user_content_manager() else {
            eprintln!(
                "omfp desktop: no web views, the window's web view has no user content manager"
            );
            return;
        };
        if !manager.register_script_message_handler(&name) {
            eprintln!("omfp desktop: no web views, cannot register their message handler");
            return;
        }
        let overlay = gtk::Overlay::new();
        parent.remove(&page);
        overlay.add(&page);
        parent.pack_start(&overlay, true, true, 0);
        overlay.show_all();
        let engine = Gtk {
            overlay,
            context: web_context(&data),
            window: owner,
            origin: origin.clone(),
        };
        let views = std::cell::RefCell::new(Views::new(engine, origin));
        // The view keeps its manager, which keeps this handler: a weak reference, no cycle.
        let page = page.downgrade();
        manager.connect_script_message_received(Some(&name), move |_, message| {
            let (Some(page), Some(json)) = (page.upgrade(), message.js_value()) else {
                return;
            };
            match parse_message(&json.to_string()) {
                Some(Message::Slots { slots, setups, .. }) => {
                    views.borrow_mut().sync(&slots, setups, page.zoom_level());
                }
                Some(Message::Command { id, command }) => views.borrow().command(&id, command),
                None => {}
            }
        });
    });
    if let Err(error) = followed {
        eprintln!("omfp desktop: no web views: {error}");
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

/// Runs `js` in the window's page.
fn tell(window: &WebviewWindow, js: String) {
    if let Err(error) = window.eval(js) {
        eprintln!("omfp desktop: cannot tell the page about a web view: {error}");
    }
}

/// Tells the page where the view of placeholder `id` is now.
fn report(window: &WebviewWindow, id: &str, view: &WebView) {
    if let Some(uri) = view.uri() {
        tell(
            window,
            navigated_call(
                id,
                &uri,
                Some(view.can_go_back()),
                Some(view.can_go_forward()),
            ),
        );
    }
}

impl Engine for Gtk {
    type View = WebView;

    /// A link to a new window opens in the view itself (UI-684); the view gets no permission (`WebKitGTK` denies the
    /// requests nobody handles), and only its widget's scripts (UI-687).
    fn open(&mut self, id: &str, url: &str, setup: &Setup) -> Option<WebView> {
        let manager = UserContentManager::new();
        let script = |source: &str| {
            UserScript::new(
                source,
                UserContentInjectedFrames::TopFrame,
                UserScriptInjectionTime::Start,
                &[],
                &[],
            )
        };
        if setup.channel {
            if manager.register_script_message_handler(PAGE_HANDLER) {
                manager.add_script(&script(&channel_script(Transport::Handler)));
                let (window, placeholder) = (self.window.clone(), id.to_owned());
                manager.connect_script_message_received(Some(PAGE_HANDLER), move |_, message| {
                    let Some(json) = message.js_value().map(|value| value.to_string()) else {
                        return;
                    };
                    if json.len() <= MAX_PAGE_MESSAGE {
                        tell(&window, message_call(&placeholder, &json));
                    }
                });
            } else {
                eprintln!(
                    "omfp desktop: a web view gets no channel, cannot register its message handler"
                );
            }
        }
        for source in &setup.scripts {
            manager.add_script(&script(source));
        }
        let view = WebView::builder()
            .web_context(&self.context)
            .user_content_manager(&manager)
            .build();
        view.set_halign(gtk::Align::Start);
        view.set_valign(gtk::Align::Start);
        // Every load of the page or its frames, a redirect or a link included, keeps to what a view may show.
        let origin = self.origin.clone();
        view.connect_decide_policy(move |_, decision, kind| {
            if !matches!(
                kind,
                PolicyDecisionType::NavigationAction | PolicyDecisionType::NewWindowAction
            ) {
                return false;
            }
            let target = decision
                .downcast_ref::<NavigationPolicyDecision>()
                .and_then(NavigationPolicyDecisionExt::navigation_action)
                .and_then(|action| action.request())
                .and_then(|request| request.uri());
            if target.is_some_and(|uri| Url::parse(&uri).is_ok_and(|url| view_loads(&url, &origin)))
            {
                return false;
            }
            decision.ignore();
            true
        });
        let origin = self.origin.clone();
        view.connect_create(move |view, action| {
            let target = action.clone().request().and_then(|request| request.uri());
            if let Some(target) =
                target.filter(|target| Url::parse(target).is_ok_and(|url| shows(&url, &origin)))
            {
                view.load_uri(&target);
            }
            None
        });
        let (window, placeholder) = (self.window.clone(), id.to_owned());
        view.connect_uri_notify(move |view| report(&window, &placeholder, view));
        let (window, placeholder) = (self.window.clone(), id.to_owned());
        view.connect_load_changed(move |view, event| {
            if event == LoadEvent::Finished {
                report(&window, &placeholder, view);
                tell(&window, loaded_call(&placeholder));
            }
        });
        self.overlay.add_overlay(&view);
        view.load_uri(url);
        Some(view)
    }

    fn load(&self, view: &WebView, url: &str) {
        view.load_uri(url);
    }

    fn current(&self, view: &WebView) -> Option<String> {
        view.uri().map(String::from)
    }

    fn place(&self, view: &WebView, at: Placement) {
        view.set_margin_start(at.x);
        view.set_margin_top(at.y);
        view.set_size_request(at.width, at.height);
    }

    fn show(&self, view: &WebView) {
        view.show();
    }

    fn hide(&self, view: &WebView) {
        view.hide();
    }

    /// Without a capture the view hides all the same: omfp's UI must stay on top.
    fn freeze(&self, view: &WebView, id: &str, frozen: &Arc<AtomicBool>) {
        let (shown, frozen, window, id) = (
            view.clone(),
            Arc::clone(frozen),
            self.window.clone(),
            id.to_owned(),
        );
        view.snapshot(
            SnapshotRegion::Visible,
            SnapshotOptions::NONE,
            None::<&gtk::gio::Cancellable>,
            move |surface| {
                if !frozen.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(src) = surface.ok().and_then(|surface| capture_url(&surface)) {
                    tell(
                        &window,
                        format!(
                            "window.__omfpWebViews?.freeze({}, {})",
                            serde_json::Value::from(id),
                            serde_json::Value::from(src)
                        ),
                    );
                }
                shown.hide();
            },
        );
    }

    fn thaw(&self, id: &str) {
        tell(&self.window, thaw_call(id));
    }

    fn act(&self, view: &WebView, command: Command) {
        match command {
            Command::Reload => view.reload(),
            Command::Back => view.go_back(),
            Command::Forward => view.go_forward(),
        }
    }

    fn close(&self, view: WebView) {
        self.overlay.remove(&view);
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
