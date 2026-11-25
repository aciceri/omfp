# AGENTS.md

Guidance for coding agents working on **omfp**, a GUI client (web + desktop) for the omp coding agent (oh-my-pi). One
binary, `omfp`: `omfp daemon` runs one `omp --mode rpc-ui` process per thread and serves the WebSocket API and the web
UI; `omfp desktop` opens one more window on the local daemon, starting it if needed (D-15, D-16). Threads run in
**workspace folders** whose top-level entries are repositories (D-11). The UI is TypeScript/React built and tested with
Bun (D-18). Direction (D-17): agents get full access to the protocol and build widgets in JavaScript; never design
something only the human UI can reach.

## Read first

- `docs/specs/README.md` — index, conventions (row style, retired IDs), decision log (D-01…D-22).
- The spec for the area you touch (`00`…`09`); `07-roadmap.md` is one line per milestone and spike.

## Ground rules

- Everything in **English**: code, comments, docs, commit messages.
- **TDD**: tests first. Every test covering a requirement has `// spec: ID[, ID…]` right above it, and every requirement
  row needs such a test that runs (checked by `cargo xtask spec-trace`; a rule no test can check is prose, not a row;
  never put the marker plus an ID inside a string literal). Deterministic and isolated tests
  only (TST-003): no sleeps (wait on conditions with timeouts), temporary directories only (never the user's), port 0,
  no network beyond loopback, no real models.
- **Conventional Commits** (checked by commitizen in the commit-msg hook). Commits are SSH-signed: if signing fails,
  stop and tell the user, never disable it.
- Prefer rebasing PR branches onto the latest `origin/master` rather than merging `master` into them. Keep rewritten
  commits signed; update a published branch with `git push --force-with-lease`, never plain `--force`, and wait for CI
  on the rebased head before merging the PR.
- Prefer `git absorb` for follow-up fixes to a PR's existing changes, then autosquash the fixups, rather than leaving
  separate correction commits. Keep unrelated changes in their own commits and preserve signatures when rewriting.
- Keep docs in sync: when behavior or a decision changes, update the spec rows in the same change. Spec rows follow the
  row style in `docs/specs/README.md`; implementation detail goes in the module's doc comment.
- No new dependencies without a reason; workspace dependencies live in the root `Cargo.toml`.
- No `Box::leak`, no placeholder stubs, no `unwrap()` in non-test code where an error can be returned.

## Commands (always inside the dev shell)

`just --list` describes every recipe. The ones you need most:

```sh
nix develop                      # or: nix develop -c sh -c '<command>'
just ci                          # everything CI runs (ci_recipes), on one web bundle
just check                       # lint, check-versions, spec-trace, deny, check-workflow
just test                        # fast Rust tests (no omp process)
just test-ts                     # bun tests in ui/ (reducer goldens, client, kit + app components, transport suite)
just test-contract               # contract tests against the pinned omp + fake LLM
just test-e2e                    # L5 journeys: private web bundle, headless Chromium; shots in a new /tmp/omfp-shots-*
just test-desktop                # the Tauri window under xvfb-run (TST-033), one test at a time
just mutants-gate; just cov      # TST-050 gates
just bench                       # e2e benchmarks by hand, not in `just ci`
just record-fixtures             # re-record fixtures/ (review the diff!), then `just gen-states`
just gen-states; just gen-ts     # golden reducer states; ui/packages/protocol/src/generated.ts
just gen-bun-nix                 # ui/bun.nix from ui/bun.lock, after every JS dependency change
just serve; just desktop         # try the app with your HOME and omfp's isolated omp profile
just brand                       # regenerate the logo files
```

Clippy runs with `pedantic` as errors. Notable: `assert_is_empty` forbids `assert!(x.is_empty())` — use
`assert_eq!(x, Vec::new())`; `large_enum_variant` is allowed only where we mirror omp-rpc unions.

## Layout

Every module starts with a doc comment (`//!` in Rust, a header comment in TypeScript) saying what it does and which
requirement IDs it implements; `rg 'spec: <ID>'` finds the tests of a requirement.

| Path | What |
| --- | --- |
| `crates/omfp-protocol` | UI↔daemon protocol types and their JSON Schemas |
| `crates/omfp-core` | Domain model and the pure reducer over omp events (no I/O) |
| `crates/omfp-daemon` | The daemon: supervisor of omp processes, workspaces, store, jobs, terminals, MCP server and client, agent tools, the `omfp://` host URIs (`src/widgets/*.md` are `omfp://docs/…`) |
| `crates/omfp` | The `omfp` binary: `daemon`, `desktop` (feature `desktop`, Tauri), `theme set`, `mcp config`, tray |
| `crates/fake-llm` | Scripted OpenAI-compatible server for tests |
| `crates/omfp-testkit` | Test helpers: isolated omp environment (`OmpEnv`), `DenyProxy`, temporary repos, test daemon, fixture normalization |
| `xtask` | `spec-trace`, `check-versions` |
| `e2e` | WebDriver journeys (L5) and benchmarks |
| `ui/packages/{protocol,core,client}` | Generated protocol types, the reducer port, the client (transport, session, model) |
| `ui/packages/kit` | React Aria components, tokens, themes, fonts |
| `ui/packages/app` | The web app: shell, frame, views, view models, built-in widgets (value imports only from `react`, `omfp/sdk`, `omfp/kit`, so each folder can be copied) |
| `ui/packages/site` | The project site |
| `ui/test/dom.ts` | happy-dom preload for the kit and app component tests |
| `fixtures/` | Recorded omp frames, thread events, golden states, views cases |
| `docs/brand`, `contrib/`, `nix/` | Logo; release scripts, Flatpak, systemd unit, matugen template; the flake's packages |

## Version pinning

omp is pinned in four places that must agree (checked by `cargo xtask check-versions`): the `oh-my-pi` flake input
tag, the `omp-rpc` git tag in `Cargo.toml`, `omfp_core::OMP_VERSION` and the release assets' tag in
`contrib/release/omp.sha256`. To upgrade, follow the playbook in `docs/specs/01-dev-environment.md` (ENV-020).

## omp facts (omp 19.2.0)

What you need before touching the code that talks to omp; details sit in the doc comment of the module handling each.

- Launch with `omp --mode rpc-ui --session-dir <dir> --continue --add-dir …`: `--continue` resumes the newest session
  of that directory that no other running omp has open, whatever the cwd; `--add-dir` roots (and their `AGENTS.md`)
  are lost if the session is switched later with `open_session` (S-07). omp reads project config from its cwd (the
  workspace folder), loads a repo's `AGENTS.md` only for `--add-dir` roots, never resolves symlinked roots, and keeps
  roots saved in a session across a restart without their flag: detach with `/remove-dir` (S-10).
- `/add-dir` and `/remove-dir` work over RPC as `prompt` (local command, no model turn) but are refused while the agent
  streams: the daemon syncs a busy thread's roots once its session settles.
- `--append-system-prompt` disables every `APPEND_SYSTEM.md` (user and project); the daemon concatenates them.
- TUI-only builtins (`/settings`, `/login`, `/fork`, …) are not announced and, sent as `prompt`, reach the model as
  text; refusals of announced commands come as `command_output`.
- Run lifecycle: `agent_end` with `isTerminal: false` is a pause; every run ends with exactly one terminal
  `agent_end`; `session_settled` follows it. The reducer reads busy/settled from `agent_start`/`session_settled` only,
  never `agent_end`. `abort_and_restore_queue` with a non-empty queue detaches the running `bash`/`eval`, which goes on
  as a background job instead of being killed with the turn.
- The `ask` tool sends its whole HTML `<form>` as one `form` UI request; the host answers `{ data }` or cancels, and omp
  fails the whole call on a reply of the wrong shape with no second round, so the window runs omp's check first. A
  broken rule (`required`, `pattern`, a range) is no refusal: omp's dialogs only flag it (UI-074).
- `--approval-mode` takes `always-ask`, `always-ask-cached`, `write` or `yolo` only. On a bad flag value, no usable
  model (`No default model selected`) or an unrestorable saved model (`SessionModelRestoreError`), omp exits at startup
  with the reason on stderr, which omp-rpc reports as `server closed`: the daemon validates first and returns omp's
  stderr tail (DMN-210, DMN-212, DMN-337).
- Discoverable tools (`lsp`, `ast_grep`, `ast_edit`, `debug`, MCP tools) are commands of the `bash` tool
  (`tools.commands`, on by default): the model runs `<tool> -j '<JSON>'` and the bash result's `details.toolCommands`
  lists each dispatch. omp exposes no MCP server state over RPC. Language servers come from the daemon's `PATH`.
- omp leaves the tool-call intent field out of RPC sessions unless `PI_INTENT_TRACING=1`, which the daemon sets.
- At startup omp always fetches its model catalog and no setting disables it: contract tests run omp behind
  `DenyProxy` and fail on hosts outside `KNOWN_DISCOVERY_HOSTS`. On an EC2 host omp takes the instance role as an AWS
  credential, so `OmpEnv` sets `AWS_EC2_METADATA_DISABLED=true` (TST-022).
- omp-rpc's client is blocking, closes the omp process group on drop, and times commands out after 30 s. Where omp and
  omp-rpc disagree on number types the frame is `Undecodable`: add the field to `FRACTIONAL_INTEGER_FIELDS`.
- omp creates `agent.db`'s `settings` table with a check, then a `CREATE TABLE` outside any transaction: two omps
  starting at once on a fresh profile race and the loser exits (`table settings already exists`, seen as `omp did
  not become ready: server closed`). The daemon starts a profile's omps one at a time until one started (DMN-913).
- Our harness docs (`omp://…`) may describe a different omp version than the pinned one: verify against the source at
  `~/.cargo/git/checkouts/oh-my-pi-*/<rev>/` (docs in `docs/`, Rust client in `sdk/rust/omp-rpc/`).

## Tooling gotchas

- Browsers in tests, spikes and smoke checks always run **headless** (Chromium `--headless=new`; WebKitGTK under
  `xvfb-run`), never on the user's display: a visible window can be disturbed by the mouse and changes results.
- `WebKitWebDriver`'s wheel actions fire DOM `wheel` events in WebKitGTK but scroll nothing: click the wheel buttons on
  the `xvfb-run` display with `xdotool` instead (`e2e::WebKitDriver::x_environment`). WebKitGTK then scrolls smoothly,
  a few pixels per frame, and dispatches scroll events at the next frame.
- The desktop window (`--features desktop`) is plain `cargo build`: no Tauri CLI, no JS, no IPC, no capabilities.
  `tauri-build` rewrites `crates/omfp/gen/schemas/` on every build (gitignored) and needs `crates/omfp/icons/icon.png`
  (RGBA). WebKitGTK has no `RTCPeerConnection` (the desktop's live voice uses omp's `live_start`, S-09) and keys
  `localStorage` by origin, port included. A page wheel handler with side effects of scrolling must skip `ctrlKey`
  (UI-283). Parallel `xvfb-run -a` can pick the same display: `just test-desktop` runs one test at a time.
- macOS: the Mac's login shell may not be POSIX, so run remote commands as `sh -s < script`; Nix there may need
  `--option builders ''`; a flake build sees git-tracked files only (`git add -A` in a scratch copy). The desktop tests
  open windows on the logged-in user's screen (from SSH too); microphone requests from an SSH session are denied
  without a dialog, so the microphone dialog is checked by hand through `omfp.app`. AppKit objects (the menu bar item)
  must be created and dropped on the main thread (`run_on_main_thread`).
- Component tests need a DOM: `bun run test` (in `ui/`) preloads `ui/test/dom.ts` (happy-dom) only for
  `packages/kit` and `packages/app`, because happy-dom replaces `WebSocket`, `fetch` and timers, which the client's
  transport suite needs from Bun. Never run the client tests with the preload.
- Never pass DOM elements to `expect(…).toEqual`/`toBe` in component tests: when it fails, Bun's diff walks React's
  whole fiber graph — gigabytes of output, then the OOM killer. Compare counts, test ids or text instead.
- Fixtures are normalized (timestamps, durations, token counts, UUIDs, random ids, temp roots); if a re-record shows
  churn, add the volatile field to `omfp_testkit::normalize_frame` rather than accepting noise.
- TypeScript (`ui/`): one root `tsconfig.json` (strict, `exactOptionalPropertyTypes`, `noUncheckedIndexedAccess`),
  type-checked by the native `tsc` (`bun run check`); no `any`. JS dependencies come from Nix (ENV-001): after
  changing `ui/package.json`/`bun.lock`, run `just gen-bun-nix` and re-enter the dev shell; `just ui-install` then
  installs offline from the shell's Bun cache.
