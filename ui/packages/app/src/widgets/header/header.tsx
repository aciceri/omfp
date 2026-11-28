/**
 * `omfp:header`, the selected thread's header (UI-030): title with rename in place, the model as a brain button at the
 * title row's end (its list ends with a login, UI-030), thinking selector, context usage, session cost, the process
 * state with stop/restart, the restart the daemon holds back named by its reasons (UI-349), and
 * archive/unarchive/immediate delete (UI-026). For the draft thread (UI-023): its placeholder title and the model. The
 * title row gives the title the room (UI-482): the state, one state-dependent action, the model and a `⋯` popover
 * holding rename, archive and delete. Narrower than {@link COMPACT_BELOW_REM} it is one row (UI-202): title, state and
 * model, the rest in the `⋯` popover. When omp has no model it can use — its start failed saying so, or it lists no
 * model — it offers Log in (UI-388). A forked thread says where it comes from (UI-664). When a failed start's
 * saved model is not among the usable ones, it says so and offers them and Log in; only a pick changes it (UI-751).
 */

import type { ThreadSummary } from "@omfp/protocol";
import {
  Button,
  ChatHeader,
  ChatTitle,
  DialogTrigger,
  Icon,
  IconButton,
  type Key,
  Meter,
  ModeButton,
  PopoverDialog,
  Select,
  StateBadge,
  TextField,
  useWidthClass,
} from "omfp/kit";
import {
  AUTH_REVISION,
  compactionsEnded,
  costLabel,
  DRAFT_CREATING,
  DRAFT_THREAD,
  type DraftThread,
  decodeModels,
  decodeSession,
  decodeThinkingLevels,
  FRAME_LAYOUT,
  forkLineage,
  type HeaderView,
  headerView,
  MODEL_CHANGING,
  modelOption,
  NO_DETAILS,
  RENAMING_THREAD,
  REVEAL_WIDGET,
  requestLogin,
  type SdkClient,
  SELECTED_THREAD,
  savedModel,
  splitModel,
  statusLabel,
  strings,
  type ThreadDetails,
  type ThreadStatus,
  threadJob,
  threadMark,
  useCall,
  useCommand,
  useConversationActions,
  useHost,
  useReportError,
  useRunningInBackground,
  useSnapshot,
  useThreadActions,
  useThreadCost,
  useThreadSummary,
  useThreads,
  useWindowState,
  type WidgetProps,
} from "omfp/sdk";
import { type FormEvent, type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";

const text = strings.shell.header;

/** Below this width (in `rem`) the header is one row (UI-202). */
const COMPACT_BELOW_REM = 48;

export function HeaderWidget(_props: WidgetProps): ReactNode {
  const [selected] = useWindowState(SELECTED_THREAD);
  const [draft] = useWindowState(DRAFT_THREAD);
  const [layout] = useWindowState(FRAME_LAYOUT);
  const summary = useThreadSummary(selected);
  if (selected === undefined && draft !== undefined) {
    return <DraftHeader draft={draft} />;
  }
  // On a phone the mode line carries the thread's title, state and `⋯` (UI-419).
  if (layout === "mobile") {
    return null;
  }
  if (summary === undefined) {
    return (
      <ChatHeader className="omfp-thread-header" data-testid="thread-header" data-empty="true">
        <p className="omfp-thread-header__empty" data-testid="header-empty">
          {text.noThread}
        </p>
      </ChatHeader>
    );
  }
  return <ThreadHeader key={summary.id} summary={summary} />;
}

function DraftHeader({ draft }: { draft: DraftThread }): ReactNode {
  const [details] = useThreadDetails(undefined, undefined);
  const [, setDraft] = useWindowState(DRAFT_THREAD);
  // The thread is created with the model chosen so far: no change while `threads.create` is on its way (UI-023).
  const [creating] = useWindowState(DRAFT_CREATING);
  return (
    <ChatHeader className="omfp-thread-header" data-testid="thread-header" data-draft="true">
      <ChatTitle data-testid="thread-title" placeholder>
        {strings.shell.untitled}
      </ChatTitle>
      <ModelPicker
        model={draft.model}
        models={details.models.map(modelOption)}
        noModels={details.modelsLoaded && details.models.length === 0}
        disabled={creating}
        onSelectionChange={(key) => {
          if (key !== null) {
            setDraft((current) => (current === undefined ? current : { ...current, model: String(key) }));
          }
        }}
      />
    </ChatHeader>
  );
}

/** The header of a thread. It does not read the reducer state, so a streamed delta does not render it again: only
 * {@link SessionCost} follows the thread's events (UI-191). */
function ThreadHeader({ summary }: { summary: ThreadSummary }): ReactNode {
  const [view, refreshSession] = useHeaderView(summary);
  const header = useRef<HTMLElement>(null);
  // Its own width decides, wherever the view puts it (UI-202).
  const compact = useWidthClass(header, (width, rem) => width > 0 && width < COMPACT_BELOW_REM * rem);
  const [modelOpen, setModelOpen] = useState(false);
  usePickModel(view, () => setModelOpen(true));
  const model = (
    <ModelSelect
      threadId={summary.id}
      view={view}
      onChanged={refreshSession}
      isOpen={modelOpen}
      onOpenChange={setModelOpen}
    />
  );
  return (
    <ChatHeader
      ref={header}
      className="omfp-thread-header"
      data-testid="thread-header"
      data-thread-id={summary.id}
      data-omfp-scope="header"
      data-compact={compact || undefined}
      bar={
        compact ? undefined : (
          <>
            <Chips summary={summary} />
            <UnavailableModel view={view} />
            <div className="omfp-thread-header__session">
              <ThinkingSelect threadId={summary.id} view={view} onChanged={refreshSession} />
              <Usage threadId={summary.id} view={view} />
            </div>
          </>
        )
      }
    >
      {compact ? (
        <>
          <Title threadId={summary.id} view={view} />
          <HeaderState status={view.status} failure={view.failure} />
          <BackgroundBadge threadId={summary.id} />
          <PendingRestart threadId={summary.id} view={view} />
          {model}
          <More threadId={summary.id} summary={summary} view={view} onChanged={refreshSession} compact />
        </>
      ) : (
        <>
          <Title threadId={summary.id} view={view} />
          <div className="omfp-thread-header__process">
            <HeaderState status={view.status} failure={view.failure} />
            <BackgroundBadge threadId={summary.id} />
            <ProcessAction threadId={summary.id} view={view} />
            <PendingRestart threadId={summary.id} view={view} />
          </div>
          {model}
          <More threadId={summary.id} summary={summary} view={view} onChanged={refreshSession} compact={false} />
        </>
      )}
    </ChatHeader>
  );
}

/**
 * The thread's `⋯` in a phone's mode line (UI-419), where the chat header gives way: the compact header's popover
 * (UI-202) with the model selector in it. `thread.pickModel` opens the popover with the model list. It stays mounted
 * while the `:` line hides the button (`shown` false), so `:model` stays registered there and opens the popover once
 * the line closes.
 */
export function ThreadMoreButton({ summary, shown }: { summary: ThreadSummary; shown: boolean }): ReactNode {
  const [view, refreshSession] = useHeaderView(summary);
  const [moreOpen, setMoreOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  usePickModel(view, () => {
    setMoreOpen(true);
    setModelOpen(true);
  });
  if (!shown) {
    return null;
  }
  return (
    <More
      threadId={summary.id}
      summary={summary}
      view={view}
      onChanged={refreshSession}
      compact
      trigger={<ModeButton label={text.more} icon="ellipsis" data-testid="header-more" />}
      model={
        <ModelSelect
          threadId={summary.id}
          view={view}
          onChanged={refreshSession}
          isOpen={modelOpen}
          onOpenChange={setModelOpen}
        />
      }
      isOpen={moreOpen}
      onOpenChange={setMoreOpen}
    />
  );
}

/**
 * A phone's rename (UI-419): while the thread is being renamed (`Space r`, `:rename`, the `⋯` popover's Rename) the
 * mode line shows this editor in place of its title, as the header's title does elsewhere; `null` otherwise.
 */
export function ThreadRename({ summary }: { summary: ThreadSummary }): ReactNode {
  const [renaming, setRenaming] = useWindowState(RENAMING_THREAD);
  const actions = useThreadActions();
  if (renaming !== summary.id) {
    return null;
  }
  const untitled = summary.titleSource === "placeholder" || summary.title.trim() === "";
  return (
    <InlineEditor
      testid="rename-form"
      inputTestId="title-input"
      label={text.renameLabel}
      initial={untitled ? "" : summary.title}
      onSave={(value) => {
        setRenaming(undefined);
        if (value !== "") {
          void actions.rename(summary.id, value);
        }
      }}
      onCancel={() => setRenaming(undefined)}
    />
  );
}

/** The header's view of the thread (UI-202) and the function refreshing its session after a model or thinking change. */
function useHeaderView(summary: ThreadSummary): [HeaderView, () => void] {
  const [details, refreshSession] = useThreadDetails(summary.id, summary.process.state);
  return [headerView(summary, details), refreshSession];
}

/** `thread.pickModel` (`:model`, omp's `/model`, UI-980): `open` shows the model list; without a model, logs in. */
function usePickModel(view: HeaderView, open: () => void): void {
  const host = useHost();
  useCommand({
    id: "thread.pickModel",
    title: strings.shell.commands.pickModel,
    aliases: ["model"],
    run: () => {
      if (view.noModels) {
        requestLogin(host);
      } else {
        open();
      }
    },
  });
}

/**
 * The header's `⋯` (UI-482): a popover with why the process failed, rename, archive and delete, copying and exporting
 * the conversation (UI-793), compacting the context with an optional focus (UI-912); the compact header's (UI-202) adds
 * stop or restart, the thinking level, the fork and job lineage, context usage and cost, and in a phone's mode line the model
 * (UI-419). An action closes it.
 */
function More({
  threadId,
  summary,
  view,
  onChanged,
  compact,
  trigger,
  model,
  isOpen,
  onOpenChange,
}: SessionProps & {
  summary: ThreadSummary;
  compact: boolean;
  /** The button opening it; the header's bordered `⋯` by default. */
  trigger?: ReactNode;
  /** The model selector, where nothing beside the popover shows it. */
  model?: ReactNode;
  /** Open, when the caller controls it (`thread.pickModel`). */
  isOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}): ReactNode {
  const [, setRenaming] = useWindowState(RENAMING_THREAD);
  return (
    <DialogTrigger
      {...(isOpen === undefined ? {} : { isOpen })}
      {...(onOpenChange === undefined ? {} : { onOpenChange })}
    >
      {trigger ?? (
        <IconButton variant="secondary" label={text.more} icon={<Icon name="ellipsis" />} data-testid="header-more" />
      )}
      <PopoverDialog title={text.moreTitle} className="omfp-thread-header__more" data-testid="header-more-dialog">
        {({ close }) => (
          <>
            {view.failure !== undefined && (
              <p className="omfp-thread-header__failure" data-testid="thread-failure">
                {view.failure}
              </p>
            )}
            {compact && <UnavailableModel view={view} />}
            <div className="omfp-thread-header__actions">
              <Button
                size="sm"
                icon={<Icon name="pencil" />}
                data-testid="rename-thread"
                onPress={() => {
                  close();
                  setRenaming(threadId);
                }}
              >
                {text.rename}
              </Button>
              {compact && <ProcessAction threadId={threadId} view={view} onDone={close} />}
            </div>
            <Lifecycle threadId={threadId} archived={view.archived} onDone={close} />
            <ConversationActions threadId={threadId} onDone={close} />
            {!view.archived && <Compaction threadId={threadId} onDone={close} />}
            {compact && (
              <>
                {model}
                <ThinkingSelect threadId={threadId} view={view} onChanged={onChanged} />
                <Chips summary={summary} onDone={close} />
                <div className="omfp-thread-header__actions">
                  <Usage threadId={threadId} view={view} />
                </div>
              </>
            )}
          </>
        )}
      </PopoverDialog>
    </DialogTrigger>
  );
}

/** Reads thread details while running, otherwise the usable models without starting a thread (shared by the window,
 * {@link usableModels}); after a failed start, the model it resumes with (`thread.session`, UI-751). Login/logout
 * changes refresh either list; the session (model, thinking and context usage) is read again whenever the thread
 * settles or starts working and whenever a compaction ends (UI-030); the function refreshes it after a model or
 * thinking change. */
function useThreadDetails(
  threadId: string | undefined,
  process: ThreadSummary["process"]["state"] | undefined,
): [ThreadDetails, () => void] {
  const host = useHost();
  const call = useCall();
  const [details, setDetails] = useState<ThreadDetails>(NO_DETAILS);
  const [saved, setSaved] = useState<string | undefined>(undefined);
  const [authRevision] = useWindowState(AUTH_REVISION);
  // The daemon's count of usable models (PRT-141) follows the logins of every window.
  const usableCount = useSnapshot((snapshot) => snapshot.index.models);
  const report = useReportError();
  const running = process === "running";
  const failed = process === "failed";
  const settled = useSnapshot((snapshot) =>
    threadId === undefined ? undefined : snapshot.index.threads.get(threadId)?.settled,
  );
  const compactions = useSnapshot((snapshot) => {
    const notices = threadId === undefined ? undefined : snapshot.threads.get(threadId)?.state.notices;
    return notices === undefined ? 0 : compactionsEnded(notices);
  });
  useEffect(() => {
    // Read again on every login or logout.
    let live = true;
    const request =
      running && threadId !== undefined
        ? fetchDetails(call, threadId)
        : usableModels(host, call, `${authRevision}:${usableCount}`).then((models) => ({
            ...NO_DETAILS,
            models,
            modelsLoaded: true,
          }));
    void request.then(
      (fetched) => {
        if (live) {
          setDetails((previous) => ({ ...fetched, session: previous.session }));
        }
      },
      (error: unknown) => {
        if (live) {
          report(error);
        }
      },
    );
    return () => {
      live = false;
    };
  }, [host, call, threadId, running, authRevision, usableCount, report]);
  useEffect(() => {
    // Read again on every login or logout, turn boundary and compaction: omp sends no event with the context usage.
    void authRevision;
    void settled;
    void compactions;
    if (!running || threadId === undefined) {
      return;
    }
    let live = true;
    call("thread.state", { threadId }).then(
      (session) => {
        if (live) {
          setDetails((previous) => ({ ...previous, session: decodeSession(session) }));
        }
      },
      // Without it the header keeps what it showed.
      () => {},
    );
    return () => {
      live = false;
    };
  }, [call, threadId, running, authRevision, settled, compactions]);
  useEffect(() => {
    setSaved(undefined);
    if (!failed || threadId === undefined) {
      return;
    }
    let live = true;
    call("thread.session", { threadId }).then(
      (session) => {
        if (live) {
          setSaved(savedModel(session));
        }
      },
      // Without it the header shows the failure as before.
      () => {},
    );
    return () => {
      live = false;
    };
  }, [call, threadId, failed]);
  const refresh = () => {
    if (threadId === undefined) {
      return;
    }
    void call("thread.state", { threadId }).then(
      (session) => setDetails((previous) => ({ ...previous, session: decodeSession(session) })),
      () => {},
    );
  };
  return [{ ...details, saved }, refresh];
}

/** The models and thinking levels of a running thread; its session has a read of its own. */
async function fetchDetails(call: SdkClient["call"], threadId: string): Promise<ThreadDetails> {
  const [models, levels] = await Promise.allSettled([
    call("thread.models", { threadId }),
    call("thread.thinking_levels", { threadId }),
  ]);
  return {
    session: undefined,
    models: models.status === "fulfilled" ? decodeModels(models.value) : [],
    modelsLoaded: models.status === "fulfilled",
    thinkingLevels: levels.status === "fulfilled" ? decodeThinkingLevels(levels.value) : [],
    saved: undefined,
  };
}

/** Each window's last `auth.models` answer and the login state it is for. */
const USABLE_MODELS = new WeakMap<
  object,
  { readonly state: string; readonly models: Promise<ThreadDetails["models"]> }
>();

/**
 * The usable models a thread without a running omp offers (`auth.models`, UI-030), asked once per window and login
 * `state` rather than for every thread shown: each answer comes from a short-lived omp (UI-998). A failed ask is
 * asked again next time.
 */
function usableModels(host: object, call: SdkClient["call"], state: string): Promise<ThreadDetails["models"]> {
  const kept = USABLE_MODELS.get(host);
  if (kept?.state === state) {
    return kept.models;
  }
  const models = call("auth.models", {}).then(decodeModels);
  USABLE_MODELS.set(host, { state, models });
  void models.catch(() => {
    if (USABLE_MODELS.get(host)?.models === models) {
      USABLE_MODELS.delete(host);
    }
  });
  return models;
}

/** The title, or its editor while renaming (`Space r`, `:rename`, the `⋯` popover's Rename). Saving or cancelling
 * leaves the focus on the title (UI-286). */
function Title({ threadId, view }: { threadId: string; view: HeaderView }) {
  const [renaming, setRenaming] = useWindowState(RENAMING_THREAD);
  const actions = useThreadActions();
  const title = useRef<HTMLHeadingElement>(null);
  const refocus = useRef(false);
  useEffect(() => {
    if (renaming !== threadId && refocus.current) {
      refocus.current = false;
      title.current?.focus();
    }
  }, [renaming, threadId]);
  const close = () => {
    refocus.current = true;
    setRenaming(undefined);
  };
  if (renaming !== threadId) {
    return (
      <ChatTitle
        ref={title}
        // A focus target only, after renaming (UI-286).
        tabIndex={-1}
        placeholder={view.placeholder}
        data-testid="thread-title"
        title={view.title}
      >
        {view.title}
      </ChatTitle>
    );
  }
  return (
    <InlineEditor
      testid="rename-form"
      inputTestId="title-input"
      label={text.renameLabel}
      initial={view.placeholder ? "" : view.title}
      onSave={(value) => {
        close();
        if (value !== "") {
          void actions.rename(threadId, value);
        }
      }}
      onCancel={close}
    />
  );
}

/** A one-field form: Enter saves, Escape cancels; the field has the focus when it opens. */
function InlineEditor({
  testid,
  inputTestId,
  label,
  initial,
  onSave,
  onCancel,
}: {
  testid: string;
  inputTestId: string;
  label: string;
  initial: string;
  onSave: (value: string) => void;
  onCancel: () => void;
}): ReactNode {
  const [value, setValue] = useState(initial);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onSave(value.trim());
  };
  const onEscape = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  };
  return (
    <form className="omfp-thread-header__editor" data-testid={testid} onSubmit={submit}>
      <TextField
        label={label}
        hideLabel
        value={value}
        onChange={setValue}
        // The editor opens on request and takes the focus (UI-092)
        autoFocus
        inputProps={{ "data-testid": inputTestId, onKeyDown: onEscape }}
      />
      <Button type="submit" size="sm" variant="primary" data-testid={`${testid}-save`}>
        {text.save}
      </Button>
      <Button size="sm" variant="quiet" data-testid={`${testid}-cancel`} onPress={onCancel}>
        {text.cancel}
      </Button>
    </form>
  );
}

function Chips({ summary, onDone }: { summary: ThreadSummary; onDone?: () => void }): ReactNode {
  return (
    <div className="omfp-thread-header__chips">
      <Lineage summary={summary} onDone={onDone} />
      <MadeByJob summary={summary} onDone={onDone} />
    </div>
  );
}

/** A thread a declared job made (UI-746): `job <name>` in the provenance place; pressing it shows `omfp:jobs`. */
function MadeByJob({ summary, onDone }: { summary: ThreadSummary; onDone?: (() => void) | undefined }): ReactNode {
  const [, reveal] = useWindowState(REVEAL_WIDGET);
  const job = threadJob(summary);
  if (job === undefined) {
    return null;
  }
  return (
    <Button
      size="sm"
      variant="quiet"
      icon={<Icon name="clock" />}
      className="omfp-thread-header__lineage"
      data-testid="thread-job"
      data-job={job}
      aria-label={text.showJobs(job)}
      onPress={() => {
        onDone?.();
        reveal({ reference: "omfp:jobs", nonce: Date.now(), workspaceId: summary.workspaceId });
      }}
    >
      <span className="omfp-thread-header__lineage-label" title={text.showJobs(job)}>
        {text.madeByJob(job)}
      </span>
    </Button>
  );
}

/** Where a fork comes from (UI-664): `Forked from <source> at “<message>”`; pressing it shows the source while it
 * exists. */
function Lineage({ summary, onDone }: { summary: ThreadSummary; onDone?: (() => void) | undefined }): ReactNode {
  const threads = useThreads();
  const actions = useThreadActions();
  const lineage = forkLineage(summary, threads);
  if (lineage === undefined) {
    return null;
  }
  const label = (
    <span className="omfp-thread-header__lineage-label" title={lineage.label}>
      {lineage.label}
    </span>
  );
  if (!lineage.sourceExists) {
    return (
      <span className="omfp-thread-header__lineage" data-testid="fork-lineage">
        <Icon name="git-fork" />
        {label}
      </span>
    );
  }
  return (
    <Button
      size="sm"
      variant="quiet"
      icon={<Icon name="git-fork" />}
      className="omfp-thread-header__lineage"
      data-testid="fork-lineage"
      data-source={lineage.sourceId}
      aria-label={strings.shell.fork.goToSource(lineage.sourceTitle)}
      onPress={() => {
        onDone?.();
        actions.select(lineage.sourceId);
      }}
    >
      {label}
    </Button>
  );
}

interface SessionProps {
  readonly threadId: string;
  readonly view: HeaderView;
  /** Refreshes the session after a change. */
  readonly onChanged: () => void;
}

/** Sends a model or thinking change, then refreshes the session; failures go to the frame's error list. */
function useSessionChange(onChanged: () => void): (request: Promise<unknown>) => void {
  const report = useReportError();
  return (request) => {
    request.then(onChanged, report);
  };
}

/** The model list; when omp offers no model, Log in instead (UI-388). Picking a usable model for a thread whose start
 * failed on its saved one stores it and starts omp with it on the same conversation (DMN-339, UI-751). `:model`
 * (`thread.pickModel`, omp's `/model` too) opens the list, or logs in without a model (UI-980). */
function ModelSelect({
  threadId,
  view,
  onChanged,
  isOpen,
  onOpenChange,
}: SessionProps & { isOpen: boolean; onOpenChange: (open: boolean) => void }): ReactNode {
  const call = useCall();
  const report = useReportError();
  const [changing, setChanging] = useWindowState(MODEL_CHANGING);
  const pending = changing.includes(threadId);
  const onModel = (key: Key | null) => {
    const model = splitModel(String(key ?? ""));
    if (!pending && model !== undefined && String(key) !== view.model) {
      setChanging((previous) => [...previous, threadId]);
      void call("thread.set_model", { threadId, ...model })
        .then(onChanged, report)
        .finally(() => setChanging((previous) => previous.filter((id) => id !== threadId)));
    }
  };
  return (
    <ModelPicker {...view} disabled={pending} isOpen={isOpen} onOpenChange={onOpenChange} onSelectionChange={onModel} />
  );
}

/** The model list's last entry (UI-030): starts a login instead of picking a model; model values hold a `/`. */
const LOGIN_ITEM = "omfp:login";

/** The same usable-model control before and after daemon thread creation; a saved model omp cannot use shows as the
 * selection, marked unavailable and not pickable (UI-751). Its last entry logs in to another provider. */
function ModelPicker({
  model,
  models,
  noModels,
  onSelectionChange,
  disabled = false,
  isOpen,
  onOpenChange,
}: Pick<HeaderView, "model" | "models" | "noModels"> & {
  onSelectionChange: (key: Key | null) => void;
  disabled?: boolean;
  /** Open, when the caller controls it (`thread.pickModel`). */
  isOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}): ReactNode {
  const host = useHost();
  if (noModels) {
    return (
      <span className="omfp-thread-header__no-models" data-testid="no-models">
        {text.noModels}
        <Button
          size="sm"
          variant="primary"
          aria-description={text.logInHint}
          onPress={() => requestLogin(host)}
          data-testid="models-login"
        >
          {text.logIn}
        </Button>
      </span>
    );
  }
  const unavailable = models.some((option) => option.unavailable === true && option.value === model);
  return (
    <Select
      label={text.model}
      hideLabel
      iconOnly
      icon={<Icon name={unavailable ? "circle-alert" : "brain"} />}
      placeholder={text.modelPlaceholder}
      data-testid="model-select"
      data-unavailable={unavailable || undefined}
      className="omfp-thread-header__model"
      isDisabled={disabled || models.length === 0}
      items={[
        ...models.map((option) => ({
          id: option.value,
          label: option.label,
          ...(option.unavailable === true ? { isDisabled: true } : {}),
        })),
        { id: LOGIN_ITEM, label: text.logInAnother },
      ]}
      selectedKey={model ?? null}
      onSelectionChange={(key) => {
        if (key === LOGIN_ITEM) {
          requestLogin(host);
        } else {
          onSelectionChange(key);
        }
      }}
      {...(isOpen === undefined ? {} : { isOpen })}
      {...(onOpenChange === undefined ? {} : { onOpenChange })}
    />
  );
}

/** Why the model list shows the saved model unavailable and what to do (UI-751): pick a usable model, or Log in. */
function UnavailableModel({ view }: { view: HeaderView }): ReactNode {
  if (view.unavailable === undefined) {
    return null;
  }
  const sentence = text.modelUnavailable(view.unavailable);
  return (
    <p className="omfp-thread-header__unavailable" data-testid="model-unavailable" role="status" title={sentence}>
      {sentence}
    </p>
  );
}

function ThinkingSelect({ threadId, view, onChanged }: SessionProps): ReactNode {
  const call = useCall();
  const change = useSessionChange(onChanged);
  const onThinking = (key: Key | null) => {
    const level = String(key ?? "");
    if (view.thinkingLevels.includes(level) && level !== view.thinking) {
      change(call("thread.set_thinking", { threadId, level }));
    }
  };
  return (
    <Select
      label={text.thinking}
      hideLabel
      placeholder={text.thinkingPlaceholder}
      data-testid="thinking-select"
      items={view.thinkingLevels.map((level) => ({ id: level, label: level }))}
      selectedKey={view.thinking ?? null}
      onSelectionChange={onThinking}
    />
  );
}

/** Context usage and session cost. */
function Usage({ threadId, view }: { threadId: string; view: HeaderView }): ReactNode {
  return (
    <>
      {view.usage !== undefined && (
        <span className="omfp-thread-header__meter" title={view.context}>
          <Meter
            value={view.usage.used}
            max={view.usage.total}
            label={`${text.contextUsage}: ${view.context ?? ""}`}
            data-testid="context-usage"
          />
        </span>
      )}
      <SessionCost threadId={threadId} />
    </>
  );
}

/** The cost (UI-690): the session's own, then its subagents' in parentheses; follows the thread while mounted. */
function SessionCost({ threadId }: { threadId: string }): ReactNode {
  const cost = costLabel(useThreadCost(threadId));
  return (
    cost !== undefined && (
      <span
        className="omfp-thread-header__cost"
        data-testid="session-cost"
        role="img"
        title={cost.description}
        aria-label={cost.description}
      >
        {cost.text}
      </span>
    )
  );
}

/** Log in when omp needs one (UI-388), and stop/restart; `onDone` runs after pressing one (closes the compact header's
 * popover). Why the process failed is the state's tooltip and the `⋯` popover's first line (UI-482). */
function ProcessAction({ threadId, view, onDone }: { threadId: string; view: HeaderView; onDone?: () => void }) {
  const actions = useThreadActions();
  const host = useHost();
  return (
    <>
      {view.login && (
        <Button
          size="sm"
          variant="primary"
          aria-description={text.logInHint}
          data-testid="header-login"
          onPress={() => {
            onDone?.();
            requestLogin(host);
          }}
        >
          {text.logIn}
        </Button>
      )}
      {view.action !== undefined && (
        <Button
          size="sm"
          icon={<Icon name={view.action === "stop" ? "square" : "refresh-cw"} />}
          data-testid="process-action"
          data-action={view.action}
          onPress={() => {
            onDone?.();
            void (view.action === "stop" ? actions.stop(threadId) : actions.restart(threadId));
          }}
        >
          {view.action === "stop" ? text.stop : text.restart}
        </Button>
      )}
    </>
  );
}

/** The restart the daemon holds back, named by its reasons (UI-349: "Reload to load MCP tools", "Restart to load new
 * models"…): `thread.mcp_reconnect` restarts the thread's omp now; a refusal (`busy` while the turn runs) goes to the
 * error toasts. */
function PendingRestart({ threadId, view }: { threadId: string; view: HeaderView }): ReactNode {
  const call = useCall();
  const report = useReportError();
  const [pending, setPending] = useState(false);
  if (view.restart === undefined) {
    return null;
  }
  return (
    <Button
      size="sm"
      variant="primary"
      icon={<Icon name="refresh-cw" />}
      isPending={pending}
      aria-description={view.restart.hint}
      data-testid="restart-pending"
      onPress={() => {
        setPending(true);
        call("thread.mcp_reconnect", { threadId })
          .catch(report)
          .finally(() => setPending(false));
      }}
    >
      {view.restart.label}
    </Button>
  );
}

/** Copy and export the whole conversation (UI-793), as `:copy` and `:export` do (UI-790, UI-791). */
function ConversationActions({ threadId, onDone }: { threadId: string; onDone: () => void }): ReactNode {
  const conversation = useConversationActions();
  return (
    <div className="omfp-thread-header__actions">
      <Button
        size="sm"
        icon={<Icon name="copy" />}
        data-testid="copy-conversation"
        onPress={() => {
          onDone();
          void conversation.copy(threadId);
        }}
      >
        {strings.shell.conversation.copy}
      </Button>
      <Button
        size="sm"
        icon={<Icon name="download" />}
        data-testid="export-conversation"
        onPress={() => {
          onDone();
          void conversation.export(threadId);
        }}
      >
        {strings.shell.conversation.export}
      </Button>
    </div>
  );
}

/** Compact the context, with the summary's focus when the field has one (UI-912), as `:compact` does (UI-910). */
function Compaction({ threadId, onDone }: { threadId: string; onDone: () => void }): ReactNode {
  const actions = useThreadActions();
  const [focus, setFocus] = useState("");
  const labels = strings.shell.compaction;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onDone();
    void actions.compact(threadId, focus);
  };
  return (
    <form className="omfp-thread-header__compaction" data-testid="compaction" onSubmit={submit}>
      <TextField
        label={labels.focus}
        hideLabel
        placeholder={labels.focusPlaceholder}
        value={focus}
        onChange={setFocus}
        inputProps={{ "data-testid": "compaction-focus" }}
      />
      <Button type="submit" size="sm" icon={<Icon name="fold-vertical" />} data-testid="compact-context">
        {labels.compact}
      </Button>
    </form>
  );
}

/** Archive (unarchive when archived) and immediate delete (UI-026). */
function Lifecycle({ threadId, archived, onDone }: { threadId: string; archived: boolean; onDone?: () => void }) {
  const actions = useThreadActions();
  return (
    <div className="omfp-thread-header__lifecycle">
      <Button
        size="sm"
        icon={<Icon name="archive" />}
        data-testid="archive-thread"
        onPress={() => {
          onDone?.();
          void (archived ? actions.unarchive(threadId) : actions.archive(threadId));
        }}
      >
        {archived ? text.unarchive : text.archive}
      </Button>
      <Button
        size="sm"
        variant="danger"
        icon={<Icon name="trash-2" />}
        data-testid="delete-thread"
        onPress={() => {
          onDone?.();
          void actions.remove(threadId);
        }}
      >
        {text.delete}
      </Button>
    </div>
  );
}

/** The thread's state (UI-157, UI-483): glyph and word without a box, popping when it changes after the header opened;
 * why the process failed as its tooltip and accessible description. */
function HeaderState({ status, failure }: { status: ThreadStatus; failure: string | undefined }): ReactNode {
  const first = useRef(status);
  return (
    <StateBadge
      key={status}
      state={threadMark(status)}
      fresh={status !== first.current}
      bare
      title={failure}
      aria-description={failure}
      data-testid="thread-state"
      data-status={status}
    >
      {statusLabel(status)}
    </StateBadge>
  );
}

/**
 * `N in background` while background commands or subagents of the thread run (UI-274, UI-747), saying so in its
 * tooltip; a press shows `omfp:background` — its tab, or a floating panel when the view lacks it. Its own component:
 * only it follows the thread's events.
 */
function BackgroundBadge({ threadId }: { threadId: string }): ReactNode {
  const running = useRunningInBackground(threadId);
  const [, reveal] = useWindowState(REVEAL_WIDGET);
  if (running === 0) {
    return null;
  }
  return (
    <Button
      size="sm"
      data-testid="background-badge"
      data-count={running}
      onPress={() => reveal({ reference: "omfp:background", nonce: Date.now() })}
    >
      <span title={text.inBackgroundTitle(running)}>{text.inBackground(running)}</span>
    </Button>
  );
}
