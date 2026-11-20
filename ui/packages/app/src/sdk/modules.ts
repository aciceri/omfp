/**
 * Widget modules of workspace and user widgets (UI-105): imported once per URL with the browser's `import()` (the URL
 * carries the folder's version, so a new version is a new import, AGT-013), and classified into a component or the
 * error to show in the widget's place (AGT-014). A card's one-file widget (UI-920) is compiled by the daemon
 * (`widgets.compile`, PRT-225) and imported from a `blob:` URL, once per source.
 */

import { CallError } from "@omfp/client";
import type { WidgetComponent } from "./widget.ts";

/** Imports an ES module by URL; the window uses `import()`, tests a fake. */
export type Importer = (url: string) => Promise<unknown>;

/** The browser's `import()`. */
export const browserImporter: Importer = (url) => import(url);

/** Compiles a card's source into its ES module (`widgets.compile`, PRT-225). */
export type Compiler = (code: string) => Promise<string>;

/** Why a widget module cannot render: the daemon's stand-in for a compile error (DMN-085), or a failed import. */
export interface WidgetLoadError {
  readonly kind: "compile" | "load";
  readonly message: string;
  /** Relative to the widget folder; `undefined` when not known. */
  readonly file: string | undefined;
  /** 1-based, `null` when not known. */
  readonly line: number | null;
  readonly column: number | null;
}

/** A settled import. */
export type LoadedWidget =
  | { readonly kind: "component"; readonly component: WidgetComponent }
  | { readonly kind: "error"; readonly error: WidgetLoadError };

function loadError(message: string): LoadedWidget {
  return { kind: "error", error: { kind: "load", message, file: undefined, line: null, column: null } };
}

function position(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
}

/** What an imported module is: the stand-in's `compileError`, its default export, or an error. */
export function classifyModule(module: unknown, noDefaultExport: string): LoadedWidget {
  if (typeof module !== "object" || module === null) {
    return loadError(noDefaultExport);
  }
  const { compileError, default: component } = module as { compileError?: unknown; default?: unknown };
  if (typeof compileError === "object" && compileError !== null) {
    const { file, line, column, message } = compileError as Record<string, unknown>;
    return {
      kind: "error",
      error: {
        kind: "compile",
        message: typeof message === "string" ? message : String(message),
        file: typeof file === "string" ? file : undefined,
        line: position(line),
        column: position(column),
      },
    };
  }
  // A function component or class, or React's `memo`/`forwardRef`/`lazy` wrappers (objects tagged `$$typeof`).
  if (
    typeof component === "function" ||
    (typeof component === "object" && component !== null && "$$typeof" in component)
  ) {
    return { kind: "component", component: component as WidgetComponent };
  }
  return loadError(noDefaultExport);
}

/** The daemon's compile error (PRT-225): `card.tsx:line:column: message`. */
const COMPILE_ERROR = /^([^:\n]+):(\d+):(\d+): ([\s\S]*)$/;

/** A refused compile's message as the widget's compile error, with its position when the message starts with one. */
export function compileError(message: string): WidgetLoadError {
  const match = COMPILE_ERROR.exec(message);
  const [, file, line, column, text] = match ?? [];
  return file === undefined || text === undefined
    ? { kind: "compile", message, file: undefined, line: null, column: null }
    : { kind: "compile", message: text, file, line: position(Number(line)), column: position(Number(column)) };
}

/** The window's widget modules, by URL, and its cards' compiled sources, by source. */
export class WidgetModules {
  readonly #importer: Importer;
  readonly #noDefaultExport: string;
  readonly #pending = new Map<string, Promise<LoadedWidget>>();
  readonly #settled = new Map<string, LoadedWidget>();
  readonly #sources = new Map<string, Promise<LoadedWidget>>();
  readonly #settledSources = new Map<string, LoadedWidget>();

  constructor(importer: Importer, noDefaultExport: string) {
    this.#importer = importer;
    this.#noDefaultExport = noDefaultExport;
  }

  /** The module at `url` when its import already settled. */
  get(url: string): LoadedWidget | undefined {
    return this.#settled.get(url);
  }

  /** Imports `url` once; later calls share the same import. Never rejects. */
  load(url: string): Promise<LoadedWidget> {
    let pending = this.#pending.get(url);
    if (pending === undefined) {
      pending = this.#import(url);
      this.#pending.set(url, pending);
    }
    return pending;
  }

  /** The module of the card source `code` when its compile and import already settled. */
  getSource(code: string): LoadedWidget | undefined {
    return this.#settledSources.get(code);
  }

  /**
   * Compiles the card source `code` with `compile` and imports its module from a `blob:` URL, once per source (UI-920);
   * later calls share the same load. The daemon's refusal is the widget's compile error, kept like a module; a call
   * that got no answer from the daemon (connection lost) is a load error that is not kept, so a later load tries
   * again. Never rejects.
   */
  loadSource(code: string, compile: Compiler): Promise<LoadedWidget> {
    let pending = this.#sources.get(code);
    if (pending === undefined) {
      pending = this.#compile(code, compile);
      this.#sources.set(code, pending);
    }
    return pending;
  }

  async #import(url: string): Promise<LoadedWidget> {
    const loaded = await this.#classify(url);
    this.#settled.set(url, loaded);
    return loaded;
  }

  async #classify(url: string): Promise<LoadedWidget> {
    try {
      return classifyModule(await this.#importer(url), this.#noDefaultExport);
    } catch (error) {
      return loadError(error instanceof Error ? error.message : String(error));
    }
  }

  async #compile(code: string, compile: Compiler): Promise<LoadedWidget> {
    let module: string;
    try {
      module = await compile(code);
    } catch (error) {
      if (!(error instanceof CallError) || error.failure.kind !== "daemon") {
        this.#sources.delete(code);
        return loadError(error instanceof Error ? error.message : String(error));
      }
      const refused: LoadedWidget = { kind: "error", error: compileError(error.failure.error.message) };
      this.#settledSources.set(code, refused);
      return refused;
    }
    const url = URL.createObjectURL(new Blob([module], { type: "text/javascript" }));
    let loaded: LoadedWidget;
    try {
      loaded = await this.#classify(url);
    } finally {
      URL.revokeObjectURL(url);
    }
    this.#settledSources.set(code, loaded);
    return loaded;
  }
}
