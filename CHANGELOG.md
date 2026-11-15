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
- File editing with vim keys and git diffs; PDF, image, audio, video, Markdown, CSV, JSON and YAML viewers.
  Images support zoom, pan and comparison with HEAD. Absolute paths outside the workspace open read-only.
- A keyboard-driven command line, configurable bindings, phone layouts, notifications and message history.
- Light and dark themes, including Catppuccin, Gruvbox, Tokyo Night, Nord, Solarized and Rosé Pine, custom CSS themes
  and a macOS system-accent theme.
- Nix packages and standalone downloads bundling the pinned oh-my-pi: a headless Linux x86_64 tarball
  (glibc 2.34+), a Linux x86_64 desktop Flatpak, and macOS Apple silicon DMG and ZIP packages (macOS 14+).

### Fixed

- Keep the detached sidebar's logo, workspace field and thread filter visible while its thread list scrolls on
  phones and narrow windows.

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
