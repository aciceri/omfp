/**
 * Window reports (AGT-026, PRT-043): what this window shows of the workspace and user widgets of its view — import
 * state and errors, error boundary errors, console errors from the widget's module, the rendered text and its
 * `useJob` jobs — sent to the daemon with `widgets.report`, so the agent's `omfp_widget_check` sees them (DMN-063),
 * the errors of its init files (`fileErrors`, UI-304), which agents read with `views.errors`, and the
 * window's own state (`window`, UI-350, UI-351: thread, focus, views and where each comes from, added commands), which
 * agents read with `windows.list` / `windows.state`. Each widget slot owns a {@link SlotReport}; the
 * {@link WidgetReporter} batches changes (at most one report every {@link REPORT_DELAY_MS}) and sends only what
 * differs from the last report.
 */

import type {
  FileErrorReport,
  Position,
  ReportWidgetsParams,
  WidgetJobReport,
  WidgetReport,
  WidgetReportState,
  WidgetSource,
  WindowReport,
} from "@omfp/protocol";
import { createContext, useContext } from "react";

/** The shortest time between two reports. */
export const REPORT_DELAY_MS = 250;

/** Most characters of rendered text a report carries (the daemon keeps 4,000, PRT-043). */
const MAX_TEXT = 4000;
/** Most console entries kept per widget (the daemon keeps 20). */
const MAX_CONSOLE = 20;
/** Output lines a job report carries. */
export const JOB_TAIL = 10;

/** The client side of the reports: `widgets.report`. */
export interface ReportSink {
  report(params: ReportWidgetsParams): Promise<unknown>;
}

/** Which widget a slot shows. */
export interface SlotIdentity {
  readonly reference: string;
  readonly instance: string;
  readonly source: WidgetSource;
  readonly name: string;
  /** What appears in the stack of code from the widget's module: `<widgetBase>/<workspace id | user>/<name>/`. */
  readonly modulePath: string;
}

/** The load state of a slot's module. */
export interface SlotLoad {
  readonly state: WidgetReportState;
  readonly version: string | null;
  /** Errors of the import or the compile stand-in. */
  readonly errors: readonly string[];
}

/** One module widget slot's part of the report; the slot {@link WidgetReporter.add}s it while mounted. */
export class SlotReport {
  readonly #reporter: WidgetReporter;
  identity: SlotIdentity;
  load: SlotLoad = { state: "loading", version: null, errors: [] };
  /** The render error the slot's error boundary shows, if any. */
  boundary: string | undefined;
  console: string[] = [];
  readonly jobs = new Map<string, WidgetJobReport>();
  /** Reads the rendered text when a report is made. */
  text: () => string = () => "";

  constructor(reporter: WidgetReporter, identity: SlotIdentity) {
    this.#reporter = reporter;
    this.identity = identity;
  }

  setIdentity(identity: SlotIdentity): void {
    this.identity = identity;
    this.changed();
  }

  setLoad(load: SlotLoad): void {
    if (load.version !== this.load.version) {
      // A new version starts afresh: its predecessor's console errors no longer apply (AGT-013).
      this.console = [];
    }
    this.load = load;
    this.changed();
  }

  /** The error boundary caught `message`; `undefined` when the boundary that showed an error is gone (a new
   * version is a new boundary, AGT-014). */
  setBoundary(message: string | undefined): void {
    this.boundary = message;
    this.changed();
  }

  consoleError(message: string): void {
    this.console = [...this.console, message].slice(-MAX_CONSOLE);
    this.changed();
  }

  setJob(command: string, job: WidgetJobReport | undefined): void {
    if (job === undefined) {
      this.jobs.delete(command);
    } else {
      this.jobs.set(command, job);
    }
    this.changed();
  }

  /** Something the report shows changed: report again soon. */
  changed(): void {
    this.#reporter.schedule();
  }

  report(): WidgetReport {
    const { reference, instance, source, name } = this.identity;
    const text = this.text();
    return {
      reference,
      instance,
      source,
      name,
      version: this.load.version,
      state: this.boundary === undefined ? this.load.state : "render_error",
      errors: this.boundary === undefined ? [...this.load.errors] : [...this.load.errors, this.boundary],
      console: this.console,
      text: text.length > MAX_TEXT ? `${text.slice(0, MAX_TEXT)}…` : text,
      jobs: [...this.jobs.values()],
    };
  }
}

/** Timers, so tests can drive the batching. */
export interface ReportTimers {
  setTimeout(run: () => void, ms: number): void;
}

const globalTimers: ReportTimers = {
  setTimeout: (run, ms) => {
    globalThis.setTimeout(run, ms);
  },
};

/** A console argument as text: an error's name and message, other values as `String` or JSON. */
function describe(value: unknown): string {
  if (value instanceof Error) {
    return `${value.name}: ${value.message}`;
  }
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

/** The window's reporter: one per host. */
export class WidgetReporter {
  readonly #sink: ReportSink;
  readonly #timers: ReportTimers;
  readonly #slots = new Set<SlotReport>();
  #workspaceId: string | null = null;
  #views: Position | null = null;
  #fileErrors: readonly FileErrorReport[] = [];
  #window: WindowReport | undefined;
  #scheduled = false;
  #last: string | undefined;

  constructor(sink: ReportSink, timers: ReportTimers = globalTimers) {
    this.#sink = sink;
    this.#timers = timers;
  }

  /** The timers its batching runs on. */
  get timers(): ReportTimers {
    return this.#timers;
  }

  /** The workspace whose view the window shows and the views topic position it rendered (`undefined`: none). */
  show(workspaceId: string | undefined, views: Position | undefined): void {
    const workspace = workspaceId ?? null;
    const position = views ?? null;
    if (
      workspace === this.#workspaceId &&
      position?.epoch === this.#views?.epoch &&
      position?.seq === this.#views?.seq
    ) {
      return;
    }
    this.#workspaceId = workspace;
    this.#views = position;
    this.schedule();
  }

  /** The window's errors of init files, as it shows them now (UI-304). */
  setFileErrors(errors: readonly FileErrorReport[]): void {
    if (JSON.stringify(errors) !== JSON.stringify(this.#fileErrors)) {
      this.#fileErrors = errors;
      this.schedule();
    }
  }

  /** What the window shows besides its widgets (UI-350, UI-351); sent with every report once set. */
  setWindow(window: WindowReport): void {
    if (JSON.stringify(window) !== JSON.stringify(this.#window)) {
      this.#window = window;
      this.schedule();
    }
  }

  /** Makes `slot` part of every report until it is {@link release}d. */
  add(slot: SlotReport): void {
    if (!this.#slots.has(slot)) {
      this.#slots.add(slot);
      this.schedule();
    }
  }

  release(slot: SlotReport): void {
    if (this.#slots.delete(slot)) {
      this.schedule();
    }
  }

  /** The connection changed (the daemon forgot this window's report): send the next one even if it is the same. */
  resend(): void {
    this.#last = undefined;
    this.schedule();
  }

  /** Attributes a console error or an uncaught error to the slots whose module its stacks name. */
  consoleError(args: readonly unknown[], stack: string | undefined): void {
    const stacks = [stack ?? "", ...args.map((arg) => (arg instanceof Error ? (arg.stack ?? "") : ""))].join("\n");
    const message = args.map(describe).join(" ");
    for (const slot of this.#slots) {
      if (stacks.includes(slot.identity.modulePath)) {
        slot.consoleError(message);
      }
    }
  }

  /** Reports soon, at most once per {@link REPORT_DELAY_MS}. */
  schedule(): void {
    if (this.#scheduled) {
      return;
    }
    this.#scheduled = true;
    this.#timers.setTimeout(() => {
      this.#scheduled = false;
      this.flush();
    }, REPORT_DELAY_MS);
  }

  /** The report now. */
  current(): ReportWidgetsParams {
    return {
      workspaceId: this.#workspaceId,
      views: this.#views,
      widgets: this.#workspaceId === null ? [] : [...this.#slots].map((slot) => slot.report()),
      fileErrors: [...this.#fileErrors],
      ...(this.#window === undefined ? {} : { window: this.#window }),
    };
  }

  /** Sends the report unless it equals the last one sent. */
  flush(): void {
    const report = this.current();
    const text = JSON.stringify(report);
    if (text === this.#last) {
      return;
    }
    this.#last = text;
    // A lost report is replaced by the next change; it never reaches the user's error list.
    this.#sink.report(report).catch(() => {
      this.#last = undefined;
    });
  }

  /**
   * Follows `console.error`, uncaught errors and unhandled rejections of `target` (the window) for widget modules;
   * returns the function that stops.
   */
  watchConsole(target: Window & typeof globalThis): () => void {
    const original = target.console.error;
    target.console.error = (...args: unknown[]) => {
      original.apply(target.console, args);
      this.consoleError(args, new Error().stack);
    };
    const onError = (event: ErrorEvent) => {
      this.consoleError([event.error ?? event.message], `${event.filename}\n${event.error?.stack ?? ""}`);
    };
    const onRejection = (event: PromiseRejectionEvent) => {
      this.consoleError([event.reason], undefined);
    };
    target.addEventListener("error", onError);
    target.addEventListener("unhandledrejection", onRejection);
    return () => {
      target.console.error = original;
      target.removeEventListener("error", onError);
      target.removeEventListener("unhandledrejection", onRejection);
    };
  }
}

/** The report of the module widget slot being rendered; `undefined` for built-ins. */
export const SlotReportScope = createContext<SlotReport | undefined>(undefined);

/** The slot report of the widget being rendered, if it is a module widget. */
export function useSlotReport(): SlotReport | undefined {
  return useContext(SlotReportScope);
}
