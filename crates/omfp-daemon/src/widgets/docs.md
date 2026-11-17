# omfp widgets

A widget is a piece of the omfp window written in React. omfp shows the widgets a workspace's **views manifest**
lists, reloads a widget as soon as one of its files changes, and shows any error in the widget's place.

## Files

- `<workspace>/.omfp/views.json` — the views of the window (schema: `omfp://schemas/views.json`). Without it the window
  shows the default view:
  `["row", { "sizes": [3, 1] }, ["col", {}, "omfp:header", ["tabs", { "id": "main" }, "omfp:chat"]], ["tabs", { "id": "side" }, "omfp:workspace", "omfp:changes", "omfp:background", "omfp:mcp"]]`.
  A workspace that has its own `views.json` keeps it as it is (omfp never rewrites it): a widget the default view
  gained later, such as `omfp:mcp`, comes in with `:widget add omfp:mcp` there, or an agent's
  `omfp_ui_mount`. `:view-reset` replaces the whole file with the default view (after a confirmation), losing every
  view and mount the workspace added.
- `<workspace>/.omfp/widgets/<name>/index.tsx` (or `index.ts`, `index.jsx`, `index.js`) — a workspace widget,
  referenced as `./widgets/<name>`. Its default export is a React component; its props are the layout node's props.
  Other files of the folder are imported with relative paths (`./chart.tsx` or `./chart`).
- `<data-dir>/widgets/<name>/` — the user library, referenced as `user:<name>` in every workspace.
- A workspace widget named like a built-in (`.omfp/widgets/chat/` for `omfp:chat`) replaces the built-in in that
  workspace. The built-ins' sources can be copied as a starting point (`:widget copy omfp:workspace` in the window).
- `<workspace>/.omfp/keymap.json` — this workspace's key bindings (schema: `omfp://schemas/keymap.json`, see Keys).
- `<workspace>/.omfp/views/<name>.tsx` — a TSX view (see TSX views), named in `views.json` as
  `{ "title": "Ops", "view": "./views/ops.tsx" }` instead of a `layout`.
- `<workspace>/.omfp/init.ts` and the user's `init.ts` (`$XDG_CONFIG_HOME/omfp/init.ts`, else `~/.config/omfp/init.ts`,
  next to `theme.css`) — init files run by every window (see Init files).

```json
{
  "$schema": "omfp://schemas/views.json",
  "default": "work",
  "views": {
    "work": {
      "title": "Work",
      "layout": ["row", { "sizes": [3, 1] },
        ["col", {}, "omfp:header", ["tabs", { "id": "main" }, "omfp:chat"]],
        ["tabs", { "id": "side" }, "omfp:workspace", ["./widgets/tests", { "command": "make test" }]]]
    }
  }
}
```

Layout nodes: a reference (`"omfp:chat"`), `[reference, props]`, or `[container, props, ...children]` where the
container is `row`, `col`, `tabs` or `stack` and `props.sizes` (optional) gives one positive flex weight per child.
Built-in widgets: `omfp:header`, `omfp:chat` (transcript, interactions, queue and composer), `omfp:transcript`,
`omfp:interactions`, `omfp:composer`, `omfp:workspace`, `omfp:changes` (`{ repo? }`: changed files and diffs of the
workspace's repositories, or of the repository named `repo` only), `omfp:background` (the selected thread's background
jobs and subagents: state, duration, last output line; stop a job, cancel a subagent; a subagent's row opens its kind,
use, transcript and, while it runs, a steer field under it), `omfp:todos`
(its todo list and goal), `omfp:debug` (its debug session from the agent's `debug` tool results: status, where
it stopped, the stack and the variables the agent last read there, the breakpoints; each place opens the file at its
line; read-only), `omfp:tree` (its session tree: go to an entry, fork a new thread from it with `threads.fork`,
label it), `omfp:activity`
(`{ threadId? }`: the activity log of what agents did through omfp, `activity.list`), `omfp:mcp` (the MCP servers of
the selected thread, checked by its omp, or those configured for the workspace: state, tools, error, config file;
reconnect = restart the thread, `thread.mcp_servers`, `thread.mcp_reconnect`; Reauthorize on an `http`/`sse` server
sends the prompt `/mcp reauth <name>` (`thread.prompt`; its output and the URL omp opens show in the chat);
while the thread's summary has an MCP reason in `restartPending`, "Reload to load MCP tools" = `thread.mcp_reconnect { threadId }`),
`omfp:file`
(`{ path, line?, diff? }`: a file, viewed, edited with vim bindings, diffed against `HEAD`; video and audio files play
in the browser's player; `.pdf` files show in `PdfView`; images, any size, in `ImageView`, compared with `HEAD` when
they differ; Markdown files show rendered, CSV/TSV files as a sortable table and JSON/YAML files as a tree while
read-only, with a toggle to the source), `omfp:web` (`{ url, title? }`: a web page — a dashboard, docs, a local dev
server's preview — in a sandboxed frame under a toolbar with the URL, Reload and Open in browser; `url` is any absolute `http://` or `https://` URL,
`localhost` and `127.0.0.1` included, except omfp's own address; `title` labels the tab. Place one with
`omfp_ui_mount { id: "preview", widget: "omfp:web", props: { url: "http://localhost:5173/" } }` and point it elsewhere
with `omfp_ui_update { id: "preview", props: { url } }`. The page cannot navigate the window; its new windows open in the
browser. Sites that refuse framing (`X-Frame-Options`, CSP `frame-ancestors`: GitHub, Linear, Google) stay blank, and
the frame says so when nothing loaded in 10 s: reach them through their API in a widget of your own),
`omfp:terminal` (`{ command?, shell?, cwd?, cols?, rows?, key? }`: an interactive CLI on a daemon-owned PTY, run by
`sh -c` in the workspace folder or the workspace-relative `cwd`. Without props it starts the daemon's `$SHELL` (`sh`
when unset). `shell` picks another shell: a program name or path and its arguments, separated by spaces and each
passed literally (`"zsh"`, `"/run/current-system/sw/bin/fish"`, `"bash -l"`), as in
`omfp_ui_mount { id: "shell", widget: "omfp:terminal", props: { shell: "zsh" } }`. `command` runs any other command
line, e.g. `"lazygit"`, and wins over `shell`. Its session survives widget remounts while the daemon runs),
`omfp:browser` (the browser tabs the selected thread's agent drove with omp's browser in its eval cells: name, the
address of its last `open` or `goto`, its last action as omp describes it, closed or not, and the latest screenshot its
calls produced, opening at full size; read from the eval calls' results, since omp exposes no live page over RPC).
Files opened by the user open as tabs after the children of the `tabs` container with `"id": "main"`, never
written to `views.json`. Keep the main slot, `["tabs", {"id": "main"}, …"omfp:chat"]`, in every view: the files the
user opens become its tabs (without it they float in a panel). `:widget add` and `omfp_ui_mount` put side tabs in the
`tabs` container with `"id": "side"`. Props every widget accepts: `instance` and `title` (a tab's label; default: the
built-in's name, else the reference). `instance` names the node's state: `useStored` and `useJob` keep their state per
workspace, instance and key (default instance: the reference), so two nodes of the same widget with the same instance
share it — give each its own `instance` when a widget appears twice (`["./widgets/tests", { "instance": "tests-api" }]`).
It also names the node in `omfp_widget_check`'s report. Built-ins keep no stored state (`omfp:changes` twice, one per
`repo`, works without it), but an `instance` per node costs nothing. `omfp:subagents`, merged into `omfp:background`,
is no built-in any more: an older `views.json` naming it shows `omfp:background` there (titled "Background"; a
workspace folder `.omfp/widgets/subagents/` still replaces it); write `omfp:background` in new layouts.

## Views in the window

- The window shows one view at a time. With several views the mode line (the bar at the bottom) shows them as small
  tabs, `Default · Review · Ops`, the current one highlighted: a click switches. `:view <id>` switches too (`:view`
  completes ids); `Space v` opens the fuzzy view picker, whose first nine matching rows show `1`–`9` for direct selection.
  Arrows and Enter choose too; Escape cancels. `] v` / `[ v` go to the next / previous view.
  `omfp://docs/shell` describes the rest of the window (left column, mode line, `:` commands, keys).
- **Switching keeps the last views mounted**: besides the view it shows, the window keeps the 2 views of the current
  workspace shown most recently before it, hidden. A hidden view's widgets keep their React state and DOM, but their
  effects are cleaned up as on unmount — subscriptions, `useFile` polls, timers you clear in a cleanup, `useCommand`
  commands and the widget's report to agents all stop — and run again when the view shows again, which shows it as
  it was at once. So write effects that clean up after themselves and refresh when they run again; don't rely on
  an unmount to reset state. A third view pushes out the least recently shown one; views of another workspace and
  views removed from `views.json` are unmounted.
- The window remembers the view per thread and survives reloads (kept in the browser's `localStorage`): selecting a
  thread shows the view last used with it; a thread never shown before (and a new thread being started) gets the view
  last chosen in its workspace in this window, else the default view: `views.json`'s `default` (else its first id).
  While the workspace has no view of the remembered id, the workspace's last chosen view shows, else the default; the
  choice comes back with the view. Every switch (tab, `:view`, keys, an init file's `omfp.run("view.switch", [id])`)
  is the selected thread's and its workspace's last chosen. Init files keep the default when they use or transform
  views: a view `views.add` adds never becomes the default by itself, only `views.withDefault(id)` changes it (see
  Views of init files).
- Splitters between the cells of a `row` or `col` resize them for this window only (a double click goes back to the
  layout's `sizes`); nothing the user does by hand rewrites `views.json` except `:widget add <reference>`, which adds
  a widget (built-in, `./widgets/<name>`, `user:<name>`) to the current view as the last tab of its `tabs` with
  `"id": "side"`, else at the end of its root container. It refuses views that exist only in code (an init file's
  `views.add`, a TSX view) and names the file to edit. Agents change views with `omfp_ui_mount` and `views.set`.
- A **reveal** (`REVEAL_WIDGET`: the header's background badge and workspace chip, or any widget's
  `useWindowState(REVEAL_WIDGET)` setter) shows a widget: the view's tab holding
  it is selected; a view without it opens it as a closable tab of the `main` slot after the open files (titled like a
  tab, `omfp:background` → "Background"; revealing it again selects that tab), kept in this window per workspace across
  reloads and never written to `views.json`; only a view without a `main` slot floats it in a panel. The tab's node has
  no props, so its instance is the reference: its `useStored` state is that of every node of the widget without
  `instance`.

## Mounts, panels and the library

An agent shows UI to the user without editing `views.json` by hand. Place a widget with omfp_ui_mount { id, widget |
code, props?, view?, place?, panel? } (`place`: `side_tab`, the default, `main_tab` or `end`), never by rewriting
views.json: mounting an id again replaces it, omfp_ui_update { id, props } drives it, omfp_ui_remove { id } removes it.

- `omfp_ui_mount` mounts a widget: `code` (a component's source, written to `.omfp/widgets/<id>/index.tsx`) or
  `widget` (a reference: `user:<name>`, `./widgets/<name>`, `omfp:<name>`). It goes into the view `view` (default: the
  default view; a new id creates a view showing only it, a dashboard) at `place`: `side_tab` (the last tab of the
  `tabs` with `"id": "side"`), `main_tab` (the last tab of the main slot) or `end` (the end of the view's root
  container); with `panel: true` it floats over every view instead. The node gets `instance: <id>`, so `useStored`
  state follows the mount. `code` is refused (`conflict`) when `.omfp/widgets/<id>/` holds a widget no mount with
  `code` wrote, or when `<id>` names a built-in (`chat`, `file`…): pick another id.
- `omfp_ui_update { id, props?, code? }` merges `props` into the mounted widget (`null` deletes one) or replaces its
  inline code; windows re-render it in place, its state kept. Drive a mounted widget (a progress, a status) this way.
- `omfp_ui_remove { id }` removes it (a view it filled goes too); its folder stays.
- Floating panels are `"panels": { "<id>": <node> }` in `views.json`: the `title` prop labels the panel, the user moves,
  resizes, minimises and closes it (closing removes it from the file).
- The component library is the user library: `omfp_component_save { name, widget | code, replace? }` saves a
  workspace widget or code as `user:<name>`, `omfp_component_list` lists it. Start a component's entry with a comment
  saying what it shows and which props it takes: the list shows it.
- Widgets reach the same through `useCall()`, with the same arguments (`place` included) plus `workspaceId`:
  `views.mount`, `views.mount_update`, `views.unmount`, `widgets.save`, `widgets.library`.

## Keys

`<workspace>/.omfp/keymap.json` binds keys to commands in the windows showing this workspace, over the default keys and
the user's `[keymap]` config: entries apply in file order, an entry replaces the binding with the same mode and keys,
and this file wins. omfp only reads it; while it is invalid, the window shows its error and keeps the last valid
bindings. A binding runs a built-in command (`:help` lists the keys and commands in the window, `omfp://keymap` lists
the default keys and the built-in commands with their titles and aliases for agents) or one a widget registers with
`useCommand` (it runs while the widget is shown), so a widget can ship a command and the key that runs it:

```json
{
  "$schema": "omfp://schemas/keymap.json",
  "bindings": [
    { "keys": "Space T", "command": "tests.run" },
    { "keys": "g l", "command": "view.switch", "args": ["logs"] },
    { "mode": "insert", "keys": "Ctrl-t", "command": "tests.run" },
    { "keys": "Space x", "command": "" }
  ]
}
```

`mode` is `normal` (the default: the focus is not in a text field), `insert` (a text field has it) or `cmdline` (the `:`
line is open). `keys` are tokens separated by spaces: a printable key is its character (`j`, `G`, `:`), other keys
their name (`Space`, `Esc`, `Enter`, `Tab`), prefixed by `Ctrl-`, `Alt-`, `Meta-` (`g g`, `Space t`, `Ctrl-k`). `args`
are passed to the command's `run(args)`; `"command": ""` removes a binding.

While a started sequence waits for its next key (`g`, `Space`, `Space h`), the window shows the which-key
panel after 300 ms: every key that completes it with the title of the command it runs now, or the group of the longer
bindings it starts (`h → help…`). It is built from the bindings in force, so the bindings of this file, of init files
and the titles of widget commands show there as soon as they apply; give a command a short `title`. Escape cancels the
sequence. `Space` alone opens it at once: the leader menu. The leader map:

| Keys | Command |
| --- | --- |
| `Space Space`, `Space :` | the `:` command line (`:` too) |
| `Space a` / `n` / `r` / `x` / `u` / `d` | next waiting thread / new thread / rename / archive / unarchive / delete |
| `Space s` / `S` / `R` | stop the thread / with the threads it spawned / restart it |
| `Space w`, `Space p n`, `Space p r` | new workspace, again, rescan the workspace |
| `Space t` | focus the threads column (wide), show or hide it (narrow); then `za` folds the focused workspace |
| `Space v`, `] v` / `[ v` | fuzzy view picker (visible `1`–`9`, arrows and Enter); next / previous view |
| `Space f` | fuzzy file picker across all workspace repositories and the folder; each row shows repository and containing folder; arrows/Enter open the exact file, Escape cancels |
| `Space h h`, `Space h k` | the help panel (`?`, `:help`), `:describe-key` |
| `u` | normal-mode restoration of this thread's last Ctrl-C-cleared draft (text, images and paste chips), not general undo; leave the composer with Esc first |

Bind your own under a free letter (`Space T` above, or groups like `Space o a`, `Space o b`); a binding of a group's own
keys (`Space h`) replaces the group.

## Init files

An init file is the window's `init.el`: an ES module every window imports at startup and runs again whenever it changes.
The user's `init.ts` runs first, then the current workspace's `.omfp/init.ts` (switching to a thread of another
workspace swaps the latter). Both compile like widget files (TypeScript, JSX, the same imports; a single file: no
relative imports). Do the work in the default export, `export default function init(omfp: InitApi)` (the types
`InitApi`, `ViewsApi` and `Layout` come from `omfp/sdk`: `import type { InitApi } from "omfp/sdk"`; types are
stripped, so a plain `.js`-style `init(omfp)` works as well); return a cleanup function if something must be undone. Before the file runs again, or when it goes away, the window calls that cleanup and removes
everything the run registered through the API, so a run never needs to undo its own `command`, `bind`, `views.use`,
`views.transform`, `panel` or `on`. A file that read `views.json` (`views.load`) or the repositories
(`workspace.repos`) also runs again when they change.

```ts
import type { InitApi } from "omfp/sdk";

export default function init(omfp: InitApi) {
  omfp.command({
    id: "tests.run",
    title: "Run the tests",
    description: "Asks the selected thread's agent to run the test suite.",
    aliases: ["tests"],
    run: () => omfp.notify({ title: "tests", body: "started" }),
  });
  omfp.bind([
    { keys: "Space T", command: "tests.run" },
    { mode: "insert", keys: "Ctrl-t", command: "tests.run" },
    { keys: "Space x", command: "" },
  ]);
  omfp.panel("clock", ["user:clock", { title: "Clock" }]);
  omfp.on("attention", (thread) => omfp.notify({ title: thread.title ?? thread.id, body: "needs you", threadId: thread.id }));
  const timer = setInterval(() => {}, 60_000);
  return () => clearInterval(timer);
}
```

| API | What it does |
| --- | --- |
| `file`, `workspaceId` | `"user"` or `"workspace"`; the workspace whose file this is (`undefined` for the user's). |
| `command(command)` | Registers a `:` command / key binding target: `{ id, title, description?, aliases?, scope?, args?, enabled?, run(args) }` (`useCommand`'s shape; `registerCommand(host, command)` from `omfp/sdk` is the same outside an init file). |
| `bind(binding \| bindings)` | `{ keys, command, args?, mode? }` (`mode` default `normal`, `command: ""` removes a binding), applied after the defaults, the user's `[keymap]` and `.omfp/keymap.json`: the user's init file, then the workspace's, win last. |
| `views.load("views.json")` | The workspace's `views.json` as views (the default view while it is missing or invalid): see Views of init files. `"views.json"` is the only file it loads; any other argument throws. |
| `views.use(views)` | Shows `views`, as they are at the call, in this window instead of `views.json`'s (the workspace's file wins over the user's). |
| `views.transform(fn)` | `fn(views)` changes the views it gets (or returns other views): applied after `use`, the user's file's first, each in registration order; one that throws changes nothing. |
| `workspace` | `{ id, repos }`: the current workspace and its repositories (`{ name, path, kind }`, `kind` as `omfp:workspace` shows it: `link`, `worktree`, `clone`, …). |
| `panel(id, node)` | Floats a layout node over every view (its `title` prop labels it); closing it hides it until the file runs again. |
| `on(event, listener)` | `"attention"` (a thread starts needing the user), `"failed"`, `"finished"` (a turn ended): `listener(thread)` gets the thread summary. Use it for auto-actions, e.g. select or notify. `"job_started"`, `"job_ended"`, `"job_failed"`: `listener(run)` gets the run of a declared job (see Declared jobs); notify or show, never start runs. |
| `notify({ title, body, threadId? })` | Like `useNotify()`. |
| `run(name, args?)` | Runs any command by id or alias, e.g. `run("view.switch", ["logs"])`. |
| `call(method, params)` | Any protocol method, like `useCall()`. |
| `onCleanup(fn)` | Called before the file runs again or goes away. |
| `host` | The window's SDK host (`commands`, `window` state, `notices`, `client`), for anything else. |
| `jobs` | The current workspace's declared jobs: `list()`, `get(name)` and `error` as `jobs/<id>` holds them at the call; `run(name)`, `kill(name)`, `enable(name, enabled)` (see Declared jobs). |

Command ids are dotted, `<area>.<action>` like the built-ins (`thread.archive`, `view.switch`; `omfp://keymap` lists
them): name a workspace's commands `ws.<action>` (`ws.ops`, `ws.deploy`) and the user's own `my.<action>` so they
never clash. A command whose id equals a built-in's (or another widget's) replaces it while the file is loaded — keys
bound to that id run yours — and the built-in comes back when the file goes away. `aliases` are the names typed after
`:` (`:tests`); an alias another command already answers to keeps answering to that older command, so pick unique
ones. `:describe-command <name>` shows which command a name reaches.

A layout node is checked like `views.json`'s: an invalid one throws. Any failure — a compile error, a throw or rejected
promise of the run, of a cleanup, of a transform, of a command or listener it registered — shows in a banner naming the
file; the window keeps working without it, and the next save that runs cleanly removes the banner. Windows report these
errors, and those of TSX views, to the daemon: `omfp_views_errors` (or `views.errors { workspaceId }`) lists what each
window showing the workspace sees now. Before that, `omfp_widget_check { name: "init" }` (`user:init` for the user's
file, `views/ops.tsx` for a TSX view) compiles the file as the windows import it and adds the windows' errors for it.

### Views of init files

Without an init file, or one that never calls `views.use`, the window shows `views.json`'s views (that is the default
init). Everything is window-local: only `:widget add` and agents (`omfp_ui_mount`, `views.set`) write `views.json`, and
an init file that read it runs again with the new file.

```ts
// ~/.config/omfp/init.ts: every workspace gets the Todos tab.
import type { InitApi } from "omfp/sdk";

export default function init(omfp: InitApi) {
  omfp.views.transform((views) => {
    views.update("default", (layout) => layout.addTab("side", "omfp:todos"));
  });
}
```

```ts
// <workspace>/.omfp/init.ts
import type { InitApi } from "omfp/sdk";

export default function init(omfp: InitApi) {
  const views = omfp.views.load("views.json"); // views.json stays writable by `:widget add` and agents
  const repos = omfp.workspace.repos;
  views.add("review", {
    title: "Review",
    // The `main` slot keeps the chat and the files the user opens; one `omfp:changes` per repository, each its own
    // `instance` and tab title.
    layout: ["row", {},
      ["tabs", { id: "main" }, "omfp:chat"],
      ["tabs", {}, ...repos.map((r) => ["omfp:changes", { repo: r.name, instance: `changes-${r.name}`, title: r.name }])]],
  });
  views.add("ops", { title: "Ops", view: "./views/ops.tsx" });
  omfp.views.use(views); // the user switches with the mode line's tabs, `:view review`, `Space v`, `] v`
}
```

The default view stays `views.json`'s (`default`, else its first id): views `add` adds appear after `views.json`'s in
the mode line's tabs and never become the default by themselves. `views.withDefault("review")` makes one the default:
a window opens it unless the user chose another view (that choice is kept across reloads).

Views and layouts change in place, and every change returns them, so calls follow one another or chain
(`views.add(…).withDefault(…)`).

| Views | What it does |
| --- | --- |
| `ids`, `default` | The view ids in the order of the mode line's tabs; the default view's id. |
| `get(id)` | `{ id, title, layout, view, origin }`: `layout` the view's `Layout`, whose edits change the view (`undefined` for a TSX view); `origin` `"views.json"` (also once init files changed its layout) or the init file that added the view. |
| `add(id, { title?, layout \| view })` | Adds the view, or replaces the one with that id in place (it is then the file's); `layout` is a node or a `Layout`, `view` a TSX file `./views/<name>.tsx`. |
| `update(id, fn)` | `fn(layout)` edits the view's `Layout`, or returns its new layout (a `Layout` or a node). |
| `remove(id)`, `withDefault(id)` | Removes the view (the default goes back to the first view when it was the default); makes another view the default. |

| Layout | What it does |
| --- | --- |
| `node` | The layout node, as in `views.json`. |
| `addTab(tabsId, widget)` | Adds `widget` as the last tab of the `tabs` with that `id` (throws when there is none). |
| `removeWidget(reference)` | Removes every widget `reference` (containers left empty go too). |

`:widget add` writes `views.json`. On a view of `views.json` that init files only changed (the user's `addTab` above),
it adds the widget to the view as `views.json` holds it, then the init files change the new file again. On a view an
init file added (`views.add`) it writes nothing and names the file to edit.

## TSX views

A view can be a TSX file of `<workspace>/.omfp/views/`, compiled like widget files (the same imports; relative imports
of other files of `.omfp/views/`) and reloaded on every save: `{ "title": "Ops", "view": "./views/ops.tsx" }` in
`views.json`, or `add("ops", { view: "./views/ops.tsx" })` in an init file. Its default export is a React component
(hooks work: re-rendering it changes the layout) returning `omfp/sdk`'s layout components:

```tsx
// <workspace>/.omfp/views/ops.tsx
import { Row, Col, Tabs, Widget } from "omfp/sdk";

export default function Ops() {
  return (
    <Row sizes={[3, 2]}>
      <Tabs id="main"><Widget use="omfp:chat" /></Tabs>
      <Col sizes={[1, 1]}>
        <Widget use="./widgets/deploy" props={{ env: "staging" }} />
        <Tabs><Widget use="omfp:background" /><Widget use="omfp:mcp" /></Tabs>
      </Col>
    </Row>
  );
}
```

`Row`, `Col`, `Stack` (`sizes`: default flex weights) and `Tabs` (`id`; `id="main"` is where files open) nest the
others; `Widget` is a widget (`use`: its reference, `props`: its props). They are the layout nodes of `views.json`
(`<Row sizes={[3, 2]}>` is `["row", { "sizes": [3, 2] }, …]`), rendered the same way: the splitters resize the cells
for this window (kept by workspace, view and path; a double click goes back to `sizes`), files open as tabs of
`<Tabs id="main">` (without one, in a floating panel). The file holds one layout and only these components, nothing
else. A compile error, a throw while rendering or an invalid layout shows a banner naming the file and the default
view in its place until the next save works (windows report it: `omfp_views_errors`). `:widget add` refuses a TSX
view: edit the file.

`:describe-key` then a key sequence shows what it runs in every mode, the command's title and description, and where
it was bound (default keymap, `[keymap]`, `.omfp/keymap.json`, the user's or the workspace's init file);
`:describe-command <name>` shows a command's names, scope, who registered it and every key bound to it. Point the user
at them to check what an init file did.

## Checking your work in the window

Errors mostly show only in the user's window, so check there. Verify in the user's window: write → omfp_widget_check {
name } (it renders a widget no view shows out of sight) → omfp_windows (each window's views and where each comes from,
added commands, errors) → `omfp_window_run { command: "view.switch", args: ["<view>"] }` to show a view → omfp_windows
again for its errors. Step by step:

1. Write the widget, `.omfp/init.ts` or the TSX view.
2. `omfp_widget_check { name }` (`init`, `user:init`, `views/<file>` for the others): the daemon's compile, then what
   each window showing the workspace renders. A widget no window shows in its current view (placed only in an init
   file's or a TSX view, or not placed yet) is mounted out of sight in the most recently focused window — a hidden
   container outside every view, with the same error boundary and SDK — for a few seconds, and its render or error is
   reported; without a window only the compile check applies. When no view holds the widget it names the
   `omfp_ui_mount` call that places it, else the views that hold it.
3. `omfp_windows`: each window showing the workspace by id, the most recently focused first —
   its thread, its views after `views.json`, the init files and their transforms (where each comes from, its TSX
   file, its widgets, the active one), the commands init files and widgets added with their keys, and its errors.
4. `omfp_window_run { command: "view.switch", args: ["<view>"] }` shows a view in that window (`windowId` picks one);
   any registered command id or alias runs the same way, as `:<command> <args>` would (`window.theme` with a scheme
   and/or a built-in theme; the former ids `theme.set` and `widget` still name `window.theme` and `widget.manage`). It never runs code: an unknown
   command, a disabled one or a failure comes back as the tool's error.
5. `omfp_windows` again (or `omfp_views_errors`) for the errors of what it shows now; fix and repeat.

Over the protocol the same steps are `windows.list`, `windows.state`, `windows.run` and `windows.check_widget`.

## Imports

Only these specifiers resolve; everything else must be a relative file of the widget's folder:

- `react`, `react/jsx-runtime`, `react-dom` — the window's own React 19 (hooks work across widgets and the shell).
- `omfp/sdk` — hooks over the daemon (below).
- `omfp/kit` — the shell's components (below), styled by the theme. Build every control from them, never from native
  `<select>`, `<input type="checkbox|radio|range|date|color">` or `<datalist>` (see Form controls).

JSX needs no import. TypeScript types are stripped, not checked. CSS files cannot be imported: use the kit, inline
`style`, or the theme's custom properties (`var(--omfp-primary)`, `var(--omfp-surface-container)`,
`var(--omfp-on-surface)`, `var(--omfp-outline)`, `var(--omfp-s2)`, `var(--omfp-fs-sm)`, `var(--omfp-font-code)`, …).
Build on the kit's components to follow the user's skin too (`[ui] skin`: Crush, or the plain `boring` one): the kit
restyles them under `<html data-skin="boring">`, while a widget's own inline styles stay as written in either skin.

```tsx
import { Button, Card, Log, Progress } from "omfp/kit";
import { useJob } from "omfp/sdk";

export default function Tests({ command = "make test" }: { command?: string }) {
  const job = useJob(command);
  const done = job.stdout.filter((line) => line.startsWith("ok")).length;
  const running = job.status === "running";
  return (
    <Card title={`Tests ${job.status} (run ${job.runs}${job.exitCode === null ? "" : `, exit ${job.exitCode}`})`}>
      <Progress label="Tests" value={done} max={20} />
      <Log lines={job.lines.slice(-10)} />
      <Button onPress={running ? job.kill : job.restart}>{running ? "Stop" : "Run again"}</Button>
    </Card>
  );
}
```

## omfp/sdk

| Export | What it gives |
| --- | --- |
| `useWidget()` | `{ instance, reference, workspaceId }` of the calling widget. |
| `useStored(key, initial)` | `[value, set, loaded]`: JSON state the daemon keeps per workspace, widget instance and key, shared live by every window and kept across reloads and restarts. `set(value)` or `set((previous) => next)`; `set(null)` deletes the key. `loaded` is `false` while the state has not arrived yet (`value` is then `initial`). |
| `useJob(command, { key? })` | `{ status, lines, stdout, stderr, exitCode, signal, runs, startedAt, endedAt, state, refreshing, restart, kill }` for `sh -c <command>` run by the daemon in the workspace folder (environment, limits and retention: see Jobs below). `status` is `"pending"` (no job yet, or a start or restart on its way), `"running"`, `"succeeded"` (exit code 0) or `"failed"` (another code, or a signal). The first mount starts the job; reloads, other windows and moving the widget follow the same job (stored as `job:<command>`: `{ id, runs }`). `lines` are stdout and stderr interleaved, `stdout` and `stderr` each one's lines; `runs` counts the jobs started (a restart adds one), `startedAt`/`endedAt` (ms) date the current one; `restart()` kills a running job and starts a new one; `state` is the raw `job/<id>` state. A mount following the job again (the widget mounted again, its view shown again) shows the window's last copy of it at once, `refreshing: true`, until the daemon's state of the job arrives. Without `key` every distinct command is its own job and its own stored key, so a command built from input (a search per query) leaves one job behind per query: pass `key` (``useJob(`rg -n ${query}`, { key: "search" })``) to keep one job slot (stored as `jobkey:<key>`: `{ id, runs, command }`): a new command under the key replaces the previous job (stopped if it still runs, its output gone; `pending` with no lines until the new one is known), and `restart()` replaces it the same way. `omfp_widget_check` shows each job's status and last lines. |
| `useJobs(workspaceId?)` | `{ jobs, error, loaded, run(name), kill(name), enable(name, enabled) }`: the jobs the widget's workspace (or `workspaceId`) declares in `.omfp/jobs.json`, live from `jobs/<id>` (see Declared jobs). `jobs` is `[]` and `loaded` `false` until the state arrived; `error` says why the file is refused. `run` calls `jobs.run`, `kill` `jobs.kill` on the job's running run, `enable` `jobs.enable`; each rejects with the method's error. A job's runs: `useCall("jobs.runs", { workspaceId, job })`. |
| `useTerminal(command, { cwd?, cols?, rows?, key? })` | `{ id, state, status, refreshing, runs, input(data), resize(cols, rows), screen(options?), kill() }` for an interactive `sh -c <command>` on a daemon-owned PTY. The hook stores the session id per widget instance, so remounts and other windows follow the same `terminal/<id>` instead of starting another process. `cwd` is workspace-relative. Input accepts text or bytes and is never stored; output is memory-only and bounded (see Terminal sessions below). `screen({ afterRevision?, waitMs? })` reads the visible plain-text screen and may wait at most 30 seconds for its revision to change. |
| `useToolRun(toolCallId)` | A tool run of the selected thread, or `useToolRun({ match: /nextest/, threadId? })` for the latest run whose command matches: `{ toolCallId, toolName, status, output, lines, isError }` or `undefined`. While a `bash` command runs, `output` is the latest tail omp streams. |
| `useCall()` | `call(method, params)`: any daemon protocol method, e.g. `call("thread.prompt", { threadId, text })`; `call("thread.session", { threadId })` gives a thread's session id, session file, model, cwd, roots and cost without starting omp. |
| `useCall(method, params)` | `{ data, error, refreshing, refresh }`: calls `method` on mount, whenever `params` change (compared as JSON) and on `refresh()`; `params` `null` calls nothing. A new mount (the widget mounted again, its view pushed out and shown again) shows the window's last result of the same widget instance, method and params at once with `refreshing: true` until the new one replaces it; `data` is `undefined` only before the first. A failure keeps `data` and sets `error` (a `CallError` for the daemon's errors), cleared by the next success. Show "updating…" from `refreshing` if you want; otherwise the old data simply gives way to the new. |
| `useTopic(topic)` | Follows a topic (`index`, `thread/<id>`, `workspace/<id>`, `views/<id>`, `job/<id>`, `terminal/<id>`) while mounted; returns its subscription state. The 5 threads most recently unmounted stay followed, so showing one again is immediate. |
| `useSnapshot(select)` | `select(snapshot)` over the window's per-frame model (`connection`, `index`, `threads`, `workspaces`, `views`, `jobs` and `terminals` by id, `topics`), rendering again only when its result changes (by identity): for what no other hook gives. Return a part of the snapshot, never a new object or array (it would change on every call); `useTopic` follows what is not followed yet. |
| `useIndex()` | Workspaces, threads, theme and settings (the `index` topic), and `models`: how many models omp can use (`null` until the daemon checked; `0` before the first login). |
| `useThreads()` | Thread summaries by id. |
| `useThreadSummary(threadId)` | One thread's summary (title, process state, workspace). |
| `useThread(threadId)` | `{ state, closed }`: the thread's full state (transcript, tools, queue, pending interactions). |
| `useBackground(threadId)` | What `omfp:background` shows: `{ rows, sections, running, stop(jobId), cancel(subagentId), clearFinished(), opened(row) }`, the thread's background jobs and subagents (`rows`: `{ kind, id, toolCallId, label, state, durationMs, lastLine, stopRequested, subagent }`, `subagent` a subagent's `useSubagentRows` row, `undefined` for a job; running durations ticking every second) and how many still run; `stop` asks the agent to stop a job (a steer: omp has no kill command), `cancel` cancels a subagent (`thread.subagent_cancel`); `sections` is `{ running, recent, older }` (the 5 most recently finished rows and the failed ones not opened yet in `recent`, the other finished ones in `older`, cleared ones in neither), `clearFinished` hides every finished row from this window, `opened(row)` marks a row opened (a failed one leaves `recent`). |
| `useRunningInBackground(threadId)` | How many background jobs and subagents of the thread run (the header's badge). |
| `useThreadCost(threadId)` | `{ session, subagents }` in US dollars, as the header's cost shows them (`$0.3800 (+$1.27)`; `costLabel(cost)` gives that text and its description): the session's own model calls (its assistant messages, plus what omp spent outside the transcript — titles, judgments — read with `thread.session` while omp runs) and its subagents' (their `task` results' usage, or their progress while they run when that is more); `undefined` until the thread's state arrives. |
| `useSubagentRows(threadId)` | `{ rows, refresh }`: the thread's subagents (`{ id, index, agent, status, description, task, activity, meta, durationMs, live }`), from its state plus `thread.subagents` while omp runs, read again when one starts or ends and on `refresh()`. |
| `useWorkspaces()` | Workspaces by id. |
| `useWorkspace(workspaceId)` | The workspace and its repositories' git status. |
| `useViews(workspaceId)` | The workspace's views manifest, keymap (`keymap`, `keymapError`), widget folders and stored state (the `views/<id>` topic). |
| `useConnection()` | The connection status. |
| `useWindowState(key)` | Window-local state; `SELECTED_THREAD` is the thread this window shows. |
| `useCurrentWorkspaceId()` | The workspace this window works in: the selected thread's, else the draft thread's. |
| `useActiveView(viewIds)`, `chooseView(host, viewId)` | The id among the current workspace's view ids this window shows for the selected thread (its own, else its workspace's last chosen; `undefined`: the default view), and switching it (`host` from `useHost()`), as `:view <id>` does. |
| `useHost()` | The window's host (`client`, `window` store, `commands`, …): the `host` that `chooseView`, `requestLogin(host, providerId?)` and `registerCommand(host, command)` take; widgets otherwise use the hooks. |
| `useThreadActions()` | `select`, `rename`, `archive`, `stop`, `restart`, … for threads; `compact(threadId, instructions?, mode?)` has omp compact the thread's context (as `:compact` does, `instructions` the focus of its summary, `mode` one of omp's `soft`, `remote`, `snapcompact`) with a success toast, refused while the thread works, and resolves to whether omp compacted. |
| `useCommand(command)` | Registers a `:` command or key binding target while mounted: `{ id, title, description?, aliases?, scope?, args?, enabled?, run(args) }` (`description` shows in `:describe-command`, `args(before)` completes the argument being typed, `enabled()` returning `false` hides it). With `scope`, it answers only while the focus is inside an element of that scope (`SCOPE_ATTRIBUTE`), before a global command of the same id. |
| `SCOPE_ATTRIBUTE` | `"data-omfp-scope"`: put `{ [SCOPE_ATTRIBUTE]: "my-scope" }` on your widget's root to own the commands registered with `scope: "my-scope"`; the innermost scope around the focus wins. |
| `useAnnounce()` | `announce(message)` for screen readers. |
| `useReportError()` | `report(error)`: adds a failure (an `Error`, a `CallError`, any value) to the window's error list, which the frame shows until dismissed. Use it in the `catch` of every user action. |
| `useReportSuccess()` | `report(text)`: the result of a user action (`Copied …`, `Saved …`) as a success toast that goes by itself, also announced to screen readers. Every action the user starts shows its result this way; thread and agent events are notices (`useNotify`). Toasts and notices stay in `:messages`. |
| `parseYaml(text)` | `Promise<YamlResult>`: the values of a YAML text's documents (`{ ok: true, documents }`), or its first error (`{ ok: false, message, line, column }`, from 1). The parser loads on first use. |
| `useConversationActions()` | `{ copy(threadId), copyMessage(view), export(threadId) }`: a thread's conversation as Markdown (as `:copy` and `:export` make it) to the clipboard, one transcript `MessageView` without its heading to the clipboard, or the conversation saved as a `.md` file the browser downloads; each shows a success toast, a failure goes to the error list. |
| `useUrlActions()` | `{ open(url), copy(url), relay(threadId, pasted) }`: opens a URL in a new browser tab (without access to this window), or copies it to the clipboard with a success toast (a refusal goes to the error list); `relay` hands the address a browser ended on after an OAuth sign-in omp started (the thread's, or with `threadId` `undefined` the running login's) to omp's loopback listener on the daemon's host (`oauth.callback`) and resolves `true` once omp took it (a success toast), else `false` (the refusal in the error list). |
| `OAuthCallbackField` | `<OAuthCallbackField threadId={…} url={…} onRelayed={…} />`: for an authorization URL omp opened whose sign-in returns to a loopback address (`loopbackCallback(url)`), the field "Paste the address your browser ended on" with Send (`relay` above); renders nothing for other URLs and goes once omp took the address. |
| `useNotify()` | `notify({ title, body, threadId? })`: tells the user something without covering anything. While the window has focus it is a notice at the right end of the mode line (`◆ title body`, a click or `:notice` opens it, gone after 8 s); while it has not, a system notification (when the user allowed them; `[ui] notifications = "never"` shows neither). With `threadId`, opening it selects that thread, visiting the thread clears it, and nothing is shown about the thread the user is on. Use it for results worth a look (a build turned red, a long job finished), not for every output line. |
| `useSettings()` | `{ ui, keymap, error }`: the daemon's settings (`ui.density`, `ui.fontSize`, `ui.motion`, `ui.notifications`, `ui.fonts`, `ui.theme`: the built-in theme, `ui.skin`: `crush` or `boring`; `error` when the config file failed to load), live. |
| `useSetSettings()` | `set(patch)`: `settings.set` with `{ ui?, keymap? }`, merged into the daemon's config file (`ui`: `density`, `fontSize`, `fonts`, `motion`, `notifications`, `theme`, `skin`); a failure goes to the error list. |
| `useFile(path, { follow? })` | `{ status, file, error, refreshing, reload, save }`: a file of the current workspace (`workspaces.read_file`: `file.content`, `file.hash`, `file.head`, …). `path` is relative to the workspace folder (then looked up in its repositories) or absolute; an absolute path outside the folder (an agent's worktree elsewhere, `/etc/hosts`) is read too, read-only: `file.outside` is `true`, `file.path` the absolute path, `file.repo` / `file.head` come from the git working tree holding it, if any, and `save` is refused. A relative path never leaves the folder (`..` past it is an error). `status` is `"loading"`, `"ready"`, `"missing"` or `"error"` (an error keeps the `file` read before). With `follow` (default `true`) it is checked every 2 s and on every git status change. A new mount shows the window's last copy of the file at once (`ready`, `refreshing: true`) until its first read answers. `save(content, expectedHash?)` writes it only if the file still has that hash (default: the one read) and resolves to `{ ok: true, file }` or `{ ok: false, conflict, error }`. |
| `useFileUrl(path)` | The URL of a file's raw bytes in the current workspace (`path` as `useFile` takes it, an absolute path outside the folder included), for `<video src>`, `<audio src>`, `<img src>` or `fetch(url)`: served with its type, byte ranges (seeking) and cache validators, any size, binary or not. `undefined` without a workspace or before the window connected. The URL holds a secret of the daemon's run in place of the token, valid until the daemon restarts: use it inside the window, never print or send it elsewhere. Add `?v=<hash>` to load a changed file again; `?rev=HEAD` gives the file's bytes at `HEAD` of its repository (`404` when `HEAD` lacks it). |
| `useLive()` | `[live, snapshot]`: this window's live voice session (M6). `live.start(threadId)` opens the microphone and a WebRTC call relayed to omp's realtime voice (`thread.live_connect`), or, without WebRTC on the daemon's machine (`#local=1`, the desktop app), omp's own session (`thread.live_start`, the spectrum then draws omp's levels), `live.end()`, `live.endThread(threadId)` (any window's session), `live.toggleMute()`, `live.source()` (the audio for `Spectrum`), `live.support()` (`"webrtc"`, `"host"`, `"insecure"`, `"unsupported"`); `snapshot.local` is the session (`idle`, `starting`, `running`, `failed`). The thread's `state.live` has omp's phase and transcript. |
| `useOpenFile()` | `openFile(path, { line?, diff? })`: opens a file of the current workspace in a tab next to the chat (the view's `tabs` with `"id": "main"`; keep that slot in your views), else as a tab of a floating panel; `diff: true` shows it against `HEAD`. A file has one tab whatever path reached it: a path inside the workspace folder (absolute or with `.`/`..`) becomes workspace-relative, and opening an open file again moves its tab to the new `line`. An absolute path outside the workspace folder opens too, read-only. A bare or partial name (`theme.ts`, `src/theme.ts`) that does not exist as given opens the file it names, or lets the user pick when several match. |
| `useMissingFile(path, missing, { line?, diff? })` | For a widget showing a file: while `missing`, looks for the file `path` names (`workspaces.find_file`); one match opens in its place, several open the window's picker. Gives `"finding"`, `"opened"`, `"choosing"` or `"none"`. |
| `useFileTabs(workspaceId)` | This window's opened tabs of the workspace's `main` slot: `{ tabs, widgets, active, select(id), close(path), closeTab(id) }` (files, then revealed widgets). The tabs are the workspace's; `active` and `select` are the selected thread's in the view shown (each thread remembers its tab per view); `select(undefined)` shows the slot's first tab. |
| `usePathSources()` | `{ suggest(query), complete(path) }` for the kit's `PathField`: repositories the daemon has seen (`repos.suggest`) and directories on the daemon's machine (`fs.complete`), even when the browser is remote. |
| `Row`, `Col`, `Tabs`, `Stack`, `Widget` | The layout components of TSX views (see TSX views); they render only in a view file of `.omfp/views/`. |
| `SELECTED_THREAD` | Window key of the selected thread id (`useWindowState(SELECTED_THREAD)`). |
| `REVEAL_WIDGET` | Window key of the latest reveal request, `{ reference, nonce }` (`nonce`: a new number per request): shows that widget in its tab of the view, else in a tab of the `main` slot, else in a floating panel (see Views in the window). |
| `OPEN_SUBAGENT` | Window key of a request to open a subagent's transcript and steer field under its row of `omfp:background`: set `{ threadId, subagentId, nonce }` (`nonce`: a new number per request), and reveal `omfp:background` (`REVEAL_WIDGET`) to show it. |
| `OPEN_JOB` | Window key of a request to open a declared job's row (its runs) in `omfp:jobs`: set `{ workspaceId, job, nonce }` (`nonce`: a new number per request), and reveal `omfp:jobs` (`REVEAL_WIDGET`, with `workspaceId` for another workspace) to show it; `omfp:jobs` clears it once applied. The left column's job rows do this. |
| `FOCUS_COMPOSER` | Window key of the thread whose composer takes the focus once it shows; the composer clears it. |
| `DRAFT_THREAD` | Window key of the draft thread, `{ workspaceId }` (`undefined` without one): a new thread being written before its first message is sent. |

### Jobs

`jobs.start` and `useJob` start *transient runs*: runs of no declared job, for a widget's own commands (what the
workspace runs by hand, on a schedule or keeps running belongs in `.omfp/jobs.json`, see Declared jobs). What a `useJob`
command (a `jobs.start` run, followed on `job/<id>`) runs in:

- **Shell**: `sh -c <command>`, non-interactive and not a login shell, so no `~/.profile`, `~/.bashrc`, aliases or shell
  functions; `sh` is the first one on the daemon's `PATH` (bash in POSIX mode on NixOS, `/bin/sh` on macOS). Use
  `bash -c '…'` explicitly for bash syntax.
- **Working directory**: the workspace folder, whose entries are the repositories — not a repository: write
  `cd <repo> && …` (or `git -C <repo> …`).
- **Environment**: the daemon's, unchanged — whatever started the daemon decides `PATH` and the variables: a terminal
  (`omfp daemon`, `just serve`) passes its shell's, `omfp desktop` launched from the desktop starts the daemon with the
  graphical session's (often a shorter `PATH` than your terminal's), a systemd user service with the user manager's
  (`systemctl --user show-environment`). The Nix install adds the pinned `omp` in front of `PATH` and `git` at its end.
  No dev shell applies (no `nix develop`, no direnv): write it out, `nix develop -c cargo test`, `direnv exec . make`.
- **Input**: stdin is `/dev/null` and there is no terminal: a prompt reads end-of-file, and tools that check for a
  terminal print without colour or progress bars.
- **Time**: no timeout; a job runs until it exits or is killed (`kill()`, `jobs.kill`: SIGTERM to its process group,
  SIGKILL 2 s later; removing the workspace; stopping the daemon; a cascade stop of the agent thread that started it).
  What a job detaches (`setsid`, its own process group) is out of reach.
- **Output**: stdout and stderr are read as lines (invalid UTF-8 replaced, a line longer than 64 KiB cut into pieces);
  the job keeps its newest lines up to 1 MiB of text and counts the older ones in `state.dropped`.
- **Retention**: transient runs live in the daemon's memory only. A workspace keeps its 32 most recently finished jobs; an older
  one is forgotten (its `job/<id>` is then `not_found`). A daemon restart forgets them all, so `useJob` starts the
  command again the next time the widget mounts. A job replaced under a `key` is forgotten at once. What `useJob`
  stores (`job:<command>`, `jobkey:<key>`) stays in the workspace's widget state until the widget sets it to `null` or
  the workspace is removed.

### Terminal sessions

`useTerminal` is for programs that need a real terminal: prompts, password input, full-screen TUIs, colour and cursor
control. `useJob` remains preferable for commands whose output a widget parses.

- The process runs as `sh -c <command>` in the workspace folder or an existing workspace-relative `cwd`, with the
  daemon's environment, `TERM=xterm-256color`, `COLORTERM=truecolor` and a PTY. It has no timeout.
- `input()` writes raw bytes; `resize()` updates the PTY; `kill()` terminates the child. Do not put passwords in
  `command`: commands are visible in agent activity. Input bytes are redacted there and never enter topic state.
- Output is base64 byte chunks, not text lines, so feed `state.scrollback` to `TerminalEmulator` from `omfp/kit`.
  The daemon retains at most 1 MiB per session and reports older raw bytes in `state.dropped`.
- `screen({ afterRevision?, waitMs? })` returns the visible plain-text cells, cursor, dimensions, alternate-screen flag,
  process status and revision. Passing the current revision waits up to 30 seconds for output, resize or exit. The
  emulator grid is bounded to 400×200 cells and has no additional scrollback. This explicit read may enter an agent's
  conversation; input hidden by the program is absent.
- Sessions live only in daemon memory. Reconnects and widget remounts follow the same id; daemon restart forgets it
  and the hook starts a replacement. Workspace removal, daemon shutdown and cascade stop of the creating agent kill it.

The SDK also exports the view models and strings the built-in widgets use; read a copied built-in to see them.

## Declared jobs

A workspace declares its jobs in `<workspace>/.omfp/jobs.json` (schema `omfp://schemas/jobs.json`), next to
`views.json` and `keymap.json`: named shell commands or prompts the daemon runs by hand, on a schedule or keeps running.
A *job* is the definition, a *run* one execution of it. When the user asks for something recurring or lasting — "read
my mail every morning" (a `schedule` job with a `prompt`), "watch the inbox folder" (a `service`), "a deploy I can run
with a key" (a `manual` job) — declare a job instead of looping, sleeping or starting a background command.

```json
{
  "$schema": "omfp://schemas/jobs.json",
  "jobs": {
    "test": { "description": "Fast tests", "command": "nix develop -c just test", "cwd": "omfp" },
    "mail": {
      "trigger": "schedule",
      "schedule": "0 8 * * 1-5",
      "prompt": { "text": "Read my unread mail and list what needs an answer.", "thread": "reuse" }
    },
    "inbox-watch": { "trigger": "service", "command": "./scripts/watch-inbox.sh", "restart": "on-failure" }
  }
}
```

`test` runs when asked, in the repository `omfp`; `mail` runs at 08:00 Monday to Friday, sending its prompt to the
same thread every time; `inbox-watch` is kept running and started again when it fails.

A job's name is 1–64 characters of `a-z`, `0-9`, `.`, `_`, `-`, starting with a letter or digit; it is the job's
identity (its runs, its reused thread and its schedule state), so renaming a job removes one and declares another. Its
keys:

| Key | What it is |
| --- | --- |
| `description` | Optional, at most 200 characters: what the job is for. |
| `trigger` | `manual` (default: runs only when asked), `schedule` (runs at the times of `schedule`) or `service` (kept running). |
| `schedule` | A cron expression (below); required with `trigger: "schedule"`, refused otherwise. |
| `missed` | `skip` (default) or `run`, only with `schedule`: what a missed time does (below). |
| `command` | A non-blank shell command run as `sh -c`. Exactly one of `command` and `prompt`; a `service` needs `command`. |
| `prompt` | `{ text, thread?, model?, thinkingLevel?, approvalMode? }`: `text` not blank; `thread` `new` (default) or `reuse`; `model` as `provider/id`, `thinkingLevel` and `approvalMode` as `threads.create` takes them (else the defaults). |
| `cwd` | Only with `command`: a folder of the workspace, relative to the workspace folder (it may not leave it); default the workspace folder. |
| `restart` | Only with `service`: `always` (default) or `on-failure`. |
| `enabled` | Default `true`; a disabled job never runs, by schedule or by hand. |

Any other key (at the top level besides `$schema`, in a job, in a prompt) is refused, so a mistyped key never silently
changes when something runs. The daemon reads the file at startup and on every rescan and applies it by name: a new job
is set up, a removed or disabled one torn down (its running command killed), a changed one keeps its running run (a
running service is restarted with the new definition). An invalid file is refused with its first broken rule as
`<JSON path>: <reason>` (`jobs.mail.schedule: hour: 25 is out of range 0-23`) and the last valid jobs keep running, so
a bad edit never stops a watcher; the error shows in `jobs/<id>`, `useJobs().error` and the window.

- **Schedules**: five cron fields — minute (0–59), hour (0–23), day of month (1–31), month (1–12 or `jan`…`dec`),
  day of week (0–7, 0 and 7 Sunday, or `sun`…`sat`; names case-insensitive) — each `*`, a number, a range `a-b`, a
  list `a,b`, or a step `*/n` or `a-b/n`; when both day of month and day of week are restricted, a time matching either
  fires. `@yearly` (`@annually`), `@monthly`, `@weekly`, `@daily` (`@midnight`) and `@hourly` work; there is no seconds
  field and no `@reboot` (use a `service`). Times are in the daemon host's local time zone: a time a daylight-saving
  change skips does not fire, one it repeats fires once.
- **One run per job**: a job has at most one running run, whatever its trigger. A scheduled time while the run still
  runs records a `skipped` run (reason `running`); `jobs.run` on a running job is `conflict`.
- **Missed times**: a time the scheduler reaches more than 60 s late (the daemon was not running, the host slept) is
  missed. `missed: "skip"` drops it; `missed: "run"` runs the job once (trigger `missed`), however many were missed. A
  newly declared job never runs for times before the daemon first saw it.
- **Services**: an enabled service runs whenever the daemon runs, started again after its run ends — `always` after
  any end, `on-failure` after a non-zero exit or a signal the daemon did not send — after 1 s, doubled after each run
  shorter than 60 s up to 60 s, reset by a run that lasted 60 s. A service killed through `jobs.kill` (Stop in the
  window) stays stopped until it is run by hand, disabled and enabled again, changed, or the daemon starts again.
- **Command runs** run like transient runs (see Jobs above: `sh -c`, the daemon's environment, no dev shell, stdin
  `/dev/null`, no timeout, output on `job/<id>`), in `cwd` instead of the workspace folder. A run ends `succeeded` on
  exit code 0, `failed` on another code or a signal, `killed` when killed.
- **Prompt runs**: `thread: "new"` creates a thread in the workspace with `text` as its first message, titled
  `<name> · <YYYY-MM-DD HH:MM>`; `thread: "reuse"` sends `text` to the job's own thread (created and titled `<name>`
  the first time, and again when that thread was deleted or archived). A reused thread that is busy (starting, working,
  waiting on a dialog) makes a `skipped` run (reason `busy`). The run ends with the turn: `succeeded` or `failed`.
  Threads a job made say `job <name>` (`createdBy: { kind: "job", job }`).
- **Jobs belong to the workspace**: no thread owns a declared job or its runs, not even the thread whose agent declared
  it or ran it; stopping, archiving or deleting that thread leaves them running. Removing the workspace tears them down.
- **Runs are the daemon's**: the daemon keeps each job's 50 newest finished runs in its store (a command run with its
  last 200 lines), never in the workspace. The file is all a folder needs: a copy of it (a git clone) runs the same
  jobs on another daemon, starting without their runs and their reused threads.

Over the protocol (`omfp_call`), `jobs.define` is how you declare one — or edit `jobs.json` with your file tools; both
apply at once:

| Method | What it does |
| --- | --- |
| `jobs.list { workspaceId }` | `{ jobs, error }`: each job `{ name, job, status, run, lastRun, nextAt }` (`job` with its defaults filled in; `status` `disabled`, `idle`, `running`, `waiting` or `stopped`; `nextAt` the next scheduled time or service restart, ms). |
| `jobs.runs { workspaceId, job, before?, limit? }` | `{ runs, more }`: the job's kept runs, newest first (`limit` default 20, at most 50; `before` a run id). |
| `jobs.run { workspaceId, job }` | Starts a run now and answers it (`{ id, job, trigger, status, startedAt, … }`; its id is a `job/<id>` like any job). |
| `jobs.define { workspaceId, name, job, replace? }` | Writes the job into `jobs.json` and answers it once applied; `replace: true` to change an existing one. |
| `jobs.remove { workspaceId, name }` | Removes it from `jobs.json`; its kept runs stay readable. |
| `jobs.enable { workspaceId, name, enabled }` | Enables or disables it in `jobs.json`. |
| `jobs.kill { jobId }` | Stops a run (`run.id`); a killed service stays stopped. |

```json
{ "method": "jobs.define", "params": { "workspaceId": "<id>", "name": "mail", "job": {
  "trigger": "schedule", "schedule": "0 8 * * *",
  "prompt": { "text": "Read my unread mail and list what needs an answer.", "thread": "reuse" } } } }
```

The topic `jobs/<workspaceId>` holds `{ jobs, error }` and publishes `jobs_set` (the file applied or refused),
`job_updated { job }`, and `run_started { run }` / `run_ended { run }` (skipped runs included).

In the window, `omfp:jobs` lists the jobs, and the commands `job.run <name>` (`:job-run`), `job.kill <name>`,
`job.enable <name>`, `job.disable <name>` and `job.list` (`:jobs`) act on the current workspace's; none is bound to a
key, so `.omfp/keymap.json` binds `["job.run", "deploy"]` without an init file:
`{ "keys": "Space d", "command": "job.run", "args": ["deploy"] }`.

A widget follows them with `useJobs`:

```tsx
import { Button, Card } from "omfp/kit";
import { useJobs, useReportError } from "omfp/sdk";

export default function Jobs() {
  const { jobs, error, run } = useJobs();
  const report = useReportError();
  return (
    <Card title={error ?? `${jobs.length} jobs`}>
      {jobs.map((job) => (
        <Button key={job.name} isDisabled={job.status === "running"} onPress={() => run(job.name).catch(report)}>
          {`${job.name}: ${job.lastRun?.status ?? job.status}`}
        </Button>
      ))}
    </Card>
  );
}
```

An init file reads and acts on the current workspace's jobs through `omfp.jobs` (`list()`, `get(name)`, `error`,
`run(name)`, `kill(name)`, `enable(name, enabled)`) and hears their runs with `on("job_started" | "job_ended" |
"job_failed", (run) => …)`:

```ts
import type { InitApi } from "omfp/sdk";

export default function init(omfp: InitApi) {
  omfp.on("job_failed", (run) =>
    omfp.notify({ title: `${run.job ?? "job"} failed`, body: run.reason ?? `exit ${run.exitCode}` }),
  );
}
```

An init file's listener runs once per open window, and never when no window is open: it may notify or show
something, but must not start runs or chain jobs (two windows would start them twice, a closed laptop never). What
must happen without a window is a job of the daemon: a `schedule` or a `service`.

## omfp/kit

Use these components for every control a widget shows: they follow the theme (light, dark, a built-in or a user theme) in the
window's own popovers. Native `<select>`, `<input type="checkbox">`, `radio`, `range`, `date`, `color` and `<datalist>`
do not: their opened lists and pickers are drawn by the platform (a GTK menu in the Linux desktop window, white with a
blue highlight), whatever the page's styles say. `omfp_widget_check` warns about each one it finds.

`TerminalEmulator` is the reusable xterm.js surface: pass `sessionKey`, base64 `chunks`, `onData` and
`onTerminalResize`; optional `cols`, `rows` and xterm `options` configure it. It owns only emulation and fitting.
Pair it with `useTerminal` for a PTY-backed CLI, as the copyable `omfp:terminal` built-in does.

| Export | What it is |
| --- | --- |
| `Card` | A bordered surface: `title`, `actions`, `tone` (`default`, `error`, `warning`, `success`). |
| `BorderBox` | A box whose title sits on its top border (`╭─ title ──╮`): `title`, `level` (a heading level for the title), `end` (metadata at the right end of the border), children as the body. |
| `EmptyState` | What shows where nothing is yet: `icon` (an `Icon` name), `title`, the hint as children, `action` (a button under it), `bare` (without the dashed box). |
| `Button` | A button: `onPress`, `variant` (`primary`, `secondary`, `quiet`, `danger`), `size` (`sm`, `md`), `isDisabled`. |
| `IconButton` | An icon-only button: `label`, `icon`, `onPress`. |
| `Progress` | A progress bar: `label` (required), `value` (omit for indeterminate), `max` (default 100), `hideLabel`. |
| `Spectrum` | The live voice spectrum: `source` (`useLive()[0].source()`), `status`, `label`; a static meter below motion `full`. |
| `LivePanel` | The live voice panel: `status`, `statusLabel`, `spectrum`, `lines`, `message`, `hints`, `actions`. |
| `Log` | Monospace lines that follow the end: `lines`. |
| `Meter` | A small usage bar with `value/max` beside it: `value`, `max`, `label` (accessible name), `threshold` (fraction of `max` above which it turns warn, default 0.8). |
| `Badge` | A small status label: `tone` (`default`, `primary`, `error`, `warning`, `success`). |
| `Chip` | A compact labelled item: `tone`, `onRemove`. |
| `StateMark` | The glyph of a state in its status color: `state` (`running`, `awaiting`, `failed`, `unread`, `idle`, `done`), `label` (decorative without one), `spinner` (`activity` or `status`). |
| `Icon` | A decorative Lucide icon (`aria-hidden`; the control around it carries the name): `name` (`archive`, `check`, `circle-alert`, `copy`, `file`, `folder`, `inbox`, `play`, `plus`, `refresh-cw`, `search`, `x`, …). |
| `Kbd` | A key chip: the key or sequence as lowercase words (`spc n`, `enter`) as children. |
| `Markdown` | Renders Markdown text (no raw HTML): `text`. |
| `Code` | Code: `code`, `block`, `language` (a name or file extension; highlights a block). |
| `Diff` | A unified patch (`git diff` output) as numbered lines: `patch`, `language` (highlighting). |
| `languageOfPath` | `languageOfPath("src/lib.rs")` → `"rust"`: the language to pass to `Code` or `Diff`. |
| `CodeEditor` | CodeMirror with vim bindings: `value`, `path` (picks the language), `readOnly`, `line`, `onSave(text)` (`:w`, Ctrl-S), `onDirtyChange`, `onRequestEdit` (`i` while read-only). Loaded on first use. It fills the height its container gives it (a flex child or a fixed height), never takes one from its text, and scrolls inside. |
| `CodeDiff` | Two texts compared with CodeMirror: `original`, `value`, `path`, `layout` (`"unified"` or `"split"`). Sized like `CodeEditor`. |
| `PdfView` | A PDF document with pdf.js: `url` (same-origin, e.g. `` `${useFileUrl(path)}?v=${hash}` ``), `labels` (`{ loading, failed(error), zoomIn, zoomOut, fitWidth, page(page, pages), pageLabel(page), toolbar, pages }`). Its pages scroll inside it, drawn when near the view, with selectable text; a toolbar zooms out, in and to the width, and names the page at the top (`3 / 12`). Loaded on first use; sized like `CodeEditor`. |
| `ImageView` | An image fitted, zoomed (buttons, Ctrl with the wheel, a pinch) and panned by dragging: `src`, `alt`, `labels` (`{ zoomIn, zoomOut, fit, actualSize, toolbar, failed, before, after, slider }`); with `before` (`{ src, alt }`) and `compare` (`"split"` or `"slider"`) it compares the two versions side by side or under a slider. Sized like `CodeEditor`. |
| `WebFrame` | A web page in a sandboxed frame under a toolbar (the URL, Reload, Open in browser), filling its container like `CodeEditor`: `url` (an absolute `http(s)` URL other than omfp's own origin; another shows why it is refused), `title` (the frame's accessible name), `loadTimeoutMs` (default 10000: then it says the site may refuse framing). `omfp:web` is one. |
| `TerminalEmulator` | An xterm.js emulator filling its container: `sessionKey` resets replay, `chunks` are base64 bytes, `onData` forwards keyboard input, `onTerminalResize` receives fitted columns and rows, and optional `cols`, `rows`, `options` configure xterm. Pair it with `useTerminal`; `omfp:terminal` is one. |
| `PathLink` | An inline button opening a file path: `reference` (`{ path, line }`), `onOpen` (e.g. from `useOpenFile()`). |
| `Tabs` | Tabs with `TabList`, `Tab` and `TabPanel`. |
| `ListBox` | A selectable list with `ListBoxItem`. |
| `Menu` | A menu with `MenuItem` and `MenuTrigger`. |
| `Select` | A drop-down: `label` (required; `hideLabel` keeps it for screen readers only), `items` (`{ id, label }[]`, ids non-empty), `selectedKey`, `onSelectionChange(key)`. Use it for every `<select>`. |
| `TextField` | A text input: `label`, `hideLabel`, `value`, `onChange`, `placeholder`. |
| `TextArea` | A multi-line text input: `label`, `hideLabel`, `value`, `onChange`. |
| `SearchField` | A search input (Escape clears it): `label`, `hideLabel`, `value`, `onChange`. |
| `PathField` | A path input completing on the daemon's machine: `label`, `value`, `onChange`, `onSubmit`, `suggest` and `complete` (from `usePathSources()`), `placeholder`. |
| `Checkbox` | A checkbox with its label as children: `isSelected`, `onChange(selected)`. |
| `RadioGroup` | One choice among a few `Radio` children (`value` each): `label`, `hideLabel`, `value`, `onChange(value)`. |
| `Disclosure` | An expandable section. |
| `Toolbar` | A row of controls. |
| `Dialog` | A modal dialog (focus trapped, returned on close): `title`, children (or a function of `{ close }`), `isOpen` and `onOpenChange` (omit both inside a `DialogTrigger` with its trigger button), `isDismissable`. |
| `ConfirmDialog` | A confirmation step before an action: `isOpen`, `onOpenChange`, `title`, `message`, `confirmLabel`, `cancelLabel`, `onConfirm` (then it closes), `tone: "danger"` (destructive; Cancel focused first), `busy` (the caller closes it once the action settles), `error`. Use it for every confirmation: never `window.confirm` or a native `<dialog>`. |
| `Popover` | A non-modal overlay anchored to a trigger: inside a `DialogTrigger`, or `triggerRef`, `isOpen`, `onOpenChange`, `placement`. |
| `PopoverDialog` | A non-modal dialog in a popover, inside a `DialogTrigger` after its trigger button: `title` (on the top border), `placement`, children (or a function of `{ close }`); Escape and a click outside close it. |
| `Tooltip` | A hint shown on hover and focus: `<TooltipTrigger><Button …/><Tooltip>hint</Tooltip></TooltipTrigger>`; `placement`. |

### Form controls

| Native element | Kit replacement |
| --- | --- |
| `<select>` | `Select` |
| `<input type="checkbox">` | `Checkbox` |
| `<input type="radio">` | `RadioGroup` with `Radio` |
| `<input type="range">` | `Select` of the steps (or `Button`s stepping a value); `Progress` to only show one |
| `<input type="date">` | `TextField` taking `YYYY-MM-DD` |
| `<input type="color">` | `Select` of theme colors (`var(--omfp-primary)`, …) |
| `<datalist>` | `SearchField` filtering a `ListBox`, or `Select` |

A `Select` key must be non-empty, so an "All" entry takes a sentinel id such as `"*"`; `hideLabel` keeps the label
for screen readers when the context names the field:

```tsx
import { Select } from "omfp/kit";
import { useStored } from "omfp/sdk";

export default function StatusFilter({ statuses }: { statuses: string[] }) {
  const [status, setStatus] = useStored<string>("status", "*");
  const items = [{ id: "*", label: "All" }, ...statuses.map((name) => ({ id: name, label: name }))];
  return (
    <Select
      label="Status"
      hideLabel
      items={items}
      selectedKey={status}
      onSelectionChange={(key) => setStatus(key === null ? "*" : String(key))}
    />
  );
}
```

`status === "*"` then means no filter. Inline `style` cannot make a native control's popup follow the theme; a native
control left in a widget at least follows the light or dark scheme (`color-scheme` and `accent-color` on the root),
as far as the engine lets it.
