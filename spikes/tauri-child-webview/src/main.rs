//! Spike S-17: `omfp-tauri-child-spike [url]` opens one Tauri window holding two WebKit webviews: a host page
//! filling the window, standing for omfp's UI, and a child webview with `url` (default Google's sign-in) placed over
//! part of it with `Window::add_child`, as a widget's page would be. Questions: does Google let a real account sign in
//! there, and can a child webview sit over the host page on Linux. Tauri's `add_child` packs every webview into the
//! window's `GtkBox` on Linux, ignoring position and size, so the page lands under the host page; this host moves both
//! into a `gtk::Overlay` instead, the page at its place over the host (`--stacked` keeps Tauri's packing). Each run keeps
//! its web data in a temporary directory of its own. `--user-agent=<ua>` replaces WebKit's user agent in the child.
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{LogicalPosition, LogicalSize, WebviewUrl};

const WIDTH: f64 = 1280.0;
const HEIGHT: f64 = 860.0;

/// The host page: a grid of labelled lines, so the screen shows whether the child covers it or pushes it aside.
const HOST: &str = "data:text/html,<!doctype html><title>host</title>\
<body style='margin:0;background:%23202030;color:%23c8c8e0;font:15px monospace'>\
<p style='padding:12px'>HOST WEBVIEW (stands for omfp). The child webview should cover the box from x=320, y=80.</p>\
<script>for(let i=0;i<40;i++){const p=document.createElement('div');p.style.padding='2px 12px';\
p.textContent='host line '+i+' '+'.'.repeat(160);document.body.append(p)}</script></body>";

fn main() {
    let mut url = "https://accounts.google.com/".to_owned();
    let mut user_agent = None;
    let mut stacked = false;
    for arg in std::env::args().skip(1) {
        if arg == "--stacked" {
            stacked = true;
        } else if let Some(ua) = arg.strip_prefix("--user-agent=") {
            user_agent = Some(ua.to_owned());
        } else {
            url = arg;
        }
    }
    let data = std::env::temp_dir().join(format!("omfp-tauri-child-spike-{}", std::process::id()));
    let app = tauri::Builder::default().setup(move |app| {
        let window = WindowBuilder::new(app, "main")
            .title("omfp: Tauri child webview spike (S-17)")
            .inner_size(WIDTH, HEIGHT)
            .build()?;
        let host = WebviewBuilder::new("host", WebviewUrl::External(HOST.parse()?))
            .data_directory(data.join("host"));
        window.add_child(
            host,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(WIDTH, HEIGHT),
        )?;
        let mut page = WebviewBuilder::new("page", WebviewUrl::External(url.parse()?))
            .data_directory(data.join("page"));
        if let Some(ua) = &user_agent {
            page = page.user_agent(ua);
        }
        window.add_child(
            page,
            LogicalPosition::new(320.0, 80.0),
            LogicalSize::new(900.0, 740.0),
        )?;
        if !stacked {
            overlay(&window)?;
        }
        Ok(())
    });
    if let Err(error) = app.run(tauri::generate_context!()) {
        eprintln!("omfp-tauri-child-spike: {error}");
        std::process::exit(1);
    }
}

/// Moves the window's two webviews out of Tauri's `GtkBox` into a `gtk::Overlay`: the host page as its main child,
/// filling the window, the page as an overlay child at x=320, y=80, 900 × 740.
fn overlay(window: &tauri::Window) -> Result<(), Box<dyn std::error::Error>> {
    use gtk::prelude::*;
    let vbox = window.default_vbox()?;
    let children = vbox.children();
    let [host, page] = children.as_slice() else {
        return Err(format!(
            "expected 2 webviews in the window's box, found {}",
            children.len()
        )
        .into());
    };
    vbox.remove(host);
    vbox.remove(page);
    let overlay = gtk::Overlay::new();
    overlay.add(host);
    overlay.add_overlay(page);
    page.set_halign(gtk::Align::Start);
    page.set_valign(gtk::Align::Start);
    page.set_margin_start(320);
    page.set_margin_top(80);
    page.set_size_request(900, 740);
    vbox.pack_start(&overlay, true, true, 0);
    overlay.show_all();
    Ok(())
}
