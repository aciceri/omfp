/**
 * Logging in to a provider from the window (UI-380..UI-386): omp keeps its credentials on the daemon's host, and a
 * fresh install has none, so omp refuses to start a thread until the user logs in. A {@link LOGIN} request (`:login`,
 * the Log in buttons) starts `auth.login` for a provider, or opens the provider picker first. While the login runs the
 * frame's toasts show its progress with Cancel; the daemon's `login_url`, `login_progress` and `login_input` requests
 * open the provider's page (and its toast), update the progress and ask for a pasted code. A login this window did not
 * start (an agent's `auth.login`, another window's) shows from the `index` topic's `login` (UI-640): the same progress
 * toast and the page's toast. `:logout` removes omp's stored credentials of a provider. `:import-logins` (and the
 * welcome's offer) copies standalone omp's logins (UI-505, UI-506).
 */

import { Button, Dialog, Icon, ListBox, ListBoxItem, TextField, Toast } from "@omfp/kit";
import type { AuthProvider, LoginActivity, WindowAnswer } from "@omfp/protocol";
import { type FormEvent, type ReactNode, useEffect, useState } from "react";
import { keyLabel } from "../keymap/keymap.ts";
import {
  AUTH_REVISION,
  errorMessage,
  LOGIN,
  type OmfpHost,
  registerCommand,
  reportError,
  reportSuccess,
  reportTaken,
  requestLogin,
  urlHost,
  useAnnounce,
  useHost,
  useSnapshot,
  useUrlActions,
  useWindowState,
  windowKey,
} from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import {
  importedSummary,
  loginCompletions,
  logoutCompletions,
  logoutSummary,
  providerName,
  withAuthenticated,
} from "../vm/login.ts";
import { OPENED_URLS, OpenedUrlToast } from "./opened-urls.tsx";

/** The running login of the `index` topic (DMN-290), whoever started it; `undefined` when none runs. */
function useIndexLogin(): LoginActivity | undefined {
  return useSnapshot((snapshot) => snapshot.index.login ?? undefined);
}

const text = strings.shell.login;

/** The providers of `auth.providers`, asked once per window (UI-382, UI-386). */
export type ProvidersState =
  | { readonly kind: "idle" }
  | { readonly kind: "loading" }
  | { readonly kind: "loaded"; readonly providers: readonly AuthProvider[] }
  | { readonly kind: "failed"; readonly message: string };

export const AUTH_PROVIDERS = windowKey<ProvidersState>("authProviders", { kind: "idle" });

/** The login this window runs (UI-383): `auth.login` is pending. */
export interface LoginRun {
  /** Unique per page: tells a login from the one that replaced it. */
  readonly id: number;
  readonly providerId: string;
  readonly name: string;
  /** omp's latest `login_progress` message. */
  readonly progress: string | undefined;
  /** The user cancelled it (Cancel, or the paste dialog's Cancel): its rejection is no error. */
  readonly cancelled: boolean;
}

export const LOGIN_RUN = windowKey<LoginRun | undefined>("loginRun", undefined);

/** The `auth.providers` list, when known. */
function knownProviders(host: OmfpHost): readonly AuthProvider[] | undefined {
  const state = host.window.get(AUTH_PROVIDERS);
  return state.kind === "loaded" ? state.providers : undefined;
}

/** Asks the daemon for the providers once (again after a failure); it takes a few seconds (a short-lived omp). */
export function loadProviders(host: OmfpHost): void {
  const state = host.window.get(AUTH_PROVIDERS);
  if (state.kind === "loading" || state.kind === "loaded") {
    return;
  }
  host.window.set(AUTH_PROVIDERS, { kind: "loading" });
  host.client.call("auth.providers", {}).then(
    ({ providers }) => host.window.set(AUTH_PROVIDERS, { kind: "loaded", providers }),
    (error: unknown) => host.window.set(AUTH_PROVIDERS, { kind: "failed", message: errorMessage(error) }),
  );
}

/** Records that omp now holds (or no longer holds) a credential of `providerId`, and bumps {@link AUTH_REVISION}. */
function authChanged(host: OmfpHost, providerId: string, authenticated: boolean): void {
  const providers = knownProviders(host);
  if (providers !== undefined) {
    host.window.set(AUTH_PROVIDERS, {
      kind: "loaded",
      providers: withAuthenticated(providers, providerId, authenticated),
    });
  }
  host.window.set(AUTH_REVISION, (revision) => revision + 1);
}

let lastRun = 0;

/** The key prefix of the opened-URL toasts of login `id`, dropped when it ends. */
const urlKey = (id: number) => `login/${id}/`;

/** Ends login `id`: the progress toast and its URL toasts go, unless another login replaced it. */
function finish(host: OmfpHost, id: number): void {
  if (host.window.get(LOGIN_RUN)?.id === id) {
    host.window.set(LOGIN_RUN, undefined);
  }
  host.window.set(OPENED_URLS, (shown) => shown.filter((opened) => !opened.key.startsWith(urlKey(id))));
}

/**
 * Runs `auth.login` for `providerId` (UI-383, UI-385): a success shows "Logged in to <name>" and bumps
 * {@link AUTH_REVISION} so the model lists are read again; a failure goes to the error toasts unless the user cancelled
 * it or another login replaced it (the daemon cancels the earlier one).
 */
export function startLogin(host: OmfpHost, providerId: string): void {
  lastRun += 1;
  const id = lastRun;
  const name = providerName(knownProviders(host), providerId);
  host.window.set(LOGIN_RUN, { id, providerId, name, progress: undefined, cancelled: false });
  host.client.call("auth.login", { providerId }).then(
    (result) => {
      finish(host, id);
      authChanged(host, result.providerId, true);
      reportSuccess(host, text.loggedIn(result.name));
    },
    (error: unknown) => {
      const run = host.window.get(LOGIN_RUN);
      finish(host, id);
      if (run?.id === id && !run.cancelled) {
        reportError(host, error);
      }
    },
  );
}

/** Cancels the running login (`auth.login_cancel`, UI-383); its rejection is then no error. */
export function cancelLogin(host: OmfpHost): void {
  host.window.set(LOGIN_RUN, (run) => (run === undefined ? run : { ...run, cancelled: true }));
  host.client.call("auth.login_cancel", {}).catch((error: unknown) => reportError(host, error));
}

/** Logs out of `providerId` (`auth.logout`, UI-386) and says what omp removed and what still authenticates it. */
async function logout(host: OmfpHost, providerId: string): Promise<void> {
  try {
    const result = await host.client.call("auth.logout", { providerId });
    const remaining = result.remainingSource;
    authChanged(host, providerId, remaining !== null && remaining !== undefined && remaining !== "");
    reportSuccess(host, logoutSummary(result, providerName(knownProviders(host), providerId)));
  } catch (error) {
    reportError(host, error);
  }
}

/** Whether this window's copy of standalone omp's logins (`auth.import`, UI-506) runs. */
export const IMPORT_RUN = windowKey<boolean>("importRun", false);

/**
 * Copies standalone omp's logins into omfp's omp (`auth.import`, UI-505, UI-506), once at a time: a copy says which
 * providers it copied (and the OAuth caveat) and bumps {@link AUTH_REVISION}; one that found nothing says so in a
 * neutral toast, and a failure (a login running: `conflict`) is an error toast with omp's or the daemon's words.
 */
export async function importLogins(host: OmfpHost): Promise<void> {
  if (host.window.get(IMPORT_RUN)) {
    return;
  }
  host.window.set(IMPORT_RUN, true);
  try {
    const { providers } = await host.client.call("auth.import", {});
    if (providers.length === 0) {
      reportTaken(host, text.nothingToImport);
    } else {
      host.window.set(AUTH_REVISION, (revision) => revision + 1);
      reportSuccess(host, importedSummary(providers));
    }
  } catch (error) {
    reportError(host, error);
  } finally {
    host.window.set(IMPORT_RUN, false);
  }
}

/** A `login_input` request waiting for its answer (UI-384). */
interface InputRequest {
  readonly requestId: string;
  readonly name: string;
  readonly message: string;
  readonly placeholder: string | undefined;
}

/**
 * The window's side of a login: takes {@link LOGIN} requests, registers `:login`, `:logout` and `:import-logins`,
 * follows the daemon's login requests and renders the provider picker and the paste dialog. Mounted once, by the frame.
 */
export function Login(): ReactNode {
  const host = useHost();
  const urls = useUrlActions();
  const announce = useAnnounce();
  const [request, setRequest] = useWindowState(LOGIN);
  const [providers] = useWindowState(AUTH_PROVIDERS);
  const [run] = useWindowState(LOGIN_RUN);
  const [openedUrls] = useWindowState(OPENED_URLS);
  const indexed = useIndexLogin();
  const page =
    run === undefined
      ? (indexed?.url ?? undefined)
      : openedUrls.find((opened) => opened.key.startsWith(urlKey(run.id)))?.url;
  const [picking, setPicking] = useState(false);
  const [input, setInput] = useState<InputRequest | undefined>(undefined);

  useEffect(() => {
    if (request === undefined) {
      return;
    }
    setRequest(undefined);
    if (request.providerId === undefined) {
      setPicking(true);
      loadProviders(host);
    } else {
      startLogin(host, request.providerId);
    }
  }, [host, request, setRequest]);

  // The completions follow the providers: registered again once they are known, so an open `:` line shows them.
  const known = providers.kind === "loaded" ? providers.providers : undefined;
  useEffect(() => {
    // Asked on the first completion, outside the `:` line's render.
    const asked = () => queueMicrotask(() => loadProviders(host));
    const stopLogin = registerCommand(host, {
      id: "auth.login",
      title: strings.shell.commands.login,
      aliases: ["login"],
      args: (before) => {
        asked();
        return loginCompletions(known, before);
      },
      run: ([providerId]) => requestLogin(host, providerId),
    });
    const stopLogout = registerCommand(host, {
      id: "auth.logout",
      title: strings.shell.commands.logout,
      aliases: ["logout"],
      args: (before) => {
        asked();
        return logoutCompletions(known, before);
      },
      run: async ([providerId, ...rest]) => {
        if (providerId === undefined || rest.length > 0) {
          reportError(host, text.logoutUsage);
          return;
        }
        await logout(host, providerId);
      },
    });
    const stopImport = registerCommand(host, {
      id: "auth.import",
      title: strings.shell.commands.importLogins,
      description: strings.shell.commands.importLoginsDescription,
      aliases: ["import-logins"],
      run: () => importLogins(host),
    });
    return () => {
      stopLogin();
      stopLogout();
      stopImport();
    };
  }, [host, known]);

  useEffect(
    () =>
      host.client.onWindowRequest(({ id, request: asked }) => {
        switch (asked.kind) {
          case "login_url": {
            const current = host.window.get(LOGIN_RUN);
            urls.open(asked.url);
            // A login this window did not start shows its page's toast from the index (UI-640).
            if (current !== undefined) {
              host.window.set(OPENED_URLS, (shown) => [
                ...shown,
                {
                  key: `${urlKey(current.id)}${id}`,
                  threadId: undefined,
                  url: asked.url,
                  login: { name: asked.name, instructions: asked.instructions ?? undefined },
                },
              ]);
            }
            return;
          }
          case "login_progress":
            host.window.set(LOGIN_RUN, (current) =>
              current === undefined ? current : { ...current, progress: asked.message },
            );
            announce(asked.message);
            return;
          case "login_input":
            setInput({
              requestId: id,
              name: asked.name,
              message: asked.message,
              placeholder: asked.placeholder ?? undefined,
            });
            return;
          default:
            return;
        }
      }),
    [host, urls, announce],
  );

  // A login that ended takes its unanswered paste dialog with it: this window's, or one it did not start (UI-640).
  const running = run !== undefined || indexed !== undefined;
  useEffect(() => {
    if (!running) {
      setInput(undefined);
    }
  }, [running]);

  // Cancel ends the login itself: omp's OAuth flow would otherwise keep waiting for its callback and ask again.
  const answer = (requestId: string, value: string | null) => {
    setInput(undefined);
    const answered: WindowAnswer = { kind: "login_input", value };
    host.client.call("windows.answer", { requestId, answer: answered }).catch((error: unknown) => {
      reportError(host, error);
    });
    if (value === null) {
      cancelLogin(host);
    }
  };

  return (
    <>
      <ProviderPicker
        isOpen={picking}
        providers={providers}
        onClose={() => setPicking(false)}
        onPick={(providerId) => {
          setPicking(false);
          startLogin(host, providerId);
        }}
      />
      <Dialog
        isOpen={input !== undefined}
        onOpenChange={(open) => {
          if (!open && input !== undefined) {
            answer(input.requestId, null);
          }
        }}
        title={text.inputTitle(input?.name ?? "")}
        data-testid="login-input-dialog"
      >
        {input !== undefined && (
          <LoginInputForm
            key={input.requestId}
            page={page}
            input={input}
            onSubmit={(value) => answer(input.requestId, value)}
            onCancel={() => answer(input.requestId, null)}
          />
        )}
      </Dialog>
    </>
  );
}

/** The provider picker (UI-382): omp's providers with their ids, a mark on those omp holds a credential for, the ones
 * it cannot log in to here disabled; a pick starts the login. Each row puts the name on its own line and the id and
 * status on a subordinate one under it; the row grows with its wrapped text. */
function ProviderPicker({
  isOpen,
  providers,
  onClose,
  onPick,
}: {
  isOpen: boolean;
  providers: ProvidersState;
  onClose: () => void;
  onPick: (providerId: string) => void;
}): ReactNode {
  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
      isDismissable
      title={text.pickerTitle}
      className="omfp-login__picker"
      data-testid="login-picker"
    >
      <div className="omfp-dialog__message">
        {providers.kind === "failed" ? (
          <p className="omfp-dialog__error" role="alert" data-testid="login-providers-error">
            {text.providersFailed(providers.message)}
          </p>
        ) : providers.kind !== "loaded" ? (
          <p className="omfp-login__loading" data-testid="login-providers-loading">
            <Icon name="loader-circle" className="omfp-login__spinner" />
            {text.loadingProviders}
          </p>
        ) : providers.providers.length === 0 ? (
          <p data-testid="login-providers-empty">{text.noProviders}</p>
        ) : (
          <ListBox
            aria-label={text.pickerLabel}
            items={providers.providers}
            disabledKeys={providers.providers.filter((provider) => !provider.available).map((provider) => provider.id)}
            onAction={(key) => onPick(String(key))}
            data-testid="login-providers"
          >
            {(provider) => (
              <ListBoxItem
                id={provider.id}
                textValue={provider.name}
                className="omfp-login__provider"
                data-testid="login-provider"
                data-provider={provider.id}
              >
                <span className="omfp-login__name" data-testid="login-provider-name">
                  {provider.name}
                </span>
                <span className="omfp-login__meta" data-testid="login-provider-meta">
                  <code className="omfp-login__id">{provider.id}</code>
                  {provider.authenticated && (
                    <span className="omfp-login__mark" data-testid="login-authenticated">
                      <Icon name="circle-check" />
                      {text.authenticated}
                    </span>
                  )}
                  {!provider.available && <span className="omfp-login__note">{text.unavailable}</span>}
                </span>
              </ListBoxItem>
            )}
          </ListBox>
        )}
      </div>
      <div className="omfp-dialog__actions">
        <Button variant="quiet" onPress={onClose} kbd={keyLabel("Escape")} data-testid="login-picker-cancel">
          {text.cancel}
        </Button>
      </div>
    </Dialog>
  );
}

/** The paste dialog's form (UI-384): first the provider's page, whose Open stands out until pressed (a window may not
 * open it by itself: the desktop's web view lets only a click open a page), then the field omp's message labels;
 * Enter submits, Escape or Cancel cancels. */
function LoginInputForm({
  input,
  page,
  onSubmit,
  onCancel,
}: {
  input: InputRequest;
  page: string | undefined;
  onSubmit: (value: string) => void;
  onCancel: () => void;
}): ReactNode {
  const urls = useUrlActions();
  const [value, setValue] = useState("");
  const [opened, setOpened] = useState(false);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const typed = value.trim();
    if (typed !== "") {
      onSubmit(typed);
    }
  };
  const opening = page !== undefined && !opened;
  return (
    <form onSubmit={submit}>
      {page !== undefined && (
        <div className="omfp-login__page" data-testid="login-page">
          <span>{text.pageStep(input.name, urlHost(page))}</span>
          <Button
            type="button"
            variant={opening ? "primary" : "secondary"}
            icon={<Icon name="external-link" />}
            onPress={() => {
              setOpened(true);
              urls.open(page);
            }}
            data-testid="login-page-open"
          >
            {text.openPage}
          </Button>
          <Button type="button" variant="quiet" onPress={() => urls.copy(page)} data-testid="login-page-copy">
            {strings.chat.notice.copyUrlAction}
          </Button>
        </div>
      )}
      <TextField
        label={input.message}
        value={value}
        onChange={setValue}
        autoComplete="off"
        // The dialog asks for this one field
        autoFocus
        mono
        {...(input.placeholder === undefined ? {} : { placeholder: input.placeholder })}
        inputProps={{ "data-testid": "login-input" }}
      />
      <div className="omfp-dialog__actions">
        <Button variant="quiet" onPress={onCancel} kbd={keyLabel("Escape")} data-testid="login-input-cancel">
          {text.cancel}
        </Button>
        <Button
          type="submit"
          variant={opening ? "secondary" : "primary"}
          kbd={keyLabel("Enter")}
          data-testid="login-input-submit"
        >
          {text.submit}
        </Button>
      </div>
    </form>
  );
}

/** The progress toast while a login runs (UI-383): "Logging in to <name>… finish in your browser", omp's latest
 * progress message and Cancel; it stays until the login ends. A login this window did not start shows the index's
 * (UI-640). */
export function LoginProgressToast(): ReactNode {
  const host = useHost();
  const [run] = useWindowState(LOGIN_RUN);
  const indexed = useIndexLogin();
  const shown =
    run ??
    (indexed === undefined
      ? undefined
      : {
          providerId: indexed.providerId,
          name: indexed.name,
          progress: indexed.progress ?? undefined,
          cancelled: false,
        });
  if (shown === undefined) {
    return null;
  }
  return (
    <Toast
      tone="info"
      data-testid="login-progress"
      data-provider={shown.providerId}
      data-own={run === undefined ? "false" : "true"}
      action={
        <Button
          size="sm"
          variant="quiet"
          icon={<Icon name="x" />}
          aria-label={text.cancelLoginLabel(shown.name)}
          isPending={shown.cancelled}
          onPress={() => cancelLogin(host)}
          data-testid="login-cancel"
        >
          {text.cancelLogin}
        </Button>
      }
    >
      {text.progress(shown.name)}
      {shown.progress !== undefined && (
        <span className="omfp-login__progress" data-testid="login-progress-message">
          {shown.progress}
        </span>
      )}
    </Toast>
  );
}

/** Dismissed page toasts of logins this window did not start, by URL. */
const DISMISSED_LOGIN_URLS = windowKey<readonly string[]>("dismissedLoginUrls", []);

/** The page of a login this window did not start (UI-640): its opened-URL toast — "Log in to <name> at <host>", omp's
 * instructions, Open and Copy — while the index lists the login with a page, unless dismissed. */
export function IndexLoginUrlToast(): ReactNode {
  const [run] = useWindowState(LOGIN_RUN);
  const indexed = useIndexLogin();
  const [dismissed, setDismissed] = useWindowState(DISMISSED_LOGIN_URLS);
  const url = indexed?.url ?? undefined;
  if (run !== undefined || indexed === undefined || url === undefined || dismissed.includes(url)) {
    return null;
  }
  return (
    <OpenedUrlToast
      opened={{
        key: `login/index/${url}`,
        threadId: undefined,
        url,
        login: { name: indexed.name, instructions: indexed.instructions ?? undefined },
      }}
      onDismiss={() => setDismissed((all) => [...all, url])}
    />
  );
}
