# 05 — Terminal

**Status:** active for reusable widget PTYs (issue #80). The earlier thread-owned omp terminal design was removed
(D-06, PRT-135): it would have replaced a thread's RPC process and exposed TUI-only features. Its rows remain below
so their IDs are not reused. The current design is independent of omp: daemon-owned PTY sessions are data sources for
widgets and agent-built interfaces, while `useJob` remains the non-interactive command API.

| ID | Requirement |
| --- | --- |
| TRM-001 | (Removed.) Opening an `omp` terminal required the thread to be idle or stopped with no pending interactions. |
| TRM-002 | (Removed.) The RPC process was stopped and the session lease checked before spawning the terminal process. |
| TRM-003 | (Removed.) A thread with an open `omp` terminal was in the `Terminal` process state. |
| TRM-004 | (Removed.) The thread became `Stopped` when its `omp` terminal exited. |
| TRM-005 | (Removed.) `shell` terminals did not change the thread's process state. |
| TRM-006 | (Removed.) The terminal environment (`TERM`, `COLORTERM`, `PI_TUI_NATIVE`). |
| TRM-007 | (Removed.) Deleting or archiving a thread closed its terminals. |
| TRM-010 | (Removed.) The scrollback replayed on subscribe. |
| TRM-011 | (Removed.) Several clients attached to one terminal. |
| TRM-012 | (Removed.) Input frames for an unknown channel were dropped. |
| TRM-020 | (Removed.) xterm.js as the terminal panel's dependency. |
| TRM-021 | (Removed.) The terminal widget over binary WebSocket frames. |
| TRM-022 | (Removed.) The thread view's terminal mode with a "return to app" action. |

## Reusable PTY sessions

| ID | Requirement |
| --- | --- |
| TRM-100 | `terminals.start { workspaceId, command, cwd?, cols, rows }` MUST start `sh -c <command>` on a PTY in the workspace folder (or a workspace-relative existing `cwd`) with `TERM=xterm-256color` and `COLORTERM=truecolor`, and return its memory-only session; `terminal/<id>` MUST expose its state, base64 output chunks and exit status. Absolute or parent-traversing working directories, zero dimensions, and dimensions beyond 400 columns or 200 rows MUST be refused. |
| TRM-101 | `terminals.input { terminalId, data }` MUST write the base64-decoded bytes to the PTY, `terminals.resize` MUST resize it, and `terminals.kill` MUST terminate it. Input and output are byte streams: terminal control sequences and non-UTF-8 bytes MUST survive transport. |
| TRM-102 | A terminal session MUST outlive widget unmounts and WebSocket reconnects while the daemon runs. Any authorized client with its id MAY subscribe and interact, and a new subscription MUST receive the bounded snapshot before live events. Sessions are not restored after daemon restart. |
| TRM-103 | Terminal input MUST NOT be stored in topic state, logs or widget state. The terminal subsystem MUST keep raw output only in memory and retain at most 1 MiB per session; snapshots MUST report the number of older bytes dropped. An explicit `terminals.screen` call returns visible output to its caller and therefore MAY enter an agent's conversation, but the daemon MUST NOT add it to activity or persistent terminal state. JSON protocol frames carry base64 bytes in this first implementation; native binary WebSocket frames are a compatible future optimization. |
| TRM-104 | A terminal started by an agent MUST retain that thread as provenance; cascade stop MUST kill terminals started by the stopped tree, workspace removal MUST kill and forget its terminals, and daemon shutdown MUST kill every terminal. Agent activity MAY record terminal control calls but MUST replace input bytes with `[redacted]`. Commands themselves are auditable and MUST NOT contain secrets. |
| TRM-105 | `omfp/sdk` MUST export `useTerminal(command, { cwd?, cols?, rows?, key? })`, which stores the session id per widget instance and follows `terminal/<id>`. The copyable built-in `omfp:terminal` MUST use xterm.js, forward keyboard input and container resizes, and replay the session snapshot without starting a duplicate process. |
| TRM-106 | `terminals.screen { terminalId, afterRevision?, waitMs? }` MUST return the current bounded visible screen as plain text, its zero-based cursor and visibility, dimensions, alternate-screen flag, process status and a monotonically increasing revision. The revision MUST advance after PTY output, resize and exit. When `afterRevision` equals the current revision, the call MAY wait for a change for at most 30 seconds. The emulator MUST retain no scrollback beyond its 400×200 cell grid. Hidden input MUST reach the screen only when the program itself echoes it. `useTerminal` MUST expose the same operation as `screen()`. |
