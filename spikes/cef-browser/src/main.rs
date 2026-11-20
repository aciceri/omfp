//! Spike S-16 (docs/specs/07-roadmap.md): can omfp's UI run in prebuilt CEF, with widgets showing real web pages
//! (Google sign-in, YouTube) drawn by the same CEF, and omfp's own UI (the `:` line, dialogs) still on top of them?
//!
//! `omfp-cef-spike --url='http://127.0.0.1:<port>/#token=<token>'` opens one window on omfp; a widget marks a
//! placeholder with `data-omfp-cef-browser` (see `widget/index.tsx`). `--profile=<dir>` keeps the browsers' cookies
//! (default: `$XDG_DATA_HOME/omfp-cef-spike`). CEF re-runs this executable for its renderer, GPU and utility processes
//! (`--type=…`).

// cef-rs's `wrap_*` macros cast between its reference-counted wrappers with `transmute`.
#![allow(clippy::transmute_ptr_to_ptr)]

mod host;
mod slots;

use std::path::PathBuf;
use std::process::ExitCode;

// The `wrap_*` macros expand to code naming cef's traits (`Rc`, `Impl*`, `Wrap*`) unqualified.
#[allow(clippy::wildcard_imports)]
use cef::{args::Args, *};

wrap_app! {
    struct SpikeApp {
        url: String,
    }

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(host::ProcessHandler::new(self.url.clone()))
        }
    }
}

fn default_profile() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("omfp-cef-spike")
}

fn main() -> ExitCode {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();
    let Some(command_line) = args.as_cmd_line() else {
        eprintln!("omfp-cef: cannot read the command line");
        return ExitCode::FAILURE;
    };
    let switch = |name: &str| {
        CefString::from(&command_line.switch_value(Some(&CefString::from(name)))).to_string()
    };
    let subprocess = command_line.has_switch(Some(&CefString::from("type"))) == 1;

    // CEF's own processes return here with their exit code.
    let code = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if subprocess {
        return u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from);
    }

    let url = switch("url");
    if url.is_empty() {
        eprintln!(
            "usage: omfp-cef-spike --url='http://127.0.0.1:<port>/#token=<token>' [--profile=<dir>]"
        );
        return ExitCode::FAILURE;
    }
    let profile = match switch("profile") {
        profile if profile.is_empty() => default_profile(),
        profile => PathBuf::from(profile),
    };
    let profile = profile.to_string_lossy().into_owned();

    let mut app = SpikeApp::new(url);
    let settings = Settings {
        // Unsandboxed unless `--sandbox`: Chromium's Linux sandbox needs user namespaces or the setuid
        // `chrome-sandbox`, which the spike checks but does not package.
        no_sandbox: i32::from(command_line.has_switch(Some(&CefString::from("sandbox"))) != 1),
        root_cache_path: CefString::from(profile.as_str()),
        cache_path: CefString::from(profile.as_str()),
        persist_session_cookies: 1,
        log_severity: LogSeverity::WARNING,
        ..Default::default()
    };
    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) != 1
    {
        eprintln!("omfp-cef: CEF did not initialize");
        return ExitCode::FAILURE;
    }
    run_message_loop();
    shutdown();
    ExitCode::SUCCESS
}
