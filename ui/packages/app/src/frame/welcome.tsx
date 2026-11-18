/**
 * The first run (UI-500..UI-505): on a fresh install omp has no model it can use, so every thread start fails until
 * the user logs in. The window asks the daemon how many models omp can use (`auth.status`, PRT-140) and follows the
 * count on the `index` topic (PRT-141); with none and no thread selected, the main area welcomes the user and offers
 * the login instead of a view of empty panels, and the copy of standalone omp's logins when it holds some. With usable
 * models, or while the count is unknown, nothing changes.
 */

import { Button, Wordmark } from "@omfp/kit";
import type { OmpLogin } from "@omfp/protocol";
import { type ReactNode, useEffect, useId } from "react";
import { requestLogin, useConnection, useHost, useSnapshot, useWindowState, windowKey } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { importOffer } from "../vm/login.ts";
import { IMPORT_RUN, importLogins, LOGIN_RUN } from "./login.tsx";

const text = strings.shell.welcome;

/**
 * Whether the main area shows the welcome (UI-500): omp has no usable model and no thread is selected. While the
 * index does not know the count, asks `auth.status` once per connection (UI-502); the daemon publishes its answer on
 * the index, and a failed check changes nothing.
 */
export function useWelcome(selectedThread: string | undefined): boolean {
  const host = useHost();
  const connection = useConnection();
  const models = useSnapshot((snapshot) => snapshot.index.models);
  const connected = connection.kind === "connected" ? connection.hello.connection : undefined;
  const unknown = models === null;
  useEffect(() => {
    if (connected === undefined || !unknown) {
      return;
    }
    host.client.call("auth.status", {}).catch(() => {
      // omp could not start (e.g. it is missing): the frame stays as it is, and a thread's start says why.
    });
  }, [host, connected, unknown]);
  return models === 0 && selectedThread === undefined;
}

/**
 * Standalone omp's logins the window could copy (`auth.importable`), asked on the connection whose `system.hello`
 * answer is `hello`: a new connection is a new answer even when the daemon reuses its connection number.
 */
interface Importable {
  readonly hello: object;
  readonly providers: readonly OmpLogin[];
}

const IMPORTABLE = windowKey<Importable | undefined>("importable", undefined);

/**
 * Standalone omp's logins (UI-505): asked with `auth.importable` once per connection while the welcome shows; none
 * until the daemon answers, none when it fails, and an answer from an earlier connection is dropped.
 */
function useImportable(): readonly OmpLogin[] {
  const host = useHost();
  const connection = useConnection();
  const hello = connection.kind === "connected" ? connection.hello : undefined;
  const [importable] = useWindowState(IMPORTABLE);
  useEffect(() => {
    if (hello === undefined || host.window.get(IMPORTABLE)?.hello === hello) {
      return;
    }
    host.window.set(IMPORTABLE, { hello, providers: [] });
    host.client.call("auth.importable", {}).then(
      ({ providers }) =>
        host.window.set(IMPORTABLE, (asked) => (asked?.hello === hello ? { ...asked, providers } : asked)),
      () => {
        // Standalone omp could not tell (e.g. its export failed): the welcome offers the login only.
      },
    );
  }, [host, hello]);
  return importable !== undefined && importable.hello === hello ? importable.providers : [];
}

/** The welcome's offer to copy standalone omp's `logins` (UI-505), busy while a copy runs; the note describes it. */
function ImportOffer({ logins }: { readonly logins: readonly OmpLogin[] }): ReactNode {
  const host = useHost();
  const [running] = useWindowState(IMPORT_RUN);
  const offer = importOffer(logins);
  const note = useId();
  return (
    <>
      <Button
        isPending={running}
        onPress={() => void importLogins(host)}
        aria-describedby={note}
        data-testid="welcome-import"
      >
        {offer.button}
      </Button>
      <p id={note} className="omfp-welcome__note" data-testid="welcome-import-note">
        {offer.note}
      </p>
    </>
  );
}

/** The welcome (UI-500, UI-501, UI-505): why nothing can start yet, the login that fixes it, and the copy of
 * standalone omp's logins when it holds some. */
export function Welcome(): ReactNode {
  const host = useHost();
  const [run] = useWindowState(LOGIN_RUN);
  const importable = useImportable();
  return (
    <section className="omfp-welcome" data-testid="welcome" aria-labelledby="omfp-welcome-title">
      <Wordmark className="omfp-welcome__mark" aria-hidden="true" />
      <h1 id="omfp-welcome-title" className="omfp-welcome__title">
        {text.title}
      </h1>
      <p className="omfp-welcome__why" data-testid="welcome-why">
        {text.why}
      </p>
      <Button
        variant="primary"
        isPending={run !== undefined}
        onPress={() => requestLogin(host)}
        data-testid="welcome-login"
      >
        {text.logIn}
      </Button>
      {importable.length > 0 && <ImportOffer logins={importable} />}
      <p className="omfp-welcome__keys" data-testid="welcome-keys">
        {text.keys}
      </p>
    </section>
  );
}
