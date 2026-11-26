# Spike S-17: a web page as a Tauri child webview

Can omfp's desktop window (Tauri 2, WebKitGTK) show a web page that refuses framing, such as Google's sign-in, as a
native webview over omfp's UI, without CEF (S-16)? `omfp:web` cannot: `accounts.google.com` answers
`X-Frame-Options: DENY`, which every browser applies to a frame.

`src/main.rs` opens one Tauri 2.12 window holding two webviews made with `Window::add_child` (feature `unstable`): a
host page filling the window, standing for omfp's UI, and the page (default Google's sign-in) at x=320, y=80,
900 × 740, as a widget's page would be. Its own Cargo workspace; `just ci` does not build it.

```sh
nix develop -c cargo run --manifest-path spikes/tauri-child-webview/Cargo.toml -- [url] [--stacked] [--user-agent=<ua>]
```

## Found (2026-11-26, niri on picard, Wayland, WebKitGTK 2.54)

| Question | Found |
| --- | --- |
| Google sign-in | A real account signed in to the Account home, with WebKitGTK's own user agent: no "This browser or app may not be secure" refusal. |
| Placement | On Linux Tauri ignores `add_child`'s position and size: every webview is packed into the window's `GtkBox`, so the page lands under the host page, full width (`--stacked`). |
| Over the host page | Moving both webviews from that box into a `gtk::Overlay` (host as the main child, the page as an overlay child with margins and a size request) puts the page at its place over the host page, which shows around it. About 20 lines of GTK. |
| Links to a new window | `target=_blank` links and `window.open` do nothing: no new-window handler, so WebKit drops them. omfp would route them to the system browser (DMN-182) or open them in the same view. |

Not checked: keyboard focus between the two webviews, resizing the window, omfp's own overlays (`:` line, dialogs) over
the page (a native view stays on top of the host's HTML, so S-16's freeze to a capture would be needed here too),
macOS (WKWebView, where `add_child` honours position and size), codecs (WebKitGTK plays VP9/AV1 only with the system's
GStreamer plugins).

## Against S-16 (CEF)

Google sign-in works in both. This keeps the desktop's Tauri window, its 10.9 MB and decisions D-15 and D-16, and
S-16's bridge and placeholder state machine (`spikes/cef-browser/src/bridge.js`, `slots.rs`) would drive a
`gtk::Overlay` the same way.
