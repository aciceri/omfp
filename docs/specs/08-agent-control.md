# 08 — Agent control

**Status:** binding. [Views and widgets (M4)](#views-and-widgets-m4) was delivered in **M4**; the agent tools, the MCP
server, mounts and the component library ([Agent tools (M10)](#agent-tools-m10) and the sections after it) in **M10**, the MCP servers omp
loads in M13 and init files and TSX views in M14. The [Vision](#vision) explains the direction they follow, and the
[Constraints that apply from day one](#constraints-that-apply-from-day-one) still bind every change.

## Vision

The interface is a surface shared by the user and the agents, and agents get **the same power as the user** (D-17):
everything the UI can do, an agent can do — directly through tools, or through UI it builds itself.

1. **Everything in the protocol**: list threads and see what each one is doing (state, transcript, tool runs, pending
   questions), spawn threads, send them messages, wait for them, rename, archive; create workspaces and
   add or remove their repositories; read git status and diffs. Every protocol method is an agent tool (AGT-001).
2. **Arbitrary UI**: an agent writes its own pieces of interface in JavaScript (HTML, CSS, JS — anything a web page can
   do), mounted in a thread panel, a dashboard or a window, live and interactive, wired to what it is doing (e.g. a real
   progress bar driven by a long command).
3. **Reusable UI**: those pieces can be saved and shared, so later threads and sessions reuse them.
4. **Attention**: mark a thread as needing the user, send a desktop notification (questions already exist: omp's
   `ask` tool, DMN-050).

Example, the PR workflow: an orchestrating thread receives "implement checkout v2", creates `fe#412` and `be#88`
worktrees in the frontend and backend repositories, spawns one thread per PR, and builds a dashboard that lists every
thread with its live status, CI results, and buttons "rerun tests" and "open PR". The dashboard is saved, and the next
feature reuses it.

## Tool surface

Every protocol method is reachable through `omfp_call`, whose catalogue `omfp://protocol/methods` is generated from the
protocol method table (AGT-001, AGT-007). Convenience tools on top (the full list: [Agent tools (M10)](#agent-tools-m10)):

| Tool | What it does |
| --- | --- |
| `omfp_call` | Calls any protocol method by name with JSON params (the escape hatch: nothing is out of reach) |
| `omfp_thread_wait` | Waits for a thread's `prompt_result` or settled state; returns its last assistant text |
| `omfp_ui_mount` / `_update` / `_remove` | Mounts a UI piece (inline code or a saved component) with parameters |
| `omfp_component_save` / `_list` | Saves a UI piece to the library / lists the library |
| `omfp_notify` | Attention flag + desktop notification |

## Delivery to agents

One tool registry in the daemon, several adapters:

| Adapter | Reaches | Notes |
| --- | --- | --- |
| **RPC host tools** (`set_host_tools`) | every thread run by the daemon over RPC | Already used for the thread-only `omfp_open_file`, `omfp_notify`, `set_thread_title` (DMN-060, DMN-125) and every registry tool (DMN-151). omp may mount them as `xd://` devices (`tools.xdev`), invoked through `write`. |
| **MCP server** (Streamable HTTP at `/mcp`, the daemon's token as bearer, AGT-050..054) | omp sessions outside omfp, other MCP-capable agents | Only needed for agents not run by the daemon (RPC threads get host tools); configured in the user's own `mcp.json` (`omfp mcp config` prints the entry), since the daemon writes nothing into workspaces (D-11). No per-thread tokens: they were intended for the removed omp terminal handoff (05), not the independent widget PTYs (TRM-100). |
| **Host URI scheme** `omfp://` (`set_host_uri_schemes`) | RPC threads | Read with the agent's `read` tool: `omfp://docs` (the index; `docs/widgets`, `docs/shell`), `omfp://protocol/methods`, `omfp://keymap`, `omfp://windows`, `omfp://widgets/…` and `omfp://schemas/…`. |
| **SDK** | widgets | The same API from inside a widget, as React hooks (below). |

## Agent-made UI: views and widgets (M4)

The window is described by a **views manifest** per workspace; everything in it is a **widget**, a React component.
The shell (D-18) is React too, so built-in widgets, agent widgets and the shell are the same kind of code. The
binding rules are the table [Views and widgets (M4)](#views-and-widgets-m4) below; this section explains them.

**Views manifest** — `<workspace>/.omfp/views.json` (every workspace is a folder the daemon made, D-11, DMN-020).
Declarative, checked by the daemon, edited by the agent as a file (or with the `omfp_ui_mount` tool, AGT-061) and by the
user with `:widget add` and `:widget copy` (UI-300, UI-106), so both always edit the same thing. Every workspace has the
file: the daemon writes the default view into it when it creates the workspace, and at startup for older ones
(DMN-088), and never rewrites it — it is the user's and the agent's; `:view-reset` restores the current default. The
default view is the header over the `tabs` slot `main` holding the conversation (opened files show as closable tabs
next to it, UI-115), with the `tabs` slot `side` (workspace, changes and background widgets) on its right:

```jsonc
{
  "$schema": "omfp://schemas/views.json",
  "default": "work",
  "views": {
    "review": {
      "title": "Review",
      "layout": ["row", {}, "omfp:transcript", "./widgets/tests"]
    },
    "work": {
      "title": "Work",
      "layout": ["row", { "sizes": [3, 1] },
        ["col", {}, "omfp:header", ["tabs", { "id": "main" }, "omfp:chat"]],
        ["tabs", { "id": "side" }, "omfp:workspace", ["./widgets/tests", { "command": "cargo nextest run" }]]]
    }
  }
}
```

- Containers: `row`, `col`, `tabs`, `stack` with `sizes` (flex weights); leaves are widget references with props.
  Several views per workspace, switched from the frame and listed in id order.
- `omfp:*` are built-in widgets (`omfp:header`, `omfp:chat` and its parts `omfp:transcript`, `omfp:interactions`,
  `omfp:composer`, `omfp:workspace`, `omfp:changes` from M5, `omfp:file` from M5c, `omfp:terminal` from issue #80);
  `./widgets/<name>` are the workspace's own; `user:<name>` come from the user library in the data dir.
- An invalid manifest never breaks the window: the frame shows the default view and a banner with the validation error.
- The conversation with the agent is a widget like any other: `omfp:chat` (transcript, interactions such as approvals
  and `ask`, queue and composer), built from smaller `omfp:transcript`, `omfp:interactions` and `omfp:composer`, all
  copyable and replaceable. Only a thin frame stays outside the manifest: the left column (workspaces and threads),
  connection banner, view switcher, the keymap and command line, notifications, the live region, and a **safety net**
  — when the selected thread has a pending interaction that no built-in interactions widget in the current view
  renders, the frame shows it itself, so a manifest can never hide an approval.

**Widgets** — a widget is an ES module whose default export is a React component, in `<workspace>/.omfp/widgets/<name>/`
(`index.tsx`, `index.ts`, `index.jsx` or `index.js`, plus other files it imports relatively). It runs in the window's
document, under an error boundary (a crash shows the error in place of the widget, not a broken window), and gets the
SDK as hooks and its layout props as props:

- **Protocol**: `useCall()`, `useTopic(topic)` — the full daemon protocol with the replay/snapshot semantics of 03, so
  state is the same in every window and survives reconnects; helpers such as `useThreads()`, `useThread(id)`,
  `useWorkspace(id)`, `useIndex()`.
- **Sources**: `useToolRun(toolCallId)` follows an omp tool run; `useJob(command)` runs a non-interactive daemon job; `useTerminal(command, options)` starts or follows an interactive PTY session and can inspect its visible screen (TRM-105, TRM-106).
- **Files**: `useFile(path)` reads a workspace file and follows its changes, saves with a conflict check;
  `useOpenFile()` gives `openFile(path, { line, diff })`, which opens a file tab (AGT-027).
- **Storage**: `useStored(key, initial)` — state kept by the daemon per widget instance, shared across windows and
  sessions.
- **Kit and theme**: the React components of the shell (`omfp/kit`: cards, buttons, lists, tabs, progress, log,
  markdown, …) and the CSS custom properties, so widgets look native in light and dark themes. Optional to use.

```tsx
// .omfp/widgets/tests/index.tsx — written by the agent, reloaded live when it changes.
import { Button, Card, Log, Progress } from "omfp/kit";
import { useJob } from "omfp/sdk";

export default function Tests({ command = "cargo nextest run" }: { command?: string }) {
  const job = useJob(command); // runs in the daemon, in the workspace folder
  const total = Number(job.lines.find((l) => /Starting \d+ tests/.test(l))?.match(/\d+/)?.[0]) || 1;
  const done = job.lines.filter((l) => /^\s*(PASS|FAIL)/.test(l)).length;
  return (
    <Card title={job.status === "running" ? `Tests ${done}/${total}` : `Tests ${job.status} (exit ${job.exitCode})`}>
      <Progress value={done} max={total} label="Tests" />
      <Log lines={job.lines.slice(-10)} />
      <Button onPress={job.status === "running" ? job.kill : job.restart}>
        {job.status === "running" ? "Stop" : "Run again"}
      </Button>
    </Card>
  );
}
```

**System access goes through the daemon.** A widget runs in the browser (or desktop webview), possibly on another
machine; it never touches the system itself. Everything it needs runs in the daemon, on the workspace's machine, as the
user, with no restrictions (full access, D-17):

| Need | SDK | Daemon side |
| --- | --- | --- |
| Follow a command the agent runs with its `bash` tool | `useToolRun(toolCallId)` or `useToolRun({ match: /nextest/ })` (latest matching run in the thread) | omp's `tool_execution_update` frames in the thread topic; omp 18.22.0 streams the bash output's latest tail while the command runs (contract test, AGT-020) |
| Run its own non-interactive command | `useJob(command, { key? })` (fields: `omfp://docs/widgets`) | **jobs** (DMN-080, DMN-081, DMN-215): `jobs.start { workspaceId, command, replaces? }`, topic `job/<id>`, `jobs.kill` |
| Run or inspect an interactive CLI | `useTerminal(command, { cwd?, cols?, rows?, key? })` | daemon-owned PTY (TRM-100..106): `terminals.*`, topic `terminal/<id>`; input is not stored, while explicit screen reads may enter the agent conversation |
| Share state with the agent | `useStored(key, initial)` | per-instance state in the daemon (DMN-086) |

**Loading** — the daemon serves workspace and user widgets over HTTP as ES modules, transpiling TypeScript/JSX with
`oxc` (S-12), and an import map points `react`, `react/jsx-runtime`, `react-dom`, `omfp/sdk` and `omfp/kit` at the
app bundle's own modules, so widgets share one React instance with the shell. A rescan (DMN-023) or a poll that sees
a changed widget folder or manifest pushes an update; the window reloads that widget only.

**Sharing** — widgets and views are files: commit `.omfp/` with a repository to share them, or put a widget folder
into the user library (`<data-dir>/widgets/<name>/`, by hand or by the agent). Built-in widgets can be copied into
`.omfp/widgets/` (`:widget copy omfp:workspace`) and changed; a workspace widget with the same name as a built-in
replaces it in that workspace.

**How agents learn it** — a short widgets note in the daemon's `--append-system-prompt` text (AGT-024) and read-only
host URIs served to the thread's omp with `set_host_uri_schemes` (DMN-062): `omfp://docs/widgets` (the SDK and kit
reference), `omfp://schemas/views.json` (the manifest schema) and `omfp://widgets/builtin/<name>` (the built-ins'
sources), so the agent never searches the filesystem. Two host tools close the loop without a browser:
`omfp_widget_check { name }` (DMN-063) answers with the daemon's compile result and what the open windows report about
the widget (AGT-026: errors, console errors, the rendered text, its jobs), and `omfp_ui_mount` (AGT-061) places a
widget in a view without rewriting `views.json` by hand.

**Later (not in M4)**: `useAgent(threadId?)` (messages or events to a thread), the host tools `omfp_widget_state`, `omfp_job_start` and `omfp_job_output` (the agent reads
and writes widget state and starts jobs), `jobs.list`, job `cwd`/`env` options, and dragging widgets with the pointer.
Widgets in dashboards and floating panels, and saving widgets to the user library, came with M10 (AGT-060..066).

### Views and widgets (M4)

| ID | Requirement |
| --- | --- |
| AGT-010 | **Views manifest**: `<workspace>/.omfp/views.json` is `{ "$schema"?: string, "default"?: <view id>, "views": { <id>: { "title"?: string, "layout": <node> } } }` with at least one view; unknown keys are ignored. A node is a widget reference (`"omfp:chat"`), `[reference, props]`, or `[container, props, ...children]` with `container` one of `row`, `col`, `tabs`, `stack`, at least one child, and `sizes` (when present) one positive number per child; props are objects. References are `omfp:<name>` (built-in), `./widgets/<name>` (the workspace's) and `user:<name>` (the user library); an unknown reference is not a manifest error, it shows an error in place of that widget. A widget instance is identified by its `instance` prop when it is a string, else by its reference. Views are listed in id order; `default` (else the first id) is the view a window opens. The daemon and the app apply the same rules (one shared case list, `fixtures/views/cases.json`). The **default manifest** (written into every workspace, DMN-088, and the window's fallback while the file is missing or invalid) is the view `default`, titled "Default": `["row", { "sizes": [3, 1] }, ["col", {}, "omfp:header", ["tabs", { "id": "main" }, "omfp:chat"]], ["tabs", { "id": "side" }, "omfp:workspace", "omfp:changes", "omfp:background", "omfp:mcp"]]`, every built-in an explicit `omfp:*` reference; the daemon's copy (`crates/omfp-daemon/src/widgets/default-views.json`) and the app's are checked equal by a test. The `tabs` container with `"id": "main"` is the slot reserved for opened files (they will open as window-local tabs after the conversation); the one with `"id": "side"` is where `omfp_ui_mount` puts side tabs (`place: "side_tab"`, AGT-061). A `tabs` container with one child shows no tab bar. A tab's label is the widget's `title` prop, else a built-in's own name ("Workspace", "Changes", …), else its reference. |
| AGT-011 | **Invalid manifest**: a `views.json` that cannot be read, is not JSON or breaks AGT-010 never breaks the window: the frame shows the default view and a banner with the error (its JSON path and reason); a fixed file recovers on the next push, without reload. |
| AGT-012 | **Widget modules**: a widget is the folder `<workspace>/.omfp/widgets/<name>/` (`<name>` of ASCII letters, digits, `_`, `-`, `.` not first), entry `index.tsx`, `index.ts`, `index.jsx` or `index.js` (the first present); its default export is a React component receiving the node's props. It may import files of its own folder (relative paths, with or without extension, never leaving the folder) and only the specifiers `react`, `react/jsx-runtime`, `react-dom`, `omfp/sdk` and `omfp/kit`, which an import map in the page points at the app bundle's own modules (one React, one SDK). |
| AGT-013 | **Live reload**: a widget is imported from a URL carrying its folder's version (DMN-084); when the views topic pushes a new version, the window imports that widget again and re-renders only it. State in `useStored` survives a reload, component-local state does not. |
| AGT-014 | **Errors in place**: a compile error (syntax, a forbidden import, a name that `omfp/sdk`, `omfp/kit` or React do not export, in any file of the folder, DMN-085), a failed import, a missing entry or default export, and a render error show in place of the widget (file, line and column when known, and the message); the rest of the window keeps working, and the next save recovers without reload. |
| AGT-015 | **Safety net**: when the selected thread has a pending interaction and the current view contains neither built-in `omfp:chat` nor `omfp:interactions` (a workspace override of them does not count), the frame renders the interactions itself. |
| AGT-016 | **SDK** (`omfp/sdk`): the hooks of `ui/packages/app/src/sdk` (connection, index, threads, workspaces, any topic, every protocol method, window state, commands, announcements, thread actions, settings), plus `useWidget()` (instance, reference, workspace id), `useStored`, `useJob`, `useToolRun`, and the view models and strings the built-in widgets use. `omfp://docs/widgets` documents them; a test checks every name it lists is exported. |
| AGT-017 | **Kit** (`omfp/kit`): the components of `@omfp/kit` (UI-093), including `Card`, `Button`, `Progress`, `Log`, `Markdown`, `Tabs`, `ListBox`, `Menu`, `Badge`, and from M5c the CodeMirror `CodeEditor` and `CodeDiff` (loaded on first use, S-13), styled by the theme tokens (UI-091). |
| AGT-018 | **`useStored(key, initial)`** → `[value, set, loaded]`: JSON state kept by the daemon per workspace, widget instance and key (DMN-086), the same in every window and pushed live, surviving reloads and daemon restarts; `set` takes a value or an updater, `null` deletes the key; `loaded` tells whether the workspace's views state has arrived (before, `value` is `initial`). |
| AGT-019 | **`useJob(command)`** → the job's state (its fields are listed once, in `omfp://docs/widgets`): `status` is `pending` (no job known yet, or a start or restart on its way), `running`, `succeeded` (exit code 0) or `failed` (another exit code, or a signal); `lines` holds both outputs interleaved as they came, `stdout` and `stderr` each one's lines; `runs` counts the jobs this instance started for the command (a restart adds one) and `startedAt`/`endedAt` (ms) date the current job. The job's id and the runs are stored with `useStored` under `job:<command>` as `{ id, runs }`; while the instance has no job for the command (nothing stored, or the daemon no longer knows the stored job), the hook starts one (`jobs.start`); otherwise it follows the stored job, running or finished, so reloads, other windows and moving the widget never start a second one. `restart` kills a running job and starts a new one; `kill` stops it. |
| AGT-020 | **`useToolRun(toolCallId \| { match, threadId? })`**: the tool run of a thread (the selected one by default) with that id, or the latest whose command (`arguments.command` when it is a string, else its JSON arguments) matches `match`: `{ toolCallId, toolName, status, output, lines, isError }`, where `output` is the latest partial output while it runs (omp 18.22.0 sends the bash output's latest tail in `tool_execution_update`, checked by a contract test) and the result's text once it ended. |
| AGT-021 | **Built-ins are copyable**: every built-in widget (`omfp:header`, `omfp:chat`, `omfp:transcript`, `omfp:interactions`, `omfp:composer`, `omfp:workspace`, `omfp:changes`, `omfp:background`, `omfp:todos`, `omfp:debug`, `omfp:tree`, `omfp:activity`, `omfp:mcp`, `omfp:file`, `omfp:terminal`, `omfp:jobs`, `omfp:web`, `omfp:browser`; the `BUILTINS` table of `ui/packages/app/builtins.ts`, checked against the registry by `copyable.test.ts`) is written like an agent widget — its folder's value imports are only `react`, `omfp/sdk`, `omfp/kit` and files of its own folder (checked by a test) — and the release bundle ships each one's source as a widget folder (`builtin-widgets/<name>/`). `:widget copy omfp:<name> [<new name>]` (`widgets.copy`, DMN-087) copies one into `.omfp/widgets/`; a `user:<name>` widget is copied the same way. |
| AGT-022 | **Override**: in a workspace, `omfp:<name>` renders `./widgets/<name>` when that folder exists, so a copied built-in (or any widget of that name) replaces the original there. |
| AGT-023 | **User library**: `user:<name>` is the folder `<data-dir>/widgets/<name>/`, filled by hand or by the agent; it is served, versioned, reloaded and listed like workspace widgets (in every workspace). |
| AGT-024 | **Agent note**: the daemon's `--append-system-prompt` text (DMN-030) carries a short widgets note after the workspace conventions: where `views.json` (always present, DMN-088), workspace widgets and the user library live, the allowed imports, a ten-line example widget, the host URIs to read — `omfp://docs/widgets`, `omfp://schemas/views.json` and the built-in sources `omfp://widgets/builtin/<name>` (DMN-062; no filesystem path, so the agent never searches for them) — the tools that check a widget (`omfp_widget_check`, DMN-063) and place it (`omfp_ui_mount` / `_update` / `_remove`, AGT-061..063), the library tools `omfp_component_save` / `omfp_component_list`, and three rules stated word for word also in `omfp://docs/widgets` (checked by a test): keep the `main` slot (AGT-028), place widgets with `omfp_ui_mount` / `_update` / `_remove` and `place`, and the verification loop (AGT-084). It also asks the agent to **propose** a widget, in one sentence in the chat, when something would read better as a visualization or controls, and to build it only after the user agrees (no UI approval gate), delegating the building to the `widget-builder` agent (AGT-029). |
| AGT-025 | **Agent docs**: `omfp://docs/widgets` is the SDK and kit reference (hand-written Markdown in the repository, `crates/omfp-daemon/src/widgets/docs.md`) and `omfp://schemas/views.json` the manifest's JSON Schema; both are read-only host URIs of every thread (DMN-062), verified against omp 18.22.0 by a contract test. |
| AGT-026 | **Window reports**: every window reports to the daemon over its connection (`widgets.report`, PRT-043), for each workspace or user widget instance of the view it shows: the import's state, the compile stand-in's, import's and error boundary's errors, console errors and uncaught errors whose stack names the widget's module (`<widgetBase>/<source>/<name>/`), a text snapshot of the widget's DOM, and its `useJob` jobs, with the `{ epoch, seq }` of the views topic it rendered. It reports again when any of that changes (at most every 250 ms) and reports `workspaceId: null` when it shows no workspace. `omfp_widget_check` reads these reports (DMN-063). Reports also carry the init file and TSX view errors agents read (AGT-076). |
| AGT-027 | **Files** (M5c): `useFile(path, { follow? })` → `{ status: "loading" \| "ready" \| "missing" \| "error", file, error, reload, save }` reads a file of the current workspace (`workspaces.read_file`, DMN-089) and, while `follow` (default `true`), checks it again every 2 s with `knownHash` and whenever `workspace/<id>` publishes a new status, replacing `file` when it changed; `save(content)` writes it with the `expectedHash` of the shown version (`workspaces.write_file`, DMN-090) and resolves to `{ ok: true, file }` or `{ ok: false, conflict: true }` (the file changed on disk) or `{ ok: false, error }`. `useOpenFile()` returns `openFile(path, { line?, diff? })`, which opens the file in a tab of the view's `main` slot or of the floating panel (UI-115). `useMissingFile(path, missing, { line?, diff? })` looks for the file a missing path names (`workspaces.find_file`, DMN-092) and opens it or the window's picker (UI-117), giving `"finding"`, `"opened"`, `"choosing"` or `"none"`. All are exported by `omfp/sdk` and documented in `omfp://docs/widgets`. |
| AGT-028 | **Agents keep `main`**: the files the user opens become tabs of the view's `["tabs", {"id": "main"}, …"omfp:chat"]` slot (UI-115), so the widgets note (AGT-024), `omfp://docs/widgets` and the `omfp_ui_mount` description (AGT-061) tell the agent, in the same words, to keep that slot in every view it changes, and `omfp_widget_check` (DMN-063) adds a warning when the workspace's valid `views.json` has no view with a `main` slot (the default views without a file have it), ending `place widgets with omfp_ui_mount.` |
| AGT-029 | **Widget builder** (M6b, S-14): the daemon ships an omp agent `widget-builder` (DMN-107) whose frontmatter is `tools: read, write, edit, glob, grep` and `model: "@smol"` (the user's smol role; omp falls back to the thread's model), without `blocking`, so the task runs in the background. Its body carries the widgets knowledge a subagent cannot inherit from the appended prompt, generated from the same text as the widgets note (AGT-024): where widgets live, with paths relative to its cwd (the workspace folder), the user library, the allowed imports, the example, `omfp://docs/widgets`, `omfp://widgets/builtin/<name>` and `omfp://schemas/views.json`. It must read `omfp://widgets/check/<name>` (DMN-108) after every write until the report is clean, never edit `views.json`, and finish with a short summary naming the widget and its props. The widgets note tells the main agent: once the user agrees to a widget, delegate the building to a `task` with `agent: "widget-builder"` and a complete brief (what to show, its data sources, where it goes), keep talking, and place the widget with `omfp_ui_mount` when the result arrives as a follow-up turn. |
| AGT-030 | **Agent keys**: an agent binds keys for a workspace in `<workspace>/.omfp/keymap.json` (DMN-120, schema `omfp://schemas/keymap.json`), each binding a `[keymap]` entry (UI-097) with optional `args`, to a built-in command or to one a widget registers with `useCommand` (AGT-016), so a widget can ship a command and the key that runs it; the bindings apply in windows showing that workspace (UI-210). |
| AGT-031 | The widgets note (AGT-024) MUST give one line to `.omfp/keymap.json` (its path and schema, commands built-in or a widget's `useCommand`), and `omfp://docs/widgets` (AGT-025) documents the file: its shape, the precedence, the modes and key tokens, and an example. |
| AGT-032 | The widgets note (AGT-024) MUST give one line to `omfp_notify { title, body }` (DMN-125): use it when a long task finished or the agent needs the user, once per occasion, never for progress. |

### Agent tools (M10)

One registry in the daemon defines the agent tools; the RPC host tools and the MCP server are adapters over it. Its
core is `omfp_call`, any protocol method by name, discoverable through the method catalogue, plus `omfp_thread_wait`
to wait for a thread instead of polling it.

| ID | Requirement |
| --- | --- |
| AGT-040 | **Agent tool registry** (AGT-006): every M10 agent tool is defined once, in `agent_tools::registry()`: its name, label, description, the JSON Schema of its arguments and an async body over the daemon's services, run with a `ToolContext` naming the principal it acts as (DMN-150) and the calling thread's workspace (none when no thread calls, e.g. MCP with a user token). Adapters only translate: every thread gets each registry tool as an essential host tool (DMN-151), the MCP server the same tools. `agent_tools::call` runs a protocol method as the context's principal through the single authorization check (PRT-034) and the window's handlers, so a tool that adapts a method behaves exactly like that method. |
| AGT-041 | **`omfp_call { method, params? }`**: calls any protocol method by its wire name with `params` (an object; absent or `null` means `{}`) as the calling thread's agent principal (DMN-150) and answers the method's JSON result as text. A failed method is a tool error `<code>: <message>`, followed by its `details` JSON when it has some; an unknown method or params that do not decode is a tool error that names `omfp://protocol/methods`; the methods that need a window's connection (`system.hello`, `subscribe`, `unsubscribe`, `widgets.report`, `thread.live_connect`, `thread.live_start`; `CONNECTION_ONLY`) are refused with a tool error saying so. Its description lists `thread.session` among the examples. |
| AGT-042 | **Method catalogue**: `omfp://protocol/methods` (Markdown) lists every method `omfp_call` accepts, in method-table order, one line each: `` `<name>` — <description> `` (PRT-092), and says where the schemas are; `omfp://protocol/methods/<name>` (JSON) is `{ method, description, params, result }` with the method's params and result JSON Schemas (AGT-007). It leaves out the connection-only methods of AGT-041; an unknown or connection-only name is an error naming the list. Both are immutable reads of the `omfp` host URI scheme of every thread (DMN-062). |
| AGT-043 | **`omfp_thread_wait { threadId, promptId?, timeoutSeconds? }`** waits until thread `threadId` is done and answers the JSON `{ threadId, status, prompt?, lastAssistantText }`. With `promptId` (from `thread.prompt`) the thread is done once that prompt's `prompt_result` arrived; without it, once the thread's last prompt (whoever sent it, `threads.create`'s first message included, DMN-152) has its `prompt_result` and the session settled: the last prompt is the latest one omp admitted as the wait goes on, and a prompt still on its way to omp (a `thread.prompt` that has not answered yet, e.g. a parallel tool call of the caller) holds the wait until omp admits it. A `thread.prompt` that reaches the daemon after the wait found the thread done is not waited for: the tool's description says to call it once `thread.prompt` answered, not in the same batch of parallel tool calls. `status` is `done`; `needs_attention` as soon as the thread has a pending interaction (an approval or a question: the caller may answer with `interaction.respond`); `stopped` or `failed` when the thread has no process or its process failed, before it is done; `timeout` after `timeoutSeconds` (default 600, from 1 to 3600). `prompt` is the awaited `prompt_result`'s `{ id, status, error? }` when it arrived; `lastAssistantText` the text blocks of the transcript's last assistant message (`""` without one). An already done thread answers at once. |
| AGT-044 | `omfp_thread_wait` MUST refuse, as tool errors, a missing or blank `threadId`, a `promptId` that is not a non-blank string (over RPC omp drops an empty string argument before the daemon sees it, so `promptId: ""` from an omp agent arrives as absent and waits for the last prompt; the MCP tool, AGT-050, gets the arguments as sent and refuses it), a `timeoutSeconds` outside 1..=3600, the calling thread itself (it can never settle while its own tool runs) and an unknown thread (`not_found`); cancelling the tool call stops the wait. |
| AGT-045 | **Agent note**: the daemon's `--append-system-prompt` text (DMN-030) MUST end with a short `# omfp control` note (at most 7 lines): the thread's own id, `omfp_call { method, params }` reaching every protocol method as the user's windows do (e.g. `threads.list`, `threads.create`, `thread.prompt`), the catalogue `omfp://protocol/methods` and `omfp://protocol/methods/<name>`, `omfp_thread_wait { threadId, promptId? }` to wait for a thread instead of polling it, `thread.session` (AGT-085) and MCP reloads (AGT-080); the mount, component and check tools are the widgets note's (AGT-024). The appended order is workspace conventions, widgets note, control note, then the user's `APPEND_SYSTEM.md` (DMN-030). |

### MCP server (M10)

Agents the daemon does not run (an omp session outside omfp, any MCP client) reach the same registry through the MCP
server at `/mcp` (DMN-155..159). Such a session has no thread: it acts as the **MCP principal**, with the user's full
access (D-17), and what it does is logged and attributed to it.

| ID | Requirement |
| --- | --- |
| AGT-050 | **MCP tools**: `tools/list` MUST list every tool of the agent tool registry (AGT-040), in registry order, as `{ name, title: <label>, description, inputSchema: <parameters> }`, and `tools/call { name, arguments }` MUST run it as the MCP principal (AGT-051) without a workspace and answer `{ content: [{ type: "text", text }], isError }`: the tool's text, `isError: false`, or its error text, `isError: true`. An unknown tool or `arguments` that are not an object (absent means `{}`) is the JSON-RPC error `-32602`. `omfp_notify` and `omfp_open_file` are not MCP tools: both act on the calling thread (its attention flag, the windows showing its workspace) and an MCP session has none; `omfp_call` reaches every method they would need. `omfp_widget_check` and `omfp_views_errors` (DMN-063, DMN-196) are MCP tools, with `workspaceId` required. |
| AGT-051 | **MCP principal**: an MCP request runs as `Principal::Mcp { client }`, `client` being the MCP client's `clientInfo.name` (from `initialize` for a legacy session, from the request's `io.modelcontextprotocol/clientInfo` otherwise; `mcp` when it names none). It passes `authorize` like an agent principal (DMN-150): every permission but `Connect`. What it creates records the provenance `mcp` (PRT-093) and `created_client` = `client`. Without a calling thread, `omfp_thread_wait` waits for any thread and the mount and library tools need `workspaceId` (AGT-066). |
| AGT-052 | **MCP resources**: `resources/list` MUST list the method catalogue `omfp://protocol/methods` (`text/markdown`), `omfp://docs`, `omfp://docs/widgets`, `omfp://docs/shell`, `omfp://keymap` (the defaults, without workspace additions), `omfp://schemas/views.json`, `omfp://schemas/keymap.json` and `omfp://widgets/builtin`, and `resources/templates/list` the templates `omfp://protocol/methods/{name}` (`application/json`) and `omfp://widgets/builtin/{name}`; `resources/read { uri }` answers them exactly as a thread reads them (AGT-042, DMN-062), and any other URI (`omfp://windows` and `omfp://widgets/check/…` are thread-only) or an unknown method is the JSON-RPC error `-32602` (invalid params). The server's `instructions` also tell the client to read `omfp://docs/widgets` and check widgets with `omfp_widget_check`. A test checks every `omfp_*` tool and `omfp://` URI named in a text a surface serves (threads, MCP, the widget builder) exists on that surface. |
| AGT-053 | **MCP calls are logged**: the activity log (AGT-072) MUST record the calls of the MCP principal the way it records an agent's, with `threadId` `null` and the MCP client as `client` (PRT-094), so `omfp:activity` shows them as `mcp: <client>` (UI-241). |
| AGT-054 | **Configuring a client**: `omfp mcp config` (DMN-159) prints the `mcp.json` entry for the local daemon. For omp, put it in `~/.omp/agent/mcp.json` (every session) or a project's `.omp/mcp.json`: omp 18.22.0 then mounts the tools as `xd://` devices (`xd://mcp__omfp_call`, `xd://mcp__omfp_thread_wait`, `xd://mcp__omfp_ui_mount`…, called with `write`, its `content` the JSON arguments) and puts the server's instructions in the system prompt. The header reads the token from the environment, `"Authorization": "Bearer ${OMFP_TOKEN}"` (omp expands `${VAR}` in `mcp.json`; set `OMFP_TOKEN="$(cat ~/.local/share/omfp/token)"` where omp runs), so no file holds the token, which grants full access to the daemon (shell commands included, D-17): never put the literal token (`omfp mcp config --literal-token`) in a project's `.omp/mcp.json`, which may be committed. `"timeout": 0` lets `omfp_thread_wait` outlast omp's 30 s request timeout. Other MCP clients take the same URL and `Authorization` header in their own format. |

The `~/.omp/agent/mcp.json` example above is for an external **standalone omp client**. omfp's own threads already
receive the host tools; their user-level MCP configuration and credentials live only in
`<data-dir>/omp-profile/agent` (ARC-021). Do not configure omfp by changing standalone omp's user files.

### UI mounts and the component library (M10)

Agents put UI in front of the user with **mounts**: a mount is a widget node of the workspace's `views.json` whose
`instance` prop is the mount id, in a view (a dashboard is a view) or in a **floating panel** over every view. The
**component library** is the user library (AGT-023). The tools are adapters over protocol methods (PRT-080..084), so a
widget (`useCall`) or an MCP client reaches the same.

| ID | Requirement |
| --- | --- |
| AGT-060 | **Floating panels**: `views.json` MAY carry `"panels": { <id>: <node> }`, widgets floating over every view of the workspace, in id order. Each node is checked like a view's layout (AGT-010; errors at `panels.<id>`), and `panels` that is not an object is the error `panels: expected an object of panels`; a node's instance is its `instance` prop, else its reference. The schema `omfp://schemas/views.json` documents it. |
| AGT-061 | **`omfp_ui_mount { id, widget? \| code?, props?, view?, place?, panel?, workspaceId? }`** (`views.mount`, PRT-080): `id` is a widget name (AGT-012); exactly one of `widget`, a reference (a bare name means `./widgets/<name>`; `user:<name>` is a library component), and `code`, the source of a component written to `.omfp/widgets/<id>/index.tsx` (the reference is then `./widgets/<id>`). The node carries `props` with `instance: id`. With `panel: true` it becomes `panels.<id>`; else it goes into the view `view` (default: the default view; a new id creates a view showing only it, a dashboard) at `place`: `side_tab` (default: a tab of the `tabs` container whose `id` is `side`), `main_tab` (a tab of the one whose `id` is `main`, after the chat) or `end` (the end of the view's root container); errors name `place`. It is the one way to place a widget (DMN-064 removed), idempotent per id and removable with `omfp_ui_remove`; its description states the main-slot rule (AGT-028). A node of mount `id` already in that target is replaced in place and any other one (another view, a panel) is removed, so an id is mounted once. The answer says where it went and that open windows show it at once. |
| AGT-062 | **`omfp_ui_update { id, props?, code?, workspaceId? }`** (`views.mount_update`, PRT-081): merges `props` into the node of mount `id` (a `null` value deletes the prop; `instance` stays `id`) and, with `code`, rewrites `.omfp/widgets/<id>/index.tsx` (only for a mount of `./widgets/<id>`); `not_found` when nothing is mounted with that id. Windows re-render the widget with the new props in place, keeping its state (UI-232), so an agent drives a mounted widget (a value, a status) by updating its props. |
| AGT-063 | **`omfp_ui_remove { id, workspaceId? }`** (`views.unmount`, PRT-082): removes the panel `id` and every node of mount `id` from the views; a container left without children goes too (its parent's `sizes` lose its weight), a view left empty is removed (`default` dropped when it named it), and emptying the last view is a `bad_request`. The widget folder stays (save it with `omfp_component_save`); `not_found` when nothing has that id. |
| AGT-064 | **`omfp_component_save { name, widget? \| code?, replace?, workspaceId? }`** (`widgets.save`, PRT-083): saves a component to the user library, `<data-dir>/widgets/<name>/`: a copy of the workspace widget `widget` (`./widgets/<x>` or a bare name), or `code` as its `index.tsx`. An existing component is replaced only with `replace: true` (else `conflict`); the new folder appears whole (staged, then renamed). Every followed `views/<id>` topic then lists it as `user:<name>`, and windows showing it reload it. |
| AGT-065 | **`omfp_component_list {}`** (`widgets.library`, PRT-084): the user library by name, each with its reference `user:<name>`, entry, files, version and description: the entry's leading comment (its `//` lines or first `/* … */` block, whitespace collapsed, at most 200 characters; empty without one). The `omfp_component_save` description asks the agent to start a component with such a comment saying what it shows and which props it takes. |
| AGT-066 | The mount and library tools are tools of the agent tool registry (an adapter over the methods above, called as the thread's agent principal) with the calling thread's workspace; without one (the MCP principal, AGT-051) `workspaceId` is required. `omfp://docs/widgets` (AGT-025) documents panels, mounts and the library. |

## Permissions and limits

Agents run in **full-access mode by default** (D-17): an agent principal may call every method, and so may the UI pieces
it mounts; pieces run with the same access as the window they are in. Limits are opt-in settings, not defaults:
scoped principals (PRT-034), caps on spawned threads and chain depth, the global process limit (DMN-038), cost caps
(omp `session.maxCost`). What stays on regardless: provenance (ARC-009) shown in the UI with filters such as "spawned
by X", a cascade stop for everything a thread spawned, and an activity log of agent-originated actions.

### Provenance, cascade stop and activity log (M10)

| ID | Requirement |
| --- | --- |
| AGT-070 | **Spawned tree**: the threads a thread spawned are those whose provenance names it (`created_by`, ARC-009, AGT-002), and, transitively, the threads they spawned; a thread's spawned tree is the thread with all of them, archived ones included. It is what the "spawned by" filter shows (UI-240) and what the cascade stop reaches (AGT-071). |
| AGT-071 | **Cascade stop**: `thread.stop { threadId, cascade: true }` (PRT-085) MUST stop the omp process of every thread of the spawned tree (AGT-070) and kill the jobs their agents started (DMN-162), whoever asks (the user, a script, an agent); it archives, deletes and removes nothing, and a stopped thread starts again on its next command (DMN-070). Without `cascade` only the thread's process stops, as before. |
| AGT-072 | **Activity log**: every protocol method an agent principal calls (a principal whose provenance is a thread, ARC-009, or the MCP principal, AGT-053) that needs more than the `Connect` or `Read` permission (PRT-034) MUST be appended to the activity log (DMN-160, DMN-161) with its time, the calling thread (`null` for the MCP principal), its client, the method, its params and its outcome, whether it succeeded or failed; reads and the user's own calls are not logged. The log is readable with `activity.list` (PRT-086) and shown by the built-in widget `omfp:activity` (UI-241). |

### Init files

| ID | Requirement |
| --- | --- |
| AGT-073 | **Agents write init files too**: a workspace's `.omfp/init.ts` (and the user's `init.ts`, DMN-185) is an ordinary file an agent writes with its own tools, like a widget (AGT-012); `omfp://docs/widgets` (`widgets/docs.md`) MUST document the init API (UI-261), the binding order (UI-262), the thread events (UI-266), where errors show (UI-263) and `:describe-key` / `:describe-command` (UI-264, UI-265), so an agent can set up keys, commands, views and reactions to thread events for the user without editing `views.json` or `keymap.json`. |
| AGT-074 | **TSX views in the manifest**: a view of `views.json` MAY be `{ title?, view: "./views/<name>.<tsx \| ts \| jsx \| js>" }` (`<name>` a widget folder name, AGT-012) instead of `{ title?, layout }`; the daemon and the app MUST refuse both keys (`views.<id>: a layout or a view file, not both`) and any other `view` (`views.<id>.view: expected "./views/<name>.tsx"`) with the shared cases (`fixtures/views/cases.json`) and the schema (`omfp://schemas/views.json`). Mounts and layout edits by agents into a TSX view MUST fail with `view "<id>" is a TSX view (<file>): edit that file, or mount into another view`; `omfp://docs/widgets` documents the layout components and the views API of init files (UI-267, UI-268). |
| AGT-075 | **Agents learn the window**: the widgets note (AGT-024) MUST name `omfp://docs` (the index of the docs, DMN-198), `omfp://docs/shell` (the window and its `:` commands) and `omfp://keymap` (its keys and commands, DMN-199), so an agent setting up keys, commands or views knows what the window already has without a browser. |
| AGT-076 | **Agents read runtime errors**: every window reports the errors it shows of init files and TSX views (`fileErrors`, PRT-115) and agents read them three ways: `views.errors` (PRT-116, through `omfp_call`), `omfp_views_errors` (DMN-196; both are agent tools, also over MCP) and `omfp_widget_check` / `omfp://widgets/check/<name>` with `init`, `user:init` or `views/<file>` (DMN-197, which also compiles the file). The widgets note (AGT-024) MUST say that `omfp_widget_check` checks init files and TSX views and that `omfp_views_errors` lists the errors windows show. |
| AGT-078 | **What an agent writing views and init files needs to know**: `omfp://docs/widgets` (`widgets/docs.md`) MUST say how the user switches views (the mode line's view tabs, `:view <id>`, fuzzy `Space v` with visible `1`–`9` selection hints, `] v` / `[ v`, UI-301, UI-302), that the window remembers the view per thread and the choice survives reloads (UI-367), which view is the default when init files use or add views (`views.json`'s; only `withDefault` changes it), what writes `views.json` (`:widget add`, UI-300, and agents; no Edit layout, UI-303), that `views.load` accepts only `"views.json"`, the command id rules (dotted ids, `ws.<action>` for a workspace's own, an id equal to a built-in's replaces it while the file is loaded, aliases that clash keep answering to the older command), what `instance` is for when a widget appears twice (`useStored` and `useJob` state; `omfp:changes` keeps none), and its review example MUST keep the `["tabs", { "id": "main" }, "omfp:chat"]` slot and give each repeated widget its `instance`. `omfp://docs/shell` (`widgets/shell.md`) MUST describe the left column, the mode line with its view tabs, the `:` commands and every default key (checked against the default keymap by a test). |
| AGT-079 | **Typed init files**: `omfp/sdk` MUST export the types `InitApi` (what `export default function init(omfp: InitApi)` receives), `ViewsApi` (`omfp.views`) and the `Layout` class, and `omfp://docs/widgets` MUST show init files written against them (`import type { InitApi } from "omfp/sdk"`). |
| AGT-080 | **New MCP tools without a restart**: the control note (AGT-045) MUST tell the agent that MCP servers added (`/mcp add`, a config file) or reauthorized (`/mcp reauth`) give it their tools from its next turn, because omfp reloads its omp once it is idle (DMN-201..204), so it never asks the user to restart. |
| AGT-081 | **What an agent sees of the windows**: `omfp_windows { windowId?, workspaceId? }` (agent tool registry, AGT-040; also over MCP) MUST describe the windows showing the workspace (the calling thread's; every window without one), the most recently focused first, from `windows.list` and `windows.state` (PRT-123): per window `Window <id> (<client>)`, focus, workspace and thread; its views in switcher order with `*` on the one it shows, where each comes from (`views.json`, the user's or the workspace's init file, the built-in default view; the TSX file), the default one and its widgets (`not rendered in this window` for a TSX view it has not rendered); the commands init files and widgets added with aliases, keys and whether they are disabled; its workspace and user widgets; its errors (init files, TSX views, widget errors and console errors). Without windows it says so plainly (none open, or how many show something else); a window without a workspace says it shows none. `omfp://windows` (host URI of a thread only, DMN-062; not an MCP resource) reads the same for the thread's workspace, never cached. |
| AGT-082 | **Running a command in a window**: `omfp_window_run { command, args?, windowId?, workspaceId? }` MUST run a registered command id or alias in a window through `windows.run` (PRT-124, DMN-206; default: the most recently focused window showing the workspace) and answer `Window <id> ran <command> [args] (command <id>)`, or the method's error (`not_found`, `conflict`, `bad_request` with the window's message). Safety: it carries only a command name and string arguments; the window runs only commands registered in it (built-ins, init files', widgets'), in the global scope, as the user's `:<command> <args>` would — never JavaScript — and its description says so. |
| AGT-083 | **The verification loop in the docs**: `omfp://docs/widgets` MUST walk agents through checking their work in the window — write, `omfp_widget_check` (with the out-of-sight render, DMN-208), `omfp_windows`, `omfp_window_run` with `view.switch`, the errors again — naming the protocol methods behind them, and `omfp://docs/shell` MUST say what windows report to agents and that agents run only registered commands. |
| AGT-084 | **The verification loop in the widgets note**: the widgets note of the thread's system prompt (AGT-024; not the control note) MUST give the same loop in one line (write → `omfp_widget_check` → `omfp_windows` → `omfp_window_run { command: "view.switch", args: [...] }` → `omfp_windows`) and say that `omfp_window_run` runs only registered commands, never code. |
| AGT-085 | **Light session facts**: the control note (AGT-045) MUST point the agent to `thread.session { threadId }` (PRT-125) for a thread's session id, session file, model, state or cost, saying that it is small and never starts omp while `thread.state` is omp's whole state (tool definitions, system prompt). |

### Jobs

| ID | Requirement |
| --- | --- |
| AGT-086 | **`useJob(command, { key })`**: with a `key`, the job slot is the key, not the command: the hook stores `{ id, runs, command }` under `jobkey:<key>` (without a key `job:<command>`, AGT-019, unchanged). When the stored command differs from `command`, the hook starts `command` with `jobs.start { replaces: <stored id> }` (DMN-215), counted as a run, and shows `pending` with no lines until the new job is stored; `restart()` replaces the job the same way, without `jobs.kill`. So a widget running a command per input (a search per query) keeps one stored key and one daemon job. |
| AGT-087 | **Where a job runs, for agents**: `omfp://docs/widgets` MUST document `useJob`'s `key` and, in its Jobs section, what a job runs in: `sh -c` (not a login shell), cwd the workspace folder (not a repository), the daemon's environment unchanged (so whatever started the daemon decides `PATH`: a terminal, the desktop session, a systemd user service; the Nix install's `PATH` additions), no dev shell (`nix develop -c …` explicitly), stdin `/dev/null` without a terminal, no timeout and what kills a job, the output limits (64 KiB lines, the newest 1 MiB kept) and the retention (memory only, 32 finished jobs per workspace, a restart forgets all, a replaced job at once, the stored keys kept). The widgets note and the `widget-builder` agent (AGT-024, AGT-029) MUST say it in one line: the shell, the folder, the daemon's environment without login or dev shell, no timeout, `key` for commands built from input, and where to read more. |
| AGT-090 | **`useJobs(workspaceId?)`** (`omfp/sdk`, M18): the declared jobs of the widget's workspace (or `workspaceId`) live from `jobs/<id>` (PRT-200): `{ jobs, error, loaded, run(name), kill(name), enable(name, enabled) }`, the actions calling PRT-204, `jobs.kill` on the job's running run and PRT-207 and rejecting with the method's error; `jobs` is `[]` and `loaded` `false` until the topic's state arrived. A widget reads a job's runs with `useCall("jobs.runs", { workspaceId, job })` (PRT-203). |
| AGT-091 | **Declared jobs, for agents** (M18): `omfp://docs/widgets` MUST have a Declared jobs section — the file and its schema (`omfp://schemas/jobs.json`, DMN-331) with a manual, a scheduled prompt and a service example, the methods (`jobs.define` and the others, so an agent asked to "read my mail every morning" declares a `schedule` job with a `prompt`), what a run runs in (AGT-087's environment, `cwd`), one run per job, missed times, service restarts, that jobs belong to the workspace and survive the thread that declared them — and `useJobs`, init files' `omfp.jobs` and their run events (UI-748) with the warning that an init file's listener runs once per open window and never when none is open, so it may notify or show something but must not start runs or chain jobs. The widgets note (AGT-024) MUST name the file in one line. |

### MCP tools

| ID | Requirement |
| --- | --- |
| AGT-095 | **`useMcp`, `useMcpTool`, `mcpText`** (`omfp/sdk`, M19): `useMcp(server, { workspaceId? })` gives `{ tools, error, refreshing, refresh, call(tool, args?) }`, the tools of the widget's workspace's (or `workspaceId`'s) server `server` from `mcp.tools` (PRT-220) with `useCall`'s semantics (on mount, when its params change, on `refresh`; a null server lists nothing) and `call` running `mcp.call` (PRT-221, rejecting with its error); `useMcpTool(server, tool, args, { workspaceId? })` is `useCall("mcp.call", …)`'s `{ data, error, refreshing, refresh }` (`args` null calls nothing); `mcpText(result)` joins a result's `text` blocks with newlines. `omfp://docs/widgets` documents them in an MCP tools section: the daemon calls the workspace's `http`/`sse` servers itself with omp's login, `/mcp reauth <name>` in any thread renews one, stdio servers are refused. |

### Form controls

| ID | Requirement |
| --- | --- |
| AGT-088 | **Kit controls, not native ones**: a native `<select>`, `<input>` of type `checkbox`, `radio`, `range`, `date` or `color`, or `<datalist>` opens a list or picker the platform draws (in the Linux desktop window a GTK menu, white with a blue highlight), which no page style or theme reaches. `omfp://docs/widgets` MUST tell agents to build every control from `omfp/kit` and never from those elements, with a **Form controls** table (each element and its kit replacement: `Select`, `Checkbox`, `RadioGroup` with `Radio`, `TextField`, `SearchField`/`ListBox`) and the `Select` pattern: `label` with `hideLabel`, `items` with an "All" entry whose id is a sentinel such as `"*"` (a key must be non-empty), `selectedKey`, `onSelectionChange`; the widgets note (AGT-024) and the `widget-builder` agent (AGT-029) MUST say the same in their shared facts. |
| AGT-089 | **`omfp_widget_check` warns about native controls**: the check of a widget (DMN-063) MUST scan the script files of its folder (`.ts`, `.tsx`, `.mts`, `.jsx`, `.js`, `.mjs`) for JSX tags `<select`, `<datalist` and `<input` whose literal `type` (`"…"`, `'…'`, `{"…"}`, ``{`…`}``, any case) is `checkbox`, `radio`, `range`, `date` or `color` — a text scan, not a parse; a `<` right after an identifier is no tag — and, when it finds any, add after the compile result `Warning: <n> native form control(s): …` and one line per control, `- <file>:<line>: <element> → <kit replacement>`, in path then line order. It is a warning: the compile result and the windows' reports stay as they are. |

## Constraints that apply from day one

| ID | Requirement |
| --- | --- |
| AGT-001 | API-first (ARC-007, D-12): every UI action is a protocol method; agent tools will be adapters over the method table. |
| AGT-002 | Provenance is recorded on every created entity (ARC-009). |
| AGT-003 | Authorization goes through a per-connection `Principal` checked in one place (PRT-034). |
| AGT-004 | The UI uses a panel registry, not a fixed layout (UI-024). |
| AGT-005 | Threads are not bound to repositories, and workspaces can be attached and detached live (ARC-008, DMN-029). |
| AGT-006 | Host tools are registered from a single registry in the daemon (DMN-060), so adding tools needs no change in the supervisor. |
| AGT-007 | Protocol methods are self-describing: params and results have JSON Schemas generated from `omfp-protocol` types (`schemars`), so agent tool schemas can be derived automatically. Their text is pinned once, by the TypeScript types generated from them (`ui/packages/protocol/src/generated.ts`, `just gen-ts`), whose staleness test fails on any change; the wire format by the per-method goldens (PRT-031). |
| AGT-008 | No operation an agent could reasonably perform may require a human in the loop by design (e.g. creating a thread or workspace has no mandatory interactive step); where confirmation is required, it goes through interactions (DMN-050), which agent-originated requests can raise too. |
