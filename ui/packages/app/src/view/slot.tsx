/**
 * The slot every widget of a view renders in (UI-105, AGT-013, AGT-014): it resolves the reference (built-in,
 * workspace override, workspace or user widget), imports module widgets from the daemon by versioned URL, provides the
 * {@link WidgetScope} the SDK's widget hooks read, shows loading and every failure in the widget's place, and reports
 * module widgets to the daemon ({@link SlotReport}, AGT-026).
 */

import type { ViewsState, WidgetInfo, WidgetReportState } from "@omfp/protocol";
import { Component, type ReactNode, type RefObject, useEffect, useMemo, useRef, useState } from "react";
import {
  type LoadedWidget,
  SlotReport,
  SlotReportScope,
  useHost,
  useSnapshot,
  type WidgetProps,
  type WidgetReporter,
  WidgetScope,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { resolveWidget, type WidgetRegistry, widgetModulePath, widgetUrl } from "./resolve.ts";

const text = strings.widgets;

export interface WidgetSlotProps {
  /** The node's widget reference (`omfp:chat`, `./widgets/tests`, `user:clock`). */
  readonly reference: string;
  readonly props: WidgetProps;
  /** The node's path in the view (`"0.1"`, UI-103). */
  readonly path: string;
  /** The workspace whose view this is; `undefined` without a current workspace. */
  readonly workspaceId: string | undefined;
  /** That workspace's views state (`useViews`), `undefined` until it arrives. */
  readonly views: ViewsState | undefined;
  readonly registry: WidgetRegistry;
  /** Where the module widget's report goes; the window's reports by default (an out-of-sight check passes its own,
   * UI-353). */
  readonly reporter?: WidgetReporter | undefined;
}

/** A module as shown: the folder it came from, its version and what its import gave. */
interface Shown {
  readonly folder: string;
  readonly version: string;
  readonly loaded: LoadedWidget;
}

const NO_ENTRY: LoadedWidget = {
  kind: "error",
  error: { kind: "load", message: text.noEntry, file: undefined, line: null, column: null },
};

/** The `widgetBase` of the latest `system.hello` (PRT-042), kept while reconnecting. */
export function useWidgetBase(): string | undefined {
  const current = useSnapshot((snapshot) =>
    snapshot.connection.kind === "connected" ? snapshot.connection.hello.widgetBase : undefined,
  );
  const latest = useRef(current);
  if (current !== undefined) {
    latest.current = current;
  }
  return latest.current;
}

/**
 * The module of `widget`: imported once per URL; while a new version loads, the previous one of the same folder stays
 * shown, so a reload replaces the widget without a placeholder flash (AGT-013). `undefined` while nothing is loaded.
 */
function useWidgetModule(widget: WidgetInfo | undefined, workspaceId: string | undefined): Shown | undefined {
  const { modules } = useHost();
  const base = useWidgetBase();
  const folder = widget === undefined ? undefined : `${widget.source}/${widget.name}`;
  const version = widget?.version;
  const entry = widget?.entry ?? undefined;
  const url =
    widget === undefined || entry === undefined || base === undefined || workspaceId === undefined
      ? undefined
      : widgetUrl(base, workspaceId, widget, entry);
  const [shown, setShown] = useState<Shown | undefined>(undefined);
  useEffect(() => {
    if (url === undefined || folder === undefined || version === undefined || modules.get(url) !== undefined) {
      return;
    }
    let live = true;
    void modules.load(url).then((loaded) => {
      if (live) {
        setShown({ folder, version, loaded });
      }
    });
    return () => {
      live = false;
    };
  }, [modules, url, folder, version]);

  if (folder === undefined || version === undefined) {
    return undefined;
  }
  if (widget?.entry === null || widget?.entry === undefined) {
    return { folder, version, loaded: NO_ENTRY };
  }
  const settled = url === undefined ? undefined : modules.get(url);
  if (settled !== undefined) {
    return { folder, version, loaded: settled };
  }
  return shown?.folder === folder ? shown : undefined;
}

/** A module widget's load as reported (PRT-043): `loading` while the shown version is not the folder's current one. */
function loadOf(widget: WidgetInfo, module: Shown | undefined): { state: WidgetReportState; error?: string } {
  if (module === undefined || module.version !== widget.version) {
    return { state: "loading" };
  }
  const { loaded } = module;
  if (loaded.kind === "component") {
    return { state: "rendered" };
  }
  const { error } = loaded;
  const message =
    error.file === undefined
      ? error.message
      : `${text.location(error.file, error.line, error.column)}: ${error.message}`;
  return { state: error.kind === "compile" ? "compile_error" : "load_error", error: message };
}

/** The text a widget renders, as a reader sees it: line breaks between blocks, hidden parts left out. */
function renderedText(element: HTMLElement | null): string {
  // biome-ignore lint/performance/useDomNodeTextContent: the rendered text (blocks on their own lines) is what the agent reads; it is read at most once per report.
  return element === null ? "" : element.innerText.trim();
}

/**
 * The report of a module widget slot (AGT-026): part of `reporter`'s reports (the window's by default) while mounted,
 * following the widget's identity, load and rendered text (`element`'s); `undefined` for built-ins and unresolved
 * references.
 */
function useModuleReport(
  widget: WidgetInfo | undefined,
  module: Shown | undefined,
  reference: string,
  instance: string,
  workspaceId: string | undefined,
  element: RefObject<HTMLDivElement | null>,
  reporter: WidgetReporter | undefined,
): SlotReport | undefined {
  const host = useHost();
  const reports = reporter ?? host.reports;
  const base = useWidgetBase() ?? "";
  const identity =
    widget === undefined || workspaceId === undefined
      ? undefined
      : {
          reference,
          instance,
          source: widget.source,
          name: widget.name,
          modulePath: widgetModulePath(base, workspaceId, widget),
        };
  const slot = useRef<SlotReport | undefined>(undefined);
  if (identity === undefined) {
    slot.current = undefined;
  } else if (slot.current === undefined) {
    slot.current = new SlotReport(reports, identity);
  }
  const report = slot.current;

  useEffect(() => {
    if (report === undefined) {
      return;
    }
    report.text = () => renderedText(element.current);
    reports.add(report);
    const observer = new MutationObserver(() => report.changed());
    if (element.current !== null) {
      observer.observe(element.current, { childList: true, characterData: true, subtree: true });
    }
    return () => {
      observer.disconnect();
      reports.release(report);
    };
  }, [reports, report, element]);

  const source = identity?.source;
  const name = identity?.name;
  const modulePath = identity?.modulePath;
  useEffect(() => {
    if (report !== undefined && source !== undefined && name !== undefined && modulePath !== undefined) {
      const current = report.identity;
      if (
        current.reference !== reference ||
        current.instance !== instance ||
        current.source !== source ||
        current.name !== name ||
        current.modulePath !== modulePath
      ) {
        report.setIdentity({ reference, instance, source, name, modulePath });
      }
    }
  }, [report, reference, instance, source, name, modulePath]);

  const load = widget === undefined ? undefined : loadOf(widget, module);
  const state = load?.state;
  const error = load?.error;
  const version = module?.version ?? null;
  useEffect(() => {
    if (report !== undefined && state !== undefined) {
      report.setLoad({ state, version, errors: error === undefined ? [] : [error] });
    }
  }, [report, state, version, error]);
  return report;
}

export function WidgetSlot({
  reference,
  props,
  path,
  workspaceId,
  views,
  registry,
  reporter,
}: WidgetSlotProps): ReactNode {
  const instance = typeof props.instance === "string" ? props.instance : reference;
  const scope = useMemo(() => ({ instance, reference, workspaceId }), [instance, reference, workspaceId]);
  const resolved = resolveWidget(reference, workspaceId, views, registry);
  const widget = resolved.kind === "module" ? resolved.widget : undefined;
  const module = useWidgetModule(widget, workspaceId);
  const element = useRef<HTMLDivElement>(null);
  const report = useModuleReport(widget, module, reference, instance, workspaceId, element, reporter);

  let body: ReactNode;
  switch (resolved.kind) {
    case "builtin": {
      const Widget = resolved.component;
      body = (
        <WidgetBoundary key={reference} reference={reference}>
          <Widget {...props} />
        </WidgetBoundary>
      );
      break;
    }
    case "missing":
      body = <WidgetError kind="missing" title={text.missing(reference)} />;
      break;
    case "pending":
      body = <WidgetLoading reference={reference} />;
      break;
    case "module":
      // A new version is a new boundary: a render error of the previous one does not stick (AGT-014).
      body =
        module === undefined ? (
          <WidgetLoading reference={reference} />
        ) : (
          loadedBody(reference, props, module.loaded, module.version, report)
        );
      break;
  }
  const moduleAttributes =
    resolved.kind === "module"
      ? { "data-source": resolved.widget.source, ...(module === undefined ? {} : { "data-version": module.version }) }
      : {};
  return (
    <div
      ref={element}
      className="omfp-widget"
      data-testid="widget"
      data-widget={reference}
      data-instance={instance}
      data-path={path}
      {...moduleAttributes}
    >
      <WidgetScope value={scope}>
        <SlotReportScope value={report}>{body}</SlotReportScope>
      </WidgetScope>
    </div>
  );
}

/** A settled module in its place: its error, else the component in a boundary keyed `key`. */
function loadedBody(
  reference: string,
  props: WidgetProps,
  loaded: LoadedWidget,
  key: string,
  report: SlotReport | undefined,
): ReactNode {
  if (loaded.kind === "error") {
    const { error } = loaded;
    return (
      <WidgetError
        kind={error.kind}
        title={error.kind === "compile" ? text.compileFailed(reference) : text.loadFailed(reference)}
        location={error.file === undefined ? undefined : text.location(error.file, error.line, error.column)}
        message={error.message}
      />
    );
  }
  const Widget = loaded.component;
  return (
    <WidgetBoundary key={key} reference={reference} report={report}>
      <Widget {...props} />
    </WidgetBoundary>
  );
}

/** What a card's one-file widget is named in its slot and its errors: the file the daemon compiles (PRT-225). */
export const SOURCE_REFERENCE = "card.tsx";

export interface SourceSlotProps {
  /** The widget's source: one file whose default export is the component (AGT-096). */
  readonly code: string;
  readonly props: WidgetProps;
  /** Where the slot is (`card/<toolCallId>`). */
  readonly path: string;
  /** The workspace the widget keeps its state in (`useStored`). */
  readonly workspaceId: string | undefined;
}

/**
 * The slot of a card's one-file widget (UI-920): its source compiled by the daemon (`widgets.compile`, PRT-225) and
 * imported once per source, then shown as a module widget is (its {@link WidgetScope}, loading, errors in place); it
 * has no widget folder, so no report.
 */
export function SourceSlot({ code, props, path, workspaceId }: SourceSlotProps): ReactNode {
  const instance = typeof props.instance === "string" ? props.instance : SOURCE_REFERENCE;
  const scope = useMemo(() => ({ instance, reference: SOURCE_REFERENCE, workspaceId }), [instance, workspaceId]);
  const loaded = useSourceModule(code);
  return (
    <div
      className="omfp-widget"
      data-testid="widget"
      data-widget={SOURCE_REFERENCE}
      data-instance={instance}
      data-path={path}
    >
      <WidgetScope value={scope}>
        {loaded === undefined ? (
          <WidgetLoading reference={SOURCE_REFERENCE} />
        ) : (
          // A new source is a new boundary, like a new version of a folder's.
          loadedBody(SOURCE_REFERENCE, props, loaded, code, undefined)
        )}
      </WidgetScope>
    </div>
  );
}

/** The module of the card source `code`, compiled and imported once per source; `undefined` while it loads. */
function useSourceModule(code: string): LoadedWidget | undefined {
  const { client, modules } = useHost();
  const [shown, setShown] = useState<{ code: string; loaded: LoadedWidget } | undefined>(undefined);
  useEffect(() => {
    if (modules.getSource(code) !== undefined) {
      return;
    }
    let live = true;
    const compile = (source: string) =>
      client.call("widgets.compile", { code: source }).then((compiled) => compiled.module);
    void modules.loadSource(code, compile).then((loaded) => {
      if (live) {
        setShown({ code, loaded });
      }
    });
    return () => {
      live = false;
    };
  }, [client, modules, code]);
  return modules.getSource(code) ?? (shown?.code === code ? shown.loaded : undefined);
}

function WidgetLoading({ reference }: { reference: string }): ReactNode {
  return (
    <p className="omfp-widget__loading" data-testid="widget-loading" aria-busy="true">
      {text.loading(reference)}
    </p>
  );
}

export type WidgetErrorKind = "compile" | "load" | "render" | "missing";

interface WidgetErrorProps {
  readonly kind: WidgetErrorKind;
  readonly title: string;
  /** `file:line:column` when known. */
  readonly location?: string | undefined;
  readonly message?: string | undefined;
}

/** A widget's failure, in its place (AGT-014). */
export function WidgetError({ kind, title, location, message }: WidgetErrorProps): ReactNode {
  return (
    <div className="omfp-widget__error" role="alert" data-testid="widget-error" data-error-kind={kind}>
      <strong>{title}</strong>
      {location === undefined ? null : <code data-testid="widget-error-location">{location}</code>}
      {message === undefined ? null : <pre>{message}</pre>}
    </div>
  );
}

interface BoundaryProps {
  /** The widget's reference, named in the error. */
  readonly reference: string;
  /** The module widget slot's report, told about the error this boundary shows (AGT-026). */
  readonly report?: SlotReport | undefined;
  readonly children: ReactNode;
}

interface BoundaryState {
  readonly error: Error | undefined;
}

/** Catches a widget's render errors and shows them in its place (`data-error-kind="render"`). */
export class WidgetBoundary extends Component<BoundaryProps, BoundaryState> {
  override state: BoundaryState = { error: undefined };

  static getDerivedStateFromError(error: unknown): BoundaryState {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  override componentDidCatch(error: unknown): void {
    this.props.report?.setBoundary(error instanceof Error ? error.message : String(error));
  }

  override componentWillUnmount(): void {
    if (this.state.error !== undefined) {
      this.props.report?.setBoundary(undefined);
    }
  }

  override render(): ReactNode {
    const { error } = this.state;
    if (error === undefined) {
      return this.props.children;
    }
    return <WidgetError kind="render" title={text.renderFailed(this.props.reference)} message={error.message} />;
  }
}
