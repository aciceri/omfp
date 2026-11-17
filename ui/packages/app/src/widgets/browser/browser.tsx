/**
 * `omfp:browser` (UI-870..UI-873): the browser tabs the selected thread's agent drove with omp's browser, from its
 * `eval` calls (`browserView`): a box per tab titled on its border (UI-187) with its address, its last action and the
 * latest screenshot its calls produced, which opens at full size in a dialog. omp exposes no live page over RPC, so
 * this shows what the calls reported.
 */

import { BorderBox, Button, Dialog, EmptyState } from "omfp/kit";
import {
  type BrowserScreenshot,
  type BrowserTab,
  browserView,
  SELECTED_THREAD,
  strings,
  useThread,
  useWindowState,
  type WidgetProps,
} from "omfp/sdk";
import { type ReactNode, useMemo, useState } from "react";

const text = strings.shell.browser;

/** The `src` of a screenshot: its base64 data as a data URL. */
function imageSource(image: BrowserScreenshot): string {
  return `data:${image.mime};base64,${image.data}`;
}

/** An address the window may open: http(s) only, never `javascript:` or `data:` from a page the agent visited. */
function linkable(url: string): boolean {
  return /^https?:\/\//i.test(url);
}

export function BrowserWidget(_props: WidgetProps): ReactNode {
  const [threadId] = useWindowState(SELECTED_THREAD);
  const state = useThread(threadId)?.state;
  const transcript = state?.transcript;
  const tools = state?.tools;
  const view = useMemo(
    () => (transcript === undefined || tools === undefined ? undefined : browserView({ transcript, tools })),
    [transcript, tools],
  );
  // The tab as it was when its screenshot was opened: a newer screenshot does not replace the one being looked at.
  const [opened, setOpened] = useState<BrowserTab | undefined>(undefined);
  if (threadId === undefined) {
    return <EmptyState icon="inbox" title={text.noThread} data-testid="browser-no-thread" />;
  }
  return (
    <section className="omfp-browser" data-testid="browser" aria-label={text.label}>
      {view === undefined || view.tabs.length === 0 ? (
        <EmptyState icon="globe" title={text.none} data-testid="browser-none">
          {text.noneHint}
        </EmptyState>
      ) : (
        <>
          <p className="omfp-browser__note">{text.note}</p>
          {view.tabs.map((tab) => (
            <BorderBox
              key={tab.name}
              title={tab.name}
              level={2}
              end={tab.closed ? text.closed : undefined}
              data-testid="browser-tab"
              data-tab={tab.name}
              data-state={tab.closed ? "closed" : "open"}
            >
              {tab.url === undefined ? (
                <p className="omfp-browser__meta">{text.noUrl}</p>
              ) : linkable(tab.url) ? (
                <a
                  className="omfp-browser__url"
                  href={tab.url}
                  target="_blank"
                  rel="noreferrer"
                  data-testid="browser-url"
                >
                  {tab.url}
                </a>
              ) : (
                <p className="omfp-browser__url" data-testid="browser-url">
                  {tab.url}
                </p>
              )}
              {tab.url !== undefined && tab.movedBy !== undefined && (
                <p className="omfp-browser__action" data-testid="browser-moved">
                  <span className="omfp-browser__meta">{text.movedBy}</span>{" "}
                  <code title={tab.movedBy}>{tab.movedBy}</code>
                </p>
              )}
              <p className="omfp-browser__action">
                <span className="omfp-browser__meta">{text.lastAction}</span>{" "}
                <code title={tab.action} data-testid="browser-action">
                  {tab.action}
                </code>
              </p>
              {tab.screenshot === undefined ? (
                <p className="omfp-browser__meta" data-testid="browser-no-screenshot">
                  {text.noScreenshot}
                </p>
              ) : (
                <Button
                  variant="quiet"
                  className="omfp-browser__thumb"
                  aria-label={text.openScreenshot(tab.name)}
                  onPress={() => setOpened(tab)}
                  data-testid="browser-screenshot"
                >
                  <img
                    className="omfp-browser__img"
                    src={imageSource(tab.screenshot)}
                    alt={text.screenshot(tab.name)}
                    decoding="async"
                  />
                </Button>
              )}
            </BorderBox>
          ))}
        </>
      )}
      <Dialog
        isOpen={opened?.screenshot !== undefined}
        onOpenChange={(open) => {
          if (!open) {
            setOpened(undefined);
          }
        }}
        isDismissable
        title={opened === undefined ? "" : text.screenshot(opened.name)}
        className="omfp-image-dialog"
        data-testid="browser-screenshot-dialog"
      >
        {opened?.screenshot !== undefined && (
          <img
            className="omfp-image-dialog__image"
            src={imageSource(opened.screenshot)}
            alt={text.screenshot(opened.name)}
            data-testid="browser-screenshot-full"
          />
        )}
      </Dialog>
    </section>
  );
}
