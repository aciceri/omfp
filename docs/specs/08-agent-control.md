# 08 — Agent control

**Status:** binding. The [Vision](#vision) gives the direction; the
[Constraints that apply from day one](#constraints-that-apply-from-day-one) bind every change.

## Vision

The interface is a surface shared by the user and the agents, and agents get **the same power as the user** (D-17):
everything the UI can do, an agent can do — directly through tools, or through UI it builds itself.

1. **Everything in the protocol**: list threads and see what each one is doing, spawn threads, send them messages,
   wait for them, rename, archive; create workspaces and add or remove their repositories; read git status and diffs.
   Every protocol method is an agent tool (ARC-007).
2. **Arbitrary UI**: an agent writes its own pieces of interface as React widgets, mounted in a view, a dashboard, a
   floating panel or a chat card, live and wired to what it is doing (e.g. a progress bar driven by a long command).
3. **Reusable UI**: widgets and views are files, so later threads and sessions reuse them.
4. **Attention**: an agent marks a thread as needing the user and sends a desktop notification.

## Delivery to agents

One tool registry in the daemon (AGT-040) with several adapters: **RPC host tools** for every thread the daemon runs
(DMN-151), the **MCP server** at `/mcp` for agents it does not run (AGT-050..054), the read-only **host URIs**
`omfp://…` (DMN-062) and the **SDK** for widgets (`omfp/sdk`, `omfp/kit`). The widget SDK, kit, init files and jobs are
documented for agents in `omfp://docs/widgets`
([crates/omfp-daemon/src/widgets/docs.md](../../crates/omfp-daemon/src/widgets/docs.md)) and the manifest by
`omfp://schemas/views.json` ([views.schema.json](../../crates/omfp-daemon/src/widgets/views.schema.json)); the rows
below state only the behavior, not the API reference.

## Views and widgets

The window shows a workspace's **views manifest**; everything in it is a **widget**, a React component, built-in or
written by an agent as files under `<workspace>/.omfp/`. Only a thin frame stays outside the manifest: the left column,
the connection banner, the view switcher, keymap and command line, notifications and the safety net (AGT-015).

| ID | Requirement |
| --- | --- |
| AGT-010 | **Views manifest**: `<workspace>/.omfp/views.json` is `{ "$schema"?, "default"?, "views": { <id>: { "title"?, "layout": <node> } } }` with at least one view, unknown keys ignored. A node is a reference (`omfp:<name>`, `./widgets/<name>`), `[reference, props]` or `[row\|col\|tabs\|stack, props, ...children]`, `sizes` one positive weight per child. Views are listed in id order; `default`, else the first, opens. A one-child `tabs` has no tab bar; a tab's label is the `title` prop, else the built-in's name, else the reference. |
| AGT-011 | **Invalid manifest**: a `views.json` that cannot be read, is not JSON or breaks AGT-010 never breaks the window: the frame shows the default view and a banner with the error (its JSON path and reason); a fixed file recovers on the next push, without reload. An unknown reference is no manifest error: it shows an error in place of its widget. |
| AGT-012 | **Widget modules**: a widget is the folder `.omfp/widgets/<name>/` (`<name>` of ASCII letters, digits, `_`, `-`, `.` not first) with entry `index.tsx`, `index.ts`, `index.jsx` or `index.js` (the first present), whose default export is a React component taking the node's props. It imports only files of its own folder and `react`, `react/jsx-runtime`, `react-dom`, `omfp/sdk` and `omfp/kit`, which resolve to the app's own modules. |
| AGT-013 | **Live reload**: a widget is imported from a URL carrying its folder's version (DMN-084); when the views topic pushes a new version, the window imports that widget again and re-renders only it. State in `useStored` survives a reload, component-local state does not. |
| AGT-014 | **Errors in place**: a compile error in any file of the folder (syntax, a forbidden import, a name `omfp/sdk`, `omfp/kit` or React do not export, DMN-085), a failed import, a missing entry or default export and a render error show in place of the widget with file, line, column and message; the rest of the window keeps working and the next save recovers without reload. |
| AGT-015 | **Safety net**: when the selected thread has a pending interaction and the current view contains neither built-in `omfp:chat` nor `omfp:interactions` (a workspace override of them does not count), the frame renders the interactions itself. |
| AGT-016 | **SDK** (`omfp/sdk`): hooks for the connection, index, threads, workspaces, any topic, every protocol method, window state, commands, announcements, thread actions and settings, plus `useWidget()` (instance, reference, workspace id), `useStored`, `useJob`, `useToolRun` and the view models and strings the built-in widgets use; every name `omfp://docs/widgets` lists is exported. |
| AGT-017 | **Kit** (`omfp/kit`): the kit's components (UI-093), including `Card`, `Button`, `Progress`, `Log`, `Markdown`, `Tabs`, `ListBox`, `Menu`, `Badge`, `CodeEditor` and `CodeDiff` (the editors loaded on first use), styled by the theme tokens (UI-091). |
| AGT-018 | **`useStored(key, initial)`** → `[value, set, loaded]`: JSON state kept by the daemon per workspace, widget instance (its string `instance` prop, else its reference) and key (DMN-086), the same in every window, pushed live and surviving reloads and daemon restarts; `set` takes a value or an updater, `null` deletes the key; until `loaded`, `value` is `initial`. |
| AGT-019 | **`useJob(command)`** follows one daemon job per widget instance and command, stored with `useStored` under `job:<command>`: it starts one (`jobs.start`) only while the instance has none the daemon knows, so reloads, other windows and moving the widget never start a second. `status` is `pending`, `running`, `succeeded` (exit 0) or `failed`; `restart` kills and starts again, `kill` stops; `omfp://docs/widgets` lists the other fields. |
| AGT-020 | **`useToolRun(toolCallId \| { match, threadId? })`**: a tool run of a thread (the selected one by default), by id or the latest whose command (`arguments.command`, else the JSON arguments) matches `match`: `{ toolCallId, toolName, status, output, lines, isError }`, `output` being the latest partial output while it runs and the result's text once it ended. |
| AGT-021 | **Built-ins are copyable**: every built-in widget (`omfp:header`, `chat`, `transcript`, `interactions`, `composer`, `workspace`, `changes`, `background`, `todos`, `debug`, `tree`, `activity`, `mcp`, `file`, `terminal`, `jobs`, `web`, `browser`) imports only what an agent widget may (AGT-012), the release bundle ships its source as a widget folder, and `:widget copy omfp:<name> [<new name>]` (`widgets.copy`, DMN-087) copies it into `.omfp/widgets/`. |
| AGT-022 | **Override**: in a workspace, `omfp:<name>` renders `./widgets/<name>` when that folder exists, so a copied built-in (or any widget of that name) replaces the original there. |
| AGT-024 | **Widgets note**: the daemon's `--append-system-prompt` text (DMN-030) carries, after the workspace conventions, a short widgets note: where `views.json` (always present, DMN-088) and widgets live, the allowed imports, a ten-line example, the URIs `omfp://docs/widgets`, `omfp://schemas/views.json` and `omfp://widgets/builtin/<name>` (DMN-062), and the tools that check and place widgets (`omfp_widget_check`, `omfp_ui_mount` / `_update` / `_remove`). |
| AGT-025 | **Agent docs**: `omfp://docs/widgets` (the SDK, kit, views, init files and jobs reference) and `omfp://schemas/views.json` (the manifest's JSON Schema) are read-only host URIs of every thread (DMN-062). Every agent prompt (widgets note, control note, `widget-builder`, MCP instructions) gives one line per topic and points there; every `omfp_*` tool and `omfp://` URI such a text names exists where it is served. |
| AGT-026 | **Window reports**: every window reports (`widgets.report`, PRT-043), per workspace widget instance of its view, the import's state, compile, import and render errors, console and uncaught errors from the widget's module, a text snapshot of its DOM, its `useJob` jobs and the views `{ epoch, seq }` it rendered; again on change (at most every 250 ms), with `workspaceId: null` when it shows no workspace. Reports also carry init file errors (AGT-076). |
| AGT-027 | **Files**: `useFile(path, { follow? })` → `{ status, file, error, reload, save }` reads a workspace file (DMN-089) and, while `follow` (default), again every 2 s and on each workspace status push; `save(content)` writes against the shown version (DMN-090) and resolves `{ ok: true, file }`, `{ ok: false, conflict: true }` or `{ ok: false, error }`. `useOpenFile()` opens a file tab (`openFile(path, { line?, diff? })`, UI-115); `useMissingFile` finds a missing path's file (DMN-092, UI-117). |
| AGT-028 | **Agents keep `main`**: opened files become tabs of the view's `["tabs", { "id": "main" }, …, "omfp:chat"]` slot (UI-115); the default view is the header over it beside the `side` tabs (workspace, changes, background, MCP). The widgets note, `omfp://docs/widgets` and `omfp_ui_mount`'s description tell agents in the same words to keep it, and `omfp_widget_check` (DMN-063) warns, ending `place widgets with omfp_ui_mount.`, when a valid `views.json` has no `main` slot. |
| AGT-029 | **Widget builder**: the daemon ships the omp agent `widget-builder` (DMN-107; tools `read, write, edit, glob, grep`, model `@smol`, runs in the background) carrying the widgets note's facts, the phone rule (AGT-084) included; it reads `omfp://widgets/check/<name>` (DMN-108) after every write until clean, never edits `views.json` and ends naming the widget and its props. The widgets note asks the agent to propose a widget in one sentence and, once the user agrees, to brief `widget-builder` and place the result with `omfp_ui_mount`. |
| AGT-030 | **Agent keys**: an agent binds keys for a workspace in `.omfp/keymap.json` (DMN-120, schema `omfp://schemas/keymap.json`), each binding a `[keymap]` entry (UI-097) with optional `args`, to a built-in command or to one a widget registers with `useCommand` (AGT-016); the bindings apply in windows showing that workspace (UI-210). |
| AGT-031 | The widgets note MUST give one line to `.omfp/keymap.json` (its path and schema, commands built-in or a widget's `useCommand`), and `omfp://docs/widgets` documents the file: its shape, the precedence, the modes and key tokens, and an example. |
| AGT-032 | The widgets note MUST give one line to `omfp_notify { title, body }` (DMN-125): use it when a long task finished or the agent needs the user, once per occasion, never for progress. |

## Agent tools

| ID | Requirement |
| --- | --- |
| AGT-040 | **Agent tool registry** (DMN-060): every agent tool is defined once (name, label, description, argument JSON Schema) and runs as a principal (DMN-150) with the calling thread's workspace, if any; every thread gets each as an essential host tool (DMN-151) and the MCP server lists the same. A tool adapting a protocol method runs it through the one authorization check (PRT-034) and the method's handler, so it behaves exactly like the method. |
| AGT-041 | **`omfp_call { method, params? }`** calls a protocol method by wire name (`params` absent or `null` is `{}`) as the calling thread's agent principal and answers its JSON result; a failure is the tool error `<code>: <message>` plus its `details` JSON. An unknown method or bad params names `omfp://protocol/methods`; connection-only methods (`system.hello`, `subscribe`, `watch`, `widgets.report`, `thread.live_*`, …) are refused. Read-only `thread.*` methods default `threadId` to the caller; others' missing-`threadId` error gives the caller's id. |
| AGT-042 | **Method catalogue**: `omfp://protocol/methods` (Markdown) lists every method `omfp_call` accepts, one line each, `` `<name>` — <description> `` (PRT-092), and says where the schemas are; `omfp://protocol/methods/<name>` (JSON) is `{ method, description, params, result }` with their JSON Schemas (AGT-007). Connection-only methods are left out; an unknown or connection-only name is an error naming the list. |
| AGT-043 | **`omfp_thread_wait { threadId, promptId?, timeoutSeconds? }`** answers `{ threadId, status, prompt?, lastAssistantText }` once the thread is done: with `promptId`, when that prompt's `prompt_result` arrived; without, when its latest admitted prompt (anyone's, DMN-152) has one and the session settled, a `thread.prompt` still on its way holding the wait. `status` is `done`, `needs_attention` (a pending interaction), `stopped` / `failed` (no or a failed process) or `timeout` (default 600 s). |
| AGT-044 | `omfp_thread_wait` MUST refuse as tool errors a missing or blank `threadId`, a `promptId` that is not a non-blank string, a `timeoutSeconds` outside 1..=3600, the calling thread itself and an unknown thread (`not_found`); cancelling the tool call stops the wait. |
| AGT-045 | **Control note**: the daemon's `--append-system-prompt` text (DMN-030) MUST end with a `# omfp control` note of at most 7 lines: the thread's own id, `omfp_call` reaching every protocol method as windows do, the catalogue URIs, `omfp_thread_wait` instead of polling, `thread.session` (AGT-085) and MCP reloads (AGT-080). The appended order is workspace conventions, widgets note, control note, then the user's `APPEND_SYSTEM.md`. |
| AGT-099 | **Language servers**: the workspace conventions (DMN-030) MUST carry one line saying that `lsp` servers start per repository from the daemon's `PATH`, not a dev shell, and that without one the agent falls back to `grep`/`ast_grep` without raising it unless the user asks; `omfp://docs/shell` MUST say how omp finds servers, why it may find none (`lsp status`) and how to get one (the daemon's `PATH`, `<workspace>/.omp/lsp.json`, `lsp reload`). |

### MCP server

Agents the daemon does not run (an omp session outside omfp, any MCP client) reach the same registry through the MCP
server at `/mcp` (DMN-155..159) as the **MCP principal**, with the user's full access (D-17), logged and attributed.

| ID | Requirement |
| --- | --- |
| AGT-050 | **MCP tools**: `tools/list` MUST list the registry's tools in order as `{ name, title, description, inputSchema }`, and `tools/call { name, arguments }` run one as the MCP principal, answering `{ content: [{ type: "text", text }], isError }`; an unknown tool or non-object `arguments` (absent is `{}`) is JSON-RPC `-32602`. `omfp_notify` and `omfp_open_file` act on a calling thread and are not MCP tools; `omfp_widget_check` and `omfp_views_errors` are, with `workspaceId` required. |
| AGT-051 | **MCP principal**: an MCP request runs as the MCP principal of the client named by `clientInfo.name` (from `initialize`, else the request's `io.modelcontextprotocol/clientInfo`; `mcp` without one), with every permission but `Connect` (DMN-150). What it creates records the provenance `mcp` (PRT-093) and the client; without a calling thread, `omfp_thread_wait` waits for any thread and the mount tools need `workspaceId` (AGT-066). |
| AGT-052 | **MCP resources**: `resources/list` MUST list `omfp://protocol/methods`, `omfp://docs`, `omfp://docs/widgets`, `omfp://docs/shell`, `omfp://keymap` (defaults only), `omfp://schemas/views.json`, `omfp://schemas/keymap.json` and `omfp://widgets/builtin`, and `resources/templates/list` `omfp://protocol/methods/{name}` and `omfp://widgets/builtin/{name}`; `resources/read` answers them as a thread reads them, any other URI is `-32602`. Its `instructions` point to `omfp://docs/widgets` and `omfp_widget_check`. |
| AGT-053 | **MCP calls are logged**: the activity log (AGT-072) MUST record the calls of the MCP principal the way it records an agent's, with `threadId` `null` and the MCP client as `client` (PRT-094), so `omfp:activity` shows them as `mcp: <client>` (UI-241). |
| AGT-054 | **Configuring a client**: `omfp mcp config` (DMN-159) prints the `mcp.json` entry for the local daemon, for a standalone omp (`~/.omp/agent/mcp.json`, a project's `.omp/mcp.json`) or another client; omfp's own threads need none (ARC-021). The header is `"Authorization": "Bearer ${OMFP_TOKEN}"`, so no file holds the token (`--literal-token` writes it, never for a committable file), and `"timeout": 0` lets `omfp_thread_wait` outlast omp's request timeout. |

### UI mounts

A **mount** is a widget node of the workspace's `views.json` whose `instance` prop is the mount id, in a view (a
dashboard is a view) or a **floating panel** over every view. The tools adapt protocol methods (PRT-080..082), which a
widget (`useCall`) or an MCP client reaches too.

| ID | Requirement |
| --- | --- |
| AGT-060 | **Floating panels**: `views.json` MAY carry `"panels": { <id>: <node> }`, widgets floating over every view of the workspace in id order, each checked like a layout (AGT-010; errors at `panels.<id>`); a non-object `panels` is the error `panels: expected an object of panels`. |
| AGT-061 | **`omfp_ui_mount { id, widget? \| code?, props?, view?, place?, panel?, workspaceId? }`** (`views.mount`, PRT-080) mounts once per `id`, a widget name: `widget` is a reference (a bare name means `./widgets/<name>`), or `code` is written to `.omfp/widgets/<id>/index.tsx`; the node gets `props` with `instance: id`. `panel: true` makes it `panels.<id>`; else it goes into `view` (default: the default view; a new id makes a dashboard) at `place` `side_tab` (default), `main_tab` or `end`, replacing any other node of `id`. |
| AGT-062 | **`omfp_ui_update { id, props?, code?, workspaceId? }`** (`views.mount_update`, PRT-081) merges `props` into mount `id` (`null` deletes a prop; `instance` stays) and with `code` rewrites its `index.tsx` (only for `./widgets/<id>`); `not_found` when nothing has that id. Windows re-render it in place, keeping its state (UI-232). |
| AGT-063 | **`omfp_ui_remove { id, workspaceId? }`** (`views.unmount`, PRT-082) removes panel `id` and every node of mount `id`; an emptied container goes too (with its weight), an emptied view is removed (and `default` if it named it), and emptying the last view is `bad_request`. The widget folder stays; `not_found` when nothing has that id. |
| AGT-066 | The mount tools act on the calling thread's workspace; without one (the MCP principal, AGT-051) `workspaceId` is required. `omfp://docs/widgets` documents panels and mounts. |

### Cards in the chat

A **card** puts a widget in front of the user for one moment of the conversation: it is drawn in the card of its own
tool call in the transcript, never in `views.json`, and may wait for an answer that becomes the tool's result.

| ID | Requirement |
| --- | --- |
| AGT-096 | **`omfp_card { widget? \| code?, props?, title?, wait?, timeoutSeconds? }`** draws one widget, writing no file or view: `widget` a reference (a bare name means `./widgets/<name>`; `omfp:<name>`) or `code` a one-file component compiled as `card.tsx` (DMN-085, no relative import; errors `card.tsx:line:column: message`). It needs a calling thread. It answers `{"status":"shown"}`, or with `wait: true` `submitted` with `value`, `cancelled` or `timeout` (`timeoutSeconds` ≥ 1). |
| AGT-097 | **Waiting cards**: while an `omfp_card` call waits, its thread needs the user (DMN-911) and `omfp_thread_wait` on it answers `needs_attention` (AGT-043). The first `card.answer` (PRT-224) from any window or agent ends the wait, as do the timeout and omp's cancellation; a later answer is `already_resolved`. A daemon or thread restart ends the call. |
| AGT-098 | `omfp://docs/widgets` MUST document cards: `omfp_card`, `useAnswer` (UI-921), and that a card's widget mounts again from the call's arguments whenever shown, so it draws its finished state from `useAnswer().state`. The widgets note says in one line: a moment of the conversation goes in a card, content that stays in a mount (UI-375). |

## Permissions and limits

Agents run in **full-access mode by default** (D-17): an agent principal may call every method, and so may the
widgets it mounts. Limits are opt-in: scoped principals (PRT-034), the global process limit (DMN-038), cost caps (omp
`session.maxCost`). Always on: provenance (ARC-009) with a "spawned by" filter, a cascade stop for everything a thread
spawned, and an activity log of agent-originated actions.

| ID | Requirement |
| --- | --- |
| AGT-070 | **Spawned tree**: the threads a thread spawned are those whose provenance names it (`created_by`, ARC-009), and, transitively, the threads they spawned; a thread's spawned tree is the thread with all of them, archived ones included. It is what the "spawned by" filter shows (UI-240) and what the cascade stop reaches (AGT-071). |
| AGT-071 | **Cascade stop**: `thread.stop { threadId, cascade: true }` (PRT-085) MUST stop the omp process of every thread of the spawned tree and kill the jobs their agents started (DMN-162), whoever asks; it archives or deletes nothing, and a stopped thread starts again on its next command (DMN-070). Without `cascade` only the thread's process stops. |
| AGT-072 | **Activity log**: every method an agent principal (a thread's agent, ARC-009, or the MCP principal) calls that needs more than `Connect` or `Read` (PRT-034) MUST be appended to the activity log (DMN-160) with its time, the calling thread (`null` for MCP), client, method, params and outcome; reads and the user's calls are not logged. `activity.list` (PRT-086) and `omfp:activity` (UI-241) show it. |

## Init files

| ID | Requirement |
| --- | --- |
| AGT-073 | **Agents write init files too**: `.omfp/init.ts` (and the user's `init.ts`, DMN-185) is a file an agent writes like a widget; `omfp://docs/widgets` MUST document the init API (UI-261), the binding order (UI-262), the thread events (UI-266), where errors show (UI-263) and `:describe-key` / `:describe-command` (UI-264, UI-265). |
| AGT-075 | **Agents learn the window**: the widgets note MUST name `omfp://docs` (the docs index, DMN-198), `omfp://docs/shell` (the window and its `:` commands) and `omfp://keymap` (its keys and commands, DMN-199). |
| AGT-076 | **Agents read runtime errors**: windows report the init file errors they show (`fileErrors`, PRT-115), which agents read through `views.errors` (PRT-116), `omfp_views_errors` (DMN-196) and `omfp_widget_check` / `omfp://widgets/check/<name>` with `init` or `user:init` (DMN-197, which also compiles the file); the widgets note names the last two. |
| AGT-078 | **Views and init files in the docs**: `omfp://docs/widgets` MUST say how the user switches views (UI-301, UI-302) and that the choice is kept per thread (UI-367), which view is the default when init files add views (only `withDefault` changes it), what writes `views.json`, that `views.load` takes only `"views.json"`, the command id rules and what `instance` is for, with examples keeping the `main` slot; `omfp://docs/shell` MUST describe the left column, the mode line, the `:` commands and every default key. |
| AGT-079 | **Typed init files**: `omfp/sdk` MUST export the types `InitApi` (what `export default function init(omfp: InitApi)` receives), `ViewsApi` (`omfp.views`) and the `Layout` class, and `omfp://docs/widgets` MUST show init files written against them (`import type { InitApi } from "omfp/sdk"`). |
| AGT-080 | **New MCP tools without a restart**: the control note MUST say that MCP servers added (`/mcp add`, a config file) or reauthorized (`/mcp reauth`) give their tools from the next turn, as omfp reloads the thread's omp once idle (DMN-201..204), so the agent never asks for a restart. |
| AGT-081 | **What an agent sees of the windows**: `omfp_windows { windowId?, workspaceId? }` MUST describe the windows showing the workspace (all without one), most recently focused first (PRT-123): `Window <id> (<client>)`, focus, workspace, thread; its views with `*` on the shown one, each one's source and the default's widgets; commands from init files and widgets with aliases, keys and disabled state; its widgets and errors; no window is said plainly. `omfp://windows` reads the same for a thread, uncached. |
| AGT-082 | **Running a command in a window**: `omfp_window_run { command, args?, windowId?, workspaceId? }` MUST run a registered command id or alias through `windows.run` (PRT-124, DMN-206; default: the most recently focused window showing the workspace) and answer `Window <id> ran <command> [args] (command <id>)` or the method's error. It carries a command name and string arguments only, never JavaScript, as the user's `:<command> <args>` would. |
| AGT-083 | **The verification loop in the docs**: `omfp://docs/widgets` MUST walk agents through checking their work — write, `omfp_widget_check` (with the out-of-sight render, DMN-208), `omfp_windows`, `omfp_window_run` with `view.switch`, the errors again — naming the methods behind them; `omfp://docs/shell` says what windows report and that agents run only registered commands. |
| AGT-084 | **The verification loop in the widgets note**: the widgets note MUST give the loop in one line (write → `omfp_widget_check` → `omfp_windows` → `omfp_window_run { command: "view.switch", args: [...] }` → `omfp_windows`) and say that `omfp_window_run` runs only registered commands, never code. With it, the widgets note, the `widget-builder` agent and `omfp://docs/widgets` (Checking your work in the window) MUST give the same phone rule: every view, widget, init-file view and card also works in the `narrow` and `mobile` layouts (read `FRAME_LAYOUT`; stack instead of columns, no fixed widths, touch-sized controls, no hover-only actions), and its result is checked at a phone width, since the check renders in the focused window's layout only. |
| AGT-085 | **Light session facts**: the control note MUST point to `thread.session { threadId }` (PRT-125) for a thread's session id, session file, model, state or cost: small and never starting omp, unlike `thread.state`. |

## Jobs

| ID | Requirement |
| --- | --- |
| AGT-086 | **`useJob(command, { key })`**: with a `key` the job slot is the key, stored under `jobkey:<key>` as `{ id, runs, command }`. When the stored command differs from `command`, the hook starts it with `jobs.start { replaces }` (DMN-215), counted as a run and `pending` with no lines until stored; `restart()` replaces the same way. So a widget running a command per input keeps one key and one daemon job. |
| AGT-087 | **Where a job runs, for agents**: `omfp://docs/widgets` MUST document `useJob`'s `key` and what a job runs in: `sh -c` without login or dev shell, cwd the workspace folder, the daemon's environment unchanged (whatever started the daemon decides `PATH`), stdin `/dev/null`, no timeout, the output limits (64 KiB lines, the newest 1 MiB kept) and the retention (memory only, 32 finished jobs per workspace); the widgets note and `widget-builder` say it in one line. |
| AGT-090 | **`useJobs(workspaceId?)`** (`omfp/sdk`): the declared jobs of the widget's workspace, live from `jobs/<id>` (PRT-200): `{ jobs, error, loaded, run(name), kill(name), enable(name, enabled) }`, the actions calling PRT-204, `jobs.kill` and PRT-207 and rejecting with the method's error; `jobs` is `[]` until loaded. Runs come from `useCall("jobs.runs", …)` (PRT-203). |
| AGT-091 | **Declared jobs, for agents**: `omfp://docs/widgets` MUST document declared jobs: the file and its schema `omfp://schemas/jobs.json` (DMN-331) with manual, scheduled-prompt and service examples, the methods, what a run runs in, one run per job, missed times, service restarts, jobs outliving the thread that declared them, `useJobs`, and init files' `omfp.jobs` events (UI-748), whose listeners run once per open window and must not start runs. The widgets note names the file. |

## MCP tools in widgets

| ID | Requirement |
| --- | --- |
| AGT-095 | **`useMcp`, `useMcpTool`, `mcpText`** (`omfp/sdk`): `useMcp(server, { workspaceId? })` → `{ tools, error, refreshing, refresh, call(tool, args?) }` over `mcp.tools` and `mcp.call` (PRT-220, PRT-221) with `useCall`'s semantics (a null server lists nothing); `useMcpTool(server, tool, args, …)` → `{ data, error, refreshing, refresh }` (`args` null calls nothing); `mcpText(result)` joins the `text` blocks. `omfp://docs/widgets` documents them. |

## Form controls

| ID | Requirement |
| --- | --- |
| AGT-088 | **Kit controls, not native ones**: `omfp://docs/widgets` MUST tell agents to build every control from `omfp/kit`, never from `<select>`, `<datalist>` or an `<input>` of type `checkbox`, `radio`, `range`, `date` or `color`, whose lists and pickers the platform draws out of the theme's reach, with a table of kit replacements and the `Select` pattern (hidden label, an "All" item with a sentinel id such as `"*"`); the widgets note and `widget-builder` say it too. |
| AGT-089 | **`omfp_widget_check` warns about native controls**: the check (DMN-063) MUST scan the widget's script files (`.ts`, `.tsx`, `.mts`, `.jsx`, `.js`, `.mjs`) as text for `<select`, `<datalist` and `<input` tags whose literal `type` (any quoting or case) is `checkbox`, `radio`, `range`, `date` or `color`, and add after the compile result `Warning: <n> native form control(s): …` and one line `- <file>:<line>: <element> → <kit replacement>` per hit, in path then line order. |

## Constraints that apply from day one

The rest of this document builds on the rows of other specs: API-first (ARC-007, D-12), provenance on every created
entity (ARC-009), authorization in one place (PRT-034), the panel registry (UI-024) and the host tool registry
(DMN-060). No operation an agent could reasonably perform may require a human in the loop by design (e.g. creating a
thread or workspace has no mandatory interactive step); where confirmation is required, it goes through interactions
(DMN-050), which agent-originated requests can raise too.

| ID | Requirement |
| --- | --- |
| AGT-005 | Threads are not bound to repositories, and workspaces can be attached and detached live (ARC-008, DMN-029). |
| AGT-007 | Protocol methods are self-describing: params and results have JSON Schemas generated from the protocol types, from which agent tool schemas derive; the generated TypeScript types pin their text and the per-method goldens the wire format (PRT-031). |
