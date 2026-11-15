/** Texts of the frame and the shell widgets (threads, header, workspace, dialogs, command line). */

/** What a command of a scope says where it does not run (UI-352, UI-407). */
const scopedCommand = (name: string, scope: string) => `${name} runs only with the focus in the ${scope} scope`;

export const shell = {
  untitled: "New thread",
  status: {
    awaiting: "awaiting input",
    failed: "failed",
    running: "running",
    starting: "starting",
    idle: "idle",
    stopped: "stopped",
  },
  chip: {
    label: (name: string, repos: number) => `${name} · ${repos} ${repos === 1 ? "repo" : "repos"}`,
    noRepos: "No repositories",
  },
  threads: {
    label: "Threads",
    filter: "Filter threads",
    unread: "unread",
    empty: "No thread",
    noMatch: "No thread matches",
    addThread: "+ thread",
    draft: "draft",
    archived: (count: number) => `Archived (${count})`,
    unarchive: "Unarchive",
    unarchiveThread: (title: string) => `Unarchive \u201c${title}\u201d`,
    deleteThread: (title: string) => `Delete \u201c${title}\u201d`,
    collapse: (name: string) => `Collapse ${name}`,
    expand: (name: string) => `Expand ${name}`,
    menu: (name: string) => `Workspace ${name}`,
    rename: "Rename…",
    remove: "Remove…",
    renameLabel: "Workspace name",
    deleteWorkspace: (name: string) => `Delete workspace ${name}…`,
    current: "current workspace",
    /** The "spawned by" filter (UI-240). */
    spawnedFilter: (count: number, title: string) =>
      count === 1
        ? `Show the thread \u201c${title}\u201d spawned`
        : `Show the ${count} threads \u201c${title}\u201d spawned`,
    spawnedBy: (title: string) => `Spawned by ${title}`,
    clearSpawned: "Show every thread",
    /** The notice of a stop with the spawned threads (UI-242). */
    stoppedSpawned: (count: number) => (count === 1 ? "Stopped 1 thread" : `Stopped ${count} threads`),
  },
  /** Dragging and moving rows and sections of the left column (UI-620..UI-627). */
  reorder: {
    movedThread: (title: string, place: number, count: number, workspace: string) =>
      `Moved ${title} to position ${place} of ${count} in ${workspace}`,
    movedThreadTo: (title: string, workspace: string, place: number, count: number) =>
      `Moved ${title} to ${workspace}, position ${place} of ${count}`,
    movedWorkspace: (name: string, place: number, count: number) =>
      `Moved workspace ${name} to position ${place} of ${count}`,
    filtering: "Clear the filter to reorder",
  },
  newWorkspace: {
    label: "+ workspace",
    placeholder: "+ workspace: name, or /path of a repository to link, Enter",
    hint: "No workspace yet: type a name, or the path of a repository to link (/ or ~), and press Enter to start a thread in it.",
  },
  draft: {
    empty: "A new thread: send the first message to start it.",
    placeholder: "First message…",
    send: "Start thread",
  },
  connection: {
    connecting: "Connecting to the daemon…",
    reconnecting: (seconds: number, attempt: number) =>
      `Connection lost. Reconnecting in ${seconds} s (attempt ${attempt})…`,
    unauthenticated: "The daemon rejected the access token. Open the link printed by `omfp daemon`.",
    versionMismatch: (message: string) => `The daemon speaks another protocol version: ${message}`,
    closed: "Disconnected from the daemon.",
    awaitingOne: "1 thread awaits input",
    awaitingMany: (count: number) => `${count} threads await input`,
    threadState: (title: string, state: string) => `${title}: ${state}`,
  },
  token: {
    title: "Connect to the daemon",
    help: "Open the link printed by `omfp daemon`, or paste the access token from its data directory.",
    label: "Access token",
    rejected: "The daemon rejected this token.",
    submit: "Connect",
  },
  header: {
    /** The badge of running background work (UI-274). */
    inBackground: (count: number) => `${count} in background`,
    rename: "Rename",
    renameLabel: "Title",
    save: "Save",
    cancel: "Cancel",
    model: "Model",
    modelPlaceholder: "Model…",
    thinking: "Thinking level",
    thinkingPlaceholder: "Thinking…",
    contextUsage: "Context window usage",
    context: (percent: number, window: string) => `${percent}% of ${window}`,
    /** The session's own cost, then its subagents' when they spent (UI-690), as omp's status line shows them. */
    cost: (session: string, subagents: string | undefined) =>
      subagents === undefined ? session : `${session} (+${subagents})`,
    costDescription: (session: string, subagents: string | undefined) =>
      subagents === undefined ? `Cost: session ${session}` : `Cost: session ${session}, subagents ${subagents}`,
    /** Ends the thread's omp process (UI-524); the composer's Abort stops only the turn. */
    stop: "Stop process",
    restart: "Restart",
    archive: "Archive",
    unarchive: "Unarchive",
    delete: "Delete",
    noThread: "No thread selected",
    /** The compact header's `⋯` button and the popover it opens (UI-202). */
    more: "More thread actions",
    moreTitle: "Thread",
    /** Log in beside a failed start that says omp needs a login, and in an empty model list (UI-387, UI-388). */
    logIn: "Log in",
    logInHint: "omp has no model it can use: log in to a provider",
    noModels: "No models",
  },
  deleteWorkspace: {
    title: (name: string) => `Delete workspace \u201c${name}\u201d?`,
    message: (path: string) =>
      `Its folder ${path} is deleted with the clones and worktrees in it; linked projects are kept (only the link is removed).`,
    threads: (count: number) => (count === 1 ? "It still has 1 thread:" : `It still has ${count} threads:`),
    deleteThreads: "Delete these threads too (their history is removed for good)",
    confirm: "Delete",
    cancel: "Cancel",
  },
  workspace: {
    label: "Workspace",
    none: "No workspace selected",
    rename: "Rename",
    nameLabel: "Workspace name",
    save: "Save",
    cancel: "Cancel",
    path: "Folder",
    copyPath: "Copy folder path",
    copied: "Folder path copied",
    repos: "Repositories",
    noRepo: "no repository",
    shared: (count: number) => `shared by ${count} threads`,
    sharedHint: "Several threads work in this directory: their changes mix.",
    alsoIn: (names: string) => `also in ${names}`,
    alsoInHint: "The same checkout is in another workspace with live threads.",
    dirty: "dirty",
    clean: "clean",
    detached: "detached",
    aheadBehind: (ahead: number, behind: number) => `↑${ahead} ↓${behind}`,
    files: (count: number, additions: number, deletions: number) =>
      `${count} ${count === 1 ? "file" : "files"} +${additions} −${deletions}`,
    linkTarget: (target: string) => `→ ${target}`,
    worktreeOf: (main: string) => `of ${main}`,
    kinds: { link: "Link", worktree: "Worktree", clone: "Clone" },
    removeRepo: (name: string) => `Remove ${name}`,
    removeNow: "Remove",
    removeRepoTitle: (name: string) => `Remove ${name}?`,
    removeRepoMessage: "The worktree or clone is deleted from the workspace folder.",
    addRepository: "Add repository",
    addKind: "Kind of repository",
    add: "Add",
    linkPath: "Folder to link",
    linkPlaceholder: "/path/to/folder",
    cloneUrl: "Clone URL",
    clonePlaceholder: "https://…",
  },
  changes: {
    label: "Changes",
    none: "No workspace selected",
    noRepos: "No repository in this workspace",
    /** The mark of the workspace folder's own repository (UI-341). */
    workspace: "workspace",
    workspaceLabel: (name: string) => `${name}, the workspace folder`,
    refresh: "Refresh the changes",
    loading: "Reading the git status…",
    loadingDiff: "Reading the diff…",
    /** The disclosure grouping the clean repositories (UI-342). */
    cleanRepos: (count: number) => (count === 1 ? "1 clean repository" : `${count} clean repositories`),
    truncated: "The diff is too large and was cut.",
    renamedFrom: (path: string) => `from ${path}`,
    /** One letter per kind of change, as `git status --short` writes them. */
    status: { added: "A", modified: "M", deleted: "D", renamed: "R", untracked: "U" },
    statusLabel: {
      added: "added",
      modified: "modified",
      deleted: "deleted",
      renamed: "renamed",
      untracked: "untracked",
    },
    diffLabel: (path: string) => `Diff of ${path}`,
    open: (path: string) => `Open ${path}`,
  },
  /** The subagent roster and transcripts (`vm/subagents.ts`, UI-081). */
  subagents: {
    tools: (count: number) => (count === 1 ? "1 tool" : `${count} tools`),
    tokens: (count: string) => `${count} tokens`,
    toolLine: (tool: string, intent: string | undefined) =>
      intent === undefined ? `→ ${tool}` : `→ ${tool}: ${intent}`,
  },
  /** `omfp:background` (UI-271..UI-273, UI-490..UI-492, UI-641..UI-643). */
  background: {
    label: "Background",
    noThread: "No thread selected",
    none: "Nothing runs in the background",
    noneHint: "Background jobs of the agent and its subagents appear here.",
    showJob: (id: string) => `Show the card of background job ${id}`,
    subagent: (name: string) => `Subagent ${name}`,
    state: { running: "running", completed: "done", failed: "failed", cancelled: "cancelled" },
    stopRequested: "stop requested",
    stop: "Stop",
    cancel: "Cancel",
    transcript: (name: string) => `Transcript of ${name}`,
    /** The finished rows' heading and their Clear finished (UI-641, UI-642). */
    finished: "Finished",
    clearFinished: "Clear finished",
    /** The collapsed group of the older finished rows (UI-641). */
    older: (count: number) => `Finished (${count})`,
    loading: "Reading the transcript…",
    empty: "Nothing said yet",
    more: "Read new messages",
    steer: "Steer",
    steerLabel: (name: string) => `Message for ${name}`,
    steerPlaceholder: "Tell it what to do differently",
    roleUser: "task",
    roleAssistant: "subagent",
  },
  /** `omfp:mcp` (UI-290..UI-294). */
  mcp: {
    label: "MCP servers",
    noWorkspace: "No workspace",
    noWorkspaceHint: "Select a thread to see the MCP servers of its workspace.",
    loading: "Loading MCP servers…",
    none: "No MCP servers",
    noneHint: (projectConfig: string, userConfig: string | null | undefined) =>
      `Add them to ${projectConfig} (this workspace), or to ${userConfig ?? "omfp's isolated profile"} for every workspace, as ` +
      '{ "mcpServers": { "<name>": { "command": "<program>", "args": [] } } } or ' +
      '{ "mcpServers": { "<name>": { "type": "http", "url": "<url>" } } }. omp also reads the configs of ' +
      "Claude Code (.claude/), Cursor (.cursor/) and VS Code (.vscode/mcp.json).",
    workspaceOnly: "No thread selected: the servers configured for this workspace, not checked.",
    notRunning: "The thread is not running: its servers start with it.",
    checked: "Checked by the thread's omp.",
    notCheckable:
      "omp checks only its native project and isolated-profile mcp.json files: it loads this one without a check.",
    state: {
      configured: "configured",
      connected: "connected",
      failed: "failed",
      disabled: "disabled",
      checking: "checking…",
    },
    tools: (count: number) => (count === 1 ? "1 tool" : `${count} tools`),
    origin: (tool: string, scope: string) => `${tool} · ${scope}`,
    openSource: (path: string) => `Open ${path}`,
    checkAgain: "Check again",
    reloadAll: "Reload all",
    reloadAllHint: "Restarts the thread's omp, which connects every MCP server again",
    reconnect: "Reconnect",
    reconnectLabel: (name: string) => `Reconnect ${name} (restarts the thread's omp)`,
    /** `/mcp reauth <name>` in the thread (UI-348). */
    reauthorize: "Reauthorize",
    reauthorizeLabel: (name: string) => `Reauthorize ${name} (sends /mcp reauth ${name} to the thread)`,
    /** The daemon holds a restart while background work runs (UI-349), named by what it loads (PRT-134). */
    restartPending: {
      mcp: {
        label: "Reload to load MCP tools",
        hint: "The MCP servers changed while background work ran, so the thread's omp was not restarted; this restarts it now",
      },
      login: {
        label: "Restart to load new models",
        hint: "You logged in to a provider while background work ran, so the thread's omp was not restarted; this restarts it now to list the provider's models",
      },
      both: {
        label: "Restart to load new models and MCP tools",
        hint: "You logged in to a provider and the MCP servers changed while background work ran, so the thread's omp was not restarted; this restarts it now",
      },
    },
  },
  /** `omfp:todos` (UI-082). */
  todos: {
    label: "Todos",
    noThread: "No thread selected",
    none: "No todo list",
    noneHint: "The agent's todo tool fills it.",
    goal: "Goal",
    tokens: (used: string) => `${used} tokens`,
    tokensOf: (used: string, budget: string) => `${used} / ${budget} tokens`,
    goalStatus: {
      active: "active",
      paused: "paused",
      "budget-limited": "budget reached",
      complete: "complete",
      dropped: "dropped",
    } as Readonly<Record<string, string>>,
    taskStatus: {
      pending: "pending",
      in_progress: "in progress",
      completed: "done",
      abandoned: "abandoned",
      blocked: "blocked",
    } as Readonly<Record<string, string>>,
    blocked: (blocker: string) => `blocked: ${blocker}`,
  },
  /** `omfp:tree` (UI-083). */
  tree: {
    label: "Session tree",
    noThread: "No thread selected",
    loading: "Reading the session tree…",
    empty: "Nothing in this session yet",
    refresh: "Refresh the tree",
    tools: (names: readonly string[]) => (names.length === 0 ? "(no text)" : `→ ${names.join(", ")}`),
    branchSummary: (summary: string) => `Branch summary: ${summary}`,
    compaction: (summary: string) => `Compaction: ${summary}`,
    leaf: "you are here",
    roleUser: "▸",
    roleAssistant: "◆",
    roleSummary: "≡",
    navigate: "Go here",
    navigateLabel: (entry: string) => `Go to ${entry}`,
    summarize: "Summarize and go",
    summarizeLabel: (entry: string) => `Summarize the branch left and go to ${entry}`,
    fork: "Fork from here",
    forkLabel: (entry: string) => `Fork a new thread from ${entry}`,
    labelAction: "Label",
    labelLabel: (entry: string) => `Label ${entry}`,
    labelField: "Label (empty removes it)",
    save: "Save",
    announce: {
      navigated: "Moved in the session tree",
      editorText: "Moved in the session tree; the message is back in the composer",
      cancelled: "Nothing moved",
    },
  },
  /** Forking a thread into a new one (UI-660..UI-666). */
  fork: {
    action: "Fork from here",
    actionLabel: (message: string) => `Fork a new thread from \u201c${message}\u201d`,
    forked: (title: string) => `Forked into \u201c${title}\u201d`,
    lineage: (title: string, message: string | undefined) =>
      message === undefined ? `Forked from ${title}` : `Forked from ${title} at \u201c${message}\u201d`,
    goToSource: (title: string) => `Go to ${title}, the thread this one was forked from`,
    noMessage: "Select a message to fork from: j and k move between messages",
    noEntry: "Select a message of the tree to fork from",
    notOnBranch: "That message is not on the session's current branch: open omfp:tree to fork from it",
    noTimestamp: "That message has no time stamp to find it in the session by",
  },
  /** `omfp:activity` (UI-241). */
  activity: {
    label: "Activity",
    empty: "No agent activity yet",
    emptyHint: "What agents do through omfp (threads, workspaces, files, jobs) shows here.",
    thisThread: "This thread",
    older: "Load older",
    loading: "Reading the activity log…",
    failed: (message: string) => `failed: ${message}`,
    selectThread: (title: string) => `Select ${title}`,
    /** Who made an entry of an MCP client outside omfp (AGT-053). */
    mcpClient: (client: string) => `mcp: ${client}`,
  },
  /** The file viewer and editor (`omfp:file`, UI-110..114). */
  file: {
    noPath: "No file: give this widget a path prop.",
    loading: "Reading the file…",
    loadingEditor: "Loading the editor…",
    missing: (path: string) => `${path} does not exist.`,
    finding: (path: string) => `Looking for ${path} in the workspace…`,
    choosing: (path: string) => `Several files match ${path}: choose one below.`,
    failed: (error: string) => `Cannot read the file: ${error}`,
    label: (path: string) => `File ${path}`,
    editorLabel: (path: string) => `Text of ${path}`,
    diffLabel: (path: string) => `${path} against HEAD`,
    conflictLabel: (path: string) => `${path} on disk against your text`,
    edit: "Edit",
    done: "Done",
    save: "Save",
    diff: "Diff",
    unified: "Unified",
    split: "Side by side",
    layout: "Diff layout",
    /** The toggle between a file's source and its viewer (UI-560), named by the viewer. */
    viewers: { markdown: "Preview", table: "Table", tree: "Tree" },
    previewLabel: (path: string) => `${path} rendered`,
    tableLabel: (path: string) => `${path} as a table`,
    /** The table viewer's line under the table (UI-563, UI-565); `cutSize` when the daemon read only the beginning. */
    tableSummary: (rows: number, columns: number, cutSize: string | undefined) =>
      `${rows} ${rows === 1 ? "row" : "rows"}, ${columns} ${columns === 1 ? "column" : "columns"}${
        cutSize === undefined ? "" : `: the first rows of this ${cutSize} file, which is too large to read whole`
      }`,
    /** The tree viewer (UI-567..569). */
    treeLabel: (path: string) => `${path} as a tree`,
    treeKeys: (count: number) => `${count} ${count === 1 ? "key" : "keys"}`,
    treeItems: (count: number) => `${count} ${count === 1 ? "item" : "items"}`,
    treeDocuments: (count: number) => `${count} documents`,
    treeMore: (count: number) => `Show more (${count} left)`,
    copyPath: "Path",
    copyValue: "Value",
    copyPathLabel: (path: string) => `Copy the path ${path}`,
    copyValueLabel: (path: string) => `Copy the value of ${path}`,
    copiedPath: (path: string) => `Copied the path ${path}`,
    copiedValue: (path: string) => `Copied the value of ${path}`,
    treeFailed: (message: string, line: number, column: number) =>
      `Cannot show this file as a tree: ${message} (line ${line}, column ${column}).`,
    saved: (path: string) => `Saved ${path}`,
    saveFailed: (error: string) => `Cannot save: ${error}`,
    unsaved: "Unsaved changes: save with :w, or drop them with :q!",
    changedOnDisk: "The file changed on disk since you started editing; saving will show both versions.",
    truncated: (size: string) => `Only the beginning of this ${size} file is shown; it cannot be edited here.`,
    /** A file read by its absolute path outside the workspace folder (UI-700): never edited. */
    outside: "Outside the workspace: this file can be read here, not edited.",
    binary: (size: string) => `Binary file (${size}), not shown.`,
    imageLabel: (path: string) => `Image ${path}`,
    headImage: (path: string) => `${path} at HEAD`,
    slider: "Slider",
    /** The image viewer (UI-548..UI-555). */
    image: {
      zoomIn: "Zoom in",
      zoomOut: "Zoom out",
      fit: "Fit",
      actualSize: "100%",
      toolbar: "Zoom",
      failed: "This window cannot show this image.",
      before: "HEAD",
      after: "Working copy",
      slider: "HEAD | working copy",
    },
    cannotPlay: (kind: "video" | "audio", size: string) => `This window cannot play this ${kind} file (${size}).`,
    playerLabel: (kind: "video" | "audio", path: string) => `${kind === "video" ? "Video" : "Audio"} ${path}`,
    pdfLabel: (path: string) => `PDF ${path}`,
    /** The PDF viewer (UI-543..UI-547). */
    pdf: {
      loading: "Loading the PDF…",
      failed: (error: string) => `Cannot show this PDF: ${error}`,
      zoomIn: "Zoom in",
      zoomOut: "Zoom out",
      fitWidth: "Fit width",
      page: (page: number, pages: number) => `${page} / ${pages}`,
      pageLabel: (page: number) => `Page ${page}`,
      toolbar: "Zoom and pages",
      pages: "Pages",
    },
    noRepository: "Not in a repository: there is no HEAD to compare with.",
    newFile: "Not in HEAD yet: every line is new.",
    conflict:
      "The file changed on disk since you started editing. Left: the file now; right: your text. Nothing was written.",
    keepEditing: "Keep editing",
    overwrite: "Overwrite",
    discard: "Discard mine",
    size: (bytes: number) =>
      bytes < 1024
        ? `${bytes} B`
        : bytes < 1024 * 1024
          ? `${(bytes / 1024).toFixed(1)} KiB`
          : `${(bytes / 1024 / 1024).toFixed(1)} MiB`,
  },
  /** File tabs of the `main` slot, the floating file panel (UI-115) and the file picker (UI-117). */
  files: {
    tabs: "Files",
    close: (name: string) => `Close ${name}`,
    panel: (count: number) => `Open files (${count})`,
    move: "Move the panel (arrow keys)",
    resize: "Resize the panel (arrow keys)",
    minimise: "Minimise the files",
    restore: "Show the files",
    noWorkspace: (path: string) => `No workspace to open ${path} in`,
    picker: (path: string) => `Files matching ${path}`,
    pickerPrompt: "open",
    exact: "exact path",
    suffix: "path ends so",
    basename: "same name",
    mentioned: "in the conversation",
  },
  /** Floating widget panels (UI-230). */
  panels: {
    move: (title: string) => `Move ${title} (arrow keys)`,
    resize: (title: string) => `Resize ${title} (arrow keys)`,
    minimise: (title: string) => `Minimise ${title}`,
    restore: (title: string) => `Show ${title}`,
    close: (title: string) => `Close ${title}`,
  },
  /** The mode line (UI-098); vim's modes while a file editor has the focus (UI-112). */
  mode: {
    names: { normal: "NORMAL", insert: "INSERT", cmdline: "COMMAND" },
    vim: { normal: "NORMAL", insert: "INSERT", visual: "VISUAL", replace: "REPLACE" },
    announce: (name: string) => `${name} mode`,
  },
  frame: {
    /** The view tabs of the mode line (UI-301). */
    viewTabs: "Views",
    invalidView: (error: string) => `The view is invalid, showing the default view: ${error}`,
    dismiss: "Dismiss",
    /** A URL omp opened in a browser (UI-346). */
    openedUrl: (host: string) => `omp opened ${host} in the browser`,
    commandLine: "Command line",
    /** The completion menu's `╭─` title and its key row (UI-168). */
    commandMenu: "commands",
    commandKeys: { complete: "complete", run: "run", cancel: "cancel" },
    pickerKeys: { move: "preview", choose: "confirm" },
    filesMenu: "files",
    unknownCommand: (name: string) => `Unknown command: ${name}`,
    scopedCommand,
    defaultView: "Default",
    /** Tab labels of the built-in widgets without a `title` prop (AGT-010). */
    widgetTitles: {
      "omfp:header": "Header",
      "omfp:chat": "Chat",
      "omfp:transcript": "Transcript",
      "omfp:interactions": "Interactions",
      "omfp:composer": "Composer",
      "omfp:workspace": "Workspace",
      "omfp:changes": "Changes",
      "omfp:file": "File",
      "omfp:background": "Background",
      "omfp:todos": "Todos",
      "omfp:tree": "Tree",
      "omfp:activity": "Activity",
      "omfp:mcp": "MCP servers",
      "omfp:web": "Web",
    } as Readonly<Record<string, string>>,
  },
  /** The narrow and mobile frame (UI-200..UI-205). */
  responsive: {
    strip: "Threads column",
    showThreads: "Show threads",
    hideThreads: "Hide threads",
    areas: "Areas",
    threads: "Threads",
    chat: "Chat",
    side: "Side",
    leftSplitter: "Resize the threads column",
    tabs: (container: string, path: string) => `${container} ${path}`,
  },
  /** Answers to the daemon's requests for an agent (UI-352, UI-353): never shown in the window. */
  windowRequests: {
    unknown: (name: string) =>
      `no command ${JSON.stringify(name)}: omfp_windows lists the commands init files and widgets add, omfp://keymap the built-ins`,
    scoped: (name: string, scope: string) => `${scopedCommand(JSON.stringify(name), scope)}; run a global command`,
    disabled: (id: string) => `${id} is disabled now`,
    noWorkspace: "this window shows no workspace, so it renders no workspace or user widget",
    unknownWidget: (reference: string) => `this window does not know ${reference} yet`,
  },
  layout: {
    splitter: (container: string, path: string, index: number) => `Resize ${container} ${path} at ${index}`,
    noView: (id: string) => `No view ${id}`,
    /** `:widget add` on a view that exists only in code (UI-300). */
    codeFile: (id: string, path: string) =>
      `View ${id} is the TSX file ${path}: add the widget there (<Widget use="…" />), :widget add writes only views.json`,
    codeInit: (id: string, file: string) =>
      `View ${id} comes from ${file}: add the widget there (layout.addTab), :widget add writes only views.json`,
    invalidManifest: "The layout cannot change while .omfp/views.json is invalid: fix the file first",
    loading: "The views of this workspace are still loading",
    noWorkspace: "No current workspace",
    reset: ".omfp/views.json reset to the default view",
  },
  /** `:view-reset` asks first (UI-430): it replaces every view, mount and added widget of the workspace. */
  resetViews: {
    title: "Reset .omfp/views.json?",
    message:
      "The workspace's views.json is replaced by the default view: its other views, mounted widgets and panels are lost in every window. To get a newer built-in, :widget add omfp:<name> is enough.",
    confirm: "Reset",
    cancel: "Cancel",
  },
  widgetCopy: {
    usage: "Usage: :widget add <reference> | :widget copy <reference> [<name>] | :widget save[!] <widget> [<name>]",
    added: (reference: string, view: string) => `Added ${reference} to view ${view}`,
    copied: (name: string) => `Copied to ./widgets/${name}`,
    saved: (reference: string) => `Saved to the user library as ${reference}`,
  },
  settings: {
    setUsage:
      "Usage: :set <key>=<value> (density, font_size, motion, notifications, show_xdev_notices, theme, fonts.prose|ui|code, keymap[.<mode>].<keys>)",
    unknownKey: (key: string) => `Unknown setting: ${key}`,
    themeUsage:
      "Usage: :theme [system|light|dark] [<theme>] (synth, catppuccin, gruvbox, tokyo-night, nord, solarized, rose-pine, accent)",
    unknownTheme: (word: string) => `Unknown colour scheme or theme: ${word}`,
    invalidValue: (key: string, value: string) => `Invalid value for ${key}: ${value === "" ? "(empty)" : value}`,
    error: (message: string) => `The config file is invalid, the previous settings stay: ${message}`,
  },
  /** Logging in to a provider (UI-380..UI-389). */
  login: {
    logIn: "Log in",
    /** An error toast's text when omp needs a login (UI-451); omp's own output goes under Details. */
    noModel: "No model available: log in to a provider.",
    noCredential: (provider: string) => `No credential for ${provider}: log in to use its models.`,
    details: "Details",
    /** The provider picker (UI-382). */
    pickerTitle: "Log in to a provider",
    pickerLabel: "Providers",
    loadingProviders: "Asking omp which providers it can log in to…",
    noProviders: "omp offers no provider to log in to",
    providersFailed: (message: string) => `Could not list the providers: ${message}`,
    authenticated: "logged in",
    unavailable: "not available here",
    cancel: "Cancel",
    /** The progress toast while `auth.login` runs (UI-383). */
    progress: (name: string) => `Logging in to ${name}… finish in your browser`,
    cancelLogin: "Cancel",
    cancelLoginLabel: (name: string) => `Cancel the login to ${name}`,
    /** The opened-URL toast of a login (UI-380). */
    url: (name: string, host: string) => `Log in to ${name} at ${host}`,
    /** The paste dialog of a `login_input` (UI-384). */
    inputTitle: (name: string) => `Log in to ${name}`,
    submit: "Continue",
    /** Outcomes (UI-385, UI-386). */
    loggedIn: (name: string) => `Logged in to ${name}`,
    removed: (name: string, count: number) =>
      `Logged out of ${name}: removed ${count} stored ${count === 1 ? "credential" : "credentials"}.`,
    removedNone: (name: string) => `omp held no stored credential of ${name}.`,
    stillAuthenticated: (name: string, source: string) => `${name} still authenticates with ${source}.`,
    logoutUsage: "Usage: :logout <provider>",
  },
  /** The first run's welcome when omp has no usable model (UI-500, UI-501). */
  welcome: {
    title: "Welcome to omfp",
    why: "omp has no model it can use yet. Log in to a provider to start your first thread.",
    logIn: "Log in to a provider",
    keys: "Or give omp an API key (ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY, …) in the daemon's environment and restart it.",
  },
  /** Notices in the mode line and system notifications (UI-096, UI-192). */
  notices: {
    attention: "needs you",
    failed: "failed",
    finished: "finished its turn",
    open: "Open the newest notice",
    announce: (title: string, body: string) => `${title} ${body}`,
  },
  /** The window's messages (UI-521..UI-523): the notices and toasts that went by. */
  messages: {
    title: "Messages",
    /** The mode line's button that opens them. */
    open: "Messages",
    empty: "No messages yet: notices and the results of your actions land here.",
    close: "Close",
    /** A message's kind, after `◆` in its tone's color: a notice (its text says what happened), or a toast's. */
    kinds: {
      attention: "notice",
      failed: "notice",
      finished: "notice",
      info: "notice",
      success: "done",
      error: "error",
    },
  },
  commands: {
    new: "New thread in the current workspace",
    newWorkspace: "New workspace",
    rescan: "Rescan the current workspace",
    archive: "Archive the thread",
    unarchive: "Unarchive the thread",
    delete: "Delete thread now",
    deleteDescription:
      "Immediately removes the thread's history without confirmation; its workspace is kept. Use Archive to keep the conversation.",
    rename: "Rename the thread (:rename <title>, or in place without one)",
    stop: "Stop process",
    /** `:describe-command stop` (UI-524): what Stop process ends, unlike Abort. */
    stopDescription:
      "Ends the thread's omp process, with its background jobs and live voice; Restart starts it again. To stop only the running turn, use Abort (ctrl+c).",
    stopSpawned: "Stop process of the thread and of every thread it spawned",
    spawnedBy: "Show only the threads the selected one spawned (again: every thread)",
    restart: "Restart the thread's process",
    fork: "Fork a new thread from the selected message (transcript) or entry (session tree)",
    /** `:describe-command fork` (UI-663). */
    forkDescription:
      "Makes a new thread in the same workspace holding the conversation up to the message, the message included; the thread you fork from is left as it is.",
    nextAttention: "Next thread needing attention",
    theme: "Colour scheme (system, light, dark) and built-in theme (synth, catppuccin, gruvbox, …)",
    density: "Density (compact, normal)",
    set: "Change a setting (:set key=value)",
    cmdline: "Open the command line",
    search: "Search the current conversation",
    pickThread: "Pick a thread (live preview; Escape restores the original)",
    deleteFocused: "Delete focused thread or workspace",
    toggleSection: "Collapse or expand the focused workspace",
    moveUp: "Move the focused thread or workspace up",
    moveDown: "Move the focused thread or workspace down",
    moveToPrevWorkspace: "Move the focused thread to the end of the previous workspace",
    moveToNextWorkspace: "Move the focused thread to the start of the next workspace",
    view: "Switch view (:view <id>)",
    resetViews: "Replace the workspace's views.json with the default view, after a confirmation",
    nextView: "Next view",
    prevView: "Previous view",
    pickView: "Pick a view (opens :view with the views to complete)",
    widget:
      "Add a widget to the view, copy one into the workspace or save one to the user library (:widget add <reference>, :widget copy|save[!] <widget> [<name>])",
    open: "Open a file (:open <path>[:<line>]), or the focused path (g f)",
    next: "Next thread",
    prev: "Previous thread",
    top: "First thread",
    bottom: "Last thread",
    halfDown: "Half a page of threads down",
    halfUp: "Half a page of threads up",
    help: "Show every key and command (this help)",
    toggleThreads: "Focus threads (wide); show or hide threads (narrow)",
    login: "Log in to a provider (:login [<provider>]; without one, pick it)",
    logout: "Log out of a provider (:logout <provider>)",
    messages: "Show the notices and toasts that went by (newest first)",
  },
  /** The help panel (UI-198): the effective keymap by mode and the `:` commands. */
  help: {
    /** The title on the box's top border. */
    title: "help",
    label: "Keys and commands",
    filter: "Filter keys and commands",
    filterPlaceholder: "Filter by key or description",
    noMatch: "No key or command matches",
    close: "close",
    sections: {
      normal: "normal",
      insert: "insert · composer",
      transcript: "transcript",
      cmdline: "command line",
      editor: "file editor · vim",
      commands: "commands",
    },
    /** Keys a widget handles itself, outside the keymap. */
    keys: {
      send: "Send the message (steers the running turn while the agent works)",
      queue: "Queue the message as a follow-up while the agent works",
      newline: "New line",
      complete: "Complete the / command",
      pasteRaw: "Paste a large text as it is, not as a chip",
      history: "Previous / next sent prompt (caret on the first / last line)",
      page: "Scroll a page up or down (the focus follows when its message leaves the view)",
      ends: "Scroll to the top or the bottom",
      cmdComplete: "Complete the command or its argument",
      cmdMove: "Move through the completions",
      cmdRun: "Run the line (or the highlighted completion)",
      cmdCancel: "Close the line",
      vimEsc: "Back to vim's normal mode; in normal mode with nothing pending, leave the editor",
      vimSave: "Save the file",
      vimQuit: "Back to read-only (refused while there are unsaved changes)",
      vimDrop: "Drop the changes and reload the file",
    },
    editorNote: "While the file editor has the focus, vim's own keys apply; the app keymap leaves them alone.",
  },
  /** The phone's commands sheet (UI-460..UI-464). */
  commandsSheet: {
    /** The area bar's button and the sheet's title. */
    title: "Commands",
    close: "Close",
    search: "Search commands",
    placeholder: "Search",
    noMatch: "No command matches",
    /** The headings of the which-key groups (UI-326) on the sheet. */
    groups: {
      threads: "Threads",
      views: "Views",
      workspace: "Workspace",
      files: "Files",
      help: "Help",
      navigation: "Move",
      commands: "Settings and command line",
      other: "Other",
    },
  },
} as const;
