# omfp overview

omfp is a GUI client for the [omp](https://github.com/can1357/oh-my-pi) coding agent (oh-my-pi), in the browser and as
a desktop app on Linux and macOS. A daemon runs one omp process per thread, each in a workspace folder whose entries
are your repositories, and serves a web UI over WebSocket; threads keep running when the window closes. The agent can
build widgets for the UI itself, and you can talk to it with live voice.

## Features

- Threads grouped by workspace folder; omp gets every repository of the folder as a root, with its `AGENTS.md`.
- Transcript with streaming Markdown, thinking blocks and tool cards for `bash`, `read`, `edit`/`write`, `grep`,
  `glob`, `ast_grep`/`ast_edit`, `lsp`, `todo`, `web_search`, `task` and more; approvals and `ask` forms answered in
  place; queued follow-ups and steering.
- Views as layout trees in `.omfp/views.json` (or TSX views and init files), written by the agent, with `:widget add`
  in the window; widgets are React components the
  agent writes and checks without a browser (`omfp/sdk`), next to built-ins for changes and diffs, files (CodeMirror
  with vim keys; a file outside the workspace folder, such as an agent's worktree elsewhere, opens by its absolute path,
  read-only), subagents, todos and the session tree.
- Live voice through omp's live session, with a transcript and mute.
- Keyboard first: vim-style modes, a `:` command line, `?` help, per-workspace key bindings an agent can add.
- Desktop app (Tauri: WebKitGTK on Linux, WKWebView on macOS) sharing a background daemon with the browser, with a tray
  icon (a menu bar item on macOS); system notifications and in-window notices when a thread needs you.
- Themes from tokens: built-in light and dark, matugen output or your own CSS, with a contrast guard for unreadable
  roles. Narrow windows and phones get a strip and a bottom bar.

## Run

Everything runs inside the Nix dev shell:

```sh
nix develop
just serve      # build the web UI and run the daemon with omfp's isolated omp profile; open the printed URL
just desktop    # the desktop window on your local daemon (started in the background if none runs)
```

`just --list` shows the other recipes (tests, lints, fixtures); [AGENTS.md](../AGENTS.md) is the contributor guide.

## Isolated omp profile

omfp uses `<data-dir>/omp-profile`, with user configuration in `agent/`: `config.yml` (settings/model roles),
`models.yml` (custom providers), `agent.db` (provider and MCP credentials), agents, extensions, skills and
`APPEND_SYSTEM.md`. Runtime/cache state stays under the profile too. The default location is
`$XDG_DATA_HOME/omfp/omp-profile` or `~/.local/share/omfp/omp-profile` on Linux, and
`~/Library/Application Support/omfp/omp-profile` on macOS. `omfp daemon --data-dir <path>` relocates the whole profile.

**Upgrading from shared configuration:** this is a fresh profile. Run `:login` in omfp again and explicitly configure
the new `agent/` directory for the models and user settings you need. omfp never automatically imports
or copies old credentials/configuration, never falls back to `~/.omp/agent` (or standalone omp's XDG state), and has
no sharing switch. Login/logout and config changes in one app do not change the other's stored state. Existing
threads, session history and workspace/repository configuration are preserved; `.omp/` project config still applies.

This covers every daemon-owned omp, including login, model discovery, title generation and session probes.
`HOME`, cwd and repository roots are unchanged; explicit API keys in the daemon's environment still work. This is
user-profile isolation, not a filesystem sandbox: agent tools still have the user's normal file access.

## MCP servers

MCP servers belong to a workspace: put them in its `.omp/mcp.json` (or the project configs omp imports: `.claude/`,
`.cursor/mcp.json`, `.vscode/mcp.json`, a root `mcp.json`). omfp lists no user-level MCP config and refuses the `/mcp`
commands that would write one (`--scope user`, `/mcp enable|disable` of a server the workspace's `.omp/mcp.json` does
not define). Log in to a server with `/mcp reauth <name>` in a thread of the workspace; the login is kept in the
profile's `agent.db`. Widgets call the workspace's `http` and `sse` servers through the daemon (`useMcp`,
`useMcpTool`, `mcp.call`), with no thread running: the daemon uses that same login and renews it the way omp does.

## Language servers and code intelligence

omp's LSP integration is on by default (`lsp.enabled`): the agent asks language servers for diagnostics,
definitions, references, symbols, hover, renames and code actions, and omfp shows their answers as cards whose paths
open the file at the line. omp starts the servers itself, **from the daemon's environment**: a server must be on the
`PATH` of the process that started `omfp daemon` (the desktop app's background daemon, a service unit, the shell you
ran it from), not merely on your dev shell's. When omp finds none for a file, the card says so. The structural search
and rewrite tools `ast_grep` and `ast_edit` are off by default in omp; turn them on in the profile's
`agent/config.yml` (`astGrep: { enabled: true }`, `astEdit: { enabled: true }`). omp offers these tools as `xd://`
devices; omfp shows their calls as cards of the tools themselves.

## Agents outside omfp (MCP)

Threads run by omfp get its agent tools (`omfp_call` for every protocol method, `omfp_thread_wait`, the UI mount and
component tools) directly. Any other agent, such as an omp session in a terminal or another MCP client, reaches the
same tools through the daemon's MCP server at `/mcp` (Streamable HTTP, with the daemon's token as bearer):

```sh
omfp mcp config > ~/.omp/agent/mcp.json   # or merge its "omfp" entry into an existing mcp.json
export OMFP_TOKEN="$(cat ~/.local/share/omfp/token)"   # where omp runs, e.g. in your shell profile
```

This example deliberately configures a **standalone** omp client, not omfp's internal profile. omfp's own threads
already receive the host tools and do not need this MCP entry.

The command prints the entry for the running local daemon (`--daemon <url>` for another one). omp then offers the tools as
`xd://mcp__omfp_call` and so on; `"timeout": 0` in the entry lets `omfp_thread_wait` wait longer than omp's 30 s
default. The entry's header is `"Bearer ${OMFP_TOKEN}"`: omp reads the token from the environment, so no file holds
it. The token grants full access to the daemon (including shell commands): `--literal-token` writes it into the entry
instead, but never put such an entry in a project's `.omp/mcp.json`, which may be committed. Threads and workspaces
such a client creates show its name as their origin, and its actions appear in the `omfp:activity` widget. See
[08-agent-control](specs/08-agent-control.md#mcp-server-m10) for the details.

## Install

**With Nix** (Linux, macOS on Apple silicon): `nix profile install github:aciceri/omfp` (or `nix run
github:aciceri/omfp -- desktop`). It brings the right oh-my-pi with it.

`nix build .#omfp` (or the flake's `packages.<system>.omfp` in your configuration) builds `bin/omfp` with the desktop
window, the web UI and the pinned omp, on `x86_64-linux`, `aarch64-linux` and `aarch64-darwin`; `.#omfp-headless` is
the daemon alone, for servers. On macOS the package also holds `Applications/omfp.app`: in `environment.systemPackages`
of nix-darwin it shows up in `/Applications/Nix Apps`, in `home.packages` of home-manager in
`~/Applications/Home Manager Apps`, so Spotlight and Launchpad find it. The app is ad-hoc signed: macOS asks for the
microphone again after each update. The daemon keeps its data in `~/.local/share/omfp` on Linux and
`~/Library/Application Support/omfp` on macOS (`XDG_DATA_HOME` wins on both); the config and theme files are in
`~/.config/omfp` (see [Settings](#settings)).

**Without Nix**: download from the [releases](https://github.com/aciceri/omfp/releases); each one comes with its own
oh-my-pi, and you need git.

- **macOS (Apple silicon, macOS 14+)**: open `omfp-<version>-macos-arm64.dmg` and drag omfp to Applications. Unless a
  release says it is notarized, macOS refuses to open it the first time: open it once, then allow it in System
  Settings → Privacy & Security → Open Anyway (or run `xattr -dr com.apple.quarantine /Applications/omfp.app`).
- **Linux desktop (x86_64, Flatpak)**: `flatpak install --user omfp-<version>-linux-x86_64.flatpak`, then start omfp
  from your app menu or with `flatpak run io.github.aciceri.omfp desktop`. The window lives in the sandbox, but
  oh-my-pi, git and the commands of jobs run on your machine on purpose (they need your tools and repositories), so
  the app has full access to your account. Keep workspaces and repositories in your home directory. On NixOS the
  bundled oh-my-pi cannot run: set `omp_bin` in `~/.config/omfp/config.toml`, or use Nix.
- **Linux server (x86_64, glibc 2.34+)**: unpack `omfp-<version>-linux-x86_64.tar.gz` and run `./omfp daemon`, then
  open the URL it prints (the browser UI, no desktop window).

To check a download, compare `shasum -a 256 <file>` (macOS) or `sha256sum <file>` (Linux) with its line in the
release's `SHA256SUMS`.

## First run

A fresh oh-my-pi is logged in nowhere, so no thread can start yet: the window welcomes you with **Log in to a
provider** (or type `:login`); pick your plan and finish in the browser, and oh-my-pi keeps the login for every thread.
To use API keys instead, see [Logging in](../crates/omfp-daemon/src/widgets/shell.md#logging-in).

## Settings

omfp reads `$XDG_CONFIG_HOME/omfp/config.toml`, else `~/.config/omfp/config.toml` (also on macOS and in the Flatpak;
`omfp daemon --config <file>` picks another). The file is optional; unknown keys are errors.

```toml
[daemon]                       # the `omfp daemon` options, snake case; a command-line flag wins over the file
bind = "127.0.0.1:7420"
# data_dir, workspaces_root (default ~/.omfp), omp_bin, static_dir, theme_file: paths, `~` allowed
# allow_omp_mismatch = true    # run with an omp that is not the pinned release (the UI warns)
# idle_timeout_minutes = 15, max_processes = 64, event_buffer = 2000
# allowed_origins = ["https://omfp.example.ts.net"]   # the UI's URL behind a tunnel or reverse proxy
# title_generation = false     # no generated thread titles

[ui]                           # also set from the window: `:set font_size=15`, `:density compact`
density = "normal"             # compact | normal
font_size = 17                 # px, 10–28
motion = "full"                # full | reduced | none
notifications = "background"   # background | always | never
theme = "synth"                # synth | catppuccin | gruvbox | tokyo-night | nord | solarized | rose-pine | graphite
                               # | paper | clay | primer | accent | matugen (the user theme below)
show_xdev_notices = false      # informational MCP mount/unmount rows in the transcript
skin = "crush"                 # crush | boring: the calm, neutral look (`:set skin=boring`), with any theme
recent_threads = 20            # hidden threads a window keeps for instant switching (window memory, not omp), 0–100
                               # or "all" (every thread opened in the window; a long thread holds several MB)

[ui.fonts]                     # prose, ui, code: CSS font families
prose = "Literata, serif"

[keymap]                       # normal mode; [keymap.insert] and [keymap.cmdline] for the others
"Space t" = "window.theme"     # a command id (`:describe-key`, `?` lists them)
"g g" = ""                     # removes a default binding
```

`[ui]` and `[keymap]` apply live; a change to `[daemon]` needs a daemon restart. A user theme is the CSS file
`~/.config/omfp/theme.css` next to it, followed live (the matugen template in `contrib/matugen` makes one); `omfp
theme set <file>` pushes a theme to a running daemon instead. The windows show it with the `matugen` theme (`:theme
matugen`), over Synth; any other theme leaves it stored but unused.

Informational MCP mount/unmount notices are hidden in new and existing transcripts by default. Run
`:set show_xdev_notices=true` to reveal retained notices immediately; `:set show_xdev_notices=false` hides them
again. The API uses `settings.set { ui: { showXdevNotices: true } }`. This changes presentation only: events,
thread state, warnings/errors, other sources, startup inventory records, MCP tools and omp configuration stay intact.

## Status

Pre-1.0 and under active development: the done milestones and the next ones are in the
[roadmap](specs/07-roadmap.md). The specifications are in [docs/specs](specs/README.md).

## Licence

GPL-3.0-or-later, see [LICENSE](../LICENSE). The bundled fonts are under the SIL Open Font License
(`ui/packages/kit/fonts/`); the logo in [docs/brand](brand/README.md) is drawn from Commit Mono's outlines.
