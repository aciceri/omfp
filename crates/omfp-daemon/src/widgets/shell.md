# The omfp window

What the user sees around the widgets, so an agent can point at it ("press `Space v`", "pick Review in the mode line").
`omfp://docs/widgets` covers views, widgets and init files; `omfp://keymap` lists every default key and
built-in command with its title and `:` names.

## Left column

The frame's own column, outside every view: one filter field at the top (thread titles, workspace and repository
names and conversation text), "+ workspace" (a name, or a path starting with `/` or `~`, with
completion), then one section per workspace — its name (the current one highlighted) and its thread count, which a
menu to rename or remove it and a delete button replace on hover and focus, its threads with their state marks
(running, awaiting input, failed, idle, unread) and "+ thread", which opens a draft thread there (the daemon creates
the thread when its first message is sent). Then two collapsed rows, each toggled on its own: "Jobs (N)" when the
workspace declares jobs (running and failing counts and a refused `jobs.json` marked at its right end; expanded, a row
per job opens `omfp:jobs` on it, with Run or Stop on hover), and "Archived (N)", behind which archived threads hide. A section folds with its chevron or `z a`; `d d` deletes the focused thread immediately, or asks
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
folds into a strip, on a phone it is one of the bottom bar's areas. In a wide window *Hide threads* (the button at the
end of the logo's row) or `Space b` (`:threads.collapse`) collapses it to nothing, so the view takes the whole window;
*Show threads* at the start of the mode line or `Space b` again brings it back, and the window remembers it across
reloads. `Space t` focuses the selected row in a wide window (showing a collapsed column first) and toggles the
column in a narrow one; then `za` folds its workspace and keeps focus on its head.
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

- the **mode pill**: where the keys go. `THREADS` (nothing focused, or the focus in the left column: the keys switch
  threads), `VIEW` (the focus in the view, outside a text field: the keys go to the focused widget), `INSERT` (a text
  field has the focus), `COMMAND` (the `:` line is open), or vim's own mode while a file editor has the focus. The
  mode follows the focus: a click in the view is View mode, on the left column Threads mode, in a field Insert mode;
- the selected thread's title and state;
- the **view tabs**, when the current workspace has several views: `Default · Review · Ops` by title, the current
  one highlighted; a click switches the selected thread's view. Each thread keeps its view in this window across
  reloads; a thread shown for the first time, a new thread's draft and a thread whose view the workspace no longer has
  show the default view, and a thread created from a draft keeps the view chosen on it. Switching back to one of
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
reloads, per workspace, in this window only, and their titles show in italics.

The view's own tabs, those of `views.json` (`:widget add`, an agent's mount), close the same way, unless a tab is
alone in its container or holds the chat: closing one writes `views.json` without it, so every window loses it, and
the toast `Closed <title>` offers Undo while nobody changed the file since. The views of init files keep their tabs.

## The `:` line

`:` (or `Space :`) opens it; it doubles as the command palette: typing filters the commands by name, Tab completes,
the arrows move, Enter runs, Escape closes. After a space it completes the command's arguments. Commands run where the
focus was before the line opened. Every built-in command id is a dotted `<area>.<verb>` (`window.theme`,
`widget.manage`); the short names below are its aliases. The built-in ones (`omfp://keymap` has them all):

| Command | What it does |
| --- | --- |
| `:new`, `:workspace` | A draft thread in the current workspace; a new workspace. |
| `:archive`, `:unarchive`, `:delete`, `:rename [<title>]` | The selected thread; `:delete` removes its history immediately without confirmation (workspace kept). Archive keeps the conversation. |
| `:stop`, `:stop!`, `:restart`, `:next` | Stop process: end the thread's omp, with its background commands and live voice (`!`: also of every thread it spawned; Abort, `Ctrl-c`, stops only the running turn); restart it; the next thread needing the user. |
| `:spawned` | Show only the threads the selected one spawned (again: all). |
| `:fork` | A new thread in the same workspace with the conversation up to the focused message (transcript; `j`/`k` focus messages) or tree entry (`omfp:tree`), the message included (`threads.fork`); the thread forked from is untouched. A user message's *Fork from here* does the same; the fork's header and row say where it comes from. |
| `:copy`, `:export` | The selected thread's conversation as Markdown, rendered by the window from the thread's state (a stopped thread's too): `:copy` puts it on the clipboard (in the transcript, with a message focused by `j`/`k`, only that message), `:export` saves it as `<title>.md` (the browser downloads it; the desktop window saves it in the Downloads folder). Messages go under `## You` / `## Assistant`, thinking is left out, images are `[image: <type>]`, a tool call is one line with its result in a code block. Also in the header's `⋯`; no default key. |
| `:compact [soft\|remote\|snapcompact] [focus]` | Compact the selected thread's context, like omp's `/compact`: omp summarizes the conversation so far and continues from the summary (`thread.compact`; the words after it tell the summary what to focus on). A first word naming one of omp's modes sends omp's `/compact <mode> [focus]` instead, which reports in the transcript (`snapcompact` takes no focus). `/compact` in the composer runs `:compact`. Refused while the thread works (Abort ends the turn); the transcript shows the compaction as a notice and a toast says how big the context was. Also in the header's `⋯`, with a field for the focus; no default key. |
| `:open <path>[:<line>]` | Open a file of the workspace in a tab next to the chat. |
| `:rescan` | Rescan the current workspace's repositories. |
| `:login [<provider>]`, `:logout <provider>` | Log omp in to a provider (without one, pick it); remove omp's stored credentials of one. See "Logging in". |
| `:import-logins` | Copy your standalone omp's provider logins into omfp's omp (`auth.import`). See "Logging in". |
| `:model` | Open the selected thread's model list in its header (Log in when omp offers no model). |
| `:view <id>` | Switch view. |
| `:view-reset` | Write the default `views.json` again (replacing the file, every view and mount), after a confirmation. |
| `:widget add <reference>` | Add a widget to the current view (last side tab, else the end) and write `views.json`; refused on views that exist only in an init file. |
| `:widget copy <reference> [<name>]` | Copy a built-in widget into `.omfp/widgets/<name>/`. |
| `:theme [light\|dark\|system] [<theme>]`, `:density compact\|normal`, `:set <key>=<value>` | Appearance and the daemon's settings: the scheme is the window's, the theme (`synth`, `catppuccin`, `gruvbox`, `tokyo-night`, `nord`, `solarized`, `rose-pine`; the calm ones that suit the boring skin: `graphite`, `paper`, `clay`, `primer`; `accent`: the macOS window's system accent, Synth elsewhere; `matugen`: the user theme file `theme.css`, Synth without one) every window's (`[ui] theme`); the user theme shows only with `matugen`. |
| `:set skin=crush\|boring` | The look of every window (`[ui] skin`), live: Crush (the default: monospace chrome, box-drawing glyphs, gradients, motion) or the opt-in `boring` skin (system sans-serif, neutral gray chrome with the accent only for focus, links and progress, light borders, a gray bubble for your messages, plain chevrons, no decorative motion). Independent of the theme: any palette works with either. Widgets built on the kit follow it; a widget's own styles do not. |
| `:set show_xdev_notices=true\|false` | Show/hide informational MCP tool command mount/unmount notices in new and retained transcripts, live (default `false`). Only presentation changes; other notices, startup inventory records, tools and omp configuration stay intact. The API field is `ui.showXdevNotices` in `settings.set`. |
| `:set recent_threads=<n>\|all` | How many hidden threads each window keeps followed so switching back to one is instant (`[ui] recent_threads`, 0–100 or `all`, default 20), live; `all` keeps every thread shown in the window until it closes or reloads. A kept thread does not keep its omp running: idle, it stops after the idle timeout like an unwatched one, its transcript stays in the window. Each kept thread costs the window memory (about its snapshot: a few MB for a very long thread), so `all` can reach gigabytes after hundreds of long threads, too much for a phone. |
| `:voice <voice>`, `:set live_voice=<voice>` | The voice of live sessions (`[ui] live_voice`): one of omp's `arbor`, `breeze`, `cove`, `ember`, `juniper`, `maple`, `sol` (omp's default), `spruce`, `vale`, sent with every start of a live session from then on (the running one keeps its voice). The API field is `ui.liveVoice` in `settings.set`; `thread.live_connect`/`thread.live_start` take a `voice` of their own. |
| `:set delete_archived_after=<n>m\|h\|d\|w\|never` | How long a thread stays archived before the daemon deletes it, as a manual delete does (`[daemon] delete_archived_after`, default `7d`; `never` or `0` keeps archived threads), live: the daemon checks at startup and every hour. Unarchiving a thread restarts its count; a thread whose agent's background commands still run is kept until they end. `--delete-archived-after` wins over it. The API field is `daemon.deleteArchivedAfter` in `settings.set`. |
| `:live` | Start live voice on the current thread, or end the running session, as the composer's microphone does (`/live` too); on a new thread's draft it creates the thread and starts talking to it, as the draft's microphone does. The daemon ends a session after 10 minutes in which nobody spoke and no delegated work ran; what was said stays in the thread as a *Voice conversation* row. |
| `:help` | Every effective key and command, filterable (`?` too). |
| `:describe-key`, `:describe-command <name>` | What a key runs and where it was bound; a command's names, keys and who registered it. |
| `:notice` | Open the newest notice. |
| `:messages` | The notices and toasts that went by, newest first. |

Widgets (`useCommand`) and init files (`omfp.command`) add commands to the same line while they are shown or loaded.

## Default keys

Threads mode (`THREADS`) is where the window starts; `v` or `Enter` enter View mode (`VIEW`) on the widget the view had
the focus on last (the transcript the first time), `i` the composer (Insert mode), and `Esc` steps back: Insert → View
→ Threads. A started sequence (`g`, `Space h`) shows the which-key panel above the mode line after 300 ms with the keys
that complete it; `Space` alone shows it at once. `Space h` groups help and `Space p` workspace; `Space v` and
`Space f` directly open view and workspace-file pickers. Escape cancels a sequence. Unless a row says otherwise, a key
works in Threads and View mode both.

| Keys | What they do |
| --- | --- |
| `j` / `k`, `g g` / `G`, `Ctrl-d` / `Ctrl-u` | Threads mode: next / previous, first / last thread, half a page of threads, in the left column's order across workspaces, with the column shown, collapsed or hidden; archived threads, folded workspaces and threads the filter hides are skipped. Each press selects the thread. |
| `v`, `Enter` | Threads mode: focus the view where it was last (View mode). |
| `j` / `k`, `g g` / `G`, `Ctrl-d` / `Ctrl-u` | View mode: the focused widget's moves. In the transcript: next / previous, first / last message, half a window; `j` / `k` scroll a message taller than the window down to its end / up to its top before moving on, and `Ctrl-d` / `Ctrl-u` scroll half a window (`PageDown` / `PageUp` a whole one), the focus staying on its message while it shows. |
| `Ctrl-w h` / `j` / `k` / `l` | View mode: focus the pane (widget) on the left, below, above, on the right. A browser tab never gets `Ctrl-w`: the desktop window does. |
| `g t` / `g T` | View mode: next / previous tab of the tabs around the focused widget. |
| `Esc` | View mode: leave the view (Threads mode). A widget that uses `Esc` itself keeps it (the file editor's vim). |
| `/`, `Space /` | Search the current conversation; fuzzy thread picker with live preview (Enter confirms, Escape restores). |
| `i` | Focus the composer. |
| `:`, `Space :`, `Space Space` | The `:` line. |
| `?`, `Space h h` | Help; `Space h k` is `:describe-key`, `Space h e` is `:messages`. |
| `Space v` | Fuzzy view picker: type a title/id subsequence; visible `1`–`9` selects that matching row immediately, or use arrows and Enter. Escape cancels. |
| `] v`, `[ v` | Next / previous view (round the list). |
| `Space f` | Fuzzy file picker across every workspace repository and the folder. Each row shows basename, repository and containing folder; type a path subsequence, arrows/Enter choose, Escape cancels. The query searches all eligible files before the 50-result limit; refine it when the list says more matches exist. |
| `Space a`, `] a`, `Alt-a` | Next thread needing the user. |
| `Alt-j`, `Alt-k` | Next / previous thread of the left column, across workspaces (round the list), from anywhere; archived threads, folded workspaces and threads the filter hides are skipped. |
| `Space n`, `Alt-n` | New thread; `Space w` or `Space p n` new workspace, `Space p r` rescan it. |
| `Space r`, `Space x`, `Space u`, `Space d` | Rename, archive, unarchive, delete the selected thread. |
| `Space s`, `Space S`, `Space R` | Stop process of the thread (and of the threads it spawned); restart it. |
| `Space t` | Focus threads (wide); show or hide threads (narrow). Then `za` acts on the focused workspace. |
| `Space b` | Hide or show the left column (wide); the mode line's *Show threads* brings it back too. |
| `d d`, `z a` | Threads mode: delete the focused row, else the selected thread; fold the focused workspace, else the selected thread's. |
| `Alt-ArrowUp`, `Alt-ArrowDown` | Threads mode: move the focused thread (in its workspace) or workspace up or down, else the selected thread. |
| `Alt-Shift-ArrowUp`, `Alt-Shift-ArrowDown` | Threads mode: move the focused (else the selected) thread to the end of the previous workspace, the start of the next one. |
| `Ctrl-c` | Interrupt the selected thread's turn (its queued messages come back into the composer); with text selected it copies. |
| `u` | Restore this thread's last Ctrl-c-cleared composer draft, including images and paste chips, merged with any new text. Not general undo. Leave the composer with Esc first; the recovery hint stays while the draft can be restored. |

In insert mode `Esc` leaves the text field for the view (in the composer too, while the agent works: it never
aborts), `Ctrl-c` in the composer clears the draft, then aborts, `Alt-a` / `Alt-n` / `Alt-j` / `Alt-k` work too (the
new thread's composer keeps the focus); in the composer
Enter sends; while the agent works it steers the running turn, and Ctrl-Enter (⌘-Enter), Shift-Enter or Ctrl-Q
queue a follow-up for after the turn instead. Alt-Enter adds a line. A paste of more than 10 lines or 1000
characters becomes a chip (`Pasted text #1 · 120 lines`) and a `[Paste #1, +120 lines]` token, expanded when the
message is sent; `Ctrl-Shift-v` (`Cmd-Shift-v` on macOS) pastes it as plain text instead. `Up` with the caret on the
composer's first line recalls the prompts you sent (this thread's first, then its workspace's, then every other, newest
first within each); `Down` on the last line goes back and, past the newest, restores what you had typed. `@` and part
of a path suggest the workspace's files (fuzzy, like `Space f`); Tab or Enter inserts `@<path>`, which omp reads into
the prompt's context. As in omp's terminal UI, the rest of the word being typed shows dimmed after the caret (omp's
word autocomplete, `spelling.autocomplete` in omfp's omp profile; not on touch screens): Tab inserts it with a space,
`→` without one.

`/` at the start of the message suggests omp's commands. omp runs most of them, but some only work in its terminal UI:
omfp runs its own equivalent for `/login`, `/logout`, `/model` (the header's model list), `/compact [<mode>] [<focus>]` (`:compact`),
`/fork` (a new thread with the whole conversation), `/copy`, `/export`, `/new`, `/hotkeys` (`:help`), `/resume`
(`:threads`) and `/live` (`:live`); the other terminal-only ones (`/settings`, `/quit`, `/tree`, …) show greyed out with the reason and are
not sent. A command defined by the workspace, a skill or a plugin always goes to omp. When omp refuses a command, the
error says why.

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
- Providers (and `/mcp reauth <name>`) that send the browser back to omp's listener on the daemon's host
  (`http://localhost:<port>/…`) leave a browser on another machine on a page that fails to load. Paste that page's
  address into **Paste the address your browser ended on** under the URL's toast or chat card and press Send: the
  daemon requests it on its own loopback (`oauth.callback`; only the exact callback omp waits on, for 5 minutes).
- `Logged in to <name>` confirms it. The window reads the models again, and the daemon restarts each running thread's
  omp once that thread is free, so it sees the new models; a thread whose start failed needs Restart.
- **Log in** buttons show where omp says it needs a login: on an error toast quoting omp's refusal, beside Restart in
  the header of a thread whose start failed that way, and instead of the model list when omp offers no model.
- A thread whose saved model omp cannot use any more (its provider not logged in here) still opens with its
  conversation; its header says the saved model is unavailable, offers the usable models and **Log in**. Picking a
  model resumes the same conversation with it; omfp never picks one for you.
- `:logout <provider>` removes omp's stored credentials of the provider and says how many it removed and what still
  authenticates it (e.g. an environment variable).
- If your standalone omp (the one you run in a terminal) holds provider logins, the welcome also offers `Use my omp
  logins (<providers>)`, and `:import-logins` does the same at any time. Nothing is copied until you ask: you, or an
  agent or MCP client with Drive acting for you. omfp makes a read-only copy of your standalone omp's login database
  in its private data folder, runs omp's own `omp auth export` on that copy (deleting it right after) and pipes it
  into `omp auth import -` (in omfp's profile): only model-provider logins, never configuration, `models.yml`,
  `mcp.json` or MCP server credentials; nothing of your standalone omp is written. `Copied omp's logins: <providers>`
  confirms it, and threads see the new models as after a login. Copying again is harmless. The database is found from
  omfp's environment (not omp's `.env` files; in the Flatpak, one under the host's `$XDG_DATA_HOME/omp` is missed).
- A browser (OAuth) login copied this way then lives in both apps. A provider that rotates its tokens may later sign
  one of the two out; log in again there.

Instead of logging in, give omp an API key: an environment variable such as `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` or
`GEMINI_API_KEY` in the daemon's environment (omp inherits it, so restart the daemon after setting one), providers and
keys in `<data-dir>/omp-profile/agent/models.yml`, or keys stored in that profile's `agent.db`.

The profile is normally `~/.local/share/omfp/omp-profile` on Linux (`$XDG_DATA_HOME/omfp/omp-profile` when set) or
`~/Library/Application Support/omfp/omp-profile` on macOS; `--data-dir` relocates it. User `config.yml`, `models.yml`,
`mcp.json`, agent definitions and `APPEND_SYSTEM.md` belong in its `agent/` directory. Existing users start with this
fresh profile: log in again, or copy standalone omp's provider logins as above; configuration is never copied and
there is no sharing switch or standalone fallback. Threads/history and workspace `.omp/` config are kept. `HOME` stays
unchanged, and explicit API keys inherited from the daemon's environment still work. `:logout` affects only this
profile's store.

## Language servers

omp's `lsp` tool (diagnostics, definitions, references, rename, hover; in a thread a command of the `bash` tool) and
the diagnostics omp reports after an edit need a language server for the file's language. omp finds servers by itself,
from its built-in list (`rust-analyzer`, `typescript-language-server`, `nixd`, …); omfp configures none.

- **Per repository**: a thread's omp runs in the workspace folder. A server whose root marker (`Cargo.toml`,
  `package.json`, `flake.nix`, …) sits in the workspace folder itself is available at once; one whose marker sits in a
  repository starts when a file of that repository is first touched (an `lsp` call on the file, an edit), and the
  other repositories it serves join it as workspace folders when one of their files is. Before that `lsp status` may
  answer `No language servers configured for this project`: ask about a file first (the `diagnostics` action on it),
  then `lsp status` lists the servers and whether they started.
- **Why none is found**: omp looks for a server's binary in the repository's own bin folders (`node_modules/.bin`, a
  virtualenv, …), then on the `PATH` it inherits, which is the **daemon's** environment (as for jobs:
  omfp://docs/widgets, Jobs). A dev shell (`nix develop`, direnv) does not count: a server only it provides is not
  found, and that language's files answer `No language server found for this action` (or `… for this file`).
- **Getting one**: install the server where the daemon's `PATH` sees it (a changed `PATH` needs a daemon restart), or
  declare it in `<workspace>/.omp/lsp.json` with an absolute `command`. A dev shell's binary is found with
  `cd <repo> && nix develop -c sh -c 'command -v rust-analyzer'` (its path changes when the shell does):

  ```json
  { "servers": { "rust-analyzer": { "command": "/nix/store/…-rust-analyzer-…/bin/rust-analyzer" } } }
  ```

  A built-in server needs only the fields it changes (`command`, `args`); a new one also `fileTypes` and
  `rootMarkers`. Servers for every workspace go in the omp profile's `agent/lsp.json` (see Logging in; standalone
  omp's `~/.omp/agent/lsp.json` is not read). omp reads these files once per thread: the `lsp` tool's `reload` action
  (`file` `*` or none) reads them again and restarts the servers whose configuration changed; restarting the thread
  does too.
- **For agents**: a missing server is not something to raise. Without one, fall back to `grep`, `ast_grep` and reading
  the code without comment: do not report the missing server, nor suggest installing or configuring one, unless the
  user asks why `lsp` does not work or how to enable it; then explain the above. Never install a server or change the
  user's system (packages, `PATH`, the profile's `lsp.json`) yourself; write `<workspace>/.omp/lsp.json` only when the
  user asks for it.

## Agents and the window

Each window reports to the daemon what it shows — its thread, focus, views and where each comes from, the commands
init files and widgets added, its errors — which agents read with `omfp_windows`. An agent runs a command in a window
with `omfp_window_run` (`view.switch` with a view id shows that view), as `:<command> <args>` would: only registered
commands, never code. `omfp_widget_check` mounts a widget no view shows out of sight for a moment to report how it
renders; nothing of it appears on screen.
