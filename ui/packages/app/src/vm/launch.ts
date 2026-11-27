/**
 * What a window opens on, from its address bar (`omfp desktop --thread/--workspace/--view/--prompt`, UI-214, UI-218):
 * `#thread=<id>` an existing thread; `#workspace=<id, name or folder>` the draft thread of a workspace, with
 * `#prompt=<text>` in its composer; `#view=<id>` the view shown with either. Read once, then removed from the address
 * bar (`history.replaceState`) so a reload keeps the window's own selection. Pure functions, unit tested.
 */

import type { Workspace } from "@omfp/protocol";
import { fragmentParameters } from "./token.ts";

export interface Launch {
  readonly thread?: string;
  readonly workspace?: string;
  readonly view?: string;
  readonly prompt?: string;
}

const KEYS = ["thread", "workspace", "view", "prompt"] as const;
type Key = (typeof KEYS)[number];

function launchKey(parameter: string): Key | undefined {
  return KEYS.find((key) => parameter.startsWith(`${key}=`));
}

/** The decoded value of `parameter` (`key=value`); `undefined` when empty or malformed. `prompt` keeps its spaces. */
function launchValue(key: Key, parameter: string): string | undefined {
  let value: string;
  try {
    value = decodeURIComponent(parameter.slice(key.length + 1));
  } catch {
    return undefined;
  }
  if (key !== "prompt") {
    value = value.trim();
  }
  return value.trim() === "" ? undefined : value;
}

/** Reads and removes the launch parameters of the address bar; `undefined` when it has none. */
export function takeLaunch(page: {
  readonly location: Pick<Location, "hash" | "pathname" | "search">;
  readonly history: Pick<History, "replaceState">;
}): Launch | undefined {
  const { location } = page;
  const all = fragmentParameters(location.hash);
  const rest = all.filter((parameter) => launchKey(parameter) === undefined);
  if (rest.length === all.length) {
    return undefined;
  }
  page.history.replaceState(
    null,
    "",
    `${location.pathname}${location.search}${rest.length === 0 ? "" : `#${rest.join("&")}`}`,
  );
  const launch: { -readonly [key in Key]?: string } = {};
  for (const parameter of all) {
    const key = launchKey(parameter);
    const value = key === undefined ? undefined : launchValue(key, parameter);
    if (key !== undefined && value !== undefined) {
      launch[key] = value;
    }
  }
  return launch;
}

/**
 * The workspace `wanted` names: by id, else by name, else by name ignoring case when only one matches, else by folder
 * (a trailing `/` ignored).
 */
export function findWorkspace(workspaces: Iterable<Workspace>, wanted: string): Workspace | undefined {
  const all = [...workspaces];
  const lower = wanted.toLowerCase();
  const folder = wanted.length > 1 ? wanted.replace(/\/+$/, "") : wanted;
  const ignoringCase = all.filter((workspace) => workspace.name.toLowerCase() === lower);
  return (
    all.find((workspace) => workspace.id === wanted) ??
    all.find((workspace) => workspace.name === wanted) ??
    (ignoringCase.length === 1 ? ignoringCase[0] : undefined) ??
    all.find((workspace) => workspace.path === folder)
  );
}
