/**
 * The thin frame around the view (docs/specs/08-agent-control.md): connection banner (UI-003), the left column of
 * workspaces and threads (UI-100), the view of the current workspace with its error banner (UI-100..102, AGT-011)
 * or, while omp has no usable model, the first run's welcome (UI-500)
 * and its view tabs in the mode line (UI-301), the keymap, `:` command line (UI-095), which-key panel (UI-320) and
 * help panel (UI-198), the runtime errors of init files and TSX views reported to the daemon (UI-304), the window's
 * state for agents and their requests to it (UI-350..UI-353), the safety net for pending interactions (AGT-015),
 * floating panels (UI-230), notices and notifications (UI-096, UI-192), the live region (UI-092), dialogs, the files
 * the agent opens (DMN-060), and theme/density on `<html>` (UI-091, UI-094). Everything else is a widget of the view.
 */

import { type FileErrorReport, type ViewsState, viewsTopic } from "@omfp/protocol";
import { Activity, type ReactNode, type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { createKeyHandler } from "../keymap/handler.ts";
import { type Binding, DEFAULT_BINDINGS } from "../keymap/keymap.ts";
import { globalKeyTimers, type KeyTimers, WhichKeyState } from "../keymap/which-key.ts";
import { KeymapError, useBindings } from "../keymap/workspace.tsx";
import {
  chooseView,
  EDITOR_MODE,
  type ReportTimers,
  SELECTED_THREAD,
  useActiveView,
  useCurrentWorkspaceId,
  useHost,
  useSettings,
  useSnapshot,
  useViewChoicesUpkeep,
  useViews,
  useWindowState,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { useAppearance, useUserTheme } from "../theme/theme.ts";
import { useFileView } from "../view/file-view.tsx";
import { keepViews, NO_KEPT_VIEWS } from "../view/kept.ts";
import { chatCell, containsWidget, hasMainSlot } from "../view/layout.ts";
import { BUILTIN_WIDGETS, INTERACTION_WIDGETS } from "../view/registry.ts";
import { ViewRenderer } from "../view/render.tsx";
import { resolveWidget, type WidgetRegistry } from "../view/resolve.ts";
import { WidgetBoundary } from "../view/slot.tsx";
import {
  BUILTIN_VIEW,
  isFileView,
  type ResolvedView,
  resolveViews,
  type Views,
  withInitPanels,
} from "../view/views.ts";
import { InteractionsWidget } from "../widgets/chat/index.ts";
import { ThreadsWidget } from "../widgets/threads/index.ts";
import { AgentFiles } from "./agent-files.tsx";
import { AgentNotices } from "./agent-notices.tsx";
import { CommandLine } from "./command-line.tsx";
import { GlobalCommands } from "./commands.tsx";
import { Describe } from "./describe.tsx";
import { DeleteWorkspaceDialog, ResetViewsDialog } from "./dialogs.tsx";
import { FilePickerLine } from "./file-picker.tsx";
import { FloatingFile } from "./floating-file.tsx";
import { FloatingWidgets, RevealedWidget } from "./floating-widgets.tsx";
import { Help } from "./help.tsx";
import { InitErrors, InitLoader, useInit } from "./init.tsx";
import { useAddWidget, widgetReferences } from "./layout.tsx";
import { Login } from "./login.tsx";
import { Messages } from "./messages.tsx";
import { type ModeLineViews, useMode, WindowModeLine } from "./mode-line.tsx";
import type { NotificationPort } from "./notifications.ts";
import { Notifier } from "./notifier.tsx";
import { OpenedUrls } from "./opened-urls.tsx";
import { LEFT_OPEN, MOBILE_AREA } from "./responsive.ts";
import {
  Areas,
  LEFT_COLUMN_ID,
  LeftSplitter,
  LeftStrip,
  ToggleThreadsCommand,
  useFrameBreakpoint,
  useLeftDrawer,
  useVisualViewport,
} from "./responsive.tsx";
import { ConnectionBanner, StatusLiveRegion, Toasts } from "./status.tsx";
import { ThreadPicker } from "./thread-picker.tsx";
import { ViewPicker } from "./view-picker.tsx";
import { useWelcome, Welcome } from "./welcome.tsx";
import { WhichKey } from "./which-key.tsx";
import { WindowRequests } from "./window-requests.tsx";
import { useWindowFocus, useWindowStateReport } from "./window-state.ts";

export interface FrameProps {
  /** Built-in widgets by reference; {@link BUILTIN_WIDGETS} by default. */
  readonly registry?: WidgetRegistry;
  /** The defaults the daemon's `[keymap]` entries apply to; {@link DEFAULT_BINDINGS} by default. */
  readonly bindings?: readonly Binding[];
  /** Clock of key sequences; `performance.now` by default. */
  readonly now?: () => number;
  /** Timers of the which-key panel's delay (UI-321); the global timers by default. */
  readonly keyTimers?: KeyTimers;
  /** The Notification API; none by default (main.tsx passes the browser's). */
  readonly notifications?: NotificationPort;
  readonly hasFocus?: () => boolean;
  readonly focusWindow?: () => void;
  /** Timers of the out-of-sight widget checks agents ask for (UI-353); the global timers by default. */
  readonly checkTimers?: ReportTimers;
}

const performanceNow = () => performance.now();
const documentHasFocus = () => document.hasFocus();
const focusThisWindow = () => window.focus();

export function Frame({
  registry = BUILTIN_WIDGETS,
  bindings: defaults = DEFAULT_BINDINGS,
  now = performanceNow,
  keyTimers = globalKeyTimers,
  notifications,
  hasFocus = documentHasFocus,
  focusWindow = focusThisWindow,
  checkTimers,
}: FrameProps): ReactNode {
  const workspaceId = useCurrentWorkspaceId();
  const state = useViews(workspaceId);
  useWindowReports(workspaceId);
  const manifest = state?.manifest;
  const manifestError = state?.error;
  // What the init files add to this window: the views they use or transform (for the manifest they read), panels,
  // bindings over every keymap (UI-261, UI-267).
  const init = useInit();
  const views = useMemo(() => {
    const own = viewsOf(manifest, manifestError);
    const used = init.views !== undefined && init.views.manifest === manifest ? init.views.views : undefined;
    return withInitPanels(used === undefined ? own : { ...used, error: own.error }, init.panels);
  }, [manifest, manifestError, init]);
  // The selected thread's view, else its workspace's last chosen, else the default (UI-367).
  useViewChoicesUpkeep();
  const viewIds = views.views.map((candidate) => candidate.id);
  const active = useActiveView(viewIds);
  const listed =
    views.views.find((candidate) => candidate.id === active) ??
    views.views.find((candidate) => candidate.id === views.defaultId);
  // A TSX view renders its file first (UI-268); while it fails, the default view stands in.
  const file = useFileView(listed !== undefined && isFileView(listed) ? listed : undefined, workspaceId, state);
  const fileError = listed !== undefined && isFileView(listed) ? file.error : undefined;
  // The last views shown stay mounted, hidden (UI-390); a TSX view shown again shows as it was until it rendered.
  const [kept, setKept] = useState(NO_KEPT_VIEWS);
  const keptListed =
    listed === undefined || kept.workspaceId !== workspaceId
      ? undefined
      : kept.views.find((entry) => entry.id === listed.id)?.view;
  const view =
    listed === undefined || !isFileView(listed)
      ? listed
      : (file.view ??
        (file.error !== undefined ? BUILTIN_VIEW : keptListed?.file === listed.file ? keptListed : undefined));
  const keeping = keepViews(
    kept,
    workspaceId,
    listed === undefined || view === undefined ? undefined : { id: listed.id, view },
    views.views,
  );
  if (keeping !== kept) {
    setKept(keeping);
  }
  const addWidget = useAddWidget(workspaceId, state, listed);
  const fileErrors = useMemo(
    (): FileErrorReport[] => [
      ...init.errors.map(
        ({ file: which, message }): FileErrorReport => ({
          kind: "init",
          file: which === "user" ? "user" : ".omfp/init.ts",
          message,
        }),
      ),
      ...(listed === undefined || !isFileView(listed) || fileError === undefined
        ? []
        : [{ kind: "view" as const, file: strings.init.viewFile.path(listed.file), message: fileError }]),
    ],
    [init.errors, listed, fileError],
  );
  useFileErrorReports(fileErrors);
  const settings = useSettings();
  // The defaults < the daemon's `[keymap]` entries < the current workspace's `.omfp/keymap.json` < the init files',
  // live (UI-097, UI-210, UI-262).
  const bindings = useBindings(defaults, settings.keymap, state, init.bindings);
  const [whichKey] = useState(() => new WhichKeyState());
  useKeymap(bindings, now, keyTimers, whichKey);
  const [selectedThread] = useWindowState(SELECTED_THREAD);
  const welcome = useWelcome(selectedThread);
  const focused = useWindowFocus(hasFocus);
  useWindowStateReport({ workspaceId, threadId: selectedThread, focused, views, listed, shown: view, bindings });
  useAppearance();
  useUserTheme();
  const appRef = useRef<HTMLDivElement>(null);
  const asideRef = useRef<HTMLElement>(null);
  // The frame's own width picks the layout (UI-200); a short touch screen is a phone (UI-414).
  const breakpoint = useFrameBreakpoint(appRef);
  useLeftDrawer(breakpoint, asideRef);
  useVisualViewport(breakpoint);
  const [leftOpen] = useWindowState(LEFT_OPEN);
  const [area] = useWindowState(MOBILE_AREA);
  const side = view !== undefined && chatCell(view.tree) !== undefined;
  const host = useHost();
  const modeViews = useMemo(
    (): ModeLineViews => ({
      views: views.views.map(({ id, title }) => ({ id, title })),
      current: listed?.id,
      onSelect: (id) => chooseView(host, id),
    }),
    [views.views, listed?.id, host],
  );
  return (
    <div ref={appRef} className="omfp-app" data-testid="app" data-layout={breakpoint}>
      <ConnectionBanner />
      {views.error !== undefined && (
        <div className="omfp-banner" data-tone="error" role="alert" data-testid="view-error">
          {strings.shell.frame.invalidView(views.error)}
        </div>
      )}
      <KeymapError views={state} />
      <InitErrors errors={init.errors} />
      {listed !== undefined && isFileView(listed) && fileError !== undefined && (
        <div
          className="omfp-banner"
          data-tone="error"
          role="alert"
          data-testid="view-file-error"
          data-file={listed.file}
        >
          {strings.init.viewFile.banner(strings.init.viewFile.path(listed.file), fileError)}
        </div>
      )}
      {settings.error !== null && settings.error !== undefined && (
        <div className="omfp-banner" data-tone="error" role="alert" data-testid="settings-error">
          {strings.shell.settings.error(settings.error)}
        </div>
      )}
      <main className="omfp-shell" data-testid="shell">
        {breakpoint === "narrow" && <LeftStrip />}
        <aside
          ref={asideRef}
          id={LEFT_COLUMN_ID}
          className="omfp-frame__left"
          data-testid="left-column"
          data-open={(breakpoint !== "wide" && leftOpen) || undefined}
        >
          <div className="omfp-frame__threads">
            <WidgetBoundary reference="omfp:threads">
              <ThreadsWidget />
            </WidgetBoundary>
          </div>
        </aside>
        {breakpoint === "wide" && <LeftSplitter aside={asideRef} />}
        <div className="omfp-frame__main">
          {keeping.views.map((entry) => {
            const shown = !welcome && view !== undefined && entry.id === listed?.id;
            return (
              <Activity key={entry.key} mode={shown ? "visible" : "hidden"}>
                <ViewRenderer
                  view={entry.view}
                  registry={registry}
                  workspaceId={workspaceId}
                  views={state}
                  area={breakpoint === "mobile" ? (side ? area : "chat") : undefined}
                  shown={shown}
                />
              </Activity>
            );
          })}
          {welcome && <Welcome />}
          {view !== undefined && !hasMainSlot(view.tree) && (
            <FloatingFile workspaceId={workspaceId} views={state} registry={registry} />
          )}
          <FloatingWidgets panels={views.panels} workspaceId={workspaceId} views={state} registry={registry} />
          <RevealedWidget
            view={view}
            panels={views.panels}
            workspaceId={workspaceId}
            views={state}
            registry={registry}
          />
          <FrameModeLine appRef={appRef} bindings={bindings} views={modeViews} whichKey={whichKey} />
        </div>
      </main>
      {breakpoint === "mobile" && <Areas side={side} />}
      <ToggleThreadsCommand breakpoint={breakpoint} />
      {view !== undefined && <SafetyNet view={view} workspaceId={workspaceId} views={state} registry={registry} />}
      <Toasts />
      <Login />
      <StatusLiveRegion />
      <DeleteWorkspaceDialog />
      <ResetViewsDialog />
      <Help bindings={bindings} now={now} />
      <Messages />
      <Describe bindings={bindings} />
      <GlobalCommands
        viewIds={viewIds}
        currentView={listed?.id}
        widgets={widgetReferences(registry, state)}
        addWidget={addWidget}
      />
      <Notifier port={notifications} hasFocus={hasFocus} focusWindow={focusWindow} />
      <AgentFiles />
      <AgentNotices />
      <OpenedUrls />
      <InitLoader workspaceId={workspaceId} views={state} />
      <WindowRequests workspaceId={workspaceId} views={state} registry={registry} timers={checkTimers} />
      {file.source}
    </div>
  );
}

/** The mode line and the frame's `data-mode`: a mode change (focusing the composer) renders them, not the frame. */
function FrameModeLine({
  appRef,
  bindings,
  views,
  whichKey,
}: {
  appRef: RefObject<HTMLDivElement | null>;
  bindings: readonly Binding[];
  views: ModeLineViews;
  whichKey: WhichKeyState;
}): ReactNode {
  const mode = useMode();
  const [editorMode] = useWindowState(EDITOR_MODE);
  // After the frame's ref is set (a child's layout effect runs before it on mount).
  useEffect(() => {
    appRef.current?.setAttribute("data-mode", mode);
  }, [appRef, mode]);
  return (
    <WindowModeLine mode={mode} vim={mode === "insert" ? editorMode : undefined} bindings={bindings} views={views}>
      <CommandLine bindings={bindings} />
      <WhichKey state={whichKey} bindings={bindings} />
      <FilePickerLine />
      <ThreadPicker />
      <ViewPicker views={views} />
    </WindowModeLine>
  );
}

/** The views of the workspace's manifest; the default view without one, before it arrives, or with the daemon's
 * error (AGT-011). */
function viewsOf(manifest: unknown, error: string | null | undefined): Views {
  if (error !== undefined && error !== null) {
    return { ...resolveViews(undefined), error };
  }
  return resolveViews(manifest);
}

/**
 * The window's widget reports (AGT-026): tells the reporter which workspace it shows and the views topic position it
 * rendered, follows the window's console errors, and sends the report again on every new connection (the daemon
 * forgets a closed connection's report, PRT-043).
 */
function useWindowReports(workspaceId: string | undefined): void {
  const { reports } = useHost();
  const position = useSnapshot((snapshot) =>
    workspaceId === undefined
      ? undefined
      : snapshot.topics.find((topic) => topic.topic === viewsTopic(workspaceId))?.position,
  );
  const epoch = position?.epoch;
  const seq = position?.seq;
  useEffect(() => {
    reports.show(workspaceId, epoch === undefined || seq === undefined ? undefined : { epoch, seq });
  }, [reports, workspaceId, epoch, seq]);
  useEffect(() => reports.watchConsole(window), [reports]);
  const hello = useSnapshot((snapshot) =>
    snapshot.connection.kind === "connected" ? snapshot.connection.hello : undefined,
  );
  useEffect(() => {
    if (hello !== undefined) {
      reports.resend();
    }
  }, [reports, hello]);
}

/**
 * Tells the daemon the errors of the init files and of the TSX view this window shows (UI-304), with its widget
 * reports (`widgets.report` `fileErrors`), so agents read them (`views.errors`).
 */
function useFileErrorReports(errors: readonly FileErrorReport[]): void {
  const { reports } = useHost();
  useEffect(() => reports.setFileErrors(errors), [reports, errors]);
}

/**
 * One global key handler for the window (UI-095), showing waiting sequences in the which-key panel (UI-320); a click
 * or the window losing the focus drops a waiting sequence (UI-322).
 */
function useKeymap(bindings: readonly Binding[], now: () => number, timers: KeyTimers, whichKey: WhichKeyState): void {
  const { commands } = useHost();
  useEffect(() => {
    const handler = createKeyHandler({
      bindings,
      commands,
      now,
      whichKey: { timers, show: (pending) => whichKey.set(pending) },
    });
    const { cancel } = handler;
    document.addEventListener("keydown", handler);
    document.addEventListener("pointerdown", cancel, true);
    window.addEventListener("blur", cancel);
    return () => {
      document.removeEventListener("keydown", handler);
      document.removeEventListener("pointerdown", cancel, true);
      window.removeEventListener("blur", cancel);
      cancel();
    };
  }, [bindings, commands, now, timers, whichKey]);
}

interface SafetyNetProps {
  readonly view: ResolvedView;
  readonly workspaceId: string | undefined;
  readonly views: ViewsState | undefined;
  readonly registry: WidgetRegistry;
}

/**
 * A view without the built-in `omfp:chat` or `omfp:interactions` can never hide an approval: the frame renders the
 * interactions itself (AGT-015). A workspace widget overriding them (AGT-022) does not count.
 */
function SafetyNet({ view, workspaceId, views, registry }: SafetyNetProps): ReactNode {
  const covered = INTERACTION_WIDGETS.some(
    (reference) =>
      containsWidget(view.tree, [reference]) &&
      resolveWidget(reference, workspaceId, views, registry).kind === "builtin",
  );
  if (covered) {
    return null;
  }
  return (
    <aside className="omfp-safety-net" data-testid="safety-net">
      <WidgetBoundary reference="omfp:interactions">
        <InteractionsWidget />
      </WidgetBoundary>
    </aside>
  );
}
