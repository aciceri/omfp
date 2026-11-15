# omfp — Specifications

A GUI client for [oh-my-pi](https://github.com/can1357/oh-my-pi) (omp), in the spirit of Orca and T3 Code: it runs many
agent threads in parallel. A thread is not tied to a repository: it runs in a **workspace** folder whose entries are the
repositories it works on (links, clones or worktrees, possibly of several projects, e.g. frontend + backend). It ships
as a web app in the browser and as a desktop app (M7), both the same TypeScript UI served by a Rust daemon (D-18).
Long term, the interface is meant to be **driven by the agents themselves** (spawning threads, publishing views) — see
[08](08-agent-control.md).

## Documents

| # | Document | Contents |
| --- | --- | --- |
| 00 | [Overview and architecture](00-overview.md) | Goals, non-goals, components, crates, flows |
| 01 | [Development environment and versions](01-dev-environment.md) | Flake, toolchain, pinning of omp / omp-rpc |
| 02 | [Daemon](02-daemon.md) | Threads, workspaces, omp process supervision, persistence |
| 03 | [UI ↔ daemon protocol](03-protocol.md) | WebSocket, snapshot/delta, commands, interactive requests |
| 04 | [UI (TypeScript)](04-ui.md) | Views, state, tool renderers, web vs desktop |
| 05 | [Terminal](05-terminal.md) | Reusable daemon-owned PTYs; removed omp terminal-handoff requirements |
| 06 | [Testing strategy (TDD)](06-testing.md) | Process, layers, fake LLM, E2E, mutation testing |
| 07 | [Roadmap](07-roadmap.md) | Milestones with acceptance criteria |
| 08 | [Agent control](08-agent-control.md) | Agents spawning threads and drawing UI, their tools and the MCP server; constraints that apply from day one |
| 09 | [Visual design: Crush](09-design.md) | Design principles, tokens, palette, fonts, motion, component specs, `/kit` gallery |

## Conventions

- **MUST / SHOULD / MAY** as in RFC 2119.
- Every requirement has a stable ID (`DMN-012`, `PRT-003`, …). Tests covering it cite the ID in their name or in a
  `// spec: DMN-012` comment. A requirement without a test counts as not implemented (see
  [06](06-testing.md#traceability)).
- Prefixes: `ARC` architecture, `ENV` environment, `DMN` daemon, `PRT` protocol, `UI` interface, `TRM` terminal,
  `TST` testing, `AGT` agent control.
- Facts about omp cite upstream documentation (`docs/*.md` in the oh-my-pi repo, at the tag pinned in `flake.lock`).
  Unverified points are marked **[VERIFY]** and have a spike in the roadmap.
- Docs, plans, code, comments and commit messages are in English.

## Decision log

| ID | Decision | Rationale | Rejected alternatives |
| --- | --- | --- | --- |
| D-01 | Integrate with omp over **RPC** (`omp --mode rpc-ui`), one process per thread | Complete protocol, official Rust client (`omp-rpc`), per-thread isolation | ACP (too narrow), in-process SDK (Bun/TS only), TSP (single vendor, we would have to write the renderer) |
| D-02 | ~~UI in **Dioxus** 0.7~~ — **superseded by D-18** (M3). Dioxus built the M2 web UI | One codebase in Rust for web and desktop | GPUI (web support is experimental), Leptos + Tauri (two pieces to integrate) |
| D-03 | The **daemon** is the only component talking to omp; the UI only ever talks to the daemon over WebSocket, desktop included | One transport path to test; several clients at once | Desktop UI talking to omp directly (two code paths) |
| D-04 | Thread state is derived by a **pure reducer** (`omfp-core`, Rust) in the daemon; the UI runs a TypeScript port of it (D-18) that must produce the same state as the Rust reducer on every recorded fixture (golden states checked in CI) | Testable in isolation (TDD, property tests); snapshots for late-joining clients | State computed only in the daemon with a "dumb" UI (more bandwidth, less reuse); the Rust reducer compiled to wasm for the UI (state copied across the wasm boundary on every streamed delta) |
| D-05 | Worktrees are managed **by the daemon** through the `git` CLI, not by omp (`/wt`); they live inside workspace folders, and the agent may create them too (D-11) | We own the lifecycle; testable with temporary repos; agents or the user may have created worktrees elsewhere | omp's `/wt` (couples worktree to session, not drivable over RPC) |
| D-06 | Anything RPC does not expose (plan mode, `/vibe`, `/settings`, extensions with custom UI) could have gone through a **terminal panel** running interactive omp on the same session ([05](05-terminal.md)). **Removed** (QA, PRT-135): the user does not rely on those commands, and the never-implemented singular `terminal.*` methods, permission and thread process state stay absent. The independent daemon-owned PTY widget added later (#80, TRM-100..105) runs arbitrary commands and never takes over an omp session. | Less dead surface for agents and clients to stumble on; a general PTY remains composable through the protocol and widgets | Keeping the refused omp `terminal.*` handoff as a placeholder |
| D-07 | Pinned versions: omp (flake input at a release tag) = `omp-rpc` (git dependency at the same tag) | The RPC protocol changes every release; mismatched versions drop frames | Tracking `main` (unstable), hand-copied omp-rpc |
| D-08 | **TDD** is mandatory; a scripted fake LLM written in Rust makes tests against the real omp deterministic | Fast, repeatable tests without real model calls | Only protocol mocks (would not catch omp breaking changes) |
| D-09 | Daemon persistence in **SQLite** (`rusqlite`, bundled) | Simple, transactional, no external service | JSON files (no transactions), Postgres (extra service) |
| D-10 | Styling with **plain CSS** and custom properties (design tokens) shared by the shell and the widget kit; TypeScript tooling is **Bun** only (bundler, test runner, package manager via `bun2nix`), no Node toolchain | Fewer moving parts in the dev shell; omp is built with Bun already | Tailwind (needs its binary and config); Node + Vite |
| D-11 | **A workspace is a folder, its repositories are its top-level entries** (revised before M3, spike S-10). The thread's omp cwd is its workspace folder; several threads may share one. Repositories inside it are real clones, git worktrees, or **symlinks** to projects elsewhere; the daemon discovers them by scanning the folder (the filesystem is the source of truth), so an agent adds or removes repos with ordinary tools (`ln -s`, `git clone`, `git worktree add`) and the UI shows them; the user does the same from the UI. Each repo is passed to omp with `--add-dir` (live `/add-dir` / `/remove-dir`). Worktrees are optional. The daemon's conventions reach the agent through `--append-system-prompt`, never through files written into the workspace | Matches how the user works: a session starts in a workspace and the agent decides which repos it needs; the agent starts inside the project; no hidden hierarchy, everything visible in the UI | Thread home as cwd with workspaces many-to-many (M1/M2: the agent started in an empty dir, too many steps); an `AGENTS.md` written into the workspace; mandatory daemon-managed worktrees |
| D-12 | **API-first**: every capability of the UI is a daemon protocol method usable by any authorized client, including (later) agents | Agent control (08) becomes an adapter, not a rewrite | UI-only features |
| D-13 | **Network exposure is the deployer's job**: the daemon listens on a configurable address and port, with the token as its only authentication; tunnels, reverse proxies and TLS are configured by whoever installs it. The browser `Origin` check accepts the daemon's own origin plus an explicit allow-list (`allowed_origins` / `--allowed-origin`, exact origins, no wildcard) for a proxy that rewrites `Host` (M9, DMN-140, PRT-070) | Keeps the daemon small; every setup (SSH, Tailscale, reverse proxy) works the same way | A built-in login system or TLS termination; wildcard origins |
| D-14 | **omp is bundled** with the app at the pinned version; a different binary is only used when given explicitly (`--omp-bin`) and still passes the version check (DMN-001) | The client is generated for one omp version; bundling removes "wrong omp on PATH" failures | Relying on the system omp |
| D-15 | **One binary with subcommands**: `omfp daemon` runs the headless daemon (serving the web UI); `omfp desktop` opens a desktop window connected to the local daemon, starting one if none is running (one command for users who do everything on one machine). The web UI is a TypeScript bundle (D-18) the binary serves | Simple installation and UX; one artifact to ship next to the bundled omp | Separate daemon and desktop binaries |
| D-16 | **Many windows, one daemon** (like Zen browser windows): every `omfp desktop` opens a new window on the same local daemon, so all windows share threads and state, while each window chooses its own thread and layout. The daemon started by `omfp desktop` runs as a background process, so closing a window never stops the others | Several views of the same work without duplicating threads or omp processes | A daemon per window (duplicate threads); the first window owning the daemon (closing it would break the others) |
| D-17 | **Agents have full access and build UI in JavaScript**: every protocol method is an agent tool; agents mount arbitrary HTML/CSS/JS pieces of UI with an `omfp` SDK (full protocol, live sources, storage) and an optional widget kit; full-access ("yolo") is the default, limits are opt-in | Maximum freedom for agents; JavaScript is what models write best and runs in every client webview | A fixed declarative vocabulary (too limiting); a sandboxed script engine in the daemon with restricted access |
| D-18 | **UI in TypeScript** (from M3): a **React 19** web app (chosen over Preact by spike S-11) bundled and tested with **Bun**, with **React Aria Components** for behaviour and accessibility and our own token-based CSS (UI-091, UI-093); protocol types generated from the daemon's JSON Schemas (AGT-007); a TypeScript client and reducer port (D-04); desktop with **Tauri 2** (M7), which also uses the system webview. The daemon, protocol, fake LLM, testkit and L5 harness stay in Rust | With widgets (D-17, M4) most of the UI is JavaScript written by us and by agents: one language for the shell, the built-in widgets and agent widgets, so agents can copy and change any of them; models write React/TypeScript far better than Dioxus; the React ecosystem (virtualization, highlighting, charts, accessible primitives) works without adapters; one React instance is shared with every widget through an import map | Keeping Dioxus with JS widgets in frames (two UI stacks, agents cannot touch the shell); Preact (S-11: Preact 10 crashes with React Aria, Preact 11 needs three patches to React Aria internals and changes portal event bubbling, for 58 KB gzip less); Svelte or Solid (less known to models, compiler plugins under Bun); Electron (heavier than Tauri); styled component kits such as Mantine or shadcn/ui (their theme systems fit matugen and agent widgets worse) |
| D-19 | **Widgets are ES modules served by the daemon** (M4, S-12): workspace and user widgets are folders of TypeScript/JSX that the daemon transpiles with `oxc` (`=0.158.0`) on request, under versioned URLs behind a per-start secret path given in `system.hello`; the page's import map points `react`, `omfp/sdk` and `omfp/kit` at the app bundle's own modules; changes reach the window through the rescan and poll path (DMN-023). Built-in widgets are written the same way so they can be copied and replaced | Agents write TSX naturally; no build step or Node toolchain in a workspace; one React and one SDK instance shared with the shell; compile errors become modules shown in place | Buildless `htm` modules only (works, but models write JSX better; `.js` widgets still load as-is); a bundler per widget (slow, a toolchain on the daemon host); iframes per widget (no shared React, no shared SDK state) |
| D-20 | **Visual style: Crush, our own** ([09](09-design.md)): monospace chrome (Commit Mono) and serif prose (IBM Plex Serif), both bundled; a small glyph vocabulary (◆ ▸ ● ╭─); gradients and colors derived only from Material 3 roles (default palette Synth, so matugen themes recolor everything); motion as the identity, switchable with `[ui] motion`; hand-written CSS on tokens (D-10), **no CSS framework** or component library styles | Picked by the user from a design lab of several directions; reads like the terminal tools omfp sits beside while keeping long text comfortable; deriving from M3 roles keeps any theme coherent | Other lab directions (Paper, Terminal, Workbench); Tailwind or a styled component library (D-10); system fonts only (look varies per machine) |

## Open questions

1. **Reusable agent-made UI** beyond the workspace: sharing widgets across machines and whether pieces may run
   without a window. Windows, mounts, the user's widget library and agent control over the protocol and MCP were
   settled in M10 and M13 ([08](08-agent-control.md)); the views manifest, widget loading, the SDK and the kit within
   a workspace in M4 (D-19, AGT-010..025).

Resolved: desktop platforms — Linux (x86_64) and macOS on Apple silicon (M11, M15); Windows not planned; license —
GPL-3.0-or-later (`LICENSE`).
