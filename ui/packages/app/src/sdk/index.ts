/**
 * `omfp/sdk` (AGT-016): what every widget builds on, built-in or written by an agent (docs/specs/08-agent-control.md);
 * the bundle exposes this module to widget modules through the page's import map (AGT-012).
 *
 * A window creates one {@link OmfpHost} over its protocol client ({@link createHost}) and provides it with
 * {@link OmfpProvider}; widgets then use only the hooks.
 *
 * `crates/omfp-daemon/src/widgets/docs.md` (`omfp://docs/widgets`) is the reference of every hook and component agents
 * use: keep it complete there.
 */

// View models, strings and helpers the built-in widgets use (AGT-016, AGT-021): a copied built-in imports them from
// `omfp/sdk` like any agent widget.
export { useVirtualizer, type VirtualItem } from "@tanstack/react-virtual";
export { keyLabel } from "../keymap/keymap.ts";
export { strings } from "../strings/index.ts";
export { type AnsiRun, ansiRuns } from "../vm/ansi.ts";
export { type BackgroundRow, type BackgroundSections, type JobTimes, lastOutputLine } from "../vm/background.ts";
export { type BrowserScreenshot, type BrowserTab, type BrowserView, browserView } from "../vm/browser.ts";
export {
  type ChangedFile,
  type ChangesRepo,
  type ChangesSections,
  changeRunsKey,
  changesSections,
  changesView,
} from "../vm/changes.ts";
export type {
  AstMatch,
  CodeActionItem,
  CodeLocation,
  CodeSymbol,
  Diagnostic,
  EditedFile,
  LspResult,
  SourceLine,
} from "../vm/code-intel.ts";
export {
  activityOf,
  alternative,
  behavior,
  canAbort,
  commandsOf,
  complete,
  completeFile,
  type FileMention,
  fileMention,
  fileSuggestions,
  INTERRUPT_WINDOW_MS,
  imageAttachment,
  interruptAction,
  isBlank,
  isBusy,
  keyAction,
  type Modifiers,
  queueKeys,
  queueModifier,
  restoreCleared,
  restoreDraft,
  restoredTexts,
  restoreFailedPrompt,
  type SendMode,
  type Suggestion,
  sendMode,
  slashSuggestions,
  type ThreadActivity,
} from "../vm/composer.ts";
export {
  type DebugBreakpointRow,
  type DebugFrameRow,
  type DebugLocation,
  type DebugStop,
  type DebugVariableRow,
  type DebugView,
  debugView,
} from "../vm/debug.ts";
export {
  closeTab,
  type FileTab,
  type FileTabs,
  type FloatingPanel,
  fileName,
  fileTabId,
  movePanel,
  NO_FILE_TABS,
  type OpenFileOptions,
  type OpenFiles,
  openedTabIds,
  openWidgetTab,
  originalFromPatch,
  resizePanel,
  widgetTabId,
  workspaceFilePath,
} from "../vm/files.ts";
export {
  costLabel,
  decodeModels,
  decodeSession,
  decodeThinkingLevels,
  type HeaderView,
  headerView,
  modelOption,
  NO_DETAILS,
  savedModel,
  splitModel,
  type ThreadCost,
  type ThreadDetails,
  threadCost,
} from "../vm/header.ts";
export {
  type HistoryStep,
  type HistoryWalk,
  historyDown,
  historyUp,
  onFirstLine,
  onLastLine,
} from "../vm/history.ts";
export {
  type AskForm,
  cardKey,
  checkAsk,
  closedRows,
  type Dialog as InteractionDialog,
  emptyAskForm,
  formatCountdown,
  isDialog,
  remainingMs,
  setCustom,
  timeoutMs,
  title as interactionTitle,
  toggleOption,
} from "../vm/interaction.ts";
export {
  formatWhen,
  type JobCountsView,
  type JobEventKind,
  type JobRow,
  jobCountsView,
  jobRow,
  type RunRow,
  runEventKinds,
  runMark,
  runOutcome,
  runRow,
  startedByText,
  threadJob,
} from "../vm/jobs.ts";
export {
  type FieldState,
  LINE_EDIT_COMMANDS,
  type LineEdit,
  type LineEditCommand,
  lineEdit,
} from "../vm/line-edit.ts";
export {
  type LivePanelView,
  type LiveSupport,
  type LocalLive,
  livePanelView,
  type MicButton,
  micButton,
} from "../vm/live.ts";
export {
  loginCompletions,
  logoutCompletions,
  logoutSummary,
  needsLogin,
  providerName,
  withAuthenticated,
} from "../vm/login.ts";
export { markdownChunks } from "../vm/markdown.ts";
export {
  MCP_IDLE,
  type McpMark,
  type McpPending,
  type McpRow,
  mcpRows,
  mcpStatus,
  type PendingRestart,
  pendingRestart,
  reauthPrompt,
} from "../vm/mcp.ts";
export type { NoticeRow } from "../vm/notice.ts";
export { loopbackCallback, OpenedUrlWatch, openedUrls, urlHost } from "../vm/opened-url.ts";
export {
  applyEdit,
  type Edited,
  editPaste,
  expandPastes,
  insertPaste,
  isLargePaste,
  isRawPaste,
  lineCount,
  rawPasteKeys,
  removePaste,
} from "../vm/paste.ts";
export {
  type PendingQueueOp,
  type QueueChip,
  type QueueOp,
  queueChips,
  queueOpApplied,
  settleQueueOp,
} from "../vm/queue.ts";
export {
  matchingBlocks as searchMatchingBlocks,
  occurrenceIndex as searchOccurrenceIndex,
  occurrences as searchOccurrences,
  revealScroll as searchRevealScroll,
  revealScrollLeft as searchRevealScrollLeft,
  type SearchCursor,
  searchPattern,
  stepSearch,
} from "../vm/search.ts";
export { type SlashRoute, slashAction } from "../vm/slash.ts";
export {
  appendTranscriptPage,
  EMPTY_TRANSCRIPT,
  type SubagentRow,
  type SubagentTranscript,
  type TranscriptLine as SubagentLine,
  transcriptLines as subagentLines,
} from "../vm/subagents.ts";
export type { SystemRow } from "../vm/system.ts";
export {
  type ForkLineage,
  forkLineage,
  type RepoKind,
  statusLabel,
  type ThreadStatus,
  threadMark,
  type WorkspaceChip,
  workspaceChip,
} from "../vm/threads.ts";
export { type GoalView, type TodoPhaseView, type TodoRow, type TodosView, todosView } from "../vm/todos.ts";
export { type CardImage, expandedByDefault, formatDuration, type ToolCard } from "../vm/tool.ts";
export {
  type AstEditFile,
  type RepoPath,
  repoPath,
  type SearchFile,
  type SubagentRun,
  type TodoPhase,
  type TodoTask,
  type ToolView,
} from "../vm/tool-view.ts";
export {
  type Block as MessageBlock,
  type DayMark,
  type DaySeparator,
  daySeparator,
  type Item as TranscriptItem,
  items as transcriptItems,
  type MessageView,
  type Role as MessageRole,
  type RowContent as TranscriptRowContent,
  rowContent as transcriptRowContent,
  startOfDay,
} from "../vm/transcript.ts";
export {
  messageEntryId,
  type NavigateOutcome,
  navigateOutcome,
  type TreeRow,
  type TreeView,
  treeView,
  withEditorText,
} from "../vm/tree.ts";
export type { ThreadViewChoice, ViewChoices } from "../vm/view-choices.ts";
export {
  arrivalScroll,
  halfPageScrollTop,
  INITIAL_PIN,
  type ItemEdges,
  type ItemExtent,
  isScrollUpKey,
  keyScrollTop,
  type MoveCommand,
  mountScrollTop,
  movedUpByUser,
  type NavCommand,
  navTarget,
  type PinEvent,
  type PinState,
  pagerScroll,
  pinReducer,
  refocusAfterScroll,
  type ScrollPosition,
  scrollPosition,
} from "../vm/virtual.ts";
export {
  ADD_KINDS,
  type AddKind,
  addRepoSource,
  type RepoRow,
  removalNeedsConfirmation,
  type WorkspaceRow,
  workspaceRow,
} from "../vm/workspaces.ts";
export {
  closeThread,
  currentWorkspaceId,
  errorMessage,
  removeWorkspace,
  reportError,
  reportSuccess,
  requestLogin,
  type ThreadActions,
  threadActions,
  type UrlActions,
  useCurrentWorkspaceId,
  useReportError,
  useReportSuccess,
  useThreadActions,
  useUrlActions,
} from "./actions.ts";
export { chooseView, useActiveView, useViewChoicesUpkeep } from "./active-view.ts";
export { type Background, useBackground, useRunningInBackground, useSubagentRows } from "./background.ts";
export { type Command, CommandRegistry, SCOPE_ATTRIBUTE, scopesAt } from "./commands.ts";
export { type ConversationActions, conversationActions, useConversationActions } from "./conversation.ts";
export { useThreadCost } from "./cost.ts";
export {
  cancelFilePicker,
  chooseFile,
  EDITOR_MODE,
  errorCode,
  FILE_PICKER,
  type FileLoadStatus,
  type FilePicker,
  type FilePoll,
  type FileState,
  type FileTabsState,
  FLOATING_FILES,
  floatingWidgetKey,
  type MissingOutcome,
  OPEN_FILES,
  openFile,
  type SaveResult,
  useFile,
  useFileTabs,
  useFileUrl,
  useMissingFile,
  useOpenFile,
} from "./files.ts";
export type { FrameScheduler, ModelSource, Snapshot } from "./frame-store.ts";
export {
  type CallState,
  registerCommand,
  useAnnounce,
  useCall,
  useCommand,
  useConnection,
  useIndex,
  useLive,
  useSnapshot,
  useThread,
  useThreadSummary,
  useThreads,
  useTopic,
  useViews,
  useWindowState,
  useWorkspace,
  useWorkspaces,
  type WorkspaceData,
} from "./hooks.ts";
export {
  type Announcement,
  createHost,
  type HostOptions,
  type OmfpHost,
  OmfpProvider,
  type SdkClient,
  useHost,
} from "./host.tsx";
export type {
  InitApi,
  InitBinding,
  InitCommand,
  InitContext,
  InitError,
  InitEventKind,
  InitFile,
  InitJobs,
  InitRepo,
  InitSnapshot,
  InitWorkspace,
  ThreadEventKind,
  ViewsApi,
} from "./init.ts";
export { Layout, type ViewEntry, type ViewOrigin, type ViewSpec } from "./init-views.ts";
export { type JobActions, type Jobs, jobActions, useJobs } from "./jobs.ts";
export {
  AUTH_REVISION,
  CLOSE_THREAD,
  type CloseRequest,
  COLOR_SCHEME,
  type ColorScheme,
  DELETE_WORKSPACE,
  DRAFT_MESSAGE,
  DRAFT_THREAD,
  type Draft,
  type DraftThread,
  draftKey,
  EMPTY_DRAFT,
  ERRORS,
  FOCUS_COMPOSER,
  LOGIN,
  type LoginRequest,
  MODEL_CHANGING,
  OPEN_JOB,
  OPEN_SUBAGENT,
  type OpenJobRequest,
  type OpenSubagentRequest,
  type Paste,
  RENAMING_THREAD,
  RESET_VIEWS,
  REVEAL_WIDGET,
  type RevealRequest,
  SELECTED_THREAD,
  SUCCESSES,
  type Success,
} from "./keys.ts";
export {
  Col,
  type ContainerElementProps,
  Row,
  Stack,
  Tabs,
  type TabsElementProps,
  Widget,
  type WidgetElementProps,
} from "./layout.tsx";
export { LiveClient, type LiveClientOptions, type LiveLink, type LiveMedia, type LiveSnapshot } from "./live.ts";
export type { Importer, LoadedWidget, WidgetLoadError } from "./modules.ts";
export {
  MESSAGES,
  type Message,
  type MessageTone,
  NOTICES,
  type Notice,
  type NoticeRequest,
  type NoticeRoute,
  Notices,
  type NoticeTimers,
  type NoticeTone,
  useNotify,
} from "./notify.ts";
export { OAuthCallbackField } from "./oauth-callback.tsx";
export { type PathSources, usePathSources } from "./paths.ts";
export {
  type ReportSink,
  type ReportTimers,
  type SlotIdentity,
  type SlotLoad,
  SlotReport,
  SlotReportScope,
  WidgetReporter,
} from "./reports.ts";
export {
  highlightRegistry,
  paintSearchHighlights,
  SEARCH_CURRENT_HIGHLIGHT,
  SEARCH_MATCH_HIGHLIGHT,
  type SearchMarks,
} from "./search-highlights.ts";
export {
  DEFAULT_DENSITY,
  DEFAULT_FONT_SIZE,
  DEFAULT_MOTION,
  DEFAULT_NOTIFICATIONS,
  DEFAULT_SKIN,
  DEFAULT_THEME,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  useSetSettings,
  useSettings,
} from "./settings.ts";
export {
  type Job,
  type JobOptions,
  type JobRunStatus,
  type StoredUpdate,
  type Terminal,
  type TerminalOptions,
  type TerminalScreenOptions,
  type ToolRunInfo,
  type ToolRunTarget,
  useJob,
  useStored,
  useTerminal,
  useToolRun,
} from "./sources.ts";
export {
  useWidget,
  type WidgetComponent,
  type WidgetInfoScope,
  type WidgetProps,
  WidgetScope,
} from "./widget.ts";
export { type KeyValueStorage, type Update, type WindowKey, windowKey } from "./window-store.ts";
export { parseYaml, type YamlResult } from "./yaml.ts";
