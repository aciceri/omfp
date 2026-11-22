# Changelog

User-facing changes, in English. Significant changes go under `Unreleased`, grouped into Added, Changed, Fixed,
Removed or Security; omit empty groups and internal refactoring. Include migration steps for incompatible changes.
At release time, move these entries under `## <version> — YYYY-MM-DD` and leave a fresh `Unreleased` section above it.
The version's section is also the source of the GitHub release notes.

## Unreleased

### Added

- A turn cut by a daemon restart (an upgrade, `omfp desktop` restarting its background daemon, Quit) is no longer
  lost silently: the daemon remembers which threads were working when it stopped, and once it runs again their chat
  ends with a notice saying the last turn was interrupted, with a Resume button (`:resume`) that restarts the agent and
  tells it to check what was in progress and continue. Any message you send clears the notice too. Agents see it as
  `interruptedAt` on the thread's summary and resume with `thread.resume` (#264).

### Fixed

- A repository without commits (right after `git init`) no longer stops the changes view of its whole workspace: it
  shows its branch and every file as added. A repository whose git status fails for another reason shows that error
  in `omfp:changes` while the others keep updating, and the daemon logs it once instead of every 30 s (#268).

## 0.4.2 — 2026-11-22

### Added

- The voice of live sessions is now a setting: `:voice maple` (or `:set live_voice=…`, `[ui] live_voice`) picks one
  of omp's nine voices — `arbor`, `breeze`, `cove`, `ember`, `juniper`, `maple`, `sol` (the default), `spruce`,
  `vale` — for every window, from the next session on; on a phone it is two taps in the Commands sheet. Agents set
  `ui.liveVoice` with `settings.set`, or pass `voice` to `thread.live_connect`/`thread.live_start` (#240).
- A new thread can start in live voice: the draft thread's composer has the mic button too. Pressing it opens the
  microphone, creates the thread (with what you typed, if anything, as its first message) and starts talking to it in
  one tap; a thread started by voice is named after the first thing you say (#243).
- `Alt-j` / `Alt-k` (`:next-thread` / `:prev-thread`) select the next or previous thread of the left column from
  anywhere, also while typing in the composer, whose focus moves to the new thread's composer. They go on into the
  next or previous workspace and wrap around at the ends, skipping archived threads, collapsed workspaces and the
  threads the column's filter hides. Rebindable in `keymap.json` (#217).
- A wide window's left column collapses to nothing with *Hide threads* (the button at the end of its logo's row),
  `Space b` or `:threads.collapse`, so the view takes the whole window; *Show threads* at the start of the mode line,
  `Space b` again or `Space t` (which also focuses it) bring it back. Each window remembers it across reloads (#216).
- Agents can draw a widget right in the chat with the `omfp_card` tool: a terminal to show you something, a chart, a
  question with its own controls. The card lives in the conversation only (never in your views), collapses once the
  turn ends and comes back live when you open it. With `wait` the agent waits for your answer, given through the
  widget (`useAnswer()` in `omfp/sdk`) or the card's Cancel, optionally with a timeout; meanwhile the thread needs
  you, as with a question (#229).
- omfp can reuse standalone omp's logins: on a fresh install the welcome offers *Use my omp logins (…)*, and
  `:import-logins` copies them at any time. Only model-provider logins are copied, and only when you ask (or an agent
  or MCP client acting for you does); MCP credentials, settings and model configuration stay behind. omfp never runs
  omp on standalone omp's own login database: it makes a read-only copy of it in omfp's private data folder, lets omp's
  own `auth export` read that copy, deletes it right after, and imports the provider logins with `auth import`.
  The copy waits up to 2 s for a write standalone omp has in progress, is taken again when omp saved meanwhile, and
  creates nothing beside omp's database, so a read-only omp folder works too.
  The database is found with omp 18.22.0's directory rules from omfp's environment: omp's `.env` files are not read,
  and in the Flatpak a database under the host's `$XDG_DATA_HOME/omp` is not found. A browser (OAuth) login then lives
  in both apps, so a provider that rotates refresh tokens may sign one of them out later: log in there again. Copying
  is refused while a login runs (#199).

### Changed

- The threads a prompt job makes no longer fill their workspace's thread list in the left column: they are listed
  under their job in `Jobs (N)`, whose row gets an Open thread button next to Run that selects the job's thread (the
  latest run's for a job making a new thread per run). The job's row is the current one while its thread is selected,
  and shows when one of its threads needs you or failed, as do the collapsed `Jobs (N)` marks; `Space a`, the thread
  picker and the filter still find them, while `j`/`k` and `Alt-j`/`Alt-k` skip them. Archived ones stay under
  Archived, and threads of a job no longer declared stay in the list (#219).
- The terminal (`omfp:terminal` and the kit's `TerminalEmulator`) follows the app's theme instead of drawing white on
  black: its background, text, cursor, selection and 16 ANSI colors come from the theme, in both skins and color
  schemes and with user themes too, and switching theme or color scheme repaints open terminals without restarting
  them. Widgets can still set their own colors through `options.theme` (#214).
- A window keeps the 20 threads you last switched away from followed (was 5), so going back to one shows it at once,
  and a kept thread no longer keeps its omp process alive: once idle it stops after the idle timeout like a thread no
  window shows, its transcript stays in the window with the stopped state, and it starts again when you use it.
  `[ui] recent_threads` (0–100, or `"all"` to keep every thread opened in the window; `:set recent_threads=<n>`)
  sets how many threads a window keeps; each costs the window memory (about its snapshot, a few MB for a very long
  thread, so `"all"` can reach gigabytes after hundreds of long threads, too much for a phone). Clients keep a thread
  subscription without watching it with `subscribe { topic, watch: false }` and switch it with the new
  `watch { topic, watch }` (#226).
- Opening a thread no longer starts its omp: only sending something does (a message, a `/` command, a model or
  thinking change, Restart), so browsing threads with `Alt-j`/`Alt-k` costs no omp processes. A thread shown after
  the daemon restarted shows its conversation, read from its session file, in the stopped state; its `/` completion
  offers the commands its omp last announced, which the daemon now keeps with the thread, and is asked only once you
  type `/`. `thread.commands` never starts omp any more, and a window asks the usable models of stopped threads once
  per login instead of starting a short-lived omp for every thread it shows (#231).

### Fixed

- The daemon gives back the memory of threads you looked at: a thread whose omp is stopped and that no window shows
  or keeps leaves memory after the idle timeout (`idle_timeout_minutes`, 15 by default), and showing it again reads
  it from its session file. On Linux the `omfp` binary now allocates with jemalloc, which returns freed memory to the
  system; before, a daemon that had shown a few long threads stayed at a gigabyte or more until restarted (#260).
- Boring skin: the focused composer shows one accent ring around its rounded box instead of a misaligned second
  ring around the text area inside it (#215).
- Copy and paste work in the terminal (`omfp:terminal`): Ctrl+Shift+C copies the selection and Ctrl+Shift+V pastes,
  Cmd+C and Cmd+V on macOS, in the browser and in the desktop window. Plain Ctrl+C and Ctrl+V still reach the program
  (#213).
- A terminal (`omfp:terminal`, the kit's `TerminalEmulator`) closed or re-created right after it opened, e.g. by
  switching tabs quickly, no longer reports an uncaught `TypeError` (`this._renderer.value.dimensions`) from
  xterm.js among the window's errors (#225).
- The Linux desktop window zooms like a browser: Ctrl with `+` (or `=`), `-` and `0`, and Ctrl with the mouse wheel,
  which no longer scrolls. An image viewer keeps Ctrl with the wheel for its own zoom; Ctrl with the wheel over a row
  of view tabs zooms too, in browsers as well. Ctrl with the wheel over a conversation that follows its newest output
  keeps following it (#202).

### Release limitations

- macOS downloads are signed ad-hoc and are not notarized unless the release explicitly says otherwise. On first
  launch, allow the app in System Settings → Privacy & Security → Open Anyway. Updates may request microphone
  permission again.
- The Flatpak deliberately runs oh-my-pi, git and job commands on the host with your account's permissions; it is
  not an isolation boundary. Keep workspaces under your home directory. On NixOS, use the Nix package or configure
  a compatible `omp_bin`. Job processes left behind after their shell exits are not killed on the host.
- Standalone downloads do not cover Windows, Intel Macs or ARM Linux.
- Multi-select prompts of the `ask` tool do not show yet which choices are checked: oh-my-pi 18.22 sends that state,
  but omfp's prompt card does not use it yet.
- oh-my-pi always loads its own user `agent/mcp.json` from omfp's profile: omfp never writes it and neither lists nor
  calls its servers, but a thread's agent still sees a hand-written one.

## 0.4.1 — 2026-11-19

### Fixed

- A daemon started with `omfp daemon` from a terminal that was closed later no longer fails with `internal error:
  task … panicked with message "failed printing to stderr: Input/output error"` when a thread is opened (or on any
  later log line): what can no longer be printed is dropped. The tray and `omfp desktop` messages too (#209).

### Release limitations

- macOS downloads are signed ad-hoc and are not notarized unless the release explicitly says otherwise. On first
  launch, allow the app in System Settings → Privacy & Security → Open Anyway. Updates may request microphone
  permission again.
- The Flatpak deliberately runs oh-my-pi, git and job commands on the host with your account's permissions; it is
  not an isolation boundary. Keep workspaces under your home directory. On NixOS, use the Nix package or configure
  a compatible `omp_bin`. Job processes left behind after their shell exits are not killed on the host.
- Standalone downloads do not cover Windows, Intel Macs or ARM Linux.
- Multi-select prompts of the `ask` tool do not show yet which choices are checked: oh-my-pi 18.22 sends that state,
  but omfp's prompt card does not use it yet.
- oh-my-pi always loads its own user `agent/mcp.json` from omfp's profile: omfp never writes it and neither lists nor
  calls its servers, but a thread's agent still sees a hand-written one.

## 0.4.0 — 2026-11-19

### Added

- Widgets can call the tools of the workspace's MCP servers, with no thread running: `useMcp(server)` lists a
  server's tools and calls them, `useMcpTool(server, tool, args)` calls one tool like `useCall`, and `mcpText(result)`
  reads its text; agents and scripts reach the same with the protocol methods `mcp.tools` and `mcp.call`. The daemon
  talks to `http` and `sse` servers itself with the OAuth login made with `/mcp reauth <name>` and renews it the way
  oh-my-pi does, so threads and widgets share one login. `stdio` servers cannot be called this way yet (#205).
- Mermaid diagrams: a `mermaid` code block in a reply, a subagent's output, a widget or a Markdown file preview is
  drawn in the theme's colors and fonts, with a Source toggle to see its text; an invalid diagram stays code with
  Mermaid's error below it (#200).

### Changed

- MCP servers belong to the workspace: `omfp:mcp` and `workspaces.mcp_servers` list only the workspace's own config
  files (`.omp/mcp.json`, `.claude/`, `.cursor/mcp.json`, `.vscode/mcp.json`, a root `mcp.json`), no longer omfp's
  profile `agent/mcp.json` nor Claude Code's or Cursor's configs under `~`, and `/mcp` commands that would write a
  user-level config (`--scope user`, `/mcp enable|disable` of a server not in the workspace's `.omp/mcp.json`) are
  refused. `McpServers` no longer carries `userConfig`, nor a server's source its `scope` (#205).

  **Migration:** move servers from `<data-dir>/omp-profile/agent/mcp.json` into each workspace's `.omp/mcp.json`;
  existing logins keep working.
- The card of an LSP `rename` shows the old and new symbol name, the total (`6 edits in 3 files`) and each file's
  edit count; a preview's files open as they are (the renamed file at the call's line) rather than against `HEAD`, and
  a list of more than 8 files collapses behind a "… N more files" button (#193).
- When the which-key panel (`Space`, `g`, …) has more keys than fit a short window, only its keys scroll, between its
  title and the `esc` row, and the edge with more keys past it fades out, so the keys below the fold no longer go
  unnoticed where scrollbars are hidden (#192).

### Fixed

- `omfp:jobs` no longer shows "never ran" next to the running state while a job's first run is in progress (#194)
- A small image attached to a message is no longer stretched across the transcript's column: images show at most at
  their natural size, scaled down only to fit (#191).
- The chat's images can be zoomed: an image attached to a message now opens at full size on a click, and that full
  view (also of a tool's images and of `omfp:browser`'s screenshots) zooms and pans like an image file: the zoom
  buttons, `100%`, Fit, Ctrl with the wheel, two fingers on a touch screen, dragging.

### Release limitations

- macOS downloads are signed ad-hoc and are not notarized unless the release explicitly says otherwise. On first
  launch, allow the app in System Settings → Privacy & Security → Open Anyway. Updates may request microphone
  permission again.
- The Flatpak deliberately runs oh-my-pi, git and job commands on the host with your account's permissions; it is
  not an isolation boundary. Keep workspaces under your home directory. On NixOS, use the Nix package or configure
  a compatible `omp_bin`. Job processes left behind after their shell exits are not killed on the host.
- Standalone downloads do not cover Windows, Intel Macs or ARM Linux.
- Multi-select prompts of the `ask` tool do not show yet which choices are checked: oh-my-pi 18.22 sends that state,
  but omfp's prompt card does not use it yet.
- oh-my-pi always loads its own user `agent/mcp.json` from omfp's profile: omfp never writes it and neither lists nor
  calls its servers, but a thread's agent still sees a hand-written one.

## 0.3.1 — 2026-11-18

### Fixed

- Selecting a thread low in a long list of workspaces no longer scrolls the whole window up, leaving an empty band at
  the bottom: only the left column scrolls (#167).
- The header's context meter refreshes when a turn ends and after every compaction, manual or automatic, instead of
  only when the thread is selected again (#168).
- `:compact <mode>` (and `/compact soft`, …) on a thread that is already compacted shows one
  `Compaction skipped: <reason>` row instead of a second `Compaction failed: <reason>` one, and its toast, which only
  says the compaction started, no longer carries a success mark (#169).
- With `skin = boring`, the key hint in buttons (`enter` in Send) is readable again: it takes the button's own text
  color instead of a muted gray that nearly vanished on the dark Send button, also with `theme = matugen` (#170).
- A file of the workspace reached by its absolute path (a breakpoint in `omfp:debug`) and by its workspace-relative
  path (a tool card) now opens in one tab, which moves to the line of the latest opening, instead of two tabs. Tabs
  kept from before show once too (#171).
- `omfp:browser` no longer presents a tab's address as current after the agent clicked, went back or ran code in
  the page: oh-my-pi reports an address only when a tab is opened or sent somewhere with `goto`, so the box now says
  "It may have left this address since" with the call that may have moved it, until a later call reports one (#172).
- An agent's `omfp_thread_wait` without `promptId` no longer answers at once with the thread's previous prompt when
  the agent's `thread.prompt` for that thread is still being sent (two tool calls in the same turn): it waits for the
  new prompt instead. Its description now also says to call it once `thread.prompt` answered (#173).
- A window no longer flashes the default Synth palette and Crush skin for a moment while it loads: the daemon serves
  the page with the configured `[ui]` theme, skin, density, motion and font size already applied (#174).

### Release limitations

- macOS downloads are signed ad-hoc and are not notarized unless the release explicitly says otherwise. On first
  launch, allow the app in System Settings → Privacy & Security → Open Anyway. Updates may request microphone
  permission again.
- The Flatpak deliberately runs oh-my-pi, git and job commands on the host with your account's permissions; it is
  not an isolation boundary. Keep workspaces under your home directory. On NixOS, use the Nix package or configure
  a compatible `omp_bin`. Job processes left behind after their shell exits are not killed on the host.
- Standalone downloads do not cover Windows, Intel Macs or ARM Linux.
- Multi-select prompts of the `ask` tool do not show yet which choices are checked: oh-my-pi 18.22 sends that state,
  but omfp's prompt card does not use it yet.

## 0.3.0 — 2026-11-17

### Added

- omp's `/` commands that only work in its terminal UI no longer go to the model as plain text: `/login`, `/logout`,
  `/model` (the header's model list, also `:model`), `/compact`, `/fork`, `/copy`, `/export`, `/new`, `/hotkeys` and
  `/resume` run omfp's own equivalent, and the others (`/settings`, `/quit`, `/tree`, …) are listed greyed out in the
  `/` suggestions with the reason and not sent. When omp refuses a command it runs, an error now says why.
- The built-in **`omfp:debug`** widget (`:widget add omfp:debug`) shows the selected thread's debug session when the
  agent runs a program under a debugger: its status, where it stopped, the stack and variables the agent last read
  there, and the breakpoints; each place opens the file at its line. It only shows: oh-my-pi lets the agent, not the
  UI, drive the debugger.
- `:copy` copies the selected thread's conversation to the clipboard as Markdown (with a message focused in the
  transcript, only that message), and `:export` saves it as a `.md` file named after the thread: the browser
  downloads it, the desktop window saves it in the Downloads folder. Messages go under their author, tool calls are
  one line with their result, thinking is left out and images are placeholders. Both are also in the header's `⋯`.
- OAuth sign-ins started by `/mcp reauth <name>` or by logging in to a provider can be finished from a browser on
  another machine: when the provider sends the browser back to `localhost` and the page fails to load, paste its
  address into the new **Paste the address your browser ended on** field under the URL's chat card or toast, and the
  daemon hands it to oh-my-pi on its own host (`oauth.callback`, also open to agents). Only the exact callback
  oh-my-pi is waiting on is accepted.
- An opt-in **boring** skin, a calm and modern alternative to the default Crush look (in the manner of Linear,
  Notion and claude.ai): `:set skin=boring` (or `skin = "boring"` under `[ui]` in the config file) switches every
  window to the system sans-serif (monospace only for code), neutral gray chrome with the theme's accent kept for
  focus, links and progress, light borders, the user's messages in a gray bubble and replies as plain text, plain
  chevrons, a simple spinner and no decorative animation. It works with every theme; `:set skin=crush` goes back.
  The `/kit` gallery has a skin picker next to the themes.
- `:compact [focus]` compacts the selected thread's context on demand, like oh-my-pi's `/compact`: oh-my-pi
  summarizes the conversation so far, with the words after `:compact` as what the summary should focus on, and the
  transcript shows the compaction as a notice; a toast says how big the context was. It is refused while the thread
  works. `:compact soft|remote|snapcompact [focus]` picks oh-my-pi's method for that once, and `/compact` in the
  composer runs `:compact`. The header's `⋯` offers it too, with a field for the focus.
- Readable cards for oh-my-pi's code-intelligence tools. Calls of the tools oh-my-pi offers as `xd://` devices
  (language servers, `ast_grep`, `ast_edit`, MCP tools) show as cards of those tools instead of a `write` to an
  `xd://` path. `lsp` diagnostics list each problem with its severity and a `path:line` that opens the file there;
  definitions and references list their locations with the source; symbols open at their lines; renames and code
  actions list the files they changed, which open against `HEAD`; hover shows its documentation. `ast_grep` shows each
  match's code, highlighted, with a link to its line; an `ast_edit` preview shows its diff per file. Language servers
  come from the daemon's environment: a card says so when oh-my-pi finds none for a file.
- A **Browser** widget (`:widget add omfp:browser`) lists the tabs the agent drove with oh-my-pi's browser in the
  selected thread: each tab's name, the address it last opened, its last action and the latest screenshot it took,
  which opens at full size. It shows what the agent's browser calls reported, not the live page: oh-my-pi offers no
  live view or way to take over its browser to a client.
- Four calm built-in themes that suit the boring skin (and work with Crush too): `graphite` (cool neutral grays, a
  restrained indigo accent, inspired by Linear), `paper` (warm off-white or warm charcoal, a muted blue accent,
  inspired by Notion), `clay` (ivory or warm dark, a terracotta accent, inspired by claude.ai) and `primer` (GitHub's
  Primer light and dark). Choose one with `:theme graphite`, `:set theme=paper` or `theme = "clay"` under `[ui]`; the
  `/kit` picker shows them all.

### Changed

- The user theme (the matugen template's `~/.config/omfp/theme.css`, `omfp theme set` or `theme.set`) now has its own
  theme name, `matugen`, and the windows show it only while that theme is selected (`:theme matugen`, `:set
  theme=matugen`, `theme = "matugen"` under `[ui]`; the `/kit` picker shows it with the matugen fixture). With any
  other theme the built-in palette shows and the daemon keeps the user theme, still updated live, for when `matugen`
  is selected again; `matugen` without a user theme is Synth. **Migration**: if you use a matugen or hand-written
  theme, select `matugen` once (`:theme matugen`) to see it again. The `/kit` gallery's `?theme=matugen-forest` is
  now `?theme=matugen`.
- The left column's workspace sections are tidier: the thread count sits at the right end of the header, with the
  menu and delete buttons appearing in its place on hover; `+ thread` follows the threads; then two collapsed rows,
  `Jobs (N)` and `Archived (N)`, each toggled on its own. `Jobs (N)` replaces the job counts after the workspace
  name: it marks running and failing jobs and an invalid `jobs.json` at its right end, and expanded lists the jobs
  with their state or next time, a press opening that job in `omfp:jobs`, with Run or Stop on hover.
- The default view has the MCP servers (`omfp:mcp`) as a side tab after Background. Workspaces that already have a
  `.omfp/views.json` keep it: `:widget add omfp:mcp` adds the tab there.
- Consecutive file reads show as one `read N files` card instead of a card per file, also when the agent reads one
  file per reply, as long as nothing else shows between the reads: it spins while any read runs and is marked failed
  when one failed, which stays visible while the group is collapsed; open, it lists each read with its path (and line
  range), which opens the file, and each read opens to its output.
- omfp now runs oh-my-pi 18.22.0 (was 18.18.6); install that version when you bring your own `omp`.

### Removed

- Todo lists no longer show a `blocked` status with its blocker: oh-my-pi 18.19 removed blocked tasks, and a blocked
  task of an older session shows as pending.

### Fixed

- Opening or reloading a window no longer flashes the user theme (`theme.css`) when the chosen theme is not
  `matugen`: the daemon now serves `/theme.css` empty under any other theme (#154).
- A manual compaction oh-my-pi skips (`Nothing to compact (session too small)`, `Already compacted`) shows as
  `Compaction skipped: <reason>` instead of a failed compaction, and a manual compaction over the protocol
  (`thread.compact`) waits up to 10 minutes for oh-my-pi's summary instead of failing after 30 seconds.

### Release limitations

- macOS downloads are signed ad-hoc and are not notarized unless the release explicitly says otherwise. On first
  launch, allow the app in System Settings → Privacy & Security → Open Anyway. Updates may request microphone
  permission again.
- The Flatpak deliberately runs oh-my-pi, git and job commands on the host with your account's permissions; it is
  not an isolation boundary. Keep workspaces under your home directory. On NixOS, use the Nix package or configure
  a compatible `omp_bin`. Job processes left behind after their shell exits are not killed on the host.
- Standalone downloads do not cover Windows, Intel Macs or ARM Linux.
- Multi-select prompts of the `ask` tool do not show yet which choices are checked: oh-my-pi 18.22 sends that state,
  but omfp's prompt card does not use it yet.

## 0.2.1 — 2026-11-17

### Fixed

- Deleting a thread while its oh-my-pi process is starting (for example right after opening it) no longer leaves
  that process running for the deleted thread or its data directory behind.

## 0.2.0 — 2026-11-17

### Added

- Declared workspace jobs: a workspace's `.omfp/jobs.json` defines shell commands or prompts to a new or reused
  thread, run by hand, on a cron schedule or kept running as services. Job counts show next to the workspace name,
  `omfp:jobs` lists jobs with their run history and output, and archiving or deleting a thread whose agent started
  background runs asks whether to keep them running.
- Reusable daemon-owned PTY sessions for interactive CLI programs, with a `useTerminal` widget hook, a configurable
  xterm.js `TerminalEmulator` kit component and the copyable `omfp:terminal` built-in, which starts the daemon's
  `$SHELL` by default, another shell through its `shell` prop, or any `command`. Agents can read a terminal's visible
  screen through `terminals.screen`.
- The composer shows the current activity (thinking, a running tool or work) with its elapsed time and Abort; while
  the model reasons it previews the latest line of its reasoning instead of a generic "Thinking…".
- Type `@` in the composer to complete workspace files with fuzzy search; the chosen file is inserted as an
  `@path` mention, as in oh-my-pi's terminal UI.
- Readline/Emacs line-editing keys in the composer's insert mode (`Ctrl-a`, `Ctrl-e`, `Ctrl-b`, `Ctrl-f`, `Ctrl-k`,
  `Ctrl-u`, `Ctrl-w`, `Ctrl-y`, `Ctrl-t`, `Ctrl-p`/`Ctrl-n` and more), each a rebindable command. `Ctrl-a` moves to
  the start of the line; select all is `Ctrl-Shift-a`. Chords the browser reserves (`Ctrl-w`, `Ctrl-n`, `Ctrl-t`)
  work only where the browser lets them through.
- The transcript shows a day separator (`Yesterday`, `Thu 12 Nov`) where the day changes between messages, so older
  conversations show when they were written; conversations of today are unchanged.
- The draft thread's composer accepts pasted and dropped images, and its first message may be images only.

### Changed

- A non-archived thread row's `×` now archives it immediately, without confirmation. An archived row's `×` and
  explicit Delete controls still remove the conversation immediately; the workspace is kept.
- Middle-click a thread in the left list to delete it without selecting it first or starting browser autoscroll.
  Archiving or deleting the selected thread clears the selection and its draft.
- Conversation search highlights every occurrence of the query in the visible messages, the current one distinct.
  Enter and Shift-Enter visit each occurrence, expanding thinking blocks that contain one; Escape clears them.
- Switching threads restores the destination thread's view and the tab it last selected in each tab group,
  including chat, file and widget tabs. Open file and widget tabs stay shared by the workspace; a remembered tab that
  was closed falls back to the group's first tab.
- The daemon runs up to 64 oh-my-pi processes at once by default, instead of 8.

### Fixed

- Threads whose saved model can no longer be restored (for example, after moving to omfp's own profile without
  logging in to that provider) open with their conversation readable instead of only failing. The header says the
  saved model is unavailable and offers the usable models and Log in; picking one resumes the same conversation with
  it. omfp never switches the model or provider on its own.
- Agent shell tools see the host's own CLI configuration again (for example `gh` logins under `XDG_CONFIG_HOME`);
  only oh-my-pi's own configuration stays in omfp's profile.
- Adding or removing a repository in a workspace no longer posts the daemon's `Added …` / `Workspace directories:`
  output into every running thread of that workspace, nor counts as thread activity. Directory commands you type
  yourself still show their output.
- The provider login picker no longer overlaps long provider names, ids and login status. Each row shows the name
  on its own line with the id and status under it, wraps long text and grows to fit, so its highlight covers it.
- `omfp:terminal` without props starts a shell again instead of failing on a literal `${SHELL:-sh}`.
- On macOS, closing the window right after closing a file tab no longer brings the tab back on the next launch: the
  window waits for WebKit to save its last local storage writes before quitting.
- A folder or URL typed into the Workspace panel's add-repository field while the previous add is still finishing
  is no longer wiped when that add completes; the field clears only if it still holds what was added.
- Model-discovery schemas now expose `models` and each entry's `provider`, `id` and `name`, with discoverable
  create/set-model selector examples and matching generated TypeScript types.
- A failed first prompt that leaves a daemon thread now selects that thread and restores its unsent composer draft,
  including images and paste chips. Retry reuses the same conversation instead of creating a duplicate.
- Explicit `:open` arguments now accept bare media filenames, unknown extensions and spaces. They reach normal
  workspace lookup, disambiguation and missing-file feedback without loosening inline prose path recognition.
- Empty workspace/init keymap updates no longer cancel a pending leader chord during thread previews. Actual
  changed mappings still apply with normal sequence-reset behavior.

### Release limitations

- macOS downloads are signed ad-hoc and are not notarized unless the release explicitly says otherwise. On first
  launch, allow the app in System Settings → Privacy & Security → Open Anyway. Updates may request microphone
  permission again.
- The Flatpak deliberately runs oh-my-pi, git and job commands on the host with your account's permissions; it is
  not an isolation boundary. Keep workspaces under your home directory. On NixOS, use the Nix package or configure
  a compatible `omp_bin`. Job processes left behind after their shell exits are not killed on the host.
- Standalone downloads do not cover Windows, Intel Macs or ARM Linux.
- The pinned oh-my-pi's RPC prompts do not expose checkbox selection state; multi-select prompts cannot yet show
  which choices are checked.

## 0.1.0 — 2026-11-15

### Added

- Browser and desktop clients for oh-my-pi, with independent agent threads that keep working when windows close.
  Multiple windows and devices share one daemon.
- Workspaces containing repositories, symlinks and git worktrees; reorder workspaces and threads with drag-and-drop
  or keyboard controls, and fork a conversation into a new thread from a message.
- Streaming conversations with thinking blocks, tool cards and images from tool results; steer a running agent,
  queue follow-ups, answer approvals and prompts, and see session and subagent costs separately.
- Provider login from the app, a first-run welcome screen and live voice conversations through oh-my-pi.
- Customizable React widgets, live-reloaded layouts and init files, a reusable widget library, background jobs and
  subagent controls. Agents can drive the interface through the daemon protocol or its MCP server.
- File editing with vim keys and git diffs; PDF, image, audio, video, Markdown, CSV, JSON and YAML viewers.
  Images support zoom, pan and comparison with HEAD. Absolute paths outside the workspace open read-only.
- A keyboard-driven command line, configurable bindings, phone layouts, notifications and message history.
- Light and dark themes, including Catppuccin, Gruvbox, Tokyo Night, Nord, Solarized and Rosé Pine, custom CSS themes
  and a macOS system-accent theme.
- Nix packages and standalone downloads bundling the pinned oh-my-pi: a headless Linux x86_64 tarball
  (glibc 2.34+), a Linux x86_64 desktop Flatpak, and macOS Apple silicon DMG and ZIP packages (macOS 14+).
- A project website built from the same Crush component kit as the app, published independently through GitHub Pages.

### Changed

- Thread Delete controls now remove the conversation immediately, without confirmation. The workspace is kept;
  Archive remains the non-destructive alternative.
- Middle-click a thread in the left list to delete it without selecting it first or starting browser autoscroll.
  Deleting the selected thread clears the selection and its draft, as before.
- `threads.create` requires an explicit `workspace`: use `{ "id": "…" }` for an existing workspace or
  `{ "new": {} }` to create one. Clients that omitted it must add one of these choices.
- Automatically generated thread titles now follow sustained conversation changes, using bounded recent context
  with project continuity. Updates are conversation-driven and rate-limited; brief digressions, cosmetic variants
  and generation failures keep the previous title. Explicit user/API/agent titles are never overwritten.
- omfp now uses its own omp profile at `<data-dir>/omp-profile` (normally
  `~/.local/share/omfp/omp-profile` on Linux, `~/Library/Application Support/omfp/omp-profile` on macOS).
  Credentials, user settings, models, agents and MCP configuration no longer use standalone `~/.omp/agent`.
  Existing users must log in again with `:login` and configure the new `agent/` directory explicitly: nothing is
  imported or copied automatically and there is no sharing mode. Existing threads, history and project config stay.
- Informational MCP mount/unmount notices are hidden in transcripts by default. Use `:set show_xdev_notices=true`
  to reveal retained notices live, or `false` to hide them again; events, warnings/errors, startup inventory records
  and MCP tools remain unchanged.
- Keep the detached sidebar's logo, workspace field and thread filter visible while its thread list scrolls on
  phones and narrow windows.
- `/` searches the current conversation; `Space /` opens a fuzzy thread picker with live previews. Enter confirms,
  Escape restores the original thread. `Space t` focuses threads in wide windows and toggles the narrow drawer.
- `Space v` is the fuzzy view picker, with visible `1`–`9` shortcuts and arrows/Enter. Use `[ v` / `] v` to cycle;
  the old `g v` and `Space v v/n/p` paths are removed. Notices open by click or `:notice`, not `g n`.
- Cleared composer drafts show an `Esc` then `u` recovery hint. Normal-mode `u` restores only the latest draft
  this thread's `Ctrl-C` cleared, including images and paste chips; it is not general undo.
- `Space f` opens a fuzzy file picker across all workspace repositories and the workspace folder, showing each
  file's repository and containing folder. Filtering happens before the result limit, so later repositories remain
  searchable. `g f` and `Space f o` are removed; use `:open` for an explicit or focused path.

### Fixed

- Linux Tauri development builds and the Nix desktop package now include the GStreamer demuxers and codecs needed
  for common video, including H.264, instead of leaving unsupported media capable of stalling its WebKit window.
- Select a model before the first message, including in a draft that has not created a daemon thread yet. The first
  prompt uses that model, and failed creation keeps the choice for retry. Existing threads cannot send while a model
  change is still being applied.

### Release limitations

- macOS downloads are signed ad-hoc and are not notarized unless the release explicitly says otherwise. On first
  launch, allow the app in System Settings → Privacy & Security → Open Anyway. Updates may request microphone
  permission again.
- The Flatpak deliberately runs oh-my-pi, git and job commands on the host with your account's permissions; it is
  not an isolation boundary. Keep workspaces under your home directory. On NixOS, use the Nix package or configure
  a compatible `omp_bin`. Job processes left behind after their shell exits are not killed on the host.
- Standalone downloads do not cover Windows, Intel Macs or ARM Linux.
- The pinned oh-my-pi's RPC prompts do not expose checkbox selection state; multi-select prompts cannot yet show
  which choices are checked.
