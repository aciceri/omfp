/**
 * Validation of what the daemon sends: server frames, the `system.hello` and `subscribe` results, and topic snapshots.
 * A value that passes has the shape of its TypeScript type down to the fields the client and the reducer branch on;
 * nested omp wire objects are left to the reducer, which accepts any JSON.
 */

import { isObject, type ThreadEvent, type ThreadState } from "@omfp/core";
import {
  type ErrorBody,
  type ErrorCode,
  type HelloResult,
  type IndexEvent,
  type IndexSnapshot,
  type JobEvent,
  type JobInfo,
  type JobState,
  type JobsEvent,
  type JobsState,
  type JobView,
  type LoginActivity,
  type ParsedTopic,
  parseTopic,
  type RepoStatus,
  type RunInfo,
  type ServerFrame,
  type Settings,
  type SubscribeResult,
  type TerminalEvent,
  type TerminalInfo,
  type TerminalState,
  type Theme,
  type TopicEvent,
  type TransientEvent,
  type ViewsEvent,
  type ViewsState,
  type WidgetInfo,
  type WindowRequest,
  type WorkspaceBinding,
  type WorkspaceEvent,
} from "@omfp/protocol";

/** A decoded value, or why it does not decode. */
export type Decoded<T> = { ok: true; value: T } | { ok: false; error: string };

function invalid<T>(error: string): Decoded<T> {
  return { ok: false, error };
}

function valid<T>(value: T): Decoded<T> {
  return { ok: true, value };
}

const ERROR_CODES = {
  unauthorized: true,
  bad_request: true,
  not_found: true,
  conflict: true,
  already_resolved: true,
  limit_reached: true,
  version_mismatch: true,
  thread_failed: true,
  omp_error: true,
  git_error: true,
  internal: true,
} satisfies Record<ErrorCode, true>;

function isErrorCode(value: unknown): value is ErrorCode {
  return typeof value === "string" && Object.hasOwn(ERROR_CODES, value);
}

function isSeq(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isPositiveU16(value: unknown): value is number {
  return isSeq(value) && value > 0 && value <= 65_535;
}

function isNullish(value: unknown): value is null | undefined {
  return value === undefined || value === null;
}

function isOptionalString(value: unknown): value is string | null | undefined {
  return isNullish(value) || typeof value === "string";
}

function isOptionalNumber(value: unknown): boolean {
  return isNullish(value) || typeof value === "number";
}

function isNullableObject(value: unknown): boolean {
  return value === null || isObject(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

const BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;

function isBase64(value: unknown): value is string {
  return typeof value === "string" && BASE64.test(value);
}

function hasId(value: unknown): boolean {
  return isObject(value) && typeof value.id === "string";
}

function decodeErrorBody(value: unknown): Decoded<ErrorBody> {
  if (!isObject(value) || !isErrorCode(value.code) || typeof value.message !== "string") {
    return invalid("`error` needs a known `code` and a string `message`");
  }
  if (!isOptionalString(value.ompCode)) {
    return invalid("`error.ompCode` must be a string");
  }
  const body: ErrorBody = { code: value.code, message: value.message };
  if (typeof value.ompCode === "string") {
    body.ompCode = value.ompCode;
  }
  if (value.details !== undefined) {
    body.details = value.details;
  }
  return valid(body);
}

/** The running login of the `index` topic (DMN-290): `null` (none), or its provider with the optional parts of its
 * flow. */
function isLoginActivity(value: unknown): boolean {
  return (
    isNullish(value) ||
    (isObject(value) &&
      typeof value.providerId === "string" &&
      typeof value.name === "string" &&
      [value.url, value.instructions, value.progress].every(isOptionalString))
  );
}

/** {@link isLoginActivity}'s value with its missing optional fields `null`. */
function loginActivity(value: unknown): LoginActivity | null {
  if (!isObject(value)) {
    return null;
  }
  const login = value as unknown as LoginActivity;
  return {
    providerId: login.providerId,
    name: login.name,
    url: login.url ?? null,
    instructions: login.instructions ?? null,
    progress: login.progress ?? null,
  };
}

const INDEX_EVENTS: Record<IndexEvent["kind"], (event: Record<string, unknown>) => boolean> = {
  workspace_upserted: (event) => hasId(event.workspace),
  workspace_removed: (event) => typeof event.workspaceId === "string",
  thread_upserted: (event) => hasId(event.thread),
  thread_removed: (event) => typeof event.threadId === "string",
  theme_changed: (event) => isTheme(event.theme),
  settings_changed: (event) => isSettings(event.settings),
  init_changed: (event) => isOptionalString(event.init),
  models_changed: (event) => isSeq(event.models),
  login_changed: (event) => isLoginActivity(event.login),
};

/** The theme when the daemon sends none (DMN-075): the built-in themes only. */
export const NO_THEME: Theme = { css: null };

/** The settings when the daemon sends none (DMN-009): every default, no binding over the default keymap. */
export const NO_SETTINGS: Settings = { ui: {}, keymap: [], error: null };

function isTheme(value: unknown): value is Theme {
  return isObject(value) && (value.css === null || typeof value.css === "string");
}

function isKeymapEntry(value: unknown): boolean {
  return (
    isObject(value) &&
    typeof value.mode === "string" &&
    typeof value.keys === "string" &&
    typeof value.command === "string"
  );
}

/** A `views/<id>` keymap (PRT-060): `[keymap]`-shaped entries with optional string `args`. */
function isWorkspaceKeymap(value: unknown): value is WorkspaceBinding[] {
  return (
    Array.isArray(value) &&
    value.every(
      (entry) =>
        isKeymapEntry(entry) &&
        (entry.args === undefined ||
          (Array.isArray(entry.args) && entry.args.every((arg: unknown) => typeof arg === "string"))),
    )
  );
}

function isSettings(value: unknown): value is Settings {
  return (
    isObject(value) &&
    isObject(value.ui) &&
    Array.isArray(value.keymap) &&
    value.keymap.every(isKeymapEntry) &&
    (value.error === null || typeof value.error === "string")
  );
}

const THREAD_EVENTS: Record<ThreadEvent["kind"], (event: Record<string, unknown>) => boolean> = {
  omp: (event) => isObject(event.frame),
  history: (event) => isObject(event.session) && Array.isArray(event.messages),
  process: (event) => isObject(event.state) && typeof event.state.state === "string",
  interaction_resolved: (event) =>
    typeof event.id === "string" && isObject(event.resolution) && typeof event.resolution.by === "string",
  live_session: (event) => event.owner === undefined || isNullableObject(event.owner),
  open_file: (event) =>
    typeof event.id === "string" && typeof event.connection === "number" && typeof event.path === "string",
  notify: (event) =>
    typeof event.id === "string" &&
    typeof event.connection === "number" &&
    typeof event.title === "string" &&
    typeof event.body === "string",
  unknown: (event) => "raw" in event,
};

function isRepoStatusList(value: unknown): value is RepoStatus[] {
  return (
    Array.isArray(value) &&
    value.every(
      (repo) =>
        isObject(repo) && typeof repo.name === "string" && isObject(repo.status) && isOptionalString(repo.error),
    )
  );
}

const WORKSPACE_EVENTS: Record<WorkspaceEvent["kind"], (event: Record<string, unknown>) => boolean> = {
  status: (event) => isRepoStatusList(event.repos),
};

function checkKind(table: Record<string, (event: Record<string, unknown>) => boolean>, event: unknown): boolean {
  if (!isObject(event) || typeof event.kind !== "string" || !Object.hasOwn(table, event.kind)) {
    return false;
  }
  return table[event.kind]?.(event) === true;
}

function isWidgetInfo(value: unknown): value is WidgetInfo {
  return (
    isObject(value) &&
    typeof value.name === "string" &&
    value.source === "workspace" &&
    typeof value.version === "string" &&
    isOptionalString(value.entry)
  );
}

function isWidgetList(value: unknown): value is WidgetInfo[] {
  return Array.isArray(value) && value.every(isWidgetInfo);
}

const VIEWS_EVENTS: Record<ViewsEvent["kind"], (event: Record<string, unknown>) => boolean> = {
  views_manifest: (event) => isOptionalString(event.error),
  views_widgets: (event) => isWidgetList(event.widgets),
  views_keymap: (event) => isWorkspaceKeymap(event.keymap) && isOptionalString(event.error),
  views_init: (event) => isOptionalString(event.init),
  widget_stored: (event) => typeof event.instance === "string" && typeof event.key === "string" && "value" in event,
};

function isJobStream(value: unknown): boolean {
  return value === "stdout" || value === "stderr";
}

const JOB_EVENTS: Record<JobEvent["kind"], (event: Record<string, unknown>) => boolean> = {
  job_output: (event) => isJobStream(event.stream) && typeof event.text === "string",
  job_exited: (event) =>
    typeof event.endedAt === "number" && isOptionalNumber(event.code) && isOptionalNumber(event.signal),
};

const TERMINAL_EVENTS: Record<TerminalEvent["kind"], (event: Record<string, unknown>) => boolean> = {
  output: (event) => isBase64(event.data),
  exited: (event) => typeof event.endedAt === "number" && isOptionalNumber(event.code),
};

const JOBS_EVENTS: Record<JobsEvent["kind"], (event: Record<string, unknown>) => boolean> = {
  jobs_set: (event) => isJobViewList(event.jobs) && isOptionalString(event.error),
  job_updated: (event) => isJobView(event.job),
  run_started: (event) => isRunInfo(event.run),
  run_ended: (event) => isRunInfo(event.run),
};

const EVENT_TABLES: Record<ParsedTopic["kind"], Record<string, (event: Record<string, unknown>) => boolean>> = {
  index: INDEX_EVENTS,
  thread: THREAD_EVENTS,
  workspace: WORKSPACE_EVENTS,
  views: VIEWS_EVENTS,
  job: JOB_EVENTS,
  jobs: JOBS_EVENTS,
  terminal: TERMINAL_EVENTS,
};

/** Decodes the `event` of a frame on `topic`: its kind must belong to the topic's kind. */
function decodeTopicEvent(topic: ParsedTopic, event: unknown): Decoded<TopicEvent> {
  const table = EVENT_TABLES[topic.kind];
  // Checked against the topic's event table.
  return checkKind(table, event) ? valid(event as TopicEvent) : invalid(`not a ${topic.kind} event`);
}

/** A level in [0, 1]. */
function isLevel(value: unknown): value is number {
  return typeof value === "number" && value >= 0 && value <= 1;
}

/** Decodes the `event` of a transient frame (PRT-054): `live_levels` only on a thread topic (DMN-110). */
function decodeTransientEvent(topic: ParsedTopic, event: unknown): Decoded<TransientEvent> {
  if (!isObject(event) || event.kind !== "live_levels" || topic.kind !== "thread") {
    return invalid(`not a transient event of ${topic.kind}`);
  }
  const { input, output } = event;
  if (!isLevel(input) || !isLevel(output)) {
    return invalid("`live_levels` needs `input` and `output` in [0, 1]");
  }
  return valid({ kind: "live_levels", input, output });
}

/** Decodes the `request` of a window request frame (PRT-124, PRT-131, DMN-375): `run_command`, `check_widget`, a
 * login's `login_url`, `login_progress` and `login_input`, or an agent's `notify`. */
function decodeWindowRequest(request: unknown): Decoded<WindowRequest> {
  if (!isObject(request)) {
    return invalid("a window request needs a `request` object");
  }
  switch (request.kind) {
    case "run_command": {
      const { command, args } = request;
      return typeof command === "string" && isStringArray(args)
        ? valid({ kind: "run_command", command, args })
        : invalid("`run_command` needs a string `command` and string `args`");
    }
    case "check_widget": {
      const { reference, version, timeoutMs } = request;
      return typeof reference === "string" && isOptionalString(version) && isSeq(timeoutMs)
        ? valid({ kind: "check_widget", reference, version: version ?? null, timeoutMs })
        : invalid("`check_widget` needs a string `reference`, a `version` and a `timeoutMs`");
    }
    case "login_url": {
      const { providerId, name, url, launchUrl, instructions } = request;
      return typeof providerId === "string" &&
        typeof name === "string" &&
        typeof url === "string" &&
        isOptionalString(launchUrl) &&
        isOptionalString(instructions)
        ? valid({
            kind: "login_url",
            providerId,
            name,
            url,
            launchUrl: launchUrl ?? null,
            instructions: instructions ?? null,
          })
        : invalid("`login_url` needs a string `providerId`, `name` and `url`");
    }
    case "login_progress": {
      const { providerId, name, message } = request;
      return typeof providerId === "string" && typeof name === "string" && typeof message === "string"
        ? valid({ kind: "login_progress", providerId, name, message })
        : invalid("`login_progress` needs a string `providerId`, `name` and `message`");
    }
    case "login_input": {
      const { providerId, name, message, placeholder } = request;
      return typeof providerId === "string" &&
        typeof name === "string" &&
        typeof message === "string" &&
        isOptionalString(placeholder)
        ? valid({ kind: "login_input", providerId, name, message, placeholder: placeholder ?? null })
        : invalid("`login_input` needs a string `providerId`, `name` and `message`");
    }
    case "notify": {
      const { threadId, title, body } = request;
      return typeof threadId === "string" && typeof title === "string" && typeof body === "string"
        ? valid({ kind: "notify", threadId, title, body })
        : invalid("`notify` needs a string `threadId`, `title` and `body`");
    }
    default:
      return invalid("unknown window request `kind`");
  }
}

/** Decodes a JSON-parsed daemon frame (PRT-010, PRT-022, PRT-023, PRT-054, PRT-124). */
export function decodeServerFrame(value: unknown): Decoded<ServerFrame> {
  if (!isObject(value)) {
    return invalid("a frame must be a JSON object");
  }
  switch (value.type) {
    case "response": {
      const { id, ok } = value;
      if (typeof id !== "string" || typeof ok !== "boolean") {
        return invalid("a response needs a string `id` and a boolean `ok`");
      }
      if (ok) {
        if (value.error !== undefined) {
          return invalid("a successful response has no `error`");
        }
        return valid({ type: "response", id, ok, result: value.result ?? null });
      }
      if (value.error === undefined || value.result !== undefined) {
        return invalid("a failed response needs an `error` and no `result`");
      }
      const error = decodeErrorBody(value.error);
      return error.ok ? valid({ type: "response", id, ok, error: error.value }) : error;
    }
    case "event": {
      const { topic, epoch, seq } = value;
      const parsed = typeof topic === "string" ? parseTopic(topic) : undefined;
      if (typeof topic !== "string" || parsed === undefined || typeof epoch !== "string" || !isSeq(seq)) {
        return invalid("an event needs a valid `topic`, a string `epoch` and a non-negative integer `seq`");
      }
      const event = decodeTopicEvent(parsed, value.event);
      if (!event.ok) {
        return invalid(`invalid event for topic ${topic}: ${event.error}`);
      }
      return valid({ type: "event", topic, epoch, seq, event: event.value });
    }
    case "resync_required": {
      const { topic } = value;
      if (typeof topic !== "string" || parseTopic(topic) === undefined) {
        return invalid("`resync_required` needs a valid `topic`");
      }
      return valid({ type: "resync_required", topic });
    }
    case "transient": {
      const { topic } = value;
      const parsed = typeof topic === "string" ? parseTopic(topic) : undefined;
      if (typeof topic !== "string" || parsed === undefined) {
        return invalid("a transient frame needs a valid `topic`");
      }
      const event = decodeTransientEvent(parsed, value.event);
      return event.ok ? valid({ type: "transient", topic, event: event.value }) : event;
    }
    case "window_request": {
      const { id } = value;
      if (typeof id !== "string") {
        return invalid("a window request needs a string `id`");
      }
      const request = decodeWindowRequest(value.request);
      return request.ok ? valid({ type: "window_request", id, request: request.value }) : request;
    }
    default:
      return invalid("unknown frame `type`");
  }
}

/** Decodes the result of `system.hello` (PRT-005, PRT-051, PRT-150). */
export function decodeHelloResult(value: unknown): Decoded<HelloResult> {
  if (
    !isObject(value) ||
    typeof value.protocol !== "number" ||
    typeof value.daemon !== "string" ||
    typeof value.omp !== "string" ||
    !isStringArray(value.warnings) ||
    typeof value.widgetBase !== "string" ||
    typeof value.fileBase !== "string" ||
    !isSeq(value.connection)
  ) {
    return invalid("expected `protocol`, `daemon`, `omp`, `warnings`, `widgetBase`, `fileBase` and `connection`");
  }
  return valid({
    protocol: value.protocol,
    daemon: value.daemon,
    omp: value.omp,
    warnings: value.warnings,
    widgetBase: value.widgetBase,
    fileBase: value.fileBase,
    connection: value.connection,
  });
}

/** Decodes the result of `subscribe` (PRT-020). */
export function decodeSubscribeResult(value: unknown): Decoded<SubscribeResult> {
  if (isObject(value) && typeof value.epoch === "string") {
    const { epoch } = value;
    if (value.mode === "replay" && isSeq(value.fromSeq)) {
      return valid({ mode: "replay", epoch, fromSeq: value.fromSeq });
    }
    if (value.mode === "snapshot" && isSeq(value.seq) && "state" in value) {
      return valid({ mode: "snapshot", epoch, seq: value.seq, state: value.state });
    }
  }
  return invalid("expected a `replay` ({epoch, fromSeq}) or a `snapshot` ({epoch, seq, state})");
}

/** Decodes the state of an `index` snapshot; a missing `theme` is {@link NO_THEME}, missing `settings`
 * {@link NO_SETTINGS}, a missing `init` (the user's init file version, PRT-095), a missing `models` (the count of
 * usable models, PRT-141) and a missing `login` (the running login, DMN-290) `null`. */
export function decodeIndexSnapshot(value: unknown): Decoded<IndexSnapshot> {
  if (
    !isObject(value) ||
    ![value.workspaces, value.threads].every((list) => Array.isArray(list) && list.every(hasId))
  ) {
    return invalid("expected `workspaces` and `threads`, each a list of objects with an `id`");
  }
  const theme = value.theme ?? NO_THEME;
  if (!isTheme(theme)) {
    return invalid("expected `theme` to be `{ css: string | null }`");
  }
  const settings = value.settings ?? NO_SETTINGS;
  if (!isSettings(settings)) {
    return invalid("expected `settings` to be `{ ui, keymap: { mode, keys, command }[], error }`");
  }
  if (!isOptionalString(value.init)) {
    return invalid("expected `init` to be a string or `null`");
  }
  if (!(isNullish(value.models) || isSeq(value.models))) {
    return invalid("expected `models` to be a count or `null`");
  }
  if (!isLoginActivity(value.login)) {
    return invalid("expected `login` to be `{ providerId, name, url, instructions, progress }` or `null`");
  }
  const login = loginActivity(value.login);
  // Every list holds objects with a string `id`, the key the model files them under.
  return valid({
    ...(value as IndexSnapshot),
    theme,
    settings,
    init: value.init ?? null,
    models: value.models ?? null,
    login,
  });
}

const THREAD_STATE_FIELDS: Record<keyof ThreadState, (field: unknown) => boolean> = {
  process: (field) => isObject(field) && typeof field.state === "string",
  session: isNullableObject,
  settled: (field) => typeof field === "boolean",
  transcript: Array.isArray,
  streaming: isNullableObject,
  tools: Array.isArray,
  queue: isObject,
  interactions: Array.isArray,
  presentation: isObject,
  commands: Array.isArray,
  prompts: isObject,
  subagents: isObject,
  goal: isNullableObject,
  todo_phases: Array.isArray,
  notices: Array.isArray,
  live: isNullableObject,
  live_error: (field) => field === null || typeof field === "string",
  open_file: isNullableObject,
  notify: isNullableObject,
};

/** Decodes the state of a `thread/<id>` snapshot: the daemon's `ThreadState` (PRT-021). */
export function decodeThreadState(value: unknown): Decoded<ThreadState> {
  const state = value;
  if (!isObject(state)) {
    return invalid("a thread state must be an object");
  }
  for (const [field, check] of Object.entries(THREAD_STATE_FIELDS)) {
    if (!check(state[field])) {
      return invalid(`invalid or missing thread state field \`${field}\``);
    }
  }
  // Every field checked above; nested omp wire objects are the reducer's business.
  return valid(value as ThreadState);
}

/** Decodes the state of a `workspace/<id>` snapshot: the git status of each repository. */
export function decodeRepoStatusList(value: unknown): Decoded<RepoStatus[]> {
  return isRepoStatusList(value) ? valid(value) : invalid("expected a list of `{ name, status }` repository statuses");
}

function isStoredMap(value: unknown): value is Record<string, Record<string, unknown>> {
  return isObject(value) && Object.values(value).every(isObject);
}

/** Decodes the state of a `views/<workspaceId>` snapshot (PRT-040, PRT-060, PRT-095); `manifest`, `error`,
 * `keymapError` and `init` default to `null`, `keymap` to no bindings. */
export function decodeViewsState(value: unknown): Decoded<ViewsState> {
  if (
    !isObject(value) ||
    !isOptionalString(value.error) ||
    !isWidgetList(value.widgets) ||
    !isStoredMap(value.stored) ||
    !(value.keymap === undefined || isWorkspaceKeymap(value.keymap)) ||
    !isOptionalString(value.keymapError) ||
    !isOptionalString(value.init)
  ) {
    return invalid("expected `{ manifest, error, widgets: { source, name, version, entry }[], stored, keymap? }`");
  }
  return valid({
    manifest: value.manifest ?? null,
    error: value.error ?? null,
    widgets: value.widgets,
    stored: value.stored,
    keymap: value.keymap ?? [],
    keymapError: value.keymapError ?? null,
    init: value.init ?? null,
  });
}

function isJobInfo(value: unknown): value is JobInfo {
  if (
    !isObject(value) ||
    typeof value.id !== "string" ||
    typeof value.workspaceId !== "string" ||
    typeof value.command !== "string" ||
    typeof value.startedAt !== "number" ||
    !isObject(value.status)
  ) {
    return false;
  }
  const { status } = value;
  return (
    status.state === "running" ||
    (status.state === "exited" &&
      typeof status.endedAt === "number" &&
      isOptionalNumber(status.code) &&
      isOptionalNumber(status.signal))
  );
}

/** Decodes the state of a `job/<id>` snapshot (PRT-041). */
export function decodeJobState(value: unknown): Decoded<JobState> {
  if (
    !isObject(value) ||
    !isJobInfo(value.job) ||
    !isSeq(value.dropped) ||
    !Array.isArray(value.lines) ||
    !value.lines.every((line) => isObject(line) && isJobStream(line.stream) && typeof line.text === "string")
  ) {
    return invalid("expected `{ job: { id, workspaceId, command, startedAt, status }, lines, dropped }`");
  }
  // Every field checked above.
  return valid(value as JobState);
}

function isTerminalInfo(value: unknown): value is TerminalInfo {
  if (
    !isObject(value) ||
    typeof value.id !== "string" ||
    typeof value.workspaceId !== "string" ||
    typeof value.command !== "string" ||
    typeof value.cwd !== "string" ||
    !isPositiveU16(value.cols) ||
    !isPositiveU16(value.rows) ||
    typeof value.startedAt !== "number" ||
    !isObject(value.status)
  ) {
    return false;
  }
  return (
    value.status.state === "running" ||
    (value.status.state === "exited" && typeof value.status.endedAt === "number" && isOptionalNumber(value.status.code))
  );
}

/** Decodes the state of a `terminal/<id>` snapshot (TRM-100, TRM-103). */
export function decodeTerminalState(value: unknown): Decoded<TerminalState> {
  if (
    !isObject(value) ||
    !isTerminalInfo(value.terminal) ||
    !isSeq(value.dropped) ||
    !Array.isArray(value.scrollback) ||
    !value.scrollback.every(isBase64)
  ) {
    return invalid(
      "expected `{ terminal: { id, workspaceId, command, cwd, cols, rows, startedAt, status }, scrollback, dropped }`",
    );
  }
  return valid(value as TerminalState);
}

function isRunInfo(value: unknown): value is RunInfo {
  return (
    isObject(value) &&
    typeof value.id === "string" &&
    typeof value.workspaceId === "string" &&
    isOptionalString(value.job) &&
    typeof value.trigger === "string" &&
    typeof value.startedAt === "number" &&
    typeof value.status === "string" &&
    isOptionalNumber(value.endedAt) &&
    isOptionalString(value.threadId) &&
    isOptionalString(value.reason) &&
    isOptionalNumber(value.exitCode) &&
    isOptionalNumber(value.signal) &&
    (value.startedBy === undefined || isNullableObject(value.startedBy))
  );
}

function isJobView(value: unknown): value is JobView {
  return (
    isObject(value) &&
    typeof value.name === "string" &&
    typeof value.status === "string" &&
    isObject(value.job) &&
    typeof value.job.trigger === "string" &&
    typeof value.job.enabled === "boolean" &&
    [value.run, value.lastRun].every((run) => isNullish(run) || isRunInfo(run)) &&
    isOptionalNumber(value.nextAt)
  );
}

function isJobViewList(value: unknown): value is JobView[] {
  return Array.isArray(value) && value.every(isJobView);
}

/** Decodes the state of a `jobs/<workspaceId>` snapshot (PRT-200); `error` defaults to `null`. */
export function decodeJobsState(value: unknown): Decoded<JobsState> {
  if (!isObject(value) || !isJobViewList(value.jobs) || !isOptionalString(value.error)) {
    return invalid("expected `{ jobs: { name, job, status, run, lastRun, nextAt }[], error }`");
  }
  return valid({ jobs: value.jobs, error: value.error ?? null });
}
