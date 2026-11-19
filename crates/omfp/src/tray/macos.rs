//! macOS (M11, DMN-171, UI-254): the daemon in the menu bar. `AppKit`'s `NSStatusItem` lives on the main thread's run
//! loop, so `omfp daemon --tray` runs Tauri's event loop on the main thread — no window, no Dock icon (an accessory
//! app) — and the daemon ([`omfp_daemon::run_with`], its tokio runtime) on a thread of its own. The item and its menu
//! follow the thread index like the Linux tray (UI-217..219); `Quit daemon` stops the daemon (DMN-130), whose end ends
//! the loop.

use std::sync::{Arc, Mutex, PoisonError};

use omfp_daemon::DaemonHandle;
use omfp_daemon::config::Cli;
use omfp_protocol::ThreadSummary;
use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem as NativeItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{ActivationPolicy, AppHandle, Wry};

use super::{Daemon, MenuItem, ThreadIds, TrayIcon, TrayStatus, menu, open_in_browser};

/// The status item's id.
const ID: &str = "omfp";
/// The icons' size in pixels (shown at 18 pt, 36 px on a Retina menu bar).
const SIZE: u32 = 64;

/// `omfp daemon --tray` on macOS: the daemon on a thread, the menu bar on the main thread until the daemon stops.
pub fn run_daemon(cli: Cli) -> anyhow::Result<()> {
    let mut app = tauri::Builder::default()
        .enable_macos_default_menu(false)
        .build(tauri::generate_context!())?;
    app.set_activation_policy(ActivationPolicy::Accessory);
    let handle = app.handle().clone();
    let daemon = std::thread::Builder::new()
        .name("omfp-daemon".into())
        .spawn(move || {
            // Ends the run loop however the daemon thread ends, a panic included.
            let _exit = ExitLoop(handle.clone());
            omfp_daemon::run_with(cli, |daemon| MenuBar::start(&handle, daemon))
        })?;
    app.run_return(|_, _| {});
    daemon
        .join()
        .map_err(|_| anyhow::anyhow!("the daemon thread panicked"))?
}

/// Ends the main thread's run loop when dropped.
struct ExitLoop(AppHandle);

impl Drop for ExitLoop {
    fn drop(&mut self) {
        self.0.exit(0);
    }
}

/// The status item of a running daemon; dropping it removes the item (before the shutdown starts, DMN-130). `AppKit`
/// objects are main-thread-only: the item is built, updated and dropped on the main thread (objc2 panics when the
/// last handle is dropped on the daemon's thread), never held here.
pub struct MenuBar {
    app: AppHandle,
    follow: tokio::task::JoinHandle<()>,
}

impl MenuBar {
    fn start(app: &AppHandle, daemon: &DaemonHandle) -> Option<Self> {
        let target = Daemon::of(daemon, "menu bar item")?;
        let mut threads = daemon.threads();
        let status = TrayStatus::of(&threads.borrow_and_update());
        let mut ids = ThreadIds::default();
        let items = menu(&status, &mut ids);
        let shown = Arc::new(Mutex::new(items.clone()));
        let (sender, built) = std::sync::mpsc::channel();
        let summary = status.summary();
        let state = status.icon();
        let on_main = app.clone();
        let clicks = Arc::clone(&shown);
        let dispatched = app.run_on_main_thread(move || {
            let result = native_menu(&on_main, &items).and_then(|native| {
                TrayIconBuilder::with_id(ID)
                    .icon(icon(state))
                    .icon_as_template(template(state))
                    .tooltip(summary)
                    .menu(&native)
                    .show_menu_on_left_click(true)
                    .on_menu_event(move |_, event| click(&clicks, &target, &event))
                    .build(&on_main)
                    .map(drop)
            });
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
        let result = dispatched
            .map_err(|error| error.to_string())
            .and_then(|()| built.recv().map_err(|error| error.to_string()))
            .and_then(|result| result);
        if let Err(error) = result {
            note!("omfp daemon: no menu bar item: {error}");
            return None;
        }
        note!("omfp daemon: in the menu bar");
        let follow = tokio::spawn(follow(app.clone(), status, ids, shown, threads));
        Some(Self {
            app: app.clone(),
            follow,
        })
    }
}

impl Drop for MenuBar {
    fn drop(&mut self) {
        self.follow.abort();
        let app = self.app.clone();
        let _ = self.app.run_on_main_thread(move || {
            if let Some(item) = app.remove_tray_by_id(ID) {
                let _ = item.set_visible(false);
            }
        });
    }
}

/// Follows the thread index: a new menu, tooltip and, when it changes, icon, applied on the main thread.
async fn follow(
    app: AppHandle,
    mut status: TrayStatus,
    mut ids: ThreadIds,
    shown: Arc<Mutex<Vec<MenuItem>>>,
    mut threads: tokio::sync::watch::Receiver<Vec<ThreadSummary>>,
) {
    while threads.changed().await.is_ok() {
        let next = TrayStatus::of(&threads.borrow_and_update());
        if next == status {
            continue;
        }
        let items = menu(&next, &mut ids);
        let summary = next.summary();
        let icon_change = (next.icon() != status.icon()).then_some(next.icon());
        let on_main = app.clone();
        let shown = Arc::clone(&shown);
        let _ = app.run_on_main_thread(move || {
            let Some(item) = on_main.tray_by_id(ID) else {
                return;
            };
            match native_menu(&on_main, &items) {
                Ok(native) => {
                    if let Err(error) = item.set_menu(Some(native)) {
                        note!("omfp daemon: cannot update the menu: {error}");
                    }
                    *shown.lock().unwrap_or_else(PoisonError::into_inner) = items;
                }
                Err(error) => note!("omfp daemon: cannot build the menu: {error}"),
            }
            let _ = item.set_tooltip(Some(summary));
            if let Some(state) = icon_change {
                let _ = item.set_icon(Some(icon(state)));
                let _ = item.set_icon_as_template(template(state));
            }
        });
        status = next;
    }
}

fn native_menu(app: &AppHandle, items: &[MenuItem]) -> tauri::Result<Menu<Wry>> {
    let native = Menu::new(app)?;
    for item in items {
        if item.separator {
            native.append(&PredefinedMenuItem::separator(app)?)?;
        } else {
            // The model doubles `_` for dbusmenu mnemonics; AppKit shows labels as they are.
            let label = item.label.replace("__", "_");
            native.append(&NativeItem::with_id(
                app,
                item.id.to_string(),
                label,
                item.enabled,
                None::<&str>,
            )?)?;
        }
    }
    Ok(native)
}

fn click(shown: &Mutex<Vec<MenuItem>>, target: &Daemon, event: &MenuEvent) {
    let action = shown
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|item| item.id.to_string() == event.id().as_ref())
        .and_then(|item| item.action.clone());
    if let Some(action) = action {
        target.act(&action, "menu bar", open_in_browser);
    }
}

/// Idle is a template image (the π's alpha, tinted by `AppKit` for the menu bar's appearance); the states with a dot
/// keep their colours.
fn template(icon: TrayIcon) -> bool {
    icon == TrayIcon::Idle
}

/// The 64 px icon as RGBA, from build.rs's big-endian ARGB rows.
fn icon(icon: TrayIcon) -> Image<'static> {
    let argb: &[u8] = match icon {
        TrayIcon::Idle => include_bytes!(concat!(env!("OUT_DIR"), "/tray-idle-64.argb")),
        TrayIcon::Running => include_bytes!(concat!(env!("OUT_DIR"), "/tray-running-64.argb")),
        TrayIcon::NeedsYou => include_bytes!(concat!(env!("OUT_DIR"), "/tray-attention-64.argb")),
    };
    let template = template(icon);
    let (pixels, _) = argb.as_chunks::<4>();
    let rgba = pixels
        .iter()
        .flat_map(|&[alpha, red, green, blue]| {
            if template {
                [0, 0, 0, alpha]
            } else {
                [red, green, blue, alpha]
            }
        })
        .collect();
    Image::new_owned(rgba, SIZE, SIZE)
}
