# Changelog

User-facing changes, in English. Significant changes go under `Unreleased`, grouped into Added, Changed, Fixed,
Removed or Security; omit empty groups and internal refactoring. Include migration steps for incompatible changes.
At release time, move these entries under `## <version> — YYYY-MM-DD` and leave a fresh `Unreleased` section above it.
The version's section is also the source of the GitHub release notes.

## Unreleased

### Fixed

- `omfp:jobs` no longer shows "never ran" next to the running state while a job's first run is in progress (#194)

### Changed

- The card of an LSP `rename` shows the old and new symbol name, the total (`6 edits in 3 files`) and each file's
  edit count; a preview's files open as they are (the renamed file at the call's line) rather than against `HEAD`, and
  a list of more than 8 files collapses behind a "… N more files" button (#193).

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
