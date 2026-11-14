/**
 * `omfp:mcp` (UI-290..UI-294): the MCP servers of the selected thread — checked by its running omp
 * (`thread.mcp_servers`, PRT-111) — or, without a thread, those configured for the current workspace
 * (`workspaces.mcp_servers`, PRT-110). One box per server, in omp's precedence order: its state mark (● connected or
 * failed, the spinner while checked), tool count, error line, command or URL, and the config file it comes from (a
 * workspace file opens in a file tab). "Reconnect" and "Reload all" restart the thread's omp, the only reconnect omp
 * offers over RPC (`thread.mcp_reconnect`, PRT-112). "Reauthorize" sends `/mcp reauth <name>` to the thread for an
 * HTTP or SSE server (UI-348); while the daemon holds a restart for new MCP tools, "Reload to load MCP tools" (or
 * "Restart to load new models and MCP tools" when a login waits for it too) does it (UI-349).
 */

import type { McpServers } from "@omfp/protocol";
import { BorderBox, Button, EmptyState, Icon, StateMark } from "omfp/kit";
import {
  MCP_IDLE,
  type McpPending,
  mcpRows,
  mcpStatus,
  pendingRestart,
  reauthPrompt,
  SELECTED_THREAD,
  strings,
  useCall,
  useCurrentWorkspaceId,
  useOpenFile,
  useReportError,
  useThreadSummary,
  useWindowState,
  type WidgetProps,
} from "omfp/sdk";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";

const text = strings.shell.mcp;

export function McpWidget(_props: WidgetProps): ReactNode {
  const [threadId] = useWindowState(SELECTED_THREAD);
  const workspaceId = useCurrentWorkspaceId();
  if (workspaceId === undefined) {
    return (
      <EmptyState icon="inbox" title={text.noWorkspace} data-testid="mcp-no-workspace">
        {text.noWorkspaceHint}
      </EmptyState>
    );
  }
  return <Mcp key={`${workspaceId}/${threadId ?? ""}`} workspaceId={workspaceId} threadId={threadId} />;
}

function Mcp({ workspaceId, threadId }: { workspaceId: string; threadId: string | undefined }): ReactNode {
  const call = useCall();
  const openFile = useOpenFile();
  const report = useReportError();
  const summary = useThreadSummary(threadId);
  // A thread that starts or stops changes what its omp can check: list again.
  const process = summary?.process.state;
  const restart = pendingRestart(summary?.restartPending);
  const [listing, setListing] = useState<McpServers | undefined>(undefined);
  const [pending, setPending] = useState<McpPending>(MCP_IDLE);
  const [error, setError] = useState<string | undefined>(undefined);
  const latest = useRef(0);

  const run = useCallback(async (next: McpPending, request: () => Promise<McpServers>) => {
    const id = ++latest.current;
    setPending(next);
    try {
      const result = await request();
      if (id === latest.current) {
        setListing(result);
        setError(undefined);
      }
    } catch (failure) {
      if (id === latest.current) {
        setError(failure instanceof Error ? failure.message : String(failure));
      }
    } finally {
      if (id === latest.current) {
        setPending(MCP_IDLE);
      }
    }
  }, []);

  const check = useCallback(
    () =>
      run({ kind: "check" }, () =>
        threadId === undefined
          ? call("workspaces.mcp_servers", { workspaceId })
          : call("thread.mcp_servers", { threadId }),
      ),
    [run, call, threadId, workspaceId],
  );

  // The restart of a reconnect moves the thread's process through `starting`: its own answer is the listing.
  const reconnecting = useRef(false);
  const reconnect = (name: string | undefined) => {
    if (threadId === undefined) {
      return;
    }
    reconnecting.current = true;
    void run({ kind: "reconnect", name }, () =>
      call("thread.mcp_reconnect", name === undefined ? { threadId } : { threadId, name }),
    ).finally(() => {
      reconnecting.current = false;
    });
  };

  useEffect(() => {
    if (!reconnecting.current && process !== "starting") {
      void check();
    }
  }, [check, process]);

  const rows = useMemo(() => (listing === undefined ? [] : mcpRows(listing, pending)), [listing, pending]);
  const busy = pending.kind !== "none";

  // The output and the URL omp opens arrive in the thread as a command card and a toast (UI-345, UI-346).
  const reauthorize = (name: string) => {
    if (threadId !== undefined) {
      call("thread.prompt", { threadId, text: reauthPrompt(name) }).catch(report);
    }
  };

  return (
    <section className="omfp-mcp" data-testid="mcp" aria-label={text.label} aria-busy={busy}>
      <div className="omfp-mcp__bar">
        <span className="omfp-mcp__status" data-testid="mcp-status">
          {listing === undefined ? text.loading : mcpStatus(listing)}
        </span>
        {threadId !== undefined && (
          <>
            <Button
              size="sm"
              variant="quiet"
              icon={<Icon name="refresh-cw" />}
              isPending={pending.kind === "check" && listing !== undefined}
              isDisabled={busy}
              onPress={() => void check()}
              data-testid="mcp-check"
            >
              {text.checkAgain}
            </Button>
            <Button
              size="sm"
              isDisabled={busy}
              onPress={() => reconnect(undefined)}
              aria-description={text.reloadAllHint}
              data-testid="mcp-reload-all"
            >
              {text.reloadAll}
            </Button>
          </>
        )}
      </div>
      {threadId !== undefined && restart?.mcp === true && (
        <div className="omfp-mcp__reload" data-testid="mcp-reload-pending">
          <Button
            size="sm"
            variant="primary"
            icon={<Icon name="refresh-cw" />}
            isDisabled={busy}
            onPress={() => reconnect(undefined)}
            aria-description={restart.hint}
            data-testid="mcp-reload-tools"
          >
            {restart.label}
          </Button>
        </div>
      )}
      {error !== undefined && (
        <p className="omfp-mcp__error" role="alert" data-testid="mcp-error">
          {error}
        </p>
      )}
      {listing !== undefined && rows.length === 0 ? (
        <EmptyState icon="inbox" title={text.none} data-testid="mcp-none">
          {text.noneHint(listing.projectConfig, listing.userConfig)}
        </EmptyState>
      ) : (
        rows.map((row) => (
          <BorderBox
            key={row.name}
            title={row.name}
            level={2}
            end={
              <span className="omfp-mcp__state" data-testid="mcp-state">
                <StateMark state={row.mark} />
                {row.stateLabel}
                {row.tools !== undefined && <span className="omfp-mcp__tools"> · {row.tools}</span>}
              </span>
            }
            className="omfp-mcp__server"
            data-testid="mcp-server"
            data-name={row.name}
            data-state={row.mark}
            data-disabled={row.disabled || undefined}
          >
            <p className="omfp-mcp__target" data-testid="mcp-target">
              <span className="omfp-mcp__transport">{row.transport}</span>
              <span className="omfp-mcp__command" title={row.target}>
                {row.target}
              </span>
            </p>
            {row.error !== undefined && (
              <p className="omfp-mcp__server-error" data-testid="mcp-server-error">
                {row.error}
              </p>
            )}
            {row.note !== undefined && <p className="omfp-mcp__note">{row.note}</p>}
            <p className="omfp-mcp__source">
              {row.openPath === undefined ? (
                <span className="omfp-mcp__path" data-testid="mcp-source">
                  {row.source}
                </span>
              ) : (
                <button
                  type="button"
                  className="omfp-mcp__path omfp-mcp__link"
                  aria-label={text.openSource(row.source)}
                  onClick={() => openFile(row.openPath ?? row.source)}
                  data-testid="mcp-source"
                >
                  {row.source}
                </button>
              )}
              <span className="omfp-mcp__origin">{row.origin}</span>
              {threadId !== undefined && !row.disabled && (
                <Button
                  size="sm"
                  isDisabled={busy}
                  onPress={() => reconnect(row.name)}
                  aria-label={text.reconnectLabel(row.name)}
                  data-testid="mcp-reconnect"
                >
                  {text.reconnect}
                </Button>
              )}
              {threadId !== undefined && row.reauthorizable && (
                <Button
                  size="sm"
                  variant="quiet"
                  isDisabled={busy}
                  onPress={() => reauthorize(row.name)}
                  aria-label={text.reauthorizeLabel(row.name)}
                  data-testid="mcp-reauthorize"
                >
                  {text.reauthorize}
                </Button>
              )}
            </p>
          </BorderBox>
        ))
      )}
    </section>
  );
}
