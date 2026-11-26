# Using omfp

How to install, run and configure omfp, a browser and desktop app for the
[oh-my-pi](https://github.com/can1357/oh-my-pi) coding agent (omp). What it is and why: [README](../README.md); how it
is built: [docs/specs](specs/README.md).

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

**Upgrading from shared configuration:** this is a fresh profile. Run `:login` in omfp again, or copy the logins
standalone omp already has: the welcome offers *Use my omp logins*, and `:import-logins` does the same at any time.
The copy makes a read-only copy of standalone omp's login database in omfp's private data folder, runs omp's own
`omp auth export` on it (deleting it right after) piped into `omp auth import -`, takes only model-provider logins (no
MCP credentials, settings, `models.yml` or `mcp.json`) and happens only when you ask — or an agent or MCP client with
Drive acting for you does. A browser (OAuth) login then lives
in both apps: a provider that rotates refresh tokens may sign one of them out later, and you log in there again.
Configure the new `agent/` directory explicitly for the models and user settings you need. omfp never
falls back to `~/.omp/agent` (or standalone omp's XDG state) and has no sharing switch. Login/logout and config
changes in one app do not change the other's stored state. Existing threads, session history and
workspace/repository configuration are preserved; `.omp/` project config still applies.

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
`agent/config.yml` (`astGrep: { enabled: true }`, `astEdit: { enabled: true }`). omp offers these tools as commands
its `bash` tool runs; omfp shows their calls as cards of the tools themselves.

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
the commands `mcp__omfp_call` and so on; `"timeout": 0` in the entry lets `omfp_thread_wait` wait longer than omp's 30 s
default. The entry's header is `"Bearer ${OMFP_TOKEN}"`: omp reads the token from the environment, so no file holds
it. The token grants full access to the daemon (including shell commands): `--literal-token` writes it into the entry
instead, but never put such an entry in a project's `.omp/mcp.json`, which may be committed. Threads and workspaces
such a client creates show its name as their origin, and its actions appear in the `omfp:activity` widget. See
[08-agent-control](specs/08-agent-control.md#mcp-server) for the details.

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

- **macOS (Apple silicon, macOS 14.5+)**: open `omfp-<version>-macos-arm64.dmg` and drag omfp to Applications. Unless a
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

### Running the daemon as a service

`omfp desktop` starts a daemon in the background when none runs, and it keeps running after the window closes. To
have it started with your session and restarted when it fails, run it as a systemd user service (Linux):

- **home-manager**: import the flake's module and enable it; it restarts the daemon whenever the package changes.

  ```nix
  imports = [ omfp.homeManagerModules.default ];
  services.omfp = {
    enable = true;
    tray = true;            # in the system tray, with the graphical session (needs the `omfp` package)
    # package = omfp.packages.${pkgs.system}.omfp-headless;   # a server: no window, no tray
    # extraArgs = [ "--bind" "127.0.0.1:7421" ];
  };
  ```

- **NixOS without home-manager**: `systemd.packages = [ omfp.packages.${pkgs.system}.omfp ];` and
  `systemd.user.services.omfp.wantedBy = [ "default.target" ];`. A profile install: `systemctl --user link
  ~/.nix-profile/lib/systemd/user/omfp.service && systemctl --user enable --now omfp`.
- **Elsewhere**: copy [contrib/systemd/omfp.service](../contrib/systemd/omfp.service) to `~/.config/systemd/user/`,
  put the path of your `omfp` in its `ExecStart` (in place of `@omfp@`), then `systemctl --user enable --now omfp`.

The service runs with the user manager's environment: tools the agents need (language servers, CLIs) must be on its
`PATH` (`systemctl --user show-environment`; `systemctl --user import-environment PATH` from your session, or
`Environment=` in the unit). Whoever started it, the daemon appends its log to `daemon.log` in the data directory (and
to stderr: the journal, for a service).

**After an upgrade**, `omfp daemon restart` switches the running daemon to the `omfp` you run it with, in place: the
daemon stops as on Quit (the turns it cuts get a Resume notice), then runs the new build in the same process, with the
same arguments, environment and address, so open windows reconnect by themselves and `omfp desktop` waits for it
instead of starting another daemon. It works from any shell, an omfp thread's own included (there `$OMFP_DATA_DIR`
names the daemon's data directory), and under systemd (`systemctl --user restart omfp` restarts onto the unit's
package instead). A daemon of a release before this command cannot restart this way: stop it once (the tray's Quit)
and start it again.

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
# delete_archived_after = "7d" # delete threads archived this long ago (m, h, d, w), checked hourly; "never" keeps
                               # them. Applies live, also from a window: `:set delete_archived_after=30d`

[ui]                           # also set from the window: `:set font_size=15`, `:density compact`
density = "normal"             # compact | normal
font_size = 17                 # px, 10–28
motion = "full"                # full | reduced | none
notifications = "background"   # background | always | never
theme = "synth"                # synth | catppuccin | gruvbox | tokyo-night | nord | solarized | rose-pine | graphite
                               # | paper | clay | primer | accent | matugen (the user theme below)
show_xdev_notices = false      # informational MCP tool command mount/unmount rows in the transcript
skin = "crush"                 # crush | boring: the calm, neutral look (`:set skin=boring`), with any theme
recent_threads = 20            # hidden threads a window keeps for instant switching (window memory, not omp), 0–100
                               # or "all" (every thread opened in the window; a long thread holds several MB)
live_voice = "sol"             # arbor | breeze | cove | ember | juniper | maple | sol | spruce | vale: the voice of
                               # live sessions (`:voice maple`), from the next session on

[ui.fonts]                     # prose, ui, code: CSS font families
prose = "Literata, serif"

[keymap]                       # normal mode (Threads and View); [keymap.threads], .view, .insert, .cmdline the others
"Space t" = "window.theme"     # a command id (`:describe-key`, `?` lists them)
"g g" = ""                     # removes a default binding
```

`[ui]`, `[keymap]` and `delete_archived_after` apply live; any other change to `[daemon]` needs a daemon restart. A
user theme is the CSS file `~/.config/omfp/theme.css` next to it, followed live (the matugen template in
`contrib/matugen` makes one); `omfp theme set <file>` pushes a theme to a running daemon instead. The windows show it
with the `matugen` theme (`:theme matugen`), over Synth; any other theme leaves it stored but unused.

## Status

Pre-1.0 and under active development: changes per release are in [CHANGELOG.md](../CHANGELOG.md) and on the
[releases](https://github.com/aciceri/omfp/releases) page. The specifications are in [docs/specs](specs/README.md).

## Licence

GPL-3.0-or-later, see [LICENSE](../LICENSE). The bundled fonts are under the SIL Open Font License
(`ui/packages/kit/fonts/`); the logo in [docs/brand](brand/README.md) is drawn from Commit Mono's outlines.
