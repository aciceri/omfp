/**
 * The URLs omp opened in a browser, offered to this window (UI-346): omp opens them on the daemon's host (`login`,
 * `/mcp reauth`), so a window on another machine never sees that browser. Each URL a followed thread's notices name
 * after the window first saw the thread's state shows once, as a toast with Open and Copy staying as long as omp's
 * OAuth wait ({@link OPENED_URL_MS}); the URLs already there when the window looked are only in the transcript's card.
 * A login this window runs (UI-380) adds its URL with the provider's name and instructions, until the login ends.
 */

import { Button, Icon, Toast } from "@omfp/kit";
import { type ReactNode, useEffect, useRef } from "react";
import {
  OAuthCallbackField,
  OpenedUrlWatch,
  urlHost,
  useHost,
  useSnapshot,
  useUrlActions,
  windowKey,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";

/** omp waits 5 minutes for an OAuth callback: the toast stays as long. */
export const OPENED_URL_MS = 5 * 60_000;

/** A login's URL stays as long as the daemon waits for the login (10 minutes), unless the login ends first. */
export const LOGIN_URL_MS = 10 * 60_000;

/** The login (UI-380) whose provider page the URL is. */
export interface OpenedLogin {
  readonly name: string;
  /** What the provider's page asks of the user, as omp words it. */
  readonly instructions: string | undefined;
}

export interface OpenedUrl {
  /** Unique per window. */
  readonly key: string;
  /** The thread whose omp opened it; `undefined` for a login of this window. */
  readonly threadId: string | undefined;
  readonly url: string;
  readonly login: OpenedLogin | undefined;
}

/** The opened URLs this window shows as toasts, oldest first. */
export const OPENED_URLS = windowKey<readonly OpenedUrl[]>("openedUrls", []);

/** Watches the followed threads and adds their newly opened URLs to {@link OPENED_URLS}. */
export function OpenedUrls(): ReactNode {
  const { window } = useHost();
  const threads = useSnapshot((snapshot) => snapshot.threads);
  const watch = useRef(new OpenedUrlWatch());
  useEffect(() => {
    const fresh: OpenedUrl[] = [];
    for (const [threadId, { state }] of threads) {
      for (const url of watch.current.take(threadId, state)) {
        fresh.push({ key: `${threadId}\n${url}`, threadId, url, login: undefined });
      }
    }
    if (fresh.length > 0) {
      window.set(OPENED_URLS, (shown) => [...shown, ...fresh]);
    }
  }, [window, threads]);
  return null;
}

const text = strings.shell.frame;

/** One opened URL's toast: "omp opened <host> in the browser" (a login's: "Log in to <name> at <host>" and omp's
 * instructions), Open, Copy and `×`; for a sign-in returning to omp's loopback listener, the field its address is pasted
 * into (UI-761), whose success dismisses the toast. */
export function OpenedUrlToast({ opened, onDismiss }: { opened: OpenedUrl; onDismiss: () => void }): ReactNode {
  const urls = useUrlActions();
  return (
    <Toast
      tone="info"
      data-testid="opened-url-toast"
      data-url={opened.url}
      data-login={opened.login === undefined ? undefined : "true"}
      duration={opened.login === undefined ? OPENED_URL_MS : LOGIN_URL_MS}
      dismissLabel={text.dismiss}
      onDismiss={onDismiss}
      title={opened.url}
      action={
        <>
          <Button
            size="sm"
            variant="quiet"
            icon={<Icon name="link-2" />}
            aria-label={strings.chat.notice.openUrlLabel(opened.url)}
            onPress={() => urls.open(opened.url)}
            data-testid="opened-url-open"
          >
            {strings.chat.notice.openUrlAction}
          </Button>
          <Button
            size="sm"
            variant="quiet"
            icon={<Icon name="copy" />}
            aria-label={strings.chat.notice.copyUrlLabel(opened.url)}
            onPress={() => urls.copy(opened.url)}
            data-testid="opened-url-copy"
          >
            {strings.chat.notice.copyUrlAction}
          </Button>
        </>
      }
    >
      {opened.login === undefined ? (
        text.openedUrl(urlHost(opened.url))
      ) : (
        <>
          {strings.shell.login.url(opened.login.name, urlHost(opened.url))}
          {opened.login.instructions !== undefined && (
            <span className="omfp-login__instructions" data-testid="login-instructions">
              {opened.login.instructions}
            </span>
          )}
        </>
      )}
      <OAuthCallbackField threadId={opened.threadId} url={opened.url} onRelayed={onDismiss} />
    </Toast>
  );
}
