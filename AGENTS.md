# AGENTS.md

Guidance for coding agents working on **omfp** (formerly pmo, renamed at the end of M1): a GUI client (web + desktop)
for the omp coding agent (oh-my-pi). One binary, `omfp` (`crates/omfp`): `omfp daemon` runs the daemon (one
`omp --mode rpc-ui` process per thread, WebSocket API, web UI); `omfp theme set <file>` sets the UI theme of a daemon;
`omfp desktop` (M7, Tauri) opens one more window on the local daemon, starting it in the background if needed (D-15,
D-16). Threads run in **workspace folders** whose top-level entries are repositories (D-11). The UI is a
**TypeScript/React** web app built and tested with **Bun** (`ui/`, D-18), served by the daemon. Direction (D-17):
agents get full access to the protocol and build widgets in JavaScript (M4, M10); never design something that would
only be reachable from the human UI.

## Read first

- `docs/specs/README.md` — index, conventions, decision log (D-01…D-20), open questions.
- `docs/specs/07-roadmap.md` — milestones and spike results (done: M0–M8 with M2b, M5b, M5c, M5d, M6b, M8b and M8c,
  M10, M12, M13, M14; in progress: M9 hardening, M11 macOS desktop, M15 downloads without Nix, M16 media and file
  viewers, M17 workspace polish; the terminal was removed, D-06); `docs/specs/milestones.toml` — requirement IDs and
  `status` per milestone (enforced by `cargo xtask spec-trace`).
- The spec for the area you touch (`00`…`09`).

## Ground rules

- Everything in **English**: code, comments, docs, commit messages.
- **TDD**: tests first. Every test covering a requirement has `// spec: ID[, ID…]` right above it (parsed by
  `cargo xtask spec-trace`; never put the marker plus an ID inside a string literal). Deterministic and isolated tests
  only (TST-003): no sleeps (wait on conditions with timeouts), temporary directories only (never the user's), port 0,
  no network beyond loopback, no real models.
- **Conventional Commits** (checked by commitizen in the commit-msg hook). Commits are SSH-signed with the user's key:
  if signing fails, ask the user to load the key, never disable it.
- Prefer rebasing PR branches onto the latest `origin/master` rather than merging `master` into them. Keep rewritten
  commits signed; update a published branch with `git push --force-with-lease`, never plain `--force`, and wait for CI
  on the rebased head before merging the PR.
- Prefer `git absorb` for follow-up fixes to a PR's existing changes, then autosquash the fixups, rather than leaving
  separate correction commits. Keep unrelated changes in their own commits and preserve signatures when rewriting.
- Keep docs in sync: when behavior or a decision changes, update the spec rows (and spike results) in the same change.
- No new dependencies without a reason; workspace dependencies live in the root `Cargo.toml`.
- No `Box::leak`, no placeholder stubs, no `unwrap()` in non-test code where an error can be returned.

## Commands (always inside the dev shell)

```sh
nix develop                      # or: nix develop -c sh -c '<command>'
just ci                          # ci_recipes: check, test-ts, test-contract, test-e2e, test-desktop, package, mutants-gate, cov on one web bundle (CI: .github/workflows/ci.yml, same recipes)
just mutants-gate                # TST-050: no surviving mutant in omfp-core, omfp-protocol (`just mutants` explores more crates)
just cov                         # TST-050: the fast Rust tests instrumented, coverage gates (pure crates ≥ 90%, daemon ≥ 75%) + HTML report
just check                       # lint, check-versions, spec-trace, deny, check-workflow (actionlint + workflow vs `just ci`)
just test                        # fast Rust tests (no omp process)
just test-ts                     # `bun run test` in ui/ (reducer goldens, client, kit + app component tests; transport suite against a fake-launcher daemon); TS view models' coverage ≥ 90%
just test-contract               # contract tests against the pinned omp + fake LLM
just record-fixtures             # re-record fixtures/ (review the diff!), then `just gen-states`
just gen-states                  # golden reducer states fixtures/states/<ver>/ from the Rust reducer
just gen-ts                      # ui/packages/protocol/src/generated.ts from the daemon's JSON Schemas
just gen-bun-nix                 # ui/bun.nix from ui/bun.lock (bun2nix), after every JS dependency change
just test-e2e                    # L5 journeys: builds a private web bundle, headless Chromium + daemon + omp + fake LLM; shots in a new /tmp/omfp-shots-*
just bench                       # e2e benchmarks (`benchmark_*`, ignored elsewhere): timings by hand, not in `just ci`
just test-desktop                # `omfp --features desktop`: desktop unit tests + the Tauri window under xvfb-run (TST-033)
just build-web                   # release bundle of the TypeScript UI into ui/packages/app/dist (`bun run build`)
just serve [daemon args]         # try the app: release bundle + `omfp daemon` with your HOME, ~/.omp, default data dir
nix build .#omfp                 # the install: omfp with the desktop window, the web bundle and the pinned omp (nix/);
                                 # .#omfp-headless without WebKit
just desktop [desktop args]      # try the desktop app: one more window on your local daemon (started in the background)
just package-linux [out]         # release download without Nix: headless x86_64 tarball (omfp, omp, web/) in dist/
just package-flatpak [out]       # release download without Nix: the Linux desktop Flatpak (omp runs on the host) in dist/
just package-macos [out]         # release download without Nix (arm64 Mac): omfp.app as .dmg and .zip in dist/
just brand                       # regenerate the logo: docs/brand SVGs, kit outlines, favicon, desktop icon
cargo xtask spec-trace           # requirement → citing tests; fails on broken traceability
cargo xtask check-versions       # omp / omp-rpc pins agree
```

Clippy runs with `pedantic` as errors. Notable: `assert_is_empty` forbids `assert!(x.is_empty())` — use
`assert_eq!(x, Vec::new())`; `large_enum_variant` is allowed only where we mirror omp-rpc unions.

## Layout

```text
crates/omfp-core      reducer over omp events (ThreadEvent → ThreadState); no I/O
crates/omfp-protocol  UI ↔ daemon protocol types, JSON Schemas
crates/omfp-daemon    daemon library: omp adapter (omp.rs), agent port + FakeLauncher (agent.rs, agent/fake.rs; omp's live voice
                      sequence in agent/fake/live.rs), live voice relay (supervisor/live.rs: one session per thread;
                      live_context.rs: omp's vendored live prompt live_instructions.txt + the thread's conversation),
                      supervisor (supervisor.rs: launch in the workspace folder, root sync; supervisor/dialogs.rs:
                      answers to omp's dialogs; supervisor/naming.rs: thread titles and session names), agent
                      conventions and the widget-builder agent (conventions.rs),
                      event log, store (+ migrations, store/migrate.rs), workspaces (folder scanner in
                      workspaces/scan.rs, cheap change check in workspaces/fingerprint.rs, reading and writing
                      workspace files with a hash check in workspaces/files.rs, add/remove repo,
                      suggestions of seen repositories), path completion (fs_complete.rs), conversation search over
                      session files (search.rs), threads, git port + FakeGit, host tools and the `omfp://` host URIs
                      (host_tools.rs), the agent tool registry (agent_tools/: `omfp_call`, `omfp_thread_wait`, the
                      method catalogue `omfp://protocol/methods`, the RPC host tool adapter, the mount tools, the
                      check tools `omfp_widget_check`/`omfp_views_errors` in agent_tools/checks.rs, the window tools `omfp_windows`/`omfp_window_run` in agent_tools/windows.rs),
                      jobs (jobs.rs: shell commands of a workspace, `job/<id>`), widgets (widgets.rs: views manifest
                      check, widget folders and versions, oxc transpiling and the `/widgets/<key>/…` route, copies;
                      widgets/docs.md is `omfp://docs/widgets`, widgets/shell.md `omfp://docs/shell`,
                      widgets/keymap-defaults.json `omfp://keymap` (written by the app's keymap-export test with
                      `OMFP_UPDATE_KEYMAP=1`; widgets/shell.rs serves them and the `omfp://docs` index;
                      server/services/file_checks.rs: `views.errors`, `omfp_views_errors` and the checks of init and
                      TSX view files); widgets/keymap.rs: the workspace `.omfp/keymap.json`;
                      widgets/init.rs: the init files; widgets/mounts.rs: mounts and floating panels as `views.json`
                      edits; widgets/library.rs: the component library; widgets/native.rs: the native form controls
                      `omfp_widget_check` warns about; server/services/widget_tools.rs: what the widget check does), WebSocket server (polling of watched workspaces:
                      server/services/polls.rs; the windows' reports, `windows.*` and the requests to windows:
                      server/services/windows.rs; the `views/<id>` topic: server/services/views.rs; `views.mount`…
                      and `widgets.save`/`library`: server/services/mounts.rs, their agent tools agent_tools/mounts.rs;
                      the raw bytes of workspace files at `/files/<key>/…`, `?rev=HEAD` their bytes at `HEAD`: server/raw_files.rs, M16),
                      the MCP server at `/mcp` (server/mcp.rs: Streamable HTTP, dual-era 2026-07-28 + legacy sessions,
                      token and origin checks; server/mcp/methods.rs: the registry as tools, the catalogue and the static docs as resources);
                      the logins to omp's providers (supervisor/login.rs: the short-lived login omp, one login at a
                      time, the reload of running threads after a login; supervisor/models.rs: the count of usable
                      models of `auth.status`; server/services/logins.rs: `auth.*` and the
                      `login_*` window requests of a login's flow);
                      the MCP servers omp loads for a workspace (omp_mcp.rs: the config files omp reads, the `/mcp test`
                      reply; supervisor/mcp.rs: checks by the thread's omp, reconnect by restart, M13; supervisor/restart.rs:
                      the one restart of a thread's omp and the restarts the daemon owes, with their reasons);
                      `thread.session`, a thread's session facts without omp's whole state (server/dispatch/session.rs);
                      the prompt history and `prompts.history` (server/prompts.rs; store migration 0010);
                      the options of `omfp daemon` over the config file's `[daemon]` (config.rs) and the config file
                      with its `[ui]`/`[keymap]` settings, written back by `settings.set` (config_file.rs);
                      widgets/layout.rs: the placement of `views.mount`/`omfp_ui_mount` in the manifest;
                      `omfp_daemon::{start, run, discover}` back `omfp daemon` and `omfp desktop`;
                      examples/test_daemon.rs: the fake-launcher daemon the TS transport suite runs against
crates/omfp           the `omfp` binary (D-15): `daemon`, `theme set` (remote.rs: protocol client of the CLI),
                      `mcp config` (mcp.rs: the `mcp.json` entry of a daemon's MCP server), and with
                      the `desktop` feature (Tauri 2: WebKitGTK on Linux, WKWebView on macOS, M11) `desktop`
                      (desktop.rs: the window on the daemon's own UI, the theme push of `--connect`, the `OMFP_SMOKE`
                      check; desktop/local.rs: find or start the background daemon; desktop/portal.rs: the color
                      scheme of the settings portal over gio's D-Bus API (Linux); desktop/macos.rs: the data store id,
                      the page's `Notification` posted to the notification center, the app menu's New Window and
                      zoom items); tray.rs: `omfp daemon
                      --tray`, the shared tray model, tray/sni.rs the
                      Linux StatusNotifierItem + dbusmenu over gio, tray/macos.rs the macOS menu bar item (AppKit on
                      the main thread, the daemon on another); build.rs (also the tray pixmaps and, on macOS, the
                      embedded Info.plist), Info.plist, tauri.conf.json, icons/ with the icons' HTML sources
                      (tray.html: the tray's π states); launch.rs (a bare launch of the release `omfp.app` opens
                      `desktop`); tests/desktop_smoke.rs, TST-033, TST-090; tests/daemon_tray.rs,
                      TST-061 (Linux); tests/menu_bar.rs, TST-091 (macOS))
crates/fake-llm       scripted OpenAI-compatible server (gated steps hold a reply to keep omp busy)
crates/omfp-testkit   TempRepo, OmpEnv (isolated omp), DenyProxy, fixture normalization
xtask                 spec-trace (Rust, TypeScript, justfile), check-versions, gen-ts (TypeScript protocol types)
e2e                   L5 harness (thirtyfour + chromedriver, daemon in-process; `Journey::start_fake` runs it on the
                      fake launcher, e.g. live.rs: live voice on a fake microphone, E2E-14), the journeys and the widget
                      journeys; kit_gallery.rs is the `/kit` contrast test with shots for review (UI-174); view_switch.rs
                      times switching back to a view whose widget's call is slow (TST-108)
ui/                   Bun workspace of the TypeScript UI (D-18): strict `tsc` + Biome (`just lint`), `bun run test`
ui/packages/protocol  @omfp/protocol: generated protocol types + method table (generated.ts), envelope, topics
ui/packages/core      @omfp/core: the reducer ported from omfp-core, checked against fixtures/states/
ui/packages/client    @omfp/client: WebSocket transport, session (hello, subscriptions, reconnect), model
ui/packages/kit       @omfp/kit: React Aria Components wrappers styled by token CSS (tokens.css: every `--omfp-*` token;
                      themes.css: the built-in themes besides Synth, on `<html data-theme>`;
                      kit.css: the Crush components and the motion catalogue; the bundled fonts and their licences
                      in fonts/, served at /fonts/ by the bundle); icons.tsx holds the Lucide icons as data
                      (lucide-LICENSE.txt), motion.ts the effective motion level; marks.tsx (state marks, badges,
                      counts, meter), logo.tsx (wordmark, mark and icon from the generated logo-paths.ts),
                      shell.tsx (mode line, command line and menu, brand, workspace sections, left
                      column rows), cards.tsx (tool, approval and ask cards), transcript.tsx (chat header, messages,
                      composer), feedback.tsx (toasts, empty states), live.tsx (live voice: spectrum, voice meter,
                      live panel), sheet.tsx (the phone's full-screen sheet), editor/ (CodeMirror and its theme),
                      pdf.tsx and pdf/viewer.tsx (`PdfView`: pdf.js, loaded on first use; its worker and data files
                      are copied to the bundle's /pdfjs/ by build.ts), image.tsx (`ImageView`: zoom, pan, compare
                      with HEAD), zoom.ts (the viewers' zoom steps and scroll anchoring)
ui/packages/app       @omfp/app: the web app; src/sdk (widget SDK `omfp/sdk`: host, frame store, hooks, commands,
                      widget hooks useStored/useJob/useToolRun, files: useFile/useOpenFile and the file tabs, live.ts:
                      the window's live voice session `LiveClient` and its browser `LiveMedia`, init.ts: the init files'
                      runs and their API, init-views.ts: the views they build, layout.tsx: the layout components of
                      TSX views, results.ts: the data hooks' last results), vm
                      (view models), widgets (built-ins, each folder copyable: value imports only from react,
                      omfp/sdk, omfp/kit; widgets/file is `omfp:file`, the CodeMirror viewer/editor), frame (left
                      column, mode line with the view tabs, the `:` line and file picker, `:widget add` (layout.tsx),
                      floating file panel and floating widget panels (floating-box.ts: their shared move/resize/minimise
                      state), toasts, dialogs, login.tsx (logging in to omp's providers, UI-380..UI-386);
                      responsive.ts(x): breakpoints, the strip, drawer and phone area
                      bar, the left column's splitter; commands-sheet.ts(x): the phone's commands sheet (UI-460);
                      init.tsx: loading the init files and their banner;
                      describe.ts(x): `:describe-key` / `:describe-command`; which-key.tsx: the which-key panel;
                      window-state.ts: the window state agents read; window-requests.tsx: the daemon's requests,
                      commands run and out-of-sight widget checks), view (layout tree, `:widget add`'s
                      manifest edit (edit.ts), reference resolution, widget slot,
                      the `main` slot's file tabs and the widget tabs reveals open there (opened-tab.tsx: their label
                      and close gestures); file-view.tsx: TSX views; kept.ts: the views kept mounted, hidden;
                      sizes.ts and splitter.tsx:
                      the window's splitter sizes), gallery
                      (the `/kit` page, UI-173: params, contrast measuring, the matugen-forest fixture theme, the
                      logo sheet), keymap (which-key.ts: the which-key panel's model and the leader groups), theme, strings; build.ts → dist/ (`just build-web`: index-html.ts, import
                      map, widget-exports.json, builtin-widgets/ from the BUILTINS table of builtins.ts, the favicon from docs/brand,
                      manifest.webmanifest with its app icons (UI-425), pdfjs/ from pdfjs-dist)
ui/test/dom.ts        happy-dom preload for the kit and app component tests
fixtures/omp/<ver>/      raw omp stdout frames (M0 smoke)
fixtures/threads/<ver>/  ThreadEvent JSONL recorded by daemon contract scenarios (reducer inputs)
fixtures/states/<ver>/   ThreadState JSONL, one line per fixture event: the Rust reducer's goldens for the TS port
fixtures/views/         cases.json: views manifests with their expected errors, checked by the daemon and the app
docs/brand/             the logo (UI-220): generate.py (`just brand`, from the bundled Commit Mono Bold), wordmark,
                        mark and icon SVGs, the 32 px favicon PNG, README screenshots
contrib/release/        release packages without Nix (M15, ENV-060..065): package-linux.sh, package-flatpak.sh,
                        package-macos.sh (lib.sh shared), omp.sha256 (the pinned omp release assets), the macOS
                        entitlements, notes.sh (release notes); run by .github/workflows/release.yml on `v*` tags
contrib/flatpak/        the Flatpak manifest (io.github.aciceri.omfp.yml), its launcher script, desktop entry and
                        AppStream metadata; in the sandbox the daemon runs omp, git and jobs on the host
                        (crates/omfp-daemon/src/flatpak.rs, ENV-064)
contrib/matugen/        a matugen template (omfp.css, config.toml) making an omfp theme from a Material 3 scheme
nix/                    the flake's packages: package.nix (`omfp`, `omfp-headless`), web.nix (the web bundle)
```

## Version pinning

omp is pinned in four places that must agree (checked by `cargo xtask check-versions`): the `oh-my-pi` flake input
tag, the `omp-rpc` git tag in `Cargo.toml`, `omfp_core::OMP_VERSION` and the release assets' tag in
`contrib/release/omp.sha256`. To upgrade, follow the playbook in `docs/specs/01-dev-environment.md` (ENV-020).

## omp facts learned the hard way (omp 18.18.6)

- Launch with `omp --mode rpc-ui --session-dir <dir> --continue --add-dir …`: `--add-dir` roots (and their
  `AGENTS.md`) are lost if the session is switched later with `open_session` (S-07). `open_session` on the same dir is
  then a no-op that reports `resumed`.
- The generated `SetEventFilterCommand` lacks `messageUpdates`; the daemon sends its own frame for delta mode.
- `/add-dir` and `/remove-dir` work over RPC as `prompt` (local command, no model turn), but omp refuses them while
  the agent streams ("Cannot add a directory while streaming."): the daemon syncs roots of a busy thread once its
  session settles.
- `--session-dir <dir> --continue` resumes the newest session of that directory whatever the cwd, so a thread keeps
  its session when it moves to another workspace folder (migration 3).
- The `ask` tool takes one HTML `<form>` and walks it with `select`/`input`/`editor` dialogs, not the RPC `ask` dialog.
- At startup omp always fetches its model catalog (`catalog.stencil.so` + keyless providers) and no setting disables
  it: contract tests run omp behind `DenyProxy` and fail on hosts outside `KNOWN_DISCOVERY_HOSTS`.
- omp reads project config from its cwd (the workspace folder) and loads a repo's `AGENTS.md` only when the repo is an
  `--add-dir` root; it does not resolve symlinked roots, and tools write through symlinks. Roots saved in a session
  survive a restart without their flag: detach with `/remove-dir` (S-10).
- `--append-system-prompt` disables every `APPEND_SYSTEM.md` (user and project); the daemon must concatenate them.
- `--approval-mode` takes `always-ask`, `always-ask-cached`, `write` or `yolo` only; on any other value (or any
  invalid flag value) omp prints a usage error to stderr and exits, which omp-rpc reports as `server closed`: the
  daemon validates the value at the protocol (DMN-210) and returns omp's stderr tail with start errors (DMN-212).
- `get_state` carries the tool definitions and the system prompt (about 35 KB); `get_session_stats` is the small read of
  a session's cost, tokens and context use (`thread.session`'s `usage`).
- Tools offered by default: read, bash, edit, ask, eval, glob, grep, task, wait, todo, web_search, write (`find`,
  `ast_grep` and `ast_edit` exist but are off by default — `find.enabled`, `astGrep.enabled`, `astEdit.enabled` —; no
  `fetch` tool: `read` fetches URLs). `edit` takes hashline input whose `[path#TAG]` needs a prior `read`.
  `web_search` failures and URL reads that fail (HTTP 404) end with `isError: false`; `task` runs subagents in the
  background unless `async.enabled: false`; open `todo` items make omp prompt the model again.
- omp-rpc's client is blocking and closes the omp process group on drop; command default timeout is 30 s.
- omp and omp-rpc disagree on number types: omp sends `auto_retry_start.delayMs` as a fractional, jittered backoff
  (`406.547…`) but omp-rpc types it `i64`, so the client reports the frame `Undecodable`. `omp::thread_event` rounds
  the fields listed in `FRACTIONAL_INTEGER_FIELDS` and decodes the frame again (DMN-042); add a field there when
  another typed frame turns up as `unknown`.
- omp exposes no MCP server state over RPC (MCP tools are `xd://` devices, absent from `get_state.dumpTools`). Over
  `prompt`, `/mcp test <name>` (omp-native configs only) answers one `command_output` frame before its response;
  `/mcp reconnect` is refused outside the TUI and `/mcp reload` reconnects nothing: restart the thread instead (M13).
  `/mcp reauth <name>` runs in the background (the prompt returns at once), opens the browser on omp's host itself and
  reports in later `command_output` frames (the OAuth URL, then `✓ Reauthorized "<name>" …`); the daemon reloads a
  thread whose MCP configs changed or whose server was reauthorized once it is free (DMN-200..204).
- `omp --mode rpc-ui` exits at startup (status 1, `No default model selected. Use /login, …` on stderr) when no model is
  usable: on a fresh install every thread start fails with that `stderrTail` (DMN-212). With an explicit `--model` of
  omp's bundled catalog (`anthropic/claude-sonnet-4-5`) it starts without any credential; the daemon's short-lived
  login omp does so (DMN-219). OAuth works over RPC: `get_login_providers`, `login { providerId }` (its flow arrives as
  `open_url`, `notify` and, for paste-code providers, `input` UI requests; secret prompts are refused; callback
  listeners take fixed loopback ports, so one login at a time), `get_logout_accounts` and `logout { providerId,
  credentialId }`. A running omp lists a new login's models only after a restart (DMN-224). `get_available_models`
  lists only the models omp can use (a credential, or a keyless provider): the daemon's `auth.status` count (DMN-235).
  A `prompt` failing omp's preflight (`No API key found for <provider>. … Use /login, …`) is answered twice with the
  same id (success, then the error) and ends with a `prompt_result` of status `error`.
- Our harness docs (`omp://…`) may describe a different omp version than the pinned one: verify against the source at
  `~/.cargo/git/checkouts/oh-my-pi-*/<rev>/` (docs in `docs/`, Rust client in `sdk/rust/omp-rpc/`).

## Tooling gotchas

- Browsers in tests, spikes and smoke checks always run **headless** (Chromium `--headless=new`; WebKitGTK under
  `xvfb-run`), never on the user's display: a visible window can be disturbed by the mouse and changes results.
- `WebKitWebDriver`'s wheel actions fire DOM `wheel` events in WebKitGTK but scroll nothing, so a test passes there
  without exercising scrolling: click the wheel buttons on the `xvfb-run` display with `xdotool` instead
  (`e2e::WebKitDriver::x_environment`, used by `e2e/tests/scroll.rs`). WebKitGTK then scrolls smoothly, a few pixels
  per frame, and dispatches scroll events at the next frame, after the wheel event and any render in between.
- The desktop window (`--features desktop`) is plain `cargo build`: no Tauri CLI, no JS, no IPC, no capabilities.
  `tauri-build` rewrites `crates/omfp/gen/schemas/` on every build (gitignored) and needs `crates/omfp/icons/icon.png`
  (RGBA). WebKitGTK has no `RTCPeerConnection` (the desktop's live voice uses omp's `live_start`, S-09) and keys
  `localStorage` by origin, port included. Parallel `xvfb-run -a` can pick the same display: `just test-desktop` runs
  one test at a time.
- macOS (M11): the Mac's login shell may not be POSIX, so run remote commands as `sh -s < script`; Nix there may need
  `--option builders ''`; a flake build sees git-tracked files only (`git add -A` in a scratch copy). The desktop tests
  open windows on the logged-in user's screen (from SSH too); microphone requests from an SSH session are denied
  without a dialog (TCC attributes them to sshd), so the microphone dialog is checked by hand through `omfp.app`.
  AppKit objects (the menu bar item) must be created and dropped on the main thread (`run_on_main_thread`).
- `bun run build` (`just build-web`) deletes `ui/packages/app/dist` first: a journey serving it while another build
  runs gets 404 pages, so `just test-e2e`/`test-desktop` serve a private bundle (`OMFP_WEB_BUNDLE`, TST-074).
- `convco` does not build from nixpkgs, so commitizen checks commit messages.
- `typos` skips `fixtures/` (streamed deltas split words).
- Component tests need a DOM: `bun run test` (in `ui/`) preloads `ui/test/dom.ts` (happy-dom) only for
  `packages/kit` and `packages/app` (their `bunfig.toml` does the same for a plain `bun test` inside them), because
  happy-dom replaces `WebSocket`, `fetch` and timers, which the client's transport suite needs from Bun. Never run the
  client tests with the preload.
- Never pass DOM elements to `expect(…).toEqual`/`toBe` in component tests (`expect(queryAllByTestId(…)).toEqual([])`,
  `expect(document.activeElement).toBe(el)`): when it fails, Bun's diff prints every own property of the element,
  React's `__reactFiber$…` included, and walks the whole fiber graph — gigabytes of output within seconds, the OOM
  killer then takes the session. Compare counts, test ids or text instead (`.length`, `getAttribute("data-testid")`).
- The web bundle has no user theme: `build.ts` adds `<link rel="stylesheet" href="/theme.css" data-omfp-theme>` after
  the bundle's CSS; the daemon serves `/theme.css` (UI-091), and the app replaces the link with a
  `<style data-omfp-theme>` when the theme changes.
- Fixtures are normalized (timestamps, durations, token counts, UUIDs, random ids, temp roots); if a re-record shows churn, add the
  volatile field to `omfp_testkit::normalize_frame` rather than accepting noise.
- TypeScript (`ui/`): one root `tsconfig.json` (strict, `exactOptionalPropertyTypes`, `noUncheckedIndexedAccess`),
  type-checked by `typescript` 7's native `tsc` (`bun run check`); no `any`. Biome comes from nixpkgs (the npm
  `@biomejs/biome` binary is dynamically linked against glibc and does not run on NixOS); its formatter skips the
  generated `generated.ts`. JS dependencies come from Nix (ENV-001): after changing `ui/package.json`/`bun.lock`
  (`bun add`, `bun update`), run `just gen-bun-nix` and re-enter the dev shell; `just ui-install` then installs
  offline from the shell's Bun cache (`OMFP_BUN_CACHE`). bun2nix carries nix-community/bun2nix#110 (`fetchpatch`) to
  read Bun 1.4's lockfile version 2.
