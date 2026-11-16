# The omfp window

What the user sees around the widgets, so an agent can point at it ("press `Space v`", "pick Review in the mode line").
`omfp://docs/widgets` covers views, widgets, init files and TSX views; `omfp://keymap` lists every default key and
built-in command with its title and `:` names.

## Left column

The frame's own column, outside every view: one filter field at the top (thread titles, workspace and repository
names and conversation text), "+ workspace" (a name, or a path starting with `/` or `~`, with
completion), then one section per workspace — its name (the current one highlighted, a menu to rename or remove it),
its threads with their state marks (running, awaiting input, failed, idle, unread) and "+ thread", which opens a draft
thread there (the daemon creates the thread when its first message is sent). Archived threads hide behind
"Archived (N)". A section folds with its chevron or `z a`; `d d` deletes the focused thread immediately, or asks
before deleting a workspace. A thread's `×`, the header's Delete, `:delete` and middle-click on a thread row all
delete its history immediately, without confirmation, keeping its workspace. Middle-click never selects the thread
first and never starts autoscroll. Deleting the selected thread clears the selection and its draft; deleting another
leaves the selected thread alone. Use Archive instead to keep a conversation for reopening or unarchiving.
Workspaces and threads keep the order the user gives them, the same in every window (new ones go last): drag a
section by its head or a thread row (a long press first on a touch screen) to another place, a thread onto another
workspace to move it there (its omp restarts in that folder); `Alt-ArrowUp` / `Alt-ArrowDown` move the focused row or
section, `Alt-Shift-ArrowUp` / `Alt-Shift-ArrowDown` the focused thread into the previous or next workspace. Agents
use `workspaces.reorder` and `threads.reorder`.
Selecting a thread makes its workspace the **current workspace**: the view, the init file `.omfp/init.ts`, the keymap
`.omfp/keymap.json` and `:widget add` all follow it. A splitter resizes the column (wide windows); under 1100 px it
folds into a strip, on a phone it is one of the bottom bar's areas. `Space t` focuses the selected row in a wide
window and toggles the column in a narrow one; then `za` folds its workspace and keeps focus on its head.
In the detached column (narrow windows and phones), the logo, filter and "+ workspace" field stay visible while the
workspace sections and their threads scroll independently below them.

Automatic thread titles are reconsidered after four new messages totaling at least 400 characters, no more often
than every five minutes of active conversation. They keep project context and recent user/assistant work; a changed
topic needs confirmation in a later batch, so a brief digression does not immediately rename the thread. Idle time
alone does nothing. An explicit rename by the user, an API client or the agent disables automatic updates permanently.
`--no-title-generation` / `[daemon] title_generation = false` disables both initial generation and refresh.

`Space /` (`:threads`) opens a fuzzy thread picker by title and workspace. Typing and Up/Down preview the selected
conversation live, Enter confirms, Escape (or leaving the picker) restores the original thread and its draft.
`/` (`:search`) instead searches **the current conversation**: text and thinking, case-insensitive, a matching-message
count, Enter/Shift-Enter for next/previous, Escape back to the selected message. It finds messages outside the
currently rendered virtual window too.

## Mode line

The bar at the bottom of the view area, like vim's:

- the **mode pill**: `NORMAL` (no text field has the focus), `INSERT` (a text field has it), `COMMAND` (the `:` line is
  open), or vim's own mode while a file editor has the focus;
- the selected thread's title and state;
- the **view tabs**, when the current workspace has several views: `Default · Review · Ops` by title, the current
  one highlighted; a click switches the selected thread's view. Each thread keeps its view in this window across
  reloads; a thread shown for the first time (or a new thread being started) gets the view last chosen in its
  workspace, else the default view, as does a thread whose view the workspace no longer has. Switching back to one of
  the last 3 views shown in the workspace shows it at once as it was left (its widgets' content, typed text and the
  focus), then updates it;
- at the right end the newest **notice** (`◆ title body`, `+n` when more wait): a thread that needs the user, failed
  or finished while the user looked elsewhere, or what a widget, an init file or a thread's agent notified. `:notice`
  or a click opens its thread; it goes after 8 s;
- last, once something was said, the **messages** button: it opens `:messages`.

In command mode the `:` line (or the file picker) replaces the title, the view tabs and the notice.

The result of something the user did (`:widget add`, copying a path, a fork in the tree, `:view-reset`, saving a file…)
shows as a success toast at the bottom right, gone after 5 s; a failure as an error toast that stays until closed.
`:messages` (`Space h e`, or the mode line's messages button) lists the window's last 100 notices and toasts, newest
first, each with its kind and time; Escape closes it.

## File and widget tabs

Files open as tabs of the view's `main` tabs, after its own tabs (the chat in the default view). A widget the view does
not show opens there too when something reveals it (the header's `N in background` badge, its workspace chip): a tab
titled with the widget's name after the files, selected, and selected again on the next reveal; a view showing the
widget selects its own tab instead, and a view without `main` floats it in a panel. Each of these tabs has its own `×`;
a middle click on a tab or Delete on the focused tab closes it too, without selecting it first. They stay open across
reloads, per workspace, in this window only.

## The `:` line

`:` (or `Space :`) opens it; it doubles as the command palette: typing filters the commands by name, Tab completes,
the arrows move, Enter runs, Escape closes. After a space it completes the command's arguments. Commands run where the
focus was before the line opened. Every built-in command id is a dotted `<area>.<verb>` (`window.theme`,
`widget.manage`); the short names below are its aliases, and the former ids `theme.set` and `widget` still run and
bind. The built-in ones (`omfp://keymap` has them all):

| Command | What it does |
| --- | --- |
| `:new`, `:workspace` | A draft thread in the current workspace; a new workspace. |
| `:archive`, `:unarchive`, `:delete`, `:rename [<title>]` | The selected thread; `:delete` removes its history immediately without confirmation (workspace kept). Archive keeps the conversation. |
| `:stop`, `:stop!`, `:restart`, `:next` | Stop process: end the thread's omp, with its background commands and live voice (`!`: also of every thread it spawned; Abort, `Ctrl-c`, stops only the running turn); restart it; the next thread needing the user. |
| `:spawned` | Show only the threads the selected one spawned (again: all). |
| `:fork` | A new thread in the same workspace with the conversation up to the focused message (transcript; `j`/`k` focus messages) or tree entry (`omfp:tree`), the message included (`threads.fork`); the thread forked from is untouched. A user message's *Fork from here* does the same; the fork's header and row say where it comes from. |
| `:open <path>[:<line>]` | Open a file of the workspace in a tab next to the chat. |
| `:rescan` | Rescan the current workspace's repositories. |
| `:login [<provider>]`, `:logout <provider>` | Log omp in to a provider (without one, pick it); remove omp's stored credentials of one. See "Logging in". |
| `:view <id>` | Switch view. |
| `:view-reset` | Write the default `views.json` again (replacing the file, every view and mount), after a confirmation. |
| `:widget add <reference>` | Add a widget to the current view (last side tab, else the end) and write `views.json`; refused on views that exist only in an init file or a TSX file. |
| `:widget copy <reference> [<name>]` | Copy a built-in or user widget into `.omfp/widgets/<name>/`. |
| `:widget save[!] <widget> [<name>]` | Save a workspace widget to the user library (`!` replaces). |
| `:theme [light\|dark\|system] [<theme>]`, `:density compact\|normal`, `:set <key>=<value>` | Appearance and the daemon's settings: the scheme is the window's, the built-in theme (`synth`, `catppuccin`, `gruvbox`, `tokyo-night`, `nord`, `solarized`, `rose-pine`, `accent`: the macOS window's system accent, Synth elsewhere) every window's (`[ui] theme`); a user theme (`theme.css`) still wins. |
| `:set show_xdev_notices=true\|false` | Show/hide informational MCP mount/unmount notices in new and retained transcripts, live (default `false`). Only presentation changes; other notices, startup inventory records, tools and omp configuration stay intact. The API field is `ui.showXdevNotices` in `settings.set`. |
| `:help` | Every effective key and command, filterable (`?` too). |
| `:describe-key`, `:describe-command <name>` | What a key runs and where it was bound; a command's names, keys and who registered it. |
| `:notice` | Open the newest notice. |
| `:messages` | The notices and toasts that went by, newest first. |

Widgets (`useCommand`) and init files (`omfp.command`) add commands to the same line while they are shown or loaded.

## Default keys

In normal mode (no text field focused). A started sequence (`g`, `Space h`) shows the which-key panel above the mode
line after 300 ms with the keys that complete it; `Space` alone shows it at once. `Space h` groups help and `Space p`
workspace; `Space v` and `Space f` directly open view and workspace-file pickers. Escape cancels a sequence.

| Keys | What they do |
| --- | --- |
| `j` / `k`, `g g` / `G`, `Ctrl-d` / `Ctrl-u` | Next / previous, first / last, half a page — threads in the left column, messages in the transcript. In the transcript `j` / `k` scroll a message taller than the window down to its end / up to its top before moving on, and `Ctrl-d` / `Ctrl-u` scroll half a window (`PageDown` / `PageUp` a whole one), the focus staying on its message while it shows. |
| `/`, `Space /` | Search the current conversation; fuzzy thread picker with live preview (Enter confirms, Escape restores). |
| `i` | Focus the composer. |
| `:`, `Space :`, `Space Space` | The `:` line. |
| `?`, `Space h h` | Help; `Space h k` is `:describe-key`, `Space h e` is `:messages`. |
| `Space v` | Fuzzy view picker: type a title/id subsequence; visible `1`–`9` selects that matching row immediately, or use arrows and Enter. Escape cancels. |
| `] v`, `[ v` | Next / previous view (round the list). |
| `Space f` | Fuzzy file picker across every workspace repository and the folder. Each row shows basename, repository and containing folder; type a path subsequence, arrows/Enter choose, Escape cancels. The query searches all eligible files before the 50-result limit; refine it when the list says more matches exist. |
| `Space a`, `] a`, `Alt-a` | Next thread needing the user. |
| `Space n`, `Alt-n` | New thread; `Space w` or `Space p n` new workspace, `Space p r` rescan it. |
| `Space r`, `Space x`, `Space u`, `Space d` | Rename, archive, unarchive, delete the selected thread. |
| `Space s`, `Space S`, `Space R` | Stop process of the thread (and of the threads it spawned); restart it. |
| `Space t` | Focus threads (wide); show or hide threads (narrow). Then `za` acts on the focused workspace. |
| `d d`, `z a` | Delete the focused row; fold the focused workspace. |
| `Alt-ArrowUp`, `Alt-ArrowDown` | Move the focused thread (in its workspace) or workspace up or down. |
| `Alt-Shift-ArrowUp`, `Alt-Shift-ArrowDown` | Move the focused thread to the end of the previous workspace, the start of the next one. |
| `Ctrl-c` | Interrupt the selected thread's turn (its queued messages come back into the composer); with text selected it copies. |
| `u` | In normal mode restore this thread's last Ctrl-c-cleared composer draft, including images and paste chips, merged with any new text. Not general undo. Leave the composer with Esc first; the recovery hint stays while the draft can be restored. |

In insert mode `Esc` leaves the text field (in the composer too, while the agent works: it never aborts), `Ctrl-c` in
the composer clears the draft, then aborts, `Alt-a` / `Alt-n` work too; in the composer
Enter sends; while the agent works it steers the running turn, and Ctrl-Enter (⌘-Enter), Shift-Enter or Ctrl-Q
queue a follow-up for after the turn instead. Alt-Enter adds a line. A paste of more than 10 lines or 1000
characters becomes a chip (`Pasted text #1 · 120 lines`) and a `[Paste #1, +120 lines]` token, expanded when the
message is sent; `Ctrl-Shift-v` (`Cmd-Shift-v` on macOS) pastes it as plain text instead. `Up` with the caret on the
composer's first line recalls the prompts you sent (this thread's first, then its workspace's, then every other, newest
first within each); `Down` on the last line goes back and, past the newest, restores what you had typed.

The composer also takes readline's line-editing keys in insert mode (commands `composer.*`): `Ctrl-a` / `Ctrl-e`
start / end of the line, `Ctrl-b` / `Ctrl-f` a character back / forward, `Alt-b` / `Alt-f` a word back / forward,
`Ctrl-p` / `Ctrl-n` the previous / next line (on the first / last line the prompt history, like `Up` / `Down`),
`Ctrl-d` / `Ctrl-h` delete the character after / before the caret, `Ctrl-k` / `Ctrl-u` kill to the end / start of
the line, `Ctrl-w` or `Alt-Backspace` / `Alt-d` kill the previous / next word, `Ctrl-y` yanks the last killed text
(a buffer of the window, not the clipboard), `Ctrl-t` swaps the characters around the caret. `Ctrl-Shift-a` (or the
context menu) selects the whole draft. The same keys run on macOS instead of its native ones. In a browser tab the
browser keeps `Ctrl-w`, `Ctrl-n` and `Ctrl-t` (close tab, new window, new tab) and never passes them to the page:
use `Alt-Backspace` and `Down` there, or the desktop window, where they work.

Keys are rebindable: the daemon's `[keymap]` config, the workspace's `.omfp/keymap.json`, then the init files' `bind`
(each layer wins over the ones before it).

## Logging in

omp needs a model it can use: a provider it is logged in to, or an API key. A fresh install has neither: the window
then welcomes you in place of the view (until a thread is selected) with **Log in to a provider**, and a thread would
fail to start with omp's `No default model selected. Use /login, …` (`/login` is omp's own command; in omfp it is
`:login`). omp keeps its credentials on the daemon's host in omfp's isolated `<data-dir>/omp-profile/agent/agent.db`,
shared by every omfp thread and window, never with standalone omp; once a login (or a logout) changes what omp can
use, every window follows (`auth.status`).

- `:login` opens the provider picker: the providers omp can log in to (asking omp takes a few seconds), each with its
  id and `logged in` on those omp already holds a credential for; those it cannot log in to here are greyed out.
  `:login <provider>` (e.g. `:login anthropic`; Tab completes the ids) skips the picker.
- The login tries to open the provider's page in a new tab of this browser. If the browser blocks the tab, the toast
  offers Open and Copy; while a code-entry dialog has focus, use Open or Copy **inside that dialog** instead. While
  the login runs, a toast `Logging in to <name>… finish in your browser` shows omp's progress and Cancel (it gives up
  after 10 minutes; closing the window cancels it too).
- Providers whose page shows a code to paste (or whose redirect lands on an address this browser cannot reach, such
  as `localhost` of a daemon on another machine) ask for it: a dialog takes the code or the redirect URL; Enter sends
  it, Escape cancels the login.
- `Logged in to <name>` confirms it. The window reads the models again, and the daemon restarts each running thread's
  omp once that thread is free, so it sees the new models; a thread whose start failed needs Restart.
- **Log in** buttons show where omp says it needs a login: on an error toast quoting omp's refusal, beside Restart in
  the header of a thread whose start failed that way, and instead of the model list when omp offers no model.
- `:logout <provider>` removes omp's stored credentials of the provider and says how many it removed and what still
  authenticates it (e.g. an environment variable).

Instead of logging in, give omp an API key: an environment variable such as `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` or
`GEMINI_API_KEY` in the daemon's environment (omp inherits it, so restart the daemon after setting one), providers and
keys in `<data-dir>/omp-profile/agent/models.yml`, or keys stored in that profile's `agent.db`.

The profile is normally `~/.local/share/omfp/omp-profile` on Linux (`$XDG_DATA_HOME/omfp/omp-profile` when set) or
`~/Library/Application Support/omfp/omp-profile` on macOS; `--data-dir` relocates it. User `config.yml`, `models.yml`,
`mcp.json`, agent definitions and `APPEND_SYSTEM.md` belong in its `agent/` directory. Existing users must log in
again and configure this fresh profile: omfp never imports/copies old credentials or configuration and has no sharing
switch or standalone fallback. Threads/history and workspace `.omp/` config are kept. `HOME` stays unchanged, and
explicit API keys inherited from the daemon's environment still work. `:logout` affects only this profile's store.

## Agents and the window

Each window reports to the daemon what it shows — its thread, focus, views and where each comes from, the commands
init files and widgets added, its errors — which agents read with `omfp_windows`. An agent runs a command in a window
with `omfp_window_run` (`view.switch` with a view id shows that view), as `:<command> <args>` would: only registered
commands, never code. `omfp_widget_check` mounts a widget no view shows out of sight for a moment to report how it
renders; nothing of it appears on screen.
