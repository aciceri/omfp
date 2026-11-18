/**
 * Logging in to a provider from the window (UI-380..UI-389): which texts of omp say it needs a login, what `:login`
 * and `:logout` complete, and the messages of a login's and a logout's outcome; the texts of copying standalone omp's
 * logins (UI-505, UI-506).
 */

import type { AuthProvider, LoggedOut, OmpLogin } from "@omfp/protocol";
import { strings } from "../strings/index.ts";

const text = strings.shell.login;

/**
 * omp's words for "no usable model or credential" (UI-381): its startup refusal (`No default model selected. Use
 * /login, …`), the prompt preflight (`No API key found for <provider>.`, `No model selected.`), the startup checks
 * (`No models available.`, `No model available matching enabledModels (…) with usable credentials.`) and every hint
 * naming `/login` as a word of its own (not a path inside a URL).
 */
const NEEDS_LOGIN =
  /(?:^|\s)\/login\b|\bNo API key found\b|\bNo default model selected\b|\bNo model selected\b|\bNo models available\b|\bNo model available matching enabledModels\b/i;

/** Whether `message` (an error, a failed start's message or stderr tail) says omp has no model it can use without a
 * login or a key (UI-381). */
export function needsLogin(message: string | null | undefined): boolean {
  return message !== null && message !== undefined && NEEDS_LOGIN.test(message);
}

/** omp's prompt preflight naming the provider it holds no credential of. */
const NO_API_KEY = /\bNo API key found for ([^\s.]+)/i;

/**
 * The short sentence an error toast shows instead of `message` when omp needs a login (UI-451): the provider without a
 * credential when omp names one, else that no model is usable; never omp's CLI advice (`/login`, `--model`).
 * `undefined` when `message` says nothing about a login.
 */
export function loginProblem(message: string | null | undefined): string | undefined {
  if (message === null || message === undefined || !needsLogin(message)) {
    return undefined;
  }
  const provider = NO_API_KEY.exec(message)?.[1];
  return provider === undefined ? text.noModel : text.noCredential(provider);
}

/** `:login`'s completions (UI-386): the ids of the providers omp can log in to here, for the first argument only. */
export function loginCompletions(
  providers: readonly AuthProvider[] | undefined,
  before: readonly string[],
): readonly string[] {
  return before.length > 0 || providers === undefined
    ? []
    : providers.filter((provider) => provider.available).map((provider) => provider.id);
}

/** `:logout`'s completions (UI-386): the ids of the providers omp holds a credential for, for the first argument only. */
export function logoutCompletions(
  providers: readonly AuthProvider[] | undefined,
  before: readonly string[],
): readonly string[] {
  return before.length > 0 || providers === undefined
    ? []
    : providers.filter((provider) => provider.authenticated).map((provider) => provider.id);
}

/** The name `providerId` shows as: its name from `auth.providers`, else the id itself. */
export function providerName(providers: readonly AuthProvider[] | undefined, providerId: string): string {
  return providers?.find((provider) => provider.id === providerId)?.name ?? providerId;
}

/** The toast of `:logout`'s result (UI-386): what omp removed and, when something else still authenticates the
 * provider (an environment variable, omp's config), that source. */
export function logoutSummary(result: LoggedOut, name: string): string {
  const removed = result.removed === 0 ? text.removedNone(name) : text.removed(name, result.removed);
  const remaining = result.remainingSource;
  return remaining === null || remaining === undefined || remaining === ""
    ? removed
    : `${removed} ${text.stillAuthenticated(name, remaining)}`;
}

/** `providers` after a login or logout of `providerId` changed whether omp holds a credential for it. */
export function withAuthenticated(
  providers: readonly AuthProvider[],
  providerId: string,
  authenticated: boolean,
): readonly AuthProvider[] {
  return providers.map((provider) => (provider.id === providerId ? { ...provider, authenticated } : provider));
}

/**
 * The texts of copying standalone omp's `logins` (UI-505, UI-506): their provider ids joined by ", ", and the OAuth
 * caveat when any of them is a browser login.
 */
function describeLogins(logins: readonly OmpLogin[]): { readonly ids: string; readonly caveat: string | undefined } {
  const ids = logins.map((login) => login.providerId).join(", ");
  return { ids, caveat: logins.some((login) => login.oauth) ? text.oauthCaveat : undefined };
}

/** The welcome's offer to copy standalone omp's `logins` (UI-505): its button text and its note. */
export function importOffer(logins: readonly OmpLogin[]): { readonly button: string; readonly note: string } {
  const { ids, caveat } = describeLogins(logins);
  const note = strings.shell.welcome.importNote;
  return { button: strings.shell.welcome.importLogins(ids), note: caveat === undefined ? note : `${note} ${caveat}` };
}

/** The success toast of a copy (UI-506) that copied `logins`, at least one. */
export function importedSummary(logins: readonly OmpLogin[]): string {
  const { ids, caveat } = describeLogins(logins);
  return caveat === undefined ? text.imported(ids) : `${text.imported(ids)}. ${caveat}`;
}
