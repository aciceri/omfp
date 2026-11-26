/**
 * L0: the agents' widget reference (`omfp://docs/widgets`) lists only names `omfp/sdk` and `omfp/kit` export, and it
 * and the window's reference (`omfp://docs/shell`) say what agents writing views and init files need.
 */

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import * as kit from "omfp/kit";
import type { InitApi, ViewsApi } from "omfp/sdk";
import * as sdk from "omfp/sdk";
import { DEFAULT_BINDINGS } from "../../src/keymap/keymap.ts";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";

const WIDGETS = join(import.meta.dir, "../../../../../crates/omfp-daemon/src/widgets");
const DOCS = readFileSync(join(WIDGETS, "docs.md"), "utf8");
const SHELL = readFileSync(join(WIDGETS, "shell.md"), "utf8");

/** The identifiers of the first column of the table under `## <heading>`, before any subsection (`useJob(command)` →
 * `useJob`). */
function documented(heading: string): string[] {
  const start = DOCS.indexOf(`\n## ${heading}\n`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = DOCS.slice(start + 1).search(/\n#{2,3} /);
  const section = DOCS.slice(start, end < 0 ? undefined : start + 1 + end);
  return section
    .split("\n")
    .filter((line) => line.startsWith("| `"))
    .map((line) => /^\| `([A-Za-z_$][\w$]*)/.exec(line)?.[1] ?? line);
}

describe("widget docs", () => {
  // spec: AGT-083
  test("the docs walk agents through checking their work in the window", () => {
    for (const needle of [
      "## Checking your work in the window",
      "omfp_widget_check { name }",
      "out of sight",
      "omfp_windows",
      'omfp_window_run { command: "view.switch", args: ["<view>"] }',
      "It never runs code",
      "`windows.list`, `windows.state`, `windows.run` and `windows.check_widget`",
    ]) {
      expect(DOCS).toContain(needle);
    }
    for (const needle of ["## Agents and the window", "omfp_windows", "omfp_window_run", "never code"]) {
      expect(SHELL).toContain(needle);
    }
  });

  // spec: AGT-016, AGT-025
  test("every name the omfp/sdk table lists is exported by omfp/sdk", () => {
    const names = documented("omfp/sdk");
    expect(names.length).toBeGreaterThan(10);
    expect(names.filter((name) => !(name in sdk))).toEqual([]);
  });

  // spec: AGT-017, AGT-025
  test("every name the omfp/kit table lists is exported by omfp/kit", () => {
    const names = documented("omfp/kit");
    expect(names.length).toBeGreaterThan(10);
    expect(names.filter((name) => !(name in kit))).toEqual([]);
  });
});

describe("views and init files for agents", () => {
  // spec: AGT-078
  test("the shell docs describe the left column, the mode line, the `:` line and every default key", () => {
    for (const heading of ["## Left column", "## Mode line", "## The `:` line", "## Default keys"]) {
      expect(SHELL.includes(heading) ? heading : `missing: ${heading}`).toBe(heading);
    }
    const missing = DEFAULT_BINDINGS.map((binding) => binding.keys).filter((keys) => !SHELL.includes(`\`${keys}\``));
    expect(missing).toEqual([]);
  });

  // spec: UI-373
  test("both docs say a reveal opens a tab of the main slot, which instance it keeps, and when it floats", () => {
    for (const fact of [
      "| `REVEAL_WIDGET` |",
      "closable tab of the `main` slot after the open files",
      "its instance is the reference",
      "only a view without a `main` slot floats it in a panel",
    ]) {
      expect(DOCS.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
    for (const fact of ["## File and widget tabs", "when something reveals it", "floats it in a panel"]) {
      expect(SHELL.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
  });

  // spec: AGT-073
  test("the widget docs teach init files: the API, the binding order, thread events, errors and describe", () => {
    for (const fact of [
      "## Init files",
      "| `command(command)` |",
      "the user's init file, then the workspace's, win last",
      '`on(event, listener)` | `"attention"`',
      "shows in a banner naming the",
      "`:describe-key`",
      "`:describe-command <name>`",
    ]) {
      expect(DOCS.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
  });

  // spec: AGT-079
  test("omfp/sdk gives init files their types and the Layout class", () => {
    // Compiles only while the types are what the docs show: `omfp.views` is the `ViewsApi`.
    const viewsOf = (omfp: InitApi): ViewsApi => omfp.views;
    expect(typeof viewsOf).toBe("function");
    expect(typeof sdk.Layout).toBe("function");
    expect(DOCS.includes("export default function init(omfp: InitApi)")).toBe(true);
    expect(DOCS.includes('import type { InitApi } from "omfp/sdk"')).toBe(true);
  });
});

describe("jobs for agents", () => {
  // spec: AGT-087
  test("the widget docs say where and how a job runs, its limits, its retention and the job key", () => {
    for (const fact of [
      "| `useJob(command, { key? })` |",
      "stored as `jobkey:<key>`: `{ id, runs, command }`",
      "### Jobs",
      "`sh -c <command>`, non-interactive and not a login shell",
      "**Working directory**: the workspace folder",
      "**Environment**: the daemon's, unchanged",
      "`systemctl --user show-environment`",
      "No dev shell applies",
      "`nix develop -c cargo test`",
      "stdin is `/dev/null`",
      "no timeout",
      "64 KiB",
      "1 MiB",
      "its 32 most recently finished jobs",
      "A daemon restart forgets them all",
      "A job replaced under a `key` is forgotten at once",
    ]) {
      expect(DOCS.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
  });
});

describe("form controls for agents", () => {
  // spec: AGT-088
  test("the widget docs send every control to the kit, with the Select pattern and its All entry", () => {
    const start = DOCS.indexOf("\n### Form controls\n");
    expect(start).toBeGreaterThanOrEqual(0);
    const section = DOCS.slice(start);
    for (const fact of [
      'never from native\n  `<select>`, `<input type="checkbox|radio|range|date|color">` or `<datalist>`',
      "drawn by the platform",
      "`omfp_widget_check` warns about each one it finds",
    ]) {
      expect(DOCS.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
    for (const fact of [
      "| `<select>` | `Select` |",
      '| `<input type="checkbox">` | `Checkbox` |',
      '| `<input type="radio">` | `RadioGroup` with `Radio` |',
      '| `<input type="range">` |',
      '| `<input type="date">` |',
      '| `<input type="color">` |',
      "| `<datalist>` |",
      'a sentinel id such as `"*"`',
      '{ id: "*", label: "All" }',
      "hideLabel",
      "selectedKey={status}",
    ]) {
      expect(section.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
    for (const name of ["Select", "Checkbox", "RadioGroup", "Radio", "TextField", "SearchField", "ListBox"]) {
      expect(name in kit ? name : `not exported: ${name}`).toBe(name);
    }
  });
});

describe("prompt history", () => {
  // spec: UI-364
  test("the shell docs say how Up and Down walk the sent prompts and in which order", () => {
    for (const fact of [
      "`Up` with the caret on the\ncomposer's first line recalls the prompts you sent",
      "this thread's first, then its workspace's, then every other",
      "`Down` on the last line goes back and, past the newest, restores what you had typed",
    ]) {
      expect(SHELL.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
  });
});

describe("built-ins for agents", () => {
  // spec: UI-679, AGT-025
  test("the widget docs and the views schema name every built-in, omfp:web with the mount that places it", () => {
    const schema = readFileSync(join(WIDGETS, "views.schema.json"), "utf8");
    const missing = [...BUILTIN_WIDGETS.keys()].filter(
      (name) => !DOCS.includes(`\`${name}\``) || !schema.includes(name),
    );
    expect(missing).toEqual([]);
    for (const fact of [
      "`omfp:web` (`{ url, title? }`",
      'omfp_ui_mount { id: "preview", widget: "omfp:web", props: { url: "http://localhost:5173/" } }',
      "except omfp's own address",
      "The page cannot navigate the window",
      "`X-Frame-Options`, CSP `frame-ancestors`",
      "| `WebView` |",
      "| `WebFrame` |",
    ]) {
      expect(DOCS.includes(fact) ? fact : `missing: ${fact}`).toBe(fact);
    }
  });
});
