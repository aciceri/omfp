/**
 * The `omfp:debug` widget (UI-946..UI-950): the agent's debug (DAP) session of a thread, rebuilt from the `details` of
 * its `debug` tool results. omp sends them on a `debug` result (`details: { action, snapshot, … }`), on a `bash` result
 * whose single dispatch is the `debug` tool command (`details.toolCommands: [{ tool: "debug", mode: "execute", args,
 * inner }]`, omp 19) or, in sessions recorded by omp 18.x, on a `write` to the `xd://debug` device
 * (`details.xdev: { tool: "debug", args, inner }`). Only the
 * transcript counts: after a branch navigation the reducer keeps the tool runs of the branch left, never replayed here.
 */

import type { ThreadState } from "@omfp/core";
import { strings } from "../strings/index.ts";

const text = strings.shell.debug;

/** A place in a source file, opened with `openFile(path, { line })`. */
export interface DebugLocation {
  readonly path: string;
  readonly line: number;
}

export interface DebugFrameRow {
  readonly id: number;
  readonly name: string;
  readonly location: DebugLocation | undefined;
  /** `path:line`, or the frame's instruction pointer without a source. */
  readonly where: string;
}

export interface DebugVariableRow {
  readonly name: string;
  readonly value: string;
  readonly type: string | undefined;
}

export interface DebugBreakpointRow {
  /** `file:line`, or the function's name. */
  readonly label: string;
  readonly location: DebugLocation | undefined;
  readonly verified: boolean;
  readonly condition: string | undefined;
  /** The adapter's message, e.g. why it could not verify the breakpoint. */
  readonly message: string | undefined;
}

export interface DebugStop {
  /** omp's `stopReason` (`breakpoint`, `step`, `exception`, `pause`, …). */
  readonly reason: string | undefined;
  readonly frame: string | undefined;
  readonly location: DebugLocation | undefined;
  /** `path:line` as the adapter gave it (the location's path may be resolved from it). */
  readonly where: string;
}

export interface DebugView {
  readonly adapter: string;
  readonly program: string | undefined;
  /** omp's session status: `launching`, `configuring`, `stopped`, `running` or `terminated`. */
  readonly status: string;
  readonly statusLabel: string;
  /** Where the program is stopped; `undefined` unless `status` is `stopped`. */
  readonly stop: DebugStop | undefined;
  readonly exitCode: number | undefined;
  /** The stack the agent last read at the current stop (`stack_trace`), innermost frame first. */
  readonly stack: readonly DebugFrameRow[];
  /** The variables the agent last read at the current stop (`variables`). */
  readonly variables: readonly DebugVariableRow[];
  readonly breakpoints: readonly DebugBreakpointRow[];
}

/** The actions that run the program or start another session: what was read before them is stale. */
const EXECUTION_ACTIONS: Readonly<Record<string, true>> = {
  launch: true,
  attach: true,
  continue: true,
  step_over: true,
  step_in: true,
  step_out: true,
  pause: true,
  terminate: true,
};

type Obj = Readonly<Record<string, unknown>>;

function isObj(value: unknown): value is Obj {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function str(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function int(value: unknown): number | undefined {
  return typeof value === "number" && Number.isInteger(value) ? value : undefined;
}

function objects(value: unknown): Obj[] {
  return Array.isArray(value) ? value.filter(isObj) : [];
}

/** A debug call's arguments and result details. */
interface DebugCall {
  readonly args: Obj;
  readonly details: Obj;
}

/** The debug call of a successful tool result, from a `debug` result, a `debug` command or an 18.x `xd://debug` `write`. */
function debugCall(message: Obj, argsById: ReadonlyMap<string, Obj>): DebugCall | undefined {
  if (message.role !== "toolResult" || message.isError === true || !isObj(message.details)) {
    return undefined;
  }
  const details = message.details;
  if (message.toolName === "debug") {
    const id = str(message.toolCallId);
    return { args: (id === undefined ? undefined : argsById.get(id)) ?? {}, details };
  }
  if (message.toolName === "bash" && Array.isArray(details.toolCommands) && details.toolCommands.length === 1) {
    const dispatch: unknown = details.toolCommands[0];
    if (isObj(dispatch) && dispatch.tool === "debug" && dispatch.mode === "execute" && isObj(dispatch.inner)) {
      return { args: isObj(dispatch.args) ? dispatch.args : {}, details: dispatch.inner };
    }
    return undefined;
  }
  const xdev = details.xdev;
  if (message.toolName === "write" && isObj(xdev) && xdev.tool === "debug" && isObj(xdev.inner)) {
    return { args: isObj(xdev.args) ? xdev.args : {}, details: xdev.inner };
  }
  return undefined;
}

/**
 * `path` against the session's folder `cwd` (`snapshot.cwd`), `.` and `..` resolved: a relative path stays the
 * session's, wherever the thread's workspace is now; an absolute path is only normalized, so every spelling of a file
 * gives one path. A relative path without a `cwd` is kept as is.
 */
function resolvePath(cwd: string | undefined, path: string): string {
  const absolute = path.startsWith("/");
  if (!absolute && cwd === undefined) {
    return path;
  }
  const parts: string[] = [];
  for (const part of (absolute ? path : `${cwd}/${path}`).split("/")) {
    if (part === "..") {
      parts.pop();
    } else if (part !== "" && part !== ".") {
      parts.push(part);
    }
  }
  return `/${parts.join("/")}`;
}

/** The place of a DAP `source` and line: the location to open, resolved against `cwd`, and its label as given. */
function place(
  source: unknown,
  line: unknown,
  cwd: string | undefined,
): { at: DebugLocation; where: string } | undefined {
  const file = isObj(source) ? str(source.path) : undefined;
  const at = int(line);
  return file === undefined || at === undefined
    ? undefined
    : { at: { path: resolvePath(cwd, file), line: at }, where: `${file}:${at}` };
}

function frameRow(frame: Obj, cwd: string | undefined): DebugFrameRow | undefined {
  const id = int(frame.id);
  if (id === undefined) {
    return undefined;
  }
  const found = place(frame.source, frame.line, cwd);
  return {
    id,
    name: str(frame.name) ?? text.unnamedFrame,
    location: found?.at,
    where: found?.where ?? str(frame.instructionPointerReference) ?? "",
  };
}

function variableRow(variable: Obj): DebugVariableRow | undefined {
  const name = str(variable.name);
  return name === undefined
    ? undefined
    : { name, value: typeof variable.value === "string" ? variable.value : "", type: str(variable.type) };
}

function sourceBreakpoints(file: string, records: Obj[], cwd: string | undefined): DebugBreakpointRow[] {
  const path = resolvePath(cwd, file);
  return records.flatMap((record) => {
    const line = int(record.line);
    return line === undefined
      ? []
      : [
          {
            label: `${file}:${line}`,
            location: { path, line },
            verified: record.verified === true,
            condition: str(record.condition),
            message: str(record.message),
          },
        ];
  });
}

function functionBreakpoints(records: Obj[]): DebugBreakpointRow[] {
  return records.flatMap((record) => {
    const name = str(record.name);
    return name === undefined
      ? []
      : [
          {
            label: name,
            location: undefined,
            verified: record.verified === true,
            condition: str(record.condition),
            message: str(record.message),
          },
        ];
  });
}

/** The tool call arguments of the transcript's assistant messages, by tool call id. */
function callArguments(transcript: ThreadState["transcript"]): Map<string, Obj> {
  const byId = new Map<string, Obj>();
  for (const { message } of transcript) {
    if (message.role !== "assistant" || !Array.isArray(message.content)) {
      continue;
    }
    for (const block of message.content as unknown[]) {
      if (isObj(block) && block.type === "toolCall" && typeof block.id === "string" && isObj(block.arguments)) {
        byId.set(block.id, block.arguments);
      }
    }
  }
  return byId;
}

interface Session {
  id: string;
  snapshot: Obj;
  stack: DebugFrameRow[];
  variables: DebugVariableRow[];
  /** Source breakpoints by their file's path resolved against the session's folder, whatever the agent's spelling. */
  files: Map<string, DebugBreakpointRow[]>;
  functions: DebugBreakpointRow[];
}

/** Applies one successful debug call to the session it reports (a new session id starts afresh). */
function apply(session: Session | undefined, { args, details }: DebugCall): Session | undefined {
  const snapshot = details.snapshot;
  const id = isObj(snapshot) ? str(snapshot.id) : undefined;
  if (!isObj(snapshot) || id === undefined) {
    return session;
  }
  const next: Session =
    session?.id === id
      ? { ...session, snapshot }
      : { id, snapshot, stack: [], variables: [], files: new Map(), functions: [] };
  const action = str(details.action) ?? "";
  if (EXECUTION_ACTIONS[action] === true) {
    next.stack = [];
    next.variables = [];
  }
  const cwd = str(snapshot.cwd);
  if (Array.isArray(details.stackFrames)) {
    next.stack = objects(details.stackFrames).flatMap((frame) => frameRow(frame, cwd) ?? []);
  }
  if (Array.isArray(details.variables)) {
    next.variables = objects(details.variables).flatMap((variable) => variableRow(variable) ?? []);
  }
  const file = str(args.file);
  if (Array.isArray(details.breakpoints) && file !== undefined) {
    next.files = new Map(next.files);
    const key = resolvePath(cwd, file);
    const rows = sourceBreakpoints(file, objects(details.breakpoints), cwd);
    if (rows.length === 0) {
      next.files.delete(key);
    } else {
      next.files.set(key, rows);
    }
  }
  if (Array.isArray(details.functionBreakpoints)) {
    next.functions = functionBreakpoints(objects(details.functionBreakpoints));
  }
  return next;
}

/** The thread's latest debug session from its transcript; `undefined` when the agent never started one. */
export function debugView(state: Pick<ThreadState, "transcript">): DebugView | undefined {
  const argsById = callArguments(state.transcript);
  let session: Session | undefined;
  for (const { message } of state.transcript) {
    const call = debugCall(message as Obj, argsById);
    if (call !== undefined) {
      session = apply(session, call);
    }
  }
  if (session === undefined) {
    return undefined;
  }
  const { snapshot } = session;
  const stop = place(snapshot.source, snapshot.line, str(snapshot.cwd));
  const status = str(snapshot.status) ?? "running";
  return {
    adapter: str(snapshot.adapter) ?? "",
    program: str(snapshot.program),
    status,
    statusLabel: text.status[status] ?? status,
    stop:
      status === "stopped"
        ? {
            reason: str(snapshot.stopReason),
            frame: str(snapshot.frameName),
            location: stop?.at,
            where: stop?.where ?? str(snapshot.frameName) ?? "",
          }
        : undefined,
    exitCode: int(snapshot.exitCode),
    stack: session.stack,
    variables: session.variables,
    breakpoints: [...[...session.files.values()].flat(), ...session.functions],
  };
}
