/** L0: the built-in widgets are written like agent widgets, so their folders can be copied into a workspace. */

import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { BUILTINS } from "../../builtins.ts";
import { BUILTIN_WIDGETS } from "../../src/view/registry.ts";
import background from "../../src/widgets/background/index.ts";
import browser from "../../src/widgets/browser/index.ts";
import changes from "../../src/widgets/changes/index.ts";
import * as chat from "../../src/widgets/chat/index.ts";
import debug from "../../src/widgets/debug/index.ts";
import file from "../../src/widgets/file/index.ts";
import header from "../../src/widgets/header/index.ts";
import jobs from "../../src/widgets/jobs/index.ts";
import mcp from "../../src/widgets/mcp/index.ts";
import todos from "../../src/widgets/todos/index.ts";
import tree from "../../src/widgets/tree/index.ts";
import web from "../../src/widgets/web/index.ts";
import workspace from "../../src/widgets/workspace/index.ts";

const WIDGETS = join(import.meta.dir, "../../src/widgets");
/** The folders the build ships as copyable widget folders (`builtins.ts`). */
const COPYABLE = [...new Set(BUILTINS.map(({ folder }) => folder))];
/** What an agent widget may import (AGT-012), besides files of its own folder. */
const ALLOWED = new Set(["react", "react/jsx-runtime", "react-dom", "omfp/sdk", "omfp/kit"]);

/** Every value import of `folder`'s TypeScript files (type-only imports are stripped like the daemon's oxc does). */
function valueImports(folder: string): { file: string; path: string }[] {
  // The production JSX runtime, as the daemon compiles widgets (DMN-084).
  const transpiler = new Bun.Transpiler({
    loader: "tsx",
    tsconfig: { compilerOptions: { jsx: "react-jsx", jsxImportSource: "react" } },
  });
  return readdirSync(join(WIDGETS, folder))
    .filter((file) => /\.(ts|tsx)$/.test(file))
    .flatMap((file) =>
      transpiler
        .scanImports(readFileSync(join(WIDGETS, folder, file), "utf8"))
        .map((entry) => ({ file: `${folder}/${file}`, path: entry.path })),
    );
}

describe("copyable built-ins", () => {
  // spec: AGT-021, AGT-012, UI-082, UI-083, UI-273, UI-290, UI-679, UI-749, UI-873, UI-950
  test("value imports come only from react, omfp/sdk, omfp/kit and the widget's own folder", () => {
    const imports = COPYABLE.flatMap(valueImports);
    expect(imports.length).toBeGreaterThan(0);
    const forbidden = imports.filter(
      ({ path }) => !ALLOWED.has(path) && !(path.startsWith("./") && !path.includes("..")),
    );
    expect(forbidden).toEqual([]);
  });

  // spec: AGT-021, UI-679, UI-950
  test("each folder's entry default-exports its widget; the chat folder also exports its parts", () => {
    expect(typeof chat.default).toBe("function");
    expect(chat.default).toBe(chat.ChatWidget);
    for (const part of [chat.TranscriptWidget, chat.InteractionsWidget, chat.ComposerWidget]) {
      expect(typeof part).toBe("function");
    }
    expect(typeof header).toBe("function");
    expect(typeof workspace).toBe("function");
    expect(typeof changes).toBe("function");
    expect(typeof file).toBe("function");
    for (const widget of [background, todos, debug, tree, mcp, web, jobs, browser]) {
      expect(typeof widget).toBe("function");
    }
  });

  // spec: AGT-021, UI-493
  test("every built-in `omfp:<name>` a view can hold ships as a copyable folder, and only those", () => {
    const shipped = BUILTINS.map(({ name }) => `omfp:${name}`).sort();
    expect(shipped).toEqual([...BUILTIN_WIDGETS.keys()].sort());
    // `omfp:subagents` merged into `omfp:background`: no built-in, no folder.
    expect(BUILTIN_WIDGETS.has("omfp:subagents")).toBe(false);
    expect(existsSync(join(WIDGETS, "subagents"))).toBe(false);
  });
});
