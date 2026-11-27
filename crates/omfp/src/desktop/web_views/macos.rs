//! The macOS engine of the native web views (UI-686, UI-687): Tauri child webviews (`WKWebView`s) over the window's
//! page. Unlike Linux, Tauri places a child webview where it is asked to on macOS, so no container is needed. The page
//! posts through navigations of hidden frames to [`SCHEME`], which the window's navigation handler hands to
//! [`on_navigation`]; a view's page posts its widget's messages the same way, to [`MESSAGE_SCHEME`], caught by the
//! view's own navigation handler. Without `unsafe` code there is no way to the `WKWebView` behind a Tauri webview, so
//! a covered view hides without a capture, whether the page can go back or forward is unknown, and back and forward
//! run the page's own `history`.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{LogicalPosition, LogicalSize, Manager as _, Url, Webview, WebviewUrl, WebviewWindow};

use super::{
    Command, Engine, MAX_PAGE_MESSAGE, MESSAGE_SCHEME, Message, Placement, SCHEME, Setup,
    Transport, Views, channel_script, loaded_call, message_call, navigated_call, page_message,
    posted,
};

/// The views of the window once it exists, shared with its navigation handler, which runs before the window is built.
pub type SharedViews = Arc<Mutex<Option<Views<Children>>>>;

/// The views of the window: child webviews of its `main` window.
pub struct Children {
    window: WebviewWindow,
    /// The data store of every view (cookies: a sign-in), apart from the daemon page's.
    store: [u8; 16],
    /// The number of the last view made: a child webview's label must be unique.
    made: u64,
}

impl std::fmt::Debug for Children {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Children")
            .field("made", &self.made)
            .finish_non_exhaustive()
    }
}

/// Starts drawing views over `window`'s page, its pages on the daemon at `origin`, their web data in `store`.
pub fn start(views: &SharedViews, window: &WebviewWindow, origin: String, store: [u8; 16]) {
    let engine = Children {
        window: window.clone(),
        store,
        made: 0,
    };
    *views.lock().unwrap_or_else(PoisonError::into_inner) = Some(Views::new(engine, origin));
}

/// The window's handling of a navigation to `url`: a message of its page's script signed with `nonce` goes to the
/// views, and every navigation to [`SCHEME`] is cancelled (`Some(false)`); `None` for any other URL.
pub fn on_navigation(views: &SharedViews, url: &Url, nonce: &str) -> Option<bool> {
    if url.scheme() != SCHEME {
        return None;
    }
    let message = posted(url, nonce);
    let mut views = views.lock().unwrap_or_else(PoisonError::into_inner);
    match (message, views.as_mut()) {
        (
            Some(Message::Slots {
                slots,
                setups,
                viewport_width,
            }),
            Some(views),
        ) => {
            let zoom = views.engine().zoom(viewport_width);
            views.sync(&slots, setups, zoom);
        }
        (Some(Message::Command { id, command }), Some(views)) => views.command(&id, command),
        _ => {}
    }
    Some(false)
}

impl Children {
    /// The page zoom: the window's page is as wide as the window, `viewport_width` CSS pixels.
    fn zoom(&self, viewport_width: f64) -> f64 {
        let width = self
            .window
            .inner_size()
            .ok()
            .zip(self.window.scale_factor().ok())
            .map(|(size, scale)| size.to_logical::<f64>(scale).width);
        match width {
            Some(width) if viewport_width > 0.0 => width / viewport_width,
            _ => 1.0,
        }
    }

    /// Runs `js` in the window's page.
    fn tell(window: &WebviewWindow, js: String) {
        if let Err(error) = window.eval(js) {
            note!("omfp desktop: cannot tell the page about a web view: {error}");
        }
    }
}

impl Engine for Children {
    type View = Webview;

    /// A link to a new window opens in the view itself (UI-684); the page gets its widget's scripts and channel
    /// (UI-687).
    fn open(&mut self, id: &str, url: &str, setup: &Setup) -> Option<Webview> {
        let url = Url::parse(url).ok()?;
        let Some(parent) = self.window.app_handle().get_window(self.window.label()) else {
            note!("omfp desktop: no web view, the window is gone");
            return None;
        };
        self.made += 1;
        let label = format!("web-view-{}", self.made);
        let (window, placeholder) = (self.window.clone(), id.to_owned());
        let app = self.window.app_handle().clone();
        let own = label.clone();
        let messages = (self.window.clone(), id.to_owned());
        let mut builder = WebviewBuilder::new(&label, WebviewUrl::External(url))
            .data_store_identifier(self.store)
            .on_navigation(move |url| {
                if url.scheme() != MESSAGE_SCHEME {
                    return true;
                }
                if let Some(json) = page_message(url).filter(|json| json.len() <= MAX_PAGE_MESSAGE)
                {
                    Self::tell(&messages.0, message_call(&messages.1, &json));
                }
                false
            })
            .on_page_load(move |_, payload| {
                if payload.event() == PageLoadEvent::Finished {
                    Self::tell(
                        &window,
                        navigated_call(&placeholder, payload.url().as_str(), None, None),
                    );
                    Self::tell(&window, loaded_call(&placeholder));
                }
            })
            .on_new_window(move |url, _| {
                if matches!(url.scheme(), "http" | "https")
                    && let Some(view) = app.get_webview(&own)
                    && let Err(error) = view.navigate(url)
                {
                    note!("omfp desktop: a web view cannot open a link: {error}");
                }
                NewWindowResponse::Deny
            });
        if setup.channel {
            builder = builder.initialization_script(channel_script(Transport::Frames));
        }
        for script in &setup.scripts {
            builder = builder.initialization_script(script);
        }
        match parent.add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        ) {
            Ok(view) => {
                if let Err(error) = view.hide() {
                    note!("omfp desktop: cannot hide a new web view: {error}");
                }
                Some(view)
            }
            Err(error) => {
                note!("omfp desktop: no web view: {error}");
                None
            }
        }
    }

    fn load(&self, view: &Webview, url: &str) {
        if let Ok(url) = Url::parse(url)
            && let Err(error) = view.navigate(url)
        {
            note!("omfp desktop: a web view cannot load its page: {error}");
        }
    }

    fn current(&self, view: &Webview) -> Option<String> {
        view.url().ok().map(String::from)
    }

    fn place(&self, view: &Webview, at: Placement) {
        let placed = view
            .set_position(LogicalPosition::new(at.x, at.y))
            .and_then(|()| view.set_size(LogicalSize::new(at.width, at.height)));
        if let Err(error) = placed {
            note!("omfp desktop: cannot place a web view: {error}");
        }
    }

    fn show(&self, view: &Webview) {
        if let Err(error) = view.show() {
            note!("omfp desktop: cannot show a web view: {error}");
        }
    }

    fn hide(&self, view: &Webview) {
        if let Err(error) = view.hide() {
            note!("omfp desktop: cannot hide a web view: {error}");
        }
    }

    /// No capture: the view hides.
    fn freeze(&self, view: &Webview, _: &str, _: &Arc<AtomicBool>) {
        self.hide(view);
    }

    fn thaw(&self, _: &str) {}

    fn act(&self, view: &Webview, command: Command) {
        let done = match command {
            Command::Reload => view.reload(),
            Command::Back => view.eval("history.back()"),
            Command::Forward => view.eval("history.forward()"),
        };
        if let Err(error) = done {
            note!("omfp desktop: a web view cannot {command:?}: {error}");
        }
    }

    fn close(&self, view: Webview) {
        if let Err(error) = view.close() {
            note!("omfp desktop: cannot close a web view: {error}");
        }
    }
}
