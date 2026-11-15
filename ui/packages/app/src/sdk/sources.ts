/**
 * Widget data sources (docs/specs/08-agent-control.md): state the daemon keeps per widget instance ({@link useStored},
 * AGT-018), daemon jobs ({@link useJob}, AGT-019) and the agent's tool runs ({@link useToolRun}, AGT-020).
 */

import type { ToolRun, ToolStatus } from "@omfp/core";
import { type JobState, jobTopic, type TerminalScreen, type TerminalState, terminalTopic } from "@omfp/protocol";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { strings } from "../strings/index.ts";
import { resultText } from "../vm/tool.ts";
import { reportError } from "./actions.ts";
import { useSnapshot, useThread, useTopic, useViews, useWindowState } from "./hooks.ts";
import { useHost } from "./host.tsx";
import { SELECTED_THREAD } from "./keys.ts";
import { JOB_TAIL, useSlotReport } from "./reports.ts";
import { resultKey } from "./results.ts";
import { useWidget } from "./widget.ts";

/** A new value, or an updater of the current one; `null` deletes the key. */
export type StoredUpdate<T> = T | null | ((current: T) => T | null);

/**
 * `[value, set, loaded]`: the JSON value this widget instance stored under `key` in its workspace (DMN-086), else
 * `initial`; the same in every window and pushed live. `set` stores a value or the result of an updater
 * (`widgets.state_set`), `null` deletes the key; failures go to the frame's error list. `loaded` is `false` until the
 * workspace's views state arrived, while `value` is only `initial`. The stored JSON is not checked against `T`.
 */
export function useStored<T>(key: string, initial: T): [T, (update: StoredUpdate<T>) => void, boolean] {
  const host = useHost();
  const { instance, workspaceId } = useWidget();
  const views = useViews(workspaceId);
  const keys = views?.stored[instance];
  // JSON from the daemon, trusted to be what this widget stored.
  const value = keys !== undefined && Object.hasOwn(keys, key) ? (keys[key] as T) : initial;
  // The value updaters start from: the pushed value, or the one set last while its push is on the way.
  const latest = useRef(value);
  useEffect(() => {
    latest.current = value;
  }, [value]);
  const set = useCallback(
    (update: StoredUpdate<T>) => {
      if (workspaceId === undefined) {
        reportError(host, strings.widgets.noWorkspace);
        return;
      }
      const next = typeof update === "function" ? (update as (current: T) => T | null)(latest.current) : update;
      latest.current = next ?? initial;
      host.client
        .call("widgets.state_set", { workspaceId, instance, key, value: next })
        .catch((error: unknown) => reportError(host, error));
    },
    [host, workspaceId, instance, key, initial],
  );
  return [value, set, views !== undefined];
}

/** Where a {@link Job} stands: `pending` while no job is known yet or a start or restart is on its way, `succeeded`
 * after exit code 0, `failed` after another exit code or a signal. */
export type JobRunStatus = "pending" | "running" | "succeeded" | "failed";

/** A daemon job of this widget instance (AGT-019). */
export interface Job {
  readonly status: JobRunStatus;
  /** Output lines (stdout and stderr interleaved as they came), oldest first; empty until the job is known. */
  readonly lines: readonly string[];
  /** The standard output's lines. */
  readonly stdout: readonly string[];
  /** The standard error's lines. */
  readonly stderr: readonly string[];
  /** The exit code once the job exited normally, else `null`. */
  readonly exitCode: number | null;
  /** The signal that ended the job, else `null`. */
  readonly signal: number | null;
  /** How many jobs this instance started for the command, or under the key (a restart or a new command adds one);
   * 0 before the first. */
  readonly runs: number;
  /** When the current job started (ms since the epoch), `null` until it is known. */
  readonly startedAt: number | null;
  /** When the current job ended, `null` while it runs or is not known. */
  readonly endedAt: number | null;
  /** The job as the daemon reports it (`job/<id>`), `undefined` until known. */
  readonly state: JobState | undefined;
  /** The job's topic is being followed again (the widget mounted again, or its view was shown again): `state` and the
   * lines are the window's last copy until its snapshot arrives (UI-392). */
  readonly refreshing: boolean;
  /** Kills the job when it runs, then starts a new one. */
  restart(): void;
  /** Stops the running job (SIGTERM, then SIGKILL after 2 s, DMN-081). */
  kill(): void;
}

/** Options of {@link useJob}. */
export interface JobOptions {
  /** The job slot: every command run under one key replaces the previous one's job (killed if it still runs, then
   * forgotten, DMN-215), so a widget running a command per query keeps one job. Default: the command itself. */
  readonly key?: string;
}

/** What {@link useJob} stores under `job:<command>` (no key) or `jobkey:<key>`; `command` only under a key. */
interface StoredJob {
  readonly id: string;
  readonly runs: number;
  readonly command: string | undefined;
}

/** The stored value as a {@link StoredJob}; anything else is no job. */
function storedJob(value: unknown): StoredJob | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const { id, runs, command } = value as { id?: unknown; runs?: unknown; command?: unknown };
  return typeof id === "string"
    ? { id, runs: typeof runs === "number" ? runs : 1, command: typeof command === "string" ? command : undefined }
    : null;
}

const NO_LINES: readonly string[] = [];

function statusOf(state: JobState | undefined): JobRunStatus {
  const status = state?.job.status;
  if (status === undefined) {
    return "pending";
  }
  if (status.state === "running") {
    return "running";
  }
  return status.code === 0 ? "succeeded" : "failed";
}

/** What {@link useJob} keeps in the window's results ({@link OmfpHost.results}) for its next mount. */
interface KeptJob {
  readonly key: string;
  readonly id: string;
  readonly state: JobState;
}

/**
 * Runs `command` in the workspace (`jobs.start`, DMN-080) once per widget instance: the job's id and the runs are
 * stored with {@link useStored} under `job:<command>` as `{ id, runs }`; while nothing is stored or the daemon no
 * longer knows the stored job, a job is started; otherwise the stored job is followed (`job/<id>`), running or
 * finished, so reloads, other windows and moving the widget never start a second one. The job is part of the widget
 * slot's report (AGT-026). Until the followed job's snapshot arrives, the window's last copy of it shows,
 * `refreshing` (UI-392).
 *
 * With `options.key` (AGT-086) the slot is the key, stored under `jobkey:<key>` as `{ id, runs, command }`: when the
 * command changes, the hook starts the new one with `replaces` set to the stored job, which the daemon kills if it
 * still runs and forgets (DMN-215); until the new job is known it is `pending` with no lines.
 */
export function useJob(command: string, options: JobOptions = {}): Job {
  const { key } = options;
  const host = useHost();
  const widget = useWidget();
  const { workspaceId } = widget;
  const storeKey = key === undefined ? `job:${command}` : `jobkey:${key}`;
  const [value, setStored, loaded] = useStored<unknown>(storeKey, null);
  const stored = storedJob(value);
  const jobId = stored?.id ?? null;
  const runs = stored?.runs ?? 0;
  // Under a key, a stored job of another command is only replaced, never shown.
  const stale = key !== undefined && stored !== null && stored.command !== command;
  const shownId = stale ? null : jobId;
  const topic = useTopic(shownId === null ? undefined : jobTopic(shownId));
  const live = useSnapshot((snapshot) => (shownId === null ? undefined : snapshot.jobs.get(shownId)));
  const gone = topic?.sync.kind === "failed" && topic.sync.error.code === "not_found";
  // The job as last shown, kept in the window's results when the widget unmounts or its view hides.
  const resultsKey = resultKey(widget, "job", storeKey);
  const kept = useRef<KeptJob | undefined>(undefined);
  if (live !== undefined && shownId !== null) {
    kept.current = { key: resultsKey, id: shownId, state: live };
  }
  useEffect(
    () => () => {
      const last = kept.current;
      if (last !== undefined) {
        // Its lines' text, plus a little per line.
        const size = last.state.lines.reduce((total, line) => total + line.text.length + 16, 256);
        host.results.set(last.key, last, size);
      }
    },
    [host],
  );
  const cached =
    live !== undefined || shownId === null || gone
      ? undefined
      : (host.results.get(resultsKey)?.value as KeptJob | undefined);
  const state = live ?? (cached?.id === shownId ? cached.state : undefined);
  // The stored id (or `null`) a start was made for: no second start until the stored id changes.
  const startedFor = useRef<string | null | undefined>(undefined);
  // The same, to render `pending` while the start is on its way.
  const [starting, setStarting] = useState<string | null | undefined>(undefined);

  const start = useCallback(
    (replacing: string | null, runsBefore: number) => {
      if (workspaceId === undefined) {
        return;
      }
      startedFor.current = replacing;
      setStarting(replacing);
      const params =
        key === undefined || replacing === null
          ? { workspaceId, command }
          : { workspaceId, command, replaces: replacing };
      host.client
        .call("jobs.start", params)
        .then((job) =>
          setStored(
            key === undefined ? { id: job.id, runs: runsBefore + 1 } : { id: job.id, runs: runsBefore + 1, command },
          ),
        )
        .catch((error: unknown) => {
          setStarting(undefined);
          reportError(host, error);
        });
    },
    [host, workspaceId, command, key, setStored],
  );

  useEffect(() => {
    if (loaded && (jobId === null || gone || stale) && startedFor.current !== jobId) {
      start(jobId, runs);
    }
  }, [loaded, jobId, gone, stale, runs, start]);

  const running = state?.job.status.state === "running";
  const restart = useCallback(() => {
    // Under a key the daemon kills the replaced job itself.
    if (jobId !== null && running && key === undefined) {
      setStarting(jobId);
      host.client
        .call("jobs.kill", { jobId })
        .then(() => start(jobId, runs))
        .catch((error: unknown) => {
          setStarting(undefined);
          reportError(host, error);
        });
    } else {
      start(jobId, runs);
    }
  }, [host, jobId, running, key, runs, start]);
  const kill = useCallback(() => {
    if (jobId !== null) {
      host.client.call("jobs.kill", { jobId }).catch((error: unknown) => reportError(host, error));
    }
  }, [host, jobId]);

  const jobLines = state?.lines;
  const output = useMemo(() => {
    if (jobLines === undefined) {
      return { lines: NO_LINES, stdout: NO_LINES, stderr: NO_LINES };
    }
    return {
      lines: jobLines.map((line) => line.text),
      stdout: jobLines.filter((line) => line.stream === "stdout").map((line) => line.text),
      stderr: jobLines.filter((line) => line.stream === "stderr").map((line) => line.text),
    };
  }, [jobLines]);
  const status = state?.job.status;
  const exited = status?.state === "exited" ? status : undefined;
  const job = {
    status: starting !== undefined && starting === jobId ? "pending" : statusOf(state),
    exitCode: exited?.code ?? null,
    signal: exited?.signal ?? null,
    runs,
    startedAt: state?.job.startedAt ?? null,
    endedAt: exited?.endedAt ?? null,
  } as const;

  const slot = useSlotReport();
  const tail = useMemo(() => output.lines.slice(-JOB_TAIL), [output.lines]);
  useEffect(() => {
    slot?.setJob(storeKey, {
      command,
      status: job.status,
      exitCode: job.exitCode,
      signal: job.signal,
      runs: job.runs,
      startedAt: job.startedAt,
      tail,
    });
  }, [slot, storeKey, command, job.status, job.exitCode, job.signal, job.runs, job.startedAt, tail]);
  useEffect(() => () => slot?.setJob(storeKey, undefined), [slot, storeKey]);

  return { ...job, ...output, state, refreshing: live === undefined && state !== undefined, restart, kill };
}

/** Options of {@link useTerminal}. */
export interface TerminalOptions {
  /** Workspace-relative working directory. */
  readonly cwd?: string;
  /** Initial terminal columns and rows. */
  readonly cols?: number;
  readonly rows?: number;
  /** Stored session slot; changing the command replaces the previous session. */
  readonly key?: string;
}

/** Options for an explicit agent-readable terminal screen (TRM-106). */
export interface TerminalScreenOptions {
  /** Return immediately on a different revision; otherwise wait for a change. */
  readonly afterRevision?: number;
  /** Long-poll duration in milliseconds, at most 30 seconds. */
  readonly waitMs?: number;
}

/** A daemon-owned interactive PTY session (TRM-100..106). */
export interface Terminal {
  readonly id: string | undefined;
  readonly state: TerminalState | undefined;
  readonly status: "pending" | "running" | "exited";
  readonly refreshing: boolean;
  readonly runs: number;
  input(data: string | Uint8Array): void;
  resize(cols: number, rows: number): void;
  screen(options?: TerminalScreenOptions): Promise<TerminalScreen | undefined>;
  kill(): void;
}

interface StoredTerminal {
  readonly id: string;
  readonly runs: number;
  readonly command?: string;
}

function storedTerminal(value: unknown): StoredTerminal | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const { id, runs, command } = value as { id?: unknown; runs?: unknown; command?: unknown };
  if (typeof id !== "string") {
    return null;
  }
  return typeof command === "string"
    ? { id, runs: typeof runs === "number" ? runs : 1, command }
    : { id, runs: typeof runs === "number" ? runs : 1 };
}

function base64(data: string | Uint8Array): string {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

/**
 * Starts or follows a daemon-owned PTY session. The id is stored in the widget instance's workspace state, so remounts
 * and other windows follow the same session instead of starting a new process. Input is base64-encoded at the SDK
 * boundary and never appears in the agent activity log.
 */
export function useTerminal(command: string, options: TerminalOptions = {}): Terminal {
  const { cwd, cols = 80, rows = 24, key } = options;
  const host = useHost();
  const { workspaceId } = useWidget();
  const storeKey = key === undefined ? `terminal:${command}` : `terminalkey:${key}`;
  const [value, setStored, loaded] = useStored<unknown>(storeKey, null);
  const stored = storedTerminal(value);
  const stale = key !== undefined && stored !== null && stored.command !== command;
  const terminalId = stale ? null : (stored?.id ?? null);
  const topic = useTopic(terminalId === null ? undefined : terminalTopic(terminalId));
  const state = useSnapshot((snapshot) => (terminalId === null ? undefined : snapshot.terminals.get(terminalId)));
  const gone = topic?.sync.kind === "failed" && topic.sync.error.code === "not_found";
  const [starting, setStarting] = useState(false);
  const startFor = `${terminalId ?? ""}\u0000${command}`;
  const started = useRef<string | undefined>(undefined);

  const start = useCallback(() => {
    if (workspaceId === undefined || starting) {
      return;
    }
    started.current = startFor;
    setStarting(true);
    const oldId = stale ? stored?.id : undefined;
    const params = {
      workspaceId,
      command,
      ...(cwd === undefined ? {} : { cwd }),
      cols,
      rows,
    };
    const request =
      oldId === undefined
        ? host.client.call("terminals.start", params)
        : host.client
            .call("terminals.kill", { terminalId: oldId })
            .then(() => host.client.call("terminals.start", params));
    request
      .then((terminal) => {
        setStored({ id: terminal.id, runs: (stored?.runs ?? 0) + 1, command });
      })
      .catch((error: unknown) => reportError(host, error))
      .finally(() => setStarting(false));
  }, [cols, command, cwd, host, rows, setStored, stale, startFor, starting, stored?.id, stored?.runs, workspaceId]);

  useEffect(() => {
    if (loaded && (terminalId === null || gone || stale) && started.current !== startFor) {
      start();
    }
  }, [loaded, gone, stale, start, startFor, terminalId]);

  const input = useCallback(
    (data: string | Uint8Array) => {
      if (terminalId !== null) {
        host.client
          .call("terminals.input", { terminalId, data: base64(data) })
          .catch((error: unknown) => reportError(host, error));
      }
    },
    [host, terminalId],
  );
  const resize = useCallback(
    (nextCols: number, nextRows: number) => {
      if (terminalId !== null) {
        host.client
          .call("terminals.resize", { terminalId, cols: nextCols, rows: nextRows })
          .catch((error: unknown) => reportError(host, error));
      }
    },
    [host, terminalId],
  );
  const screen = useCallback(
    (options: TerminalScreenOptions = {}) => {
      if (terminalId === null) {
        return Promise.resolve(undefined);
      }
      const { afterRevision, waitMs } = options;
      return host.client.call("terminals.screen", {
        terminalId,
        ...(afterRevision === undefined ? {} : { afterRevision }),
        ...(waitMs === undefined ? {} : { waitMs }),
      });
    },
    [host, terminalId],
  );
  const kill = useCallback(() => {
    if (terminalId !== null) {
      host.client.call("terminals.kill", { terminalId }).catch((error: unknown) => reportError(host, error));
    }
  }, [host, terminalId]);

  return {
    id: terminalId ?? undefined,
    state,
    status: starting ? "pending" : (state?.terminal.status.state ?? "pending"),
    refreshing: state === undefined && terminalId !== null && !gone,
    runs: stored?.runs ?? 0,
    input,
    resize,
    screen,
    kill,
  };
}

export type ToolRunTarget =
  | string
  | {
      /** Matches the run's command (`arguments.command` when it is a string, else its JSON arguments); a string
       * matches when the command contains it. */
      readonly match: RegExp | string;
      /** Default: the thread this window shows. */
      readonly threadId?: string;
    };

/** One tool run of a thread (AGT-020). */
export interface ToolRunInfo {
  readonly toolCallId: string;
  readonly toolName: string;
  readonly status: ToolStatus;
  /** The latest partial output while it runs (omp streams the bash output's latest tail), the result's text once it
   * ended. */
  readonly output: string;
  /** {@link ToolRunInfo.output} split into lines (a final line break adds no empty line). */
  readonly lines: readonly string[];
  readonly isError: boolean;
}

function commandOf(run: ToolRun): string {
  const { args } = run;
  if (typeof args === "object" && args !== null && !Array.isArray(args) && typeof args.command === "string") {
    return args.command;
  }
  return JSON.stringify(args);
}

function matches(run: ToolRun, match: RegExp | string): boolean {
  const command = commandOf(run);
  if (typeof match === "string") {
    return command.includes(match);
  }
  match.lastIndex = 0;
  return match.test(command);
}

/** The tool run `toolCallId`, or the latest whose command matches, of a thread (the selected one by default),
 * following `thread/<id>`; `undefined` while there is none. */
export function useToolRun(target: ToolRunTarget): ToolRunInfo | undefined {
  const [selected] = useWindowState(SELECTED_THREAD);
  const threadId = typeof target === "string" ? selected : (target.threadId ?? selected);
  const tools = useThread(threadId)?.state.tools;
  // Runs are immutable reducer objects: the result below changes only when the run does.
  const run =
    typeof target === "string"
      ? tools?.find((candidate) => candidate.tool_call_id === target)
      : tools?.findLast((candidate) => matches(candidate, target.match));
  return useMemo(() => {
    if (run === undefined) {
      return undefined;
    }
    const ended = run.status === "succeeded" || run.status === "failed";
    const value = ended ? run.result : run.partial_result;
    const output = value === null ? "" : resultText(value);
    return {
      toolCallId: run.tool_call_id,
      toolName: run.tool_name,
      status: run.status,
      output,
      // A final line break ends the last line, it does not start an empty one.
      lines: output === "" ? [] : output.replace(/\n$/, "").split("\n"),
      isError: run.status === "failed",
    };
  }, [run]);
}
