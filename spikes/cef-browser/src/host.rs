//! The CEF side of spike S-16: one top-level window whose main browser shows omfp's UI, and one embedded browser per
//! slot the page reports, drawn as an overlay view at the slot's place. Everything runs on CEF's UI thread; the state
//! lives in a thread-local and is never borrowed across a call into CEF, which may call back into us.

use std::cell::RefCell;
use std::collections::HashMap;

// The `wrap_*` macros expand to code naming cef's traits (`Rc`, `Impl*`, `Wrap*`) unqualified.
#[allow(clippy::wildcard_imports)]
use cef::*;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::slots::{Action, Bounds, Report, Slots};

/// The bridge injected into the main page before its own scripts.
const BRIDGE: &str = include_str!("bridge.js");
/// The CDP binding the bridge calls with its JSON messages.
const BINDING: &str = "omfpCefHost";

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host::default());
}

fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R {
    HOST.with(|host| f(&mut host.borrow_mut()))
}

#[derive(Default)]
struct Host {
    /// omfp's URL, loaded once the bridge is in place.
    url: String,
    window: Option<Window>,
    main: Option<Browser>,
    registrations: Vec<Registration>,
    next_message: i32,
    pending: HashMap<i32, Pending>,
    slots: Slots,
    embedded: HashMap<String, Embedded>,
    /// Actions waiting to run, in order: an action may produce more (a capture that arrives at once hides the view),
    /// which must run after the rest of the list that produced it.
    queue: std::collections::VecDeque<Action>,
    running: bool,
}

struct Embedded {
    view: BrowserView,
    overlay: OverlayController,
    /// The DevTools observer of its browser, added at its first capture.
    observing: bool,
}

/// What a sent DevTools method is for, by message id.
enum Pending {
    /// One of the main browser's setup methods; the last one loads omfp.
    Setup {
        last: bool,
    },
    Capture {
        id: String,
    },
}

impl Host {
    fn message(&mut self, pending: Pending) -> i32 {
        self.next_message += 1;
        self.pending.insert(self.next_message, pending);
        self.next_message
    }
}

/// Sends one DevTools method to `browser` and records what it is for.
fn send(browser: &Browser, method: &str, params: &Value, pending: Pending) {
    let id = with(|host| host.message(pending));
    let message = json!({ "id": id, "method": method, "params": params }).to_string();
    if let Some(host) = browser.host()
        && host.send_dev_tools_message(Some(message.as_bytes())) == 0
    {
        eprintln!("omfp-cef: {method} was not sent");
    }
}

/// Runs `code` in omfp's page.
fn evaluate(code: &str) {
    let Some(frame) = with(|host| host.main.clone()).and_then(|main| main.main_frame()) else {
        return;
    };
    frame.execute_java_script(
        Some(&CefString::from(code)),
        Some(&CefString::from("omfp-cef")),
        0,
    );
}

fn rect(bounds: Bounds) -> Rect {
    Rect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.w.max(1),
        height: bounds.h.max(1),
    }
}

/// Creates the top-level window on `url` (the omfp UI, with its `#token=…` fragment).
pub fn open(url: String) {
    with(|host| host.url = url);
    let mut client = MainClient::new();
    let mut delegate = AlloyBrowserView::new();
    // The main browser starts blank: omfp loads once the bridge is injected, so it runs before omfp's first script.
    let view = browser_view_create(
        Some(&mut client),
        Some(&CefString::from("about:blank")),
        Some(&BrowserSettings::default()),
        None,
        None,
        Some(&mut delegate),
    );
    let mut delegate = MainWindow::new(RefCell::new(view));
    window_create_top_level(Some(&mut delegate));
}

wrap_browser_process_handler! {
    pub struct ProcessHandler {
        url: String,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            open(self.url.clone());
        }
    }
}

wrap_window_delegate! {
    struct MainWindow {
        view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1400, height: 900 }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let (Some(window), Some(view)) = (window, self.view.borrow().clone()) else {
                return;
            };
            window.add_child_view(Some(&mut View::from(&view)));
            window.show();
            with(|host| host.window = Some(window.clone()));
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            self.view.borrow_mut().take();
            with(|host| {
                host.window = None;
                host.embedded.clear();
            });
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let (main, embedded) = with(|host| {
                let embedded: Vec<BrowserView> = host.embedded.values().map(|entry| entry.view.clone()).collect();
                (host.main.clone(), embedded)
            });
            for view in embedded {
                if let Some(host) = view.browser().and_then(|browser| browser.host()) {
                    host.close_browser(1);
                }
            }
            main.and_then(|main| main.host()).map_or(1, |host| host.try_close_browser())
        }

        // cef-rs answers 0 (no) by default: a window that cannot resize or maximize is one a tiling compositor (niri)
        // opens floating.
        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        // The same app id and WM class as the Tauri window, so the compositor's rules for omfp apply.
        fn linux_window_properties(
            &self,
            _window: Option<&mut Window>,
            properties: Option<&mut LinuxWindowProperties>,
        ) -> i32 {
            let Some(properties) = properties else {
                return 0;
            };
            if std::env::var_os("OMFP_CEF_TRACE").is_some() {
                eprintln!("omfp-cef: linux_window_properties");
            }
            properties.wayland_app_id = CefString::from("omfp");
            properties.wm_class_class = CefString::from("omfp");
            properties.wm_class_name = CefString::from("omfp");
            1
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

// Alloy style everywhere: an Alloy window can host several overlay BrowserViews, which a Chrome-style window cannot
// (CEF issue #3790).
wrap_browser_view_delegate! {
    struct AlloyBrowserView {}

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

wrap_client! {
    struct MainClient {}

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(MainLifeSpan::new())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(MainDisplay::new())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(MainLoad::new())
        }
    }
}

// The bridge again once a document of omfp's origin has loaded: a no-op when it already ran at document start.
wrap_load_handler! {
    struct MainLoad {}

    impl LoadHandler {
        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _http_status_code: i32) {
            let Some(frame) = frame else {
                return;
            };
            let url = CefString::from(&frame.url()).to_string();
            if frame.is_main() == 1 && url.starts_with("http") {
                frame.execute_java_script(Some(&CefString::from(BRIDGE)), Some(&CefString::from("omfp-cef")), 0);
            }
        }
    }
}

wrap_display_handler! {
    struct MainDisplay {}

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let Some(window) = with(|host| host.window.clone()) {
                window.set_title(title);
            }
        }
    }
}

wrap_life_span_handler! {
    struct MainLifeSpan {}

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser.cloned() else {
                return;
            };
            with(|host| host.main = Some(browser.clone()));
            observe(&browser, None);
            send(&browser, "Page.enable", &json!({}), Pending::Setup { last: false });
            send(&browser, "Runtime.enable", &json!({}), Pending::Setup { last: false });
            send(&browser, "Runtime.addBinding", &json!({ "name": BINDING }), Pending::Setup { last: false });
            send(
                &browser,
                "Page.addScriptToEvaluateOnNewDocument",
                &json!({ "source": BRIDGE }),
                Pending::Setup { last: true },
            );
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            quit_message_loop();
        }
    }
}

/// Adds the DevTools observer to `browser`: the main one when `slot` is `None`, else the embedded one of `slot`.
fn observe(browser: &Browser, slot: Option<&str>) {
    let Some(host) = browser.host() else {
        return;
    };
    let mut observer = Observer::new(slot.map(str::to_owned));
    if let Some(registration) = host.add_dev_tools_message_observer(Some(&mut observer)) {
        with(|host| host.registrations.push(registration));
    }
}

wrap_dev_tools_message_observer! {
    struct Observer {
        slot: Option<String>,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: i32,
            success: i32,
            result: Option<&[u8]>,
        ) {
            let Some(pending) = with(|host| host.pending.remove(&message_id)) else {
                return;
            };
            let result: Value = result.and_then(|bytes| serde_json::from_slice(bytes).ok()).unwrap_or(Value::Null);
            if success == 0 {
                eprintln!("omfp-cef: DevTools method {message_id} failed: {result}");
            }
            match pending {
                Pending::Setup { last: true } => load_omfp(),
                Pending::Setup { last: false } => {}
                Pending::Capture { id } => captured(&id, result.get("data").and_then(Value::as_str)),
            }
        }

        fn on_dev_tools_event(&self, _browser: Option<&mut Browser>, method: Option<&CefString>, params: Option<&[u8]>) {
            if self.slot.is_some() || method.map(CefString::to_string).as_deref() != Some("Runtime.bindingCalled") {
                return;
            }
            let Some(call) = params.and_then(|bytes| serde_json::from_slice::<BindingCalled>(bytes).ok()) else {
                return;
            };
            if call.name != BINDING {
                return;
            }
            match serde_json::from_str::<BridgeMessage>(&call.payload) {
                Ok(BridgeMessage::Slots { slots }) => {
                    let actions = with(|host| host.slots.update(&slots));
                    perform(actions);
                }
                Err(error) => eprintln!("omfp-cef: unreadable bridge message: {error}"),
            }
        }
    }
}

#[derive(Deserialize)]
struct BindingCalled {
    name: String,
    payload: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeMessage {
    Slots { slots: Vec<Report> },
}

fn load_omfp() {
    let (main, url) = with(|host| (host.main.clone(), host.url.clone()));
    if let Some(frame) = main.and_then(|main| main.main_frame()) {
        frame.load_url(Some(&CefString::from(url.as_str())));
    }
}

/// The capture of slot `id` arrived: shows it in the page, then hides the view (unless the slot was uncovered meanwhile).
fn captured(id: &str, data: Option<&str>) {
    let still_freezing = with(|host| {
        host.slots
            .get(id)
            .is_some_and(|slot| slot.phase == crate::slots::Phase::Freezing)
    });
    if !still_freezing {
        return;
    }
    if let Some(data) = data {
        let id_json = Value::from(id).to_string();
        let src = Value::from(format!("data:image/jpeg;base64,{data}")).to_string();
        evaluate(&format!("window.__omfpCef?.freeze({id_json}, {src})"));
    }
    let actions = with(|host| host.slots.frozen(id));
    perform(actions);
}

/// Queues `actions` and, unless an outer call is already running the queue, runs it to the end.
fn perform(actions: Vec<Action>) {
    let start = with(|host| {
        host.queue.extend(actions);
        !std::mem::replace(&mut host.running, true)
    });
    if !start {
        return;
    }
    while let Some(action) = with(|host| host.queue.pop_front()) {
        run(action);
    }
    with(|host| host.running = false);
}

fn run(action: Action) {
    if std::env::var_os("OMFP_CEF_TRACE").is_some() {
        eprintln!("omfp-cef: {action:?}");
    }
    match action {
        Action::Create { id, url, bounds } => create(&id, &url, bounds),
        Action::Navigate { id, url } => {
            if let Some(frame) = embedded_browser(&id).and_then(|browser| browser.main_frame()) {
                frame.load_url(Some(&CefString::from(url.as_str())));
            }
        }
        Action::Move { id, bounds } => {
            if let Some(overlay) =
                with(|host| host.embedded.get(&id).map(|entry| entry.overlay.clone()))
            {
                overlay.set_bounds(Some(&rect(bounds)));
            }
        }
        Action::Show { id } => set_visible(&id, true),
        Action::Hide { id } => set_visible(&id, false),
        Action::Freeze { id } => capture(&id),
        Action::Thaw { id } => evaluate(&format!("window.__omfpCef?.thaw({})", Value::from(id))),
        Action::Destroy { id } => destroy(&id),
    }
}

fn create(id: &str, url: &str, bounds: Bounds) {
    let Some(window) = with(|host| host.window.clone()) else {
        return;
    };
    let mut client = EmbeddedClient::new();
    let mut delegate = AlloyBrowserView::new();
    let Some(view) = browser_view_create(
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&BrowserSettings::default()),
        None,
        None,
        Some(&mut delegate),
    ) else {
        eprintln!("omfp-cef: no browser view for {id}");
        return;
    };
    let Some(overlay) =
        window.add_overlay_view(Some(&mut View::from(&view)), DockingMode::CUSTOM, 1)
    else {
        eprintln!("omfp-cef: no overlay for {id}");
        return;
    };
    overlay.set_bounds(Some(&rect(bounds)));
    overlay.set_visible(0);
    with(|host| {
        host.embedded.insert(
            id.to_owned(),
            Embedded {
                view,
                overlay,
                observing: false,
            },
        );
    });
}

fn embedded_browser(id: &str) -> Option<Browser> {
    with(|host| host.embedded.get(id).map(|entry| entry.view.clone()))
        .and_then(|view| view.browser())
}

fn set_visible(id: &str, visible: bool) {
    if let Some(overlay) = with(|host| host.embedded.get(id).map(|entry| entry.overlay.clone())) {
        overlay.set_visible(i32::from(visible));
    }
}

fn capture(id: &str) {
    let Some(browser) = embedded_browser(id) else {
        // No browser yet (still loading its first page): nothing to capture, hide it at once.
        captured(id, None);
        return;
    };
    let first = with(|host| {
        host.embedded
            .get_mut(id)
            .is_some_and(|entry| !std::mem::replace(&mut entry.observing, true))
    });
    if first {
        observe(&browser, Some(id));
    }
    send(
        &browser,
        "Page.captureScreenshot",
        &json!({ "format": "jpeg", "quality": 85 }),
        Pending::Capture { id: id.to_owned() },
    );
}

fn destroy(id: &str) {
    let Some(entry) = with(|host| host.embedded.remove(id)) else {
        return;
    };
    if let Some(host) = entry.view.browser().and_then(|browser| browser.host()) {
        host.close_browser(1);
    }
    entry.overlay.destroy();
}

wrap_client! {
    struct EmbeddedClient {}

    impl Client {}
}
