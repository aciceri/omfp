/** The frame's status surfaces: connection banner (UI-003), the toasts of failed actions, actions that worked, the
 * running login and opened URLs, and the live region (UI-092). */

import { Button, Disclosure, LiveRegion, Toast, ToastRegion } from "@omfp/kit";
import { type ReactNode, useEffect, useMemo, useSyncExternalStore } from "react";
import {
  ERRORS,
  requestLogin,
  SELECTED_THREAD,
  SUCCESSES,
  useAnnounce,
  useConnection,
  useHost,
  useSnapshot,
  useThreads,
  useWindowState,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { announcement, banner } from "../vm/connection.ts";
import { loginProblem } from "../vm/login.ts";
import { IndexLoginUrlToast, LOGIN_RUN, LoginProgressToast } from "./login.tsx";
import { OPENED_URLS, OpenedUrlToast } from "./opened-urls.tsx";

/** Non-blocking: the window stays usable (and keeps its drafts) while the client reconnects. */
export function ConnectionBanner(): ReactNode {
  const shown = banner(useConnection());
  if (shown === undefined) {
    return null;
  }
  return (
    <div className="omfp-banner" data-testid="connection-banner" data-tone={shown.tone}>
      {shown.text}
    </div>
  );
}

/**
 * The frame's one toast stack (bottom right): errors of failed actions as error toasts (UI-160), each going by itself
 * after a while unless hovered or focused (UI-535) — when omp says it needs a login, a short sentence with Log in and
 * omp's own output under Details (UI-387, UI-451) —, actions that worked as success toasts going by themselves (UI-385;
 * neutral ones for an action whose outcome shows elsewhere, UI-913), the running login (UI-383; one this window did not
 * start too, UI-640), then the URLs omp opened (UI-346) and the page of a login started elsewhere.
 */
export function Toasts(): ReactNode {
  const host = useHost();
  const [errors, setErrors] = useWindowState(ERRORS);
  const [successes, setSuccesses] = useWindowState(SUCCESSES);
  const [login] = useWindowState(LOGIN_RUN);
  const [opened, setOpened] = useWindowState(OPENED_URLS);
  const indexed = useSnapshot((snapshot) => snapshot.index.login !== null);
  if (errors.length === 0 && successes.length === 0 && login === undefined && !indexed && opened.length === 0) {
    return null;
  }
  return (
    <ToastRegion data-testid="toasts">
      {errors.map((error) => {
        const dismiss = () => setErrors((all) => all.filter((other) => other.key !== error.key));
        const problem = loginProblem(error.text);
        return (
          <Toast
            key={error.key}
            tone="error"
            className="omfp-error-toast"
            data-testid="error"
            dismissLabel={strings.shell.frame.dismiss}
            onDismiss={dismiss}
            action={
              problem !== undefined && (
                <Button
                  size="sm"
                  variant="primary"
                  onPress={() => {
                    dismiss();
                    requestLogin(host);
                  }}
                  data-testid="error-login"
                >
                  {strings.shell.login.logIn}
                </Button>
              )
            }
          >
            {problem === undefined ? (
              error.text
            ) : (
              <>
                <span className="omfp-error-toast__problem">{problem}</span>
                <Disclosure title={strings.shell.login.details} level={2} toggleTestId="error-details">
                  <span className="omfp-error-toast__raw" data-testid="error-raw">
                    {error.text}
                  </span>
                </Disclosure>
              </>
            )}
          </Toast>
        );
      })}
      {successes.map((success) => (
        <Toast
          key={success.key}
          tone={success.tone ?? "success"}
          data-testid="success"
          dismissLabel={strings.shell.frame.dismiss}
          onDismiss={() => setSuccesses((all) => all.filter((other) => other.key !== success.key))}
        >
          {success.text}
        </Toast>
      ))}
      <LoginProgressToast />
      {opened.map((item) => (
        <OpenedUrlToast
          key={item.key}
          opened={item}
          onDismiss={() => setOpened((all) => all.filter((other) => other.key !== item.key))}
        />
      ))}
      <IndexLoginUrlToast />
    </ToastRegion>
  );
}

/** Announces the status line (connection, selected thread's state, threads awaiting input) whenever it changes, and
 * renders the live region every widget speaks through (`useAnnounce`). */
export function StatusLiveRegion(): ReactNode {
  const { announcer } = useHost();
  const connection = useConnection();
  const threads = useThreads();
  const [selected] = useWindowState(SELECTED_THREAD);
  const announce = useAnnounce();
  const status = useMemo(
    () => announcement(connection, [...threads.values()], selected),
    [connection, threads, selected],
  );
  useEffect(() => {
    if (status !== "") {
      announce(status);
    }
  }, [status, announce]);
  const latest = useSyncExternalStore(announcer.subscribe, announcer.getSnapshot);
  return <LiveRegion message={latest.message} data-testid="live-region" />;
}
