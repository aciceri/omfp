# Changelog

User-facing changes, in English. Significant changes go under `Unreleased`, grouped into Added, Changed, Fixed,
Removed or Security; omit empty groups and internal refactoring. Include migration steps for incompatible changes.
At release time, move these entries under `## <version> — YYYY-MM-DD` and leave a fresh `Unreleased` section above it.
The version's section is also the source of the GitHub release notes.

## Unreleased

Initial release, planned as 0.1.0.

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
- Reusable daemon-owned PTY sessions for interactive CLI programs, with a `useTerminal` widget hook, a configurable
  xterm.js `TerminalEmulator` kit component and the copyable `omfp:terminal` built-in, which starts the daemon's
  `$SHELL` by default, another shell through its `shell` prop, or any `command`.
- File editing with vim keys and git diffs; PDF, image, audio, video, Markdown, CSV, JSON and YAML viewers.
  Images support zoom, pan and comparison with HEAD. Absolute paths outside the workspace open read-only.
- A keyboard-driven command line, configurable bindings, phone layouts, notifications and message history.
- Light and dark themes, including Catppuccin, Gruvbox, Tokyo Night, Nord, Solarized and Rosé Pine, custom CSS themes
  and a macOS system-accent theme.
- Nix packages and standalone downloads bundling the pinned oh-my-pi: a headless Linux x86_64 tarball
  (glibc 2.34+), a Linux x86_64 desktop Flatpak, and macOS Apple silicon DMG and ZIP packages (macOS 14+).
- A project website built from the same Crush component kit as the app, published independently through GitHub Pages.

### Fixed

- Linux Tauri development builds and the Nix desktop package now include the GStreamer demuxers and codecs needed
  for common video, including H.264, instead of leaving unsupported media capable of stalling its WebKit window.
- Select a model before the first message, including in a draft that has not created a daemon thread yet. The first
  prompt uses that model, and failed creation keeps the choice for retry. Existing threads cannot send while a model
  change is still being applied.
- Keep the detached sidebar's logo, workspace field and thread filter visible while its thread list scrolls on
  phones and narrow windows.
- Model-discovery schemas now expose `models` and each entry's `provider`, `id` and `name`, with discoverable
  create/set-model selector examples and matching generated TypeScript types.
- A failed first prompt that leaves a daemon thread now selects that thread and restores its unsent composer draft,
  including images and paste chips. Retry reuses the same conversation instead of creating a duplicate.
- Explicit `:open` arguments now accept bare media filenames, unknown extensions and spaces. They reach normal
  workspace lookup, disambiguation and missing-file feedback without loosening inline prose path recognition.
- Empty workspace/init keymap updates no longer cancel a pending leader chord during thread previews. Actual
  changed mappings still apply with normal sequence-reset behavior.

### Changed

- A non-archived thread row's `×` now archives it immediately, without confirmation. An archived row's `×` and
  explicit Delete controls still remove the conversation immediately; the workspace is kept.
- Middle-click a thread in the left list to delete it without selecting it first or starting browser autoscroll.
  Archiving or deleting the selected thread clears the selection and its draft.
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
- `/` searches the current conversation; `Space /` opens a fuzzy thread picker with live previews. Enter confirms,
  Escape restores the original thread. `Space t` focuses threads in wide windows and toggles the narrow drawer.
- `Space v` is the fuzzy view picker, with visible `1`–`9` shortcuts and arrows/Enter. Use `[ v` / `] v` to cycle;
  the old `g v` and `Space v v/n/p` paths are removed. Notices open by click or `:notice`, not `g n`.
- Cleared composer drafts show an `Esc` then `u` recovery hint. Normal-mode `u` restores only the latest draft
  this thread's `Ctrl-C` cleared, including images and paste chips; it is not general undo.
- `Space f` opens a fuzzy file picker across all workspace repositories and the workspace folder, showing each
  file's repository and containing folder. Filtering happens before the result limit, so later repositories remain
  searchable. `g f` and `Space f o` are removed; use `:open` for an explicit or focused path.

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
