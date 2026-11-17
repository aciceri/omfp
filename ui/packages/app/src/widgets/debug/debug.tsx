/**
 * `omfp:debug` (UI-946..UI-950): the selected thread's debug (DAP) session from its `debug` tool results — status,
 * the stop location, the stack and the variables the agent last read at that stop, and the breakpoints — each place a
 * link that opens the file at its line. omp offers no debugger control over RPC, so the widget only shows.
 */

import { BorderBox, EmptyState, type OpenPath, PathLink } from "omfp/kit";
import {
  type DebugLocation,
  debugView,
  SELECTED_THREAD,
  strings,
  useOpenFile,
  useThread,
  useWindowState,
  type WidgetProps,
} from "omfp/sdk";
import { type ReactNode, useCallback, useMemo } from "react";

const text = strings.shell.debug;

function Place({ location, label, onOpen }: { location: DebugLocation | undefined; label: string; onOpen: OpenPath }) {
  return location === undefined ? (
    <span className="omfp-debug__place">{label}</span>
  ) : (
    <PathLink reference={location} onOpen={onOpen} data-testid="debug-place">
      {label}
    </PathLink>
  );
}

export function DebugWidget(_props: WidgetProps): ReactNode {
  const [threadId] = useWindowState(SELECTED_THREAD);
  const transcript = useThread(threadId)?.state.transcript;
  const view = useMemo(() => (transcript === undefined ? undefined : debugView({ transcript })), [transcript]);
  const openFile = useOpenFile();
  const onOpen = useCallback<OpenPath>((ref) => openFile(ref.path, { line: ref.line }), [openFile]);
  if (threadId === undefined) {
    return <EmptyState icon="inbox" title={text.noThread} data-testid="debug-no-thread" />;
  }
  if (view === undefined) {
    return (
      <EmptyState icon="play" title={text.none} data-testid="debug-none">
        {text.noneHint}
      </EmptyState>
    );
  }
  const { stop } = view;
  return (
    <section className="omfp-debug" data-testid="debug" aria-label={text.label}>
      <BorderBox
        title={view.program ?? view.adapter}
        level={2}
        end={view.exitCode === undefined ? view.statusLabel : `${view.statusLabel}, ${text.exitCode(view.exitCode)}`}
        className="omfp-debug__session"
        data-testid="debug-session"
        data-status={view.status}
      >
        <p className="omfp-debug__meta">{view.adapter}</p>
        {stop !== undefined && (
          <p className="omfp-debug__stop" data-testid="debug-stop">
            {text.stoppedAt} <Place location={stop.location} label={stop.where} onOpen={onOpen} />
            {stop.frame !== undefined && stop.location !== undefined && ` (${stop.frame})`}
            {stop.reason !== undefined && ` ${text.reason(stop.reason)}`}
          </p>
        )}
      </BorderBox>
      <BorderBox title={text.stack} level={2} data-testid="debug-stack">
        {view.stack.length === 0 ? (
          <p className="omfp-debug__empty">{text.noStack}</p>
        ) : (
          <ol className="omfp-debug__list">
            {view.stack.map((frame) => (
              <li key={frame.id} data-testid="debug-frame">
                <span className="omfp-debug__name">{frame.name}</span>{" "}
                <Place location={frame.location} label={frame.where} onOpen={onOpen} />
              </li>
            ))}
          </ol>
        )}
      </BorderBox>
      <BorderBox title={text.variables} level={2} data-testid="debug-variables">
        {view.variables.length === 0 ? (
          <p className="omfp-debug__empty">{text.noVariables}</p>
        ) : (
          <dl className="omfp-debug__vars">
            {view.variables.map((variable) => (
              <div key={variable.name} data-testid="debug-variable">
                <dt>
                  {variable.name}
                  {variable.type !== undefined && <span className="omfp-debug__type">: {variable.type}</span>}
                </dt>
                <dd>{variable.value}</dd>
              </div>
            ))}
          </dl>
        )}
      </BorderBox>
      <BorderBox title={text.breakpoints} level={2} data-testid="debug-breakpoints">
        {view.breakpoints.length === 0 ? (
          <p className="omfp-debug__empty">{text.noBreakpoints}</p>
        ) : (
          <ul className="omfp-debug__list">
            {view.breakpoints.map((breakpoint) => (
              <li
                key={breakpoint.label}
                data-testid="debug-breakpoint"
                data-verified={breakpoint.verified ? "" : undefined}
              >
                <Place location={breakpoint.location} label={breakpoint.label} onOpen={onOpen} />
                {breakpoint.condition !== undefined && (
                  <code className="omfp-debug__condition"> if {breakpoint.condition}</code>
                )}
                {!breakpoint.verified && (
                  <span className="omfp-debug__unverified">
                    {" "}
                    {text.unverified}
                    {breakpoint.message === undefined ? "" : `: ${breakpoint.message}`}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </BorderBox>
    </section>
  );
}
