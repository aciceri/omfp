/**
 * Init files (UI-260..UI-263, UI-267): the user's `init.ts` (next to `theme.css`,
 * served by the daemon) and the current workspace's `.omfp/init.ts`, ES modules compiled like widgets. Their default
 * export is called with an {@link InitApi} (the user's file first, then the workspace's); what it registers through
 * the API — commands, key bindings, the views it uses and their transforms, floating panels, thread event listeners,
 * cleanups — is removed again, and the function it returned is called, before the file runs again (a new version,
 * another current workspace, or a new `views.json` or list of repositories it read) or goes away. A failure anywhere
 * (compile error, import, the run, a cleanup, a transform, a command or listener it registered) becomes the file's
 * error, shown in a banner by the frame; the window keeps working.
 */

import type { JobView, RunInfo, ThreadSummary } from "@omfp/protocol";
import {
  type InitBindings,
  type KeymapOverride,
  type Mode,
  NO_INIT_BINDINGS,
  normalizeKeys,
} from "../keymap/keymap.ts";
import { strings } from "../strings/index.ts";
import { type LayoutNode, parseLayout } from "../view/layout.ts";
import type { ResolvedPanel, Views } from "../view/views.ts";
import type { JobEventKind } from "../vm/jobs.ts";
import { errorMessage } from "./actions.ts";
import { type Command, scopesAt } from "./commands.ts";
import type { OmfpHost, SdkClient } from "./host.tsx";
import { type InitFile, InitViews } from "./init-views.ts";
import { type JobActions, jobActions } from "./jobs.ts";
import { classifyModule, type Importer } from "./modules.ts";
import type { NoticeRequest } from "./notify.ts";

const text = strings.init;

export type { InitFile } from "./init-views.ts";

/** The thread changes an init file can listen to (the same as the window's notices, UI-192). */
export type ThreadEventKind = "attention" | "failed" | "finished";

/** Every event an init file can listen to: thread changes, and runs of the declared jobs (UI-748). */
export type InitEventKind = ThreadEventKind | JobEventKind;

/** A key binding an init file makes; `mode` defaults to `normal`. */
export interface InitBinding {
  readonly mode?: Mode;
  readonly keys: string;
  /** A command id; `""` removes the binding of `keys` in `mode`. */
  readonly command: string;
  readonly args?: readonly string[];
}

/** A command an init file registers: a {@link Command} without `init`, which the API sets, nor `widget`. */
export type InitCommand = Omit<Command, "init" | "widget">;

/** A repository of the workspace, as `omfp.workspace.repos` lists it. */
export interface InitRepo {
  /** Its entry name in the workspace folder. */
  readonly name: string;
  /** Its path inside the workspace. */
  readonly path: string;
  /** `repo` (a clone), `link` (a symlink to a repository elsewhere), `worktree`, … as the daemon reports it. */
  readonly kind: string;
}

/** What the init files read from the window: the current workspace, its `views.json` and its repositories. */
export interface InitContext {
  readonly workspaceId: string | undefined;
  /** `views.json` as on the `views/<id>` topic; `undefined` before it arrives. */
  readonly manifest: unknown;
  readonly manifestError: string | null | undefined;
  readonly repos: readonly InitRepo[];
}

const NO_CONTEXT: InitContext = { workspaceId: undefined, manifest: undefined, manifestError: undefined, repos: [] };

/** `omfp.views` (UI-267). */
export interface ViewsApi {
  /** The views of the workspace's `.omfp/views.json` (only that file loads), as the window shows them. */
  load(file: "views.json"): InitViews;
  /** Shows `views` as they are now in this window instead of `views.json`'s; the last file's call wins. */
  use(views: InitViews): () => void;
  /**
   * Changes whatever views the window ends up with (after every `use`): `transform` changes the views it gets, or
   * returns other views; returns its removal.
   */
  transform(transform: (views: InitViews) => InitViews | undefined): () => void;
}

/** `omfp.workspace`: the window's current workspace. */
export interface InitWorkspace {
  readonly id: string | undefined;
  /** Its repositories; reading them runs the file again when they change. */
  readonly repos: readonly InitRepo[];
}

/** `omfp.jobs` (UI-748): the current workspace's declared jobs, and the actions on them by name. */
export interface InitJobs extends JobActions {
  /** The jobs of its `jobs/<id>` as the window holds it at the call; `[]` before it arrived. */
  list(): readonly JobView[];
  /** The job named `name` of {@link InitJobs.list}. */
  get(name: string): JobView | undefined;
  /** Why its `jobs.json` is refused; `null` when valid, absent or not known yet. */
  readonly error: string | null;
}

/** What an init file's default export receives. */
export interface InitApi {
  /** Which file this is. */
  readonly file: InitFile;
  /** The workspace whose `.omfp/init.ts` this is; `undefined` for the user's file. */
  readonly workspaceId: string | undefined;
  /** The window's host (registry, window store, notices, protocol client): everything else the SDK does. */
  readonly host: OmfpHost;
  /** Every protocol method, typed: `call("thread.prompt", { threadId, text })`. */
  readonly call: SdkClient["call"];
  /** Registers a command for the keymap and the `:` line; returns its removal. */
  command(command: InitCommand): () => void;
  /** Binds keys over the defaults, `[keymap]` and `.omfp/keymap.json`; returns the removal of these bindings. */
  bind(bindings: InitBinding | readonly InitBinding[]): () => void;
  /** A notice in the mode line, or a system notification while the window has no focus (UI-194). */
  notify(request: NoticeRequest): void;
  /** Calls `listener` whenever a thread starts needing the user, fails, or finishes a turn; returns its removal. */
  on(event: ThreadEventKind, listener: (thread: ThreadSummary) => void): () => void;
  /**
   * Calls `listener` once per run of a declared job of any workspace whose jobs the index counts, as this window hears
   * of it: when it starts, when it ends, and when it ends `failed` (UI-748); returns its removal.
   */
  on(event: JobEventKind, listener: (run: RunInfo) => void): () => void;
  /** The views of this window (UI-267): `load`, `use`, `transform`. */
  readonly views: ViewsApi;
  /** The current workspace and its repositories. */
  readonly workspace: InitWorkspace;
  /** The current workspace's declared jobs (UI-748). */
  readonly jobs: InitJobs;
  /** Floats `node` (a layout node; its `title` prop labels it) over every view of this window. */
  panel(id: string, node: unknown): () => void;
  /** Runs a command by id or alias in the scopes around the focused element; `false` when nothing answers. */
  run(name: string, args?: readonly string[]): boolean;
  /** Calls `cleanup` before the file runs again or goes away. */
  onCleanup(cleanup: () => void): void;
}

/** Why an init file failed. */
export interface InitError {
  readonly file: InitFile;
  readonly message: string;
}

/** What the init files currently add to the window. */
export interface InitSnapshot {
  readonly bindings: InitBindings;
  /**
   * The views of the window when an init file uses views or transforms them (UI-267), for the context's manifest;
   * `undefined`: `views.json`'s as they are.
   */
  readonly views: { readonly manifest: unknown; readonly views: Views } | undefined;
  readonly panels: readonly ResolvedPanel[];
  readonly errors: readonly InitError[];
}

const EMPTY: InitSnapshot = { bindings: NO_INIT_BINDINGS, views: undefined, panels: [], errors: [] };

/** A run's read of something it has not read. */
const UNREAD: unique symbol = Symbol("unread");

interface Transform {
  readonly transform: (views: InitViews) => InitViews | undefined;
}

interface Listener {
  readonly event: InitEventKind;
  readonly listener: (value: ThreadSummary | RunInfo) => void;
}

/** One run of an init file, from its import until it is replaced. */
interface Run {
  readonly file: InitFile;
  readonly url: string | undefined;
  readonly workspaceId: string | undefined;
  /** `false` once replaced: what it still registers is removed at once. */
  live: boolean;
  /** Removals and cleanups, called newest first. */
  readonly disposers: Array<() => void>;
  bindings: readonly KeymapOverride[];
  /** The views of its latest `views.use`. */
  used: InitViews | undefined;
  /** Its `views.transform`s, in registration order. */
  transforms: readonly Transform[];
  readonly panels: Map<string, ResolvedPanel>;
  readonly listeners: Set<Listener>;
  /** The manifest its `views.load` read, and the repositories it read (as JSON): a change runs it again. */
  readManifest: unknown;
  readRepos: string | typeof UNREAD;
}

function newRun(file: InitFile, url: string | undefined, workspaceId: string | undefined): Run {
  return {
    file,
    url,
    workspaceId,
    live: true,
    disposers: [],
    bindings: [],
    used: undefined,
    transforms: [],
    panels: new Map(),
    listeners: new Set(),
    readManifest: UNREAD,
    readRepos: UNREAD,
  };
}

const FILES: readonly InitFile[] = ["user", "workspace"];

/** The window's init files: their runs and what they add (the frame reads {@link InitFiles.getSnapshot}). */
export class InitFiles {
  readonly #host: () => OmfpHost;
  readonly #importer: Importer;
  readonly #runs: Record<InitFile, Run> = {
    user: newRun("user", undefined, undefined),
    workspace: newRun("workspace", undefined, undefined),
  };
  readonly #errors: Record<InitFile, string | undefined> = { user: undefined, workspace: undefined };
  /** The latest failure of each file's transforms, computed with the views. */
  #transformErrors: Record<InitFile, string | undefined> = { user: undefined, workspace: undefined };
  #context: InitContext = NO_CONTEXT;
  #queue: Promise<void> = Promise.resolve();
  #snapshot: InitSnapshot = EMPTY;
  readonly #listeners = new Set<() => void>();

  /** `host` is read lazily: the host holds this object. */
  constructor(host: () => OmfpHost, importer: Importer) {
    this.#host = host;
    this.#importer = importer;
  }

  readonly getSnapshot = (): InitSnapshot => this.#snapshot;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  /**
   * Makes `url` (`undefined`: none) the module of `file`, for `workspaceId` (the workspace's file): unless it already
   * is, the current run is cleaned up and the new module imported and run. Loads run one at a time, in call order.
   */
  load(file: InitFile, url: string | undefined, workspaceId?: string): Promise<void> {
    return this.#enqueue(() => this.#load(file, url, workspaceId, false));
  }

  /**
   * What the files read (UI-267): the current workspace, its `views.json` and repositories. A file whose run read the
   * manifest or the repositories runs again when they change.
   */
  setContext(context: InitContext): void {
    this.#context = context;
    for (const file of FILES) {
      if (this.#stale(this.#runs[file])) {
        // Checked again once the loads before it ran: one of them may have run the file on this context already.
        void this.#enqueue(async () => {
          const run = this.#runs[file];
          if (this.#stale(run)) {
            await this.#load(file, run.url, run.workspaceId, true);
          }
        });
      }
    }
    this.#changed();
  }

  /** Whether `run` read a manifest or repositories the context no longer has. */
  #stale(run: Run): boolean {
    const { manifest, repos } = this.#context;
    return (
      (run.readManifest !== UNREAD && run.readManifest !== manifest) ||
      (run.readRepos !== UNREAD && run.readRepos !== JSON.stringify(repos))
    );
  }

  #enqueue(work: () => Promise<void>): Promise<void> {
    const next = this.#queue.then(work);
    this.#queue = next;
    return next;
  }

  /** Resolves once the loads asked for so far have run (tests wait on it). */
  settled(): Promise<void> {
    return this.#queue;
  }

  /** Every listener of `event`, the user's file's first (the frame calls it for each thread change). */
  emit(event: ThreadEventKind, thread: ThreadSummary): void;
  /** The same for a run of a declared job (the frame calls it for each run event, UI-748). */
  emit(event: JobEventKind, run: RunInfo): void;
  emit(event: InitEventKind, value: ThreadSummary | RunInfo): void {
    for (const file of FILES) {
      const run = this.#runs[file];
      for (const entry of [...run.listeners]) {
        if (entry.event === event) {
          this.#guard(
            run,
            (message) => text.listener(event, message),
            () => entry.listener(value),
          );
        }
      }
    }
  }

  /** Closes a floating panel an init file added, in this window until the file runs again. */
  closePanel(id: string): void {
    for (const file of FILES) {
      if (this.#runs[file].panels.delete(id)) {
        this.#changed();
      }
    }
  }

  async #load(file: InitFile, url: string | undefined, workspaceId: string | undefined, again: boolean): Promise<void> {
    const current = this.#runs[file];
    if (current.url === url && current.workspaceId === workspaceId && !again) {
      return;
    }
    this.#errors[file] = undefined;
    this.#dispose(current);
    const run = newRun(file, url, workspaceId);
    this.#runs[file] = run;
    this.#changed();
    if (url === undefined) {
      return;
    }
    let module: unknown;
    try {
      module = await this.#importer(url);
    } catch (error) {
      this.#fail(run, text.load(errorMessage(error)));
      return;
    }
    const loaded = classifyModule(module, text.noDefaultExport);
    if (loaded.kind === "error") {
      const { file: name, line, column, message } = loaded.error;
      const at = line === null ? "" : `:${line}${column === null ? "" : `:${column}`}`;
      this.#fail(run, name === undefined ? message : `${name}${at}: ${message}`);
      return;
    }
    const main = loaded.component as unknown as (api: InitApi) => unknown;
    this.#guard(
      run,
      (message) => message,
      () => main(this.#api(run)),
      (cleanup) => {
        if (typeof cleanup === "function") {
          this.#own(run, cleanup as () => void);
        }
      },
    );
  }

  /** Runs `work`, turning a throw or a rejected promise into `run`'s error; `done` gets what it returned or resolved. */
  #guard(run: Run, describe: (message: string) => string, work: () => unknown, done?: (value: unknown) => void): void {
    const fail = (error: unknown) => this.#fail(run, describe(errorMessage(error)));
    try {
      const result = work();
      if (result instanceof Promise) {
        result.then((value: unknown) => done?.(value), fail);
      } else {
        done?.(result);
      }
    } catch (error) {
      fail(error);
    }
  }

  /** Keeps `dispose` for `run`'s cleanup and returns it callable once; called at once when `run` is gone already. */
  #own(run: Run, dispose: () => void): () => void {
    if (!run.live) {
      this.#guard(run, text.cleanup, dispose);
      return () => {};
    }
    let disposed = false;
    const once = () => {
      if (!disposed) {
        disposed = true;
        const index = run.disposers.indexOf(once);
        if (index >= 0) {
          run.disposers.splice(index, 1);
        }
        dispose();
      }
    };
    run.disposers.push(once);
    return once;
  }

  #dispose(run: Run): void {
    run.live = false;
    for (const dispose of [...run.disposers].reverse()) {
      try {
        dispose();
      } catch (error) {
        this.#errors[run.file] = text.cleanup(errorMessage(error));
      }
    }
  }

  #fail(run: Run, message: string): void {
    if (run.live) {
      this.#errors[run.file] = message;
      this.#changed();
    }
  }

  #api(run: Run): InitApi {
    const host = this.#host();
    const changedIn = (change: () => void) => () => {
      change();
      this.#changed();
    };
    const context = () => this.#context;
    const jobsState = () => {
      const { workspaceId } = this.#context;
      return workspaceId === undefined ? undefined : host.client.model.workspaceJobs.get(workspaceId)?.state;
    };
    return {
      file: run.file,
      workspaceId: run.workspaceId,
      host,
      call: host.client.call.bind(host.client),
      command: (command) =>
        this.#own(
          run,
          host.commands.register({
            ...command,
            init: run.file,
            run: (args) =>
              this.#guard(
                run,
                (message) => text.command(command.id, message),
                () => command.run(args),
              ),
          }),
        ),
      bind: (bindings) => {
        const entries: KeymapOverride[] = (Array.isArray(bindings) ? bindings : [bindings]).map(
          (binding: InitBinding) => ({
            mode: binding.mode ?? "normal",
            keys: normalizeKeys(binding.keys),
            command: binding.command,
            ...(binding.args === undefined ? {} : { args: binding.args }),
          }),
        );
        run.bindings = [...run.bindings, ...entries];
        this.#changed();
        return this.#own(
          run,
          changedIn(() => {
            run.bindings = run.bindings.filter((binding) => !entries.includes(binding));
          }),
        );
      },
      notify: (request) => host.notices.notify(request),
      on: (event: InitEventKind, listener: (value: never) => void) => {
        // Each kind's listener gets that kind's value (`emit`'s overloads).
        const entry: Listener = { event, listener: listener as (value: ThreadSummary | RunInfo) => void };
        run.listeners.add(entry);
        return this.#own(run, () => run.listeners.delete(entry));
      },
      views: {
        load: (name) => {
          if (name !== "views.json") {
            throw new Error(text.views.loadOnly(String(name)));
          }
          const { manifest, manifestError } = this.#context;
          run.readManifest = manifest;
          return InitViews.load(manifest, manifestError, run.file);
        },
        use: (views) => {
          if (!(views instanceof InitViews)) {
            throw new Error(text.views.notViews);
          }
          // Later changes to `views` are not shown: the call says when they are done.
          const used = InitViews.copy(views, run.file);
          run.used = used;
          this.#changed();
          return this.#own(
            run,
            changedIn(() => {
              if (run.used === used) {
                run.used = undefined;
              }
            }),
          );
        },
        transform: (transform) => {
          const entry: Transform = { transform };
          run.transforms = [...run.transforms, entry];
          this.#changed();
          return this.#own(
            run,
            changedIn(() => {
              run.transforms = run.transforms.filter((candidate) => candidate !== entry);
            }),
          );
        },
      },
      workspace: {
        get id() {
          return context().workspaceId;
        },
        get repos() {
          const { repos } = context();
          run.readRepos = JSON.stringify(repos);
          return repos;
        },
      },
      jobs: {
        list: () => jobsState()?.jobs ?? [],
        get: (name) => jobsState()?.jobs.find((job) => job.name === name),
        get error() {
          return jobsState()?.error ?? null;
        },
        run: (name) => jobActions(host.client, context().workspaceId).run(name),
        kill: (name) => jobActions(host.client, context().workspaceId).kill(name),
        enable: (name, enabled) => jobActions(host.client, context().workspaceId).enable(name, enabled),
      },
      panel: (id, node) => {
        const parsed = parseLayout(node, `panels.${id}`);
        if (!parsed.ok) {
          throw new Error(text.invalid(parsed.error));
        }
        const { title } = parsed.value.props;
        const resolved: ResolvedPanel = {
          id,
          title: typeof title === "string" ? title : id,
          // Checked by `parseLayout` just above.
          node: node as LayoutNode,
          tree: parsed.value,
          init: run.file,
        };
        run.panels.set(id, resolved);
        this.#changed();
        return this.#own(
          run,
          changedIn(() => {
            if (run.panels.get(id) === resolved) {
              run.panels.delete(id);
            }
          }),
        );
      },
      run: (name, args = []) => host.commands.run(name, args, scopesAt(document.activeElement)),
      onCleanup: (cleanup) => {
        this.#own(run, cleanup);
      },
    };
  }

  /**
   * The window's views (UI-267): the latest `use` (the workspace's file over the user's), else `views.json`'s, then
   * every transform, the user's file's first, each in registration order; a failing transform is skipped and becomes
   * its file's error. `undefined` when no file uses or transforms views.
   */
  #views(): InitSnapshot["views"] {
    const { user, workspace } = this.#runs;
    this.#transformErrors = { user: undefined, workspace: undefined };
    const owner = workspace.used === undefined ? user : workspace;
    const used = owner.used;
    const transforms = FILES.flatMap((file) =>
      this.#runs[file].transforms.map(({ transform }) => ({ file, transform })),
    );
    if (used === undefined && transforms.length === 0) {
      return undefined;
    }
    const { manifestError } = this.#context;
    // Views a file used are those of the manifest it read: until it runs again with a new one, the frame shows
    // `views.json`'s own, so the window never mixes views of two manifests.
    const manifest = used === undefined ? this.#context.manifest : owner.readManifest;
    let views = used ?? InitViews.load(manifest, manifestError, "user");
    for (const { file, transform } of transforms) {
      // Each transform changes a copy, so one that throws halfway changes nothing.
      const given = InitViews.copy(views, file);
      try {
        const next: unknown = transform(given);
        if (next !== undefined && !(next instanceof InitViews)) {
          throw new Error(text.views.notViews);
        }
        views = next === undefined || next === given ? given : InitViews.copy(next, file);
      } catch (error) {
        this.#transformErrors[file] ??= text.views.transform(errorMessage(error));
      }
    }
    const defaultId = views.default ?? "";
    return {
      manifest,
      views: { views: views.listed, defaultId, panels: views.panels, error: undefined },
    };
  }

  #changed(): void {
    const { user, workspace } = this.#runs;
    const views = this.#views();
    this.#snapshot = {
      bindings: { user: user.bindings, workspace: workspace.bindings },
      views,
      panels: [...user.panels.values(), ...workspace.panels.values()],
      errors: FILES.flatMap((file) => {
        const message = this.#errors[file] ?? this.#transformErrors[file];
        return message === undefined ? [] : [{ file, message }];
      }),
    };
    for (const listener of [...this.#listeners]) {
      listener();
    }
  }
}
