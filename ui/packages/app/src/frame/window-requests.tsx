/**
 * The daemon's requests to this window for agents (PRT-124): `run_command` runs a registered command by id or alias in
 * the global scope, as `:<command> <args>` would — never JavaScript (UI-352); `check_widget` renders a workspace or
 * user widget out of sight, in a hidden container outside every view with the same slot, error boundary and SDK host
 * as a view, until it rendered or failed (at most the request's time), then unmounts it (UI-353). Each answer goes
 * back with `windows.answer`. A login's requests (`login_url`, `login_progress`, `login_input`) are the login's
 * (`login.tsx`, UI-380, UI-384): never answered here.
 */

import type { ViewsState, WidgetReport, WindowAnswer } from "@omfp/protocol";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ERRORS, errorMessage, type OmfpHost, type ReportTimers, useHost, WidgetReporter } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import type { WidgetRegistry } from "../view/resolve.ts";
import { WidgetSlot } from "../view/slot.tsx";
import { onlyScope } from "./cmdline.ts";

const text = strings.shell.windowRequests;

/** How long a check waits after the widget rendered, for the errors of its effects, before it answers. */
export const SETTLE_MS = 500;

/** The global timers. */
const globalTimers: ReportTimers = {
  setTimeout: (run, ms) => {
    globalThis.setTimeout(run, ms);
  },
};

/**
 * Runs the command `name` (an id or alias) with `args` in the global scope for an agent (UI-352): unknown, disabled,
 * a throw or rejection, or an error the command reported (the window's error list, an init file's error) is the
 * answer instead of `ran`.
 */
export async function runRequested(host: OmfpHost, name: string, args: readonly string[]): Promise<WindowAnswer> {
  const command = host.commands.resolve(name, []);
  if (command === undefined) {
    const registered = host.commands.list();
    const global = registered.find(
      (candidate) =>
        candidate.scope === undefined && (candidate.id === name || (candidate.aliases?.includes(name) ?? false)),
    );
    if (global !== undefined) {
      return { kind: "not_run", reason: "disabled", message: text.disabled(global.id) };
    }
    const scoped = onlyScope(name, registered);
    return {
      kind: "not_run",
      reason: "unknown",
      message: scoped === undefined ? text.unknown(name) : text.scoped(name, scoped),
    };
  }
  const errors = host.window.get(ERRORS);
  const initErrors = host.init.getSnapshot().errors;
  try {
    await command.run(args);
  } catch (error) {
    return { kind: "not_run", reason: "failed", message: errorMessage(error) };
  }
  const reported = [
    ...host.window
      .get(ERRORS)
      .filter((error) => !errors.includes(error))
      .map((error) => error.text),
    ...host.init
      .getSnapshot()
      .errors.filter((error) => !initErrors.some((old) => old.file === error.file && old.message === error.message))
      .map((error) => error.message),
  ];
  return reported.length === 0
    ? { kind: "ran", command: command.id }
    : { kind: "not_run", reason: "failed", message: reported.join("\n") };
}

/** Sends the answer to request `requestId`; a lost answer makes the daemon's request time out. */
function answer(host: OmfpHost, requestId: string, answer: WindowAnswer): void {
  host.client.call("windows.answer", { requestId, answer }).catch(() => {});
}

/** A widget check the window is running. */
interface Check {
  readonly id: string;
  readonly reference: string;
  readonly version: string | null;
  readonly timeoutMs: number;
}

export interface WindowRequestsProps {
  readonly workspaceId: string | undefined;
  readonly views: ViewsState | undefined;
  readonly registry: WidgetRegistry;
  /** When checks give up and settle; the global timers by default. */
  readonly timers?: ReportTimers | undefined;
}

/** Answers the daemon's requests to this window (PRT-124) and renders the widgets being checked out of sight. */
export function WindowRequests({
  workspaceId,
  views,
  registry,
  timers = globalTimers,
}: WindowRequestsProps): ReactNode {
  const host = useHost();
  const [checks, setChecks] = useState<readonly Check[]>([]);
  const workspace = useRef(workspaceId);
  workspace.current = workspaceId;
  useEffect(
    () =>
      host.client.onWindowRequest(({ id, request }) => {
        if (request.kind === "run_command") {
          void runRequested(host, request.command, request.args).then((ran) => answer(host, id, ran));
        } else if (request.kind !== "check_widget") {
          return;
        } else if (workspace.current === undefined) {
          answer(host, id, { kind: "not_run", reason: "failed", message: text.noWorkspace });
        } else {
          const { reference, version, timeoutMs } = request;
          setChecks((running) => [...running, { id, reference, version: version ?? null, timeoutMs }]);
        }
      }),
    [host],
  );
  const done = useCallback(
    (id: string, checked: WindowAnswer) => {
      answer(host, id, checked);
      setChecks((running) => running.filter((check) => check.id !== id));
    },
    [host],
  );
  return checks.map((check) => (
    <OffscreenCheck
      key={check.id}
      check={check}
      workspaceId={workspaceId}
      views={views}
      registry={registry}
      timers={timers}
      onDone={done}
    />
  ));
}

interface OffscreenCheckProps {
  readonly check: Check;
  readonly workspaceId: string | undefined;
  readonly views: ViewsState | undefined;
  readonly registry: WidgetRegistry;
  readonly timers: ReportTimers;
  readonly onDone: (id: string, answer: WindowAnswer) => void;
}

/** Whether `report` is the requested version, rendered or failed. */
function settledOn(report: WidgetReport | undefined, version: string | null): report is WidgetReport {
  return report !== undefined && report.state !== "loading" && (version === null || report.version === version);
}

/**
 * One widget check (UI-353): the widget in a hidden container, reporting to its own reporter (not the window's
 * report); answered `SETTLE_MS` after it rendered or failed, or when the request's time is up.
 */
function OffscreenCheck({ check, workspaceId, views, registry, timers, onDone }: OffscreenCheckProps): ReactNode {
  const host = useHost();
  const latest = useRef<WidgetReport | undefined>(undefined);
  const finished = useRef(false);
  const finish = useCallback(() => {
    if (finished.current) {
      return;
    }
    finished.current = true;
    const report = latest.current;
    const settled = settledOn(report, check.version);
    onDone(check.id, {
      kind: "checked",
      settled,
      report: report ?? {
        reference: check.reference,
        instance: check.reference,
        source: check.reference.startsWith("user:") ? "user" : "workspace",
        name: check.reference.replace(/^(\.\/widgets\/|user:)/, ""),
        version: null,
        state: "loading",
        errors: [text.unknownWidget(check.reference)],
        console: [],
        text: "",
        jobs: [],
      },
    });
  }, [check, onDone]);
  const [reporter] = useState(
    () =>
      new WidgetReporter(
        {
          report: ({ widgets }) => {
            const [report] = widgets;
            if (report !== undefined) {
              const first = !settledOn(latest.current, check.version);
              latest.current = report;
              if (first && settledOn(report, check.version)) {
                timers.setTimeout(finish, SETTLE_MS);
              }
            }
            return Promise.resolve();
          },
        },
        host.reports.timers,
      ),
  );
  useEffect(() => reporter.show(workspaceId, undefined), [reporter, workspaceId]);
  useEffect(() => reporter.watchConsole(window), [reporter]);
  useEffect(() => {
    timers.setTimeout(finish, check.timeoutMs);
  }, [timers, finish, check.timeoutMs]);
  const props = useMemo(() => ({ instance: `${check.reference}@check` }), [check.reference]);
  return (
    <div
      className="omfp-offscreen-check"
      aria-hidden="true"
      inert
      data-testid="offscreen-check"
      data-request={check.id}
      data-widget={check.reference}
    >
      <WidgetSlot
        reference={check.reference}
        props={props}
        path="check"
        workspaceId={workspaceId}
        views={views}
        registry={registry}
        reporter={reporter}
      />
    </div>
  );
}
