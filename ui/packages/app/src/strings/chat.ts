/** Texts of the conversation widgets (`omfp:chat`, transcript, interactions, composer). */

export const chat = {
  noThread: "Select a thread, or create one.",
  loading: "Loading the thread…",
  transcript: "Transcript",
  search: "Search conversation",
  closeSearch: "Close search",
  /** Day separators in the transcript (UI-750). */
  today: "Today",
  yesterday: "Yesterday",
  roleUser: "You",
  roleAssistant: "Assistant",
  steeringMark: "steering",
  thinkingLive: "Thinking…",
  thought: "Thought",
  redactedThinking: "(redacted by the provider)",
  attachedImage: "Attached image",
  /** A message's image is a button opening it at full size (UI-476). */
  openAttachedImage: "Open the attached image",
  details: "Details",
  missingMessage: "missing",
  /** The conversation as Markdown, for `:copy` and `:export` (UI-792). */
  markdown: {
    tool: "Tool",
    error: "Error",
    image: (mime: string) => `[image: ${mime}]`,
    /** The exported file's name when the title has no letter or digit. */
    fileName: "conversation",
  },
  /** Label of omp's own bookkeeping records (device inventories, mount notices). */
  system: "system",
  records: {
    toolResult: "tool result",
    bashExecution: "bash",
    pythonExecution: "python",
    hookMessage: "hook",
    branchSummary: "branch summary",
    compactionSummary: "compaction summary",
    fileMention: "file mention",
  } as Readonly<Record<string, string>>,
  tool: {
    fallbackName: "tool",
    label: (name: string) => `Tool ${name}`,
    /** A run of consecutive reads shown as one card (UI-765): its label and the count in its head. */
    readGroup: (count: number) => `Read ${count} files`,
    readGroupFiles: (count: number) => `${count} files`,
    result: "Result",
    status: {
      composing: "writing arguments",
      pending: "pending",
      running: "running",
      succeeded: "done",
      failed: "failed",
    },
    image: "[image]",
    /** The alt text of a result's image (UI-472): `Image 1 of 2 from read /w/plot.png`. */
    resultImage: (tool: string, subject: string, index: number, count: number) =>
      `${count === 1 ? "Image" : `Image ${index} of ${count}`} from ${subject === "" ? tool : `${tool} ${subject}`}`,
    /** The thumbnails' list (UI-473). */
    resultImages: (tool: string) => `Images from ${tool}`,
    /** A thumbnail's button opens the full view (UI-474). */
    openImage: (alt: string) => `Open ${alt.charAt(0).toLowerCase()}${alt.slice(1)}`,
    /** `read` lines returned: `1–9 of 9`. */
    lines: (first: number, last: number, total: number) => `${first}–${last} of ${total}`,
    fetchFailed: "The URL could not be fetched.",
    exitCode: (code: number) => `exit ${code}`,
    /** The mark of a `bash` call that became a background command (UI-270, UI-747), by its state. */
    background: {
      running: "background",
      completed: "background · done",
      failed: "background · failed",
      cancelled: "background · cancelled",
    },
    backgroundCommand: (id: string) => `Background command ${id}`,
    cwd: (cwd: string) => `in ${cwd}`,
    output: "Output",
    noOutput: "No output",
    error: "Error",
    edit: "Edit",
    content: "Content",
    directory: "Directory listing",
    in: (scope: string) => `in ${scope}`,
    truncated: "Results were cut.",
    /** The collapsed `write` content's expander (UI-053). */
    moreLines: (count: number) => `… ${count} more ${count === 1 ? "line" : "lines"}`,
    fewerLines: "Show fewer lines",
    noMatch: "No match",
    todoStatus: {
      pending: "pending",
      in_progress: "in progress",
      completed: "done",
    } as Readonly<Record<string, string>>,
    noAnswer: "no answer",
    noLinks: "No results",
    tools: (count: number) => `${count} ${count === 1 ? "tool call" : "tool calls"}`,
    context: "Context",
    failed: "The call failed.",
    /** An `ast_edit` with several rewrites: the first one's pattern and how many more (UI-917). */
    morePatterns: (first: string, more: number) => `${first} (+${more} more)`,
    /** A staged `ast_edit` (UI-917). */
    preview: "Preview: not applied until the agent resolves it",
    discarded: "Discarded",
    /** An `lsp` diagnostics answer without any (UI-918). */
    noIssues: "No issues",
    /** Where omp looks for language servers (UI-918). */
    serverEnvironment:
      "omp starts language servers from the omfp daemon's environment (its PATH), not from your shell: install the server where the daemon runs.",
    commands: "Commands run",
    notApplied: "Preview: nothing was changed",
    /** A symbol rename whose call named no old name (UI-919). */
    renamedTo: "renamed to",
    /** A long list of changed files' expander (UI-919). */
    moreFiles: (count: number) => `… ${count} more ${count === 1 ? "file" : "files"}`,
    fewerFiles: "Show fewer files",
    /** The subject of an `omfp_card` call without a title nor a widget reference (UI-920). */
    card: "card",
    /** The Cancel of a waiting card, outside its widget (UI-922). */
    cancelCard: "Cancel",
    cancelCardLabel: (title: string) => `Cancel the card ${title}`,
  },
  duration: {
    ms: (ms: number) => `${ms.toFixed(0)} ms`,
    seconds: (seconds: number) => `${seconds.toFixed(1)} s`,
    minutes: (minutes: number, seconds: number) => `${minutes.toFixed(0)} min ${seconds.toFixed(0)} s`,
  },
  notice: {
    compacting: (reason: string, action: string) => `Compacting the context (${reason}, ${action})`,
    compactingManual: "Compacting the context",
    compactionFailed: (error: string) => `Compaction failed: ${error}`,
    compactionAborted: "Compaction aborted",
    compactionSkipped: "Compaction skipped",
    compactionSkippedBecause: (reason: string) => `Compaction skipped: ${reason}`,
    compacted: "Context compacted",
    retrying: (attempt: number, max: number, seconds: number, error: string) =>
      `Retrying (${attempt}/${max}) in ${seconds.toFixed(1)} s: ${error}`,
    retrySucceeded: (attempt: number) => `Retry succeeded (attempt ${attempt})`,
    retryFailed: (attempt: number, error: string) => `Retry failed after ${attempt} attempts: ${error}`,
    unknownError: "unknown error",
    sourced: (source: string, message: string) => `${source}: ${message}`,
    extensionError: (path: string, event: string, error: string) => `Extension ${path} failed on ${event}: ${error}`,
    frameErrorOf: (kind: string, error: string) => `omp rejected a ${kind} frame: ${error}`,
    frameError: (error: string) => `omp rejected a frame: ${error}`,
    unrecognizedOf: (kind: string) => `Unrecognized ${kind} frame`,
    unrecognized: "Unrecognized frame",
    /** An `open_url` request without instructions (UI-345). */
    openUrl: "omp asks to open this page in a browser",
    /** The command-output card (UI-345) and its opened URLs (UI-347). */
    commandOutput: "Command output",
    openedUrl: "Opened in the browser",
    openUrlAction: "Open",
    copyUrlAction: "Copy",
    openUrlLabel: (url: string) => `Open ${url} in this browser`,
    copyUrlLabel: (url: string) => `Copy ${url}`,
    copied: "Copied the URL",
    loopbackNote:
      "OAuth sign-ins send the browser back to a loopback address (localhost) on the daemon's host: if the daemon " +
      "runs on another machine, finish the last step in a browser on that machine.",
    /** UI-347 when a URL's sign-in returns to omp's loopback listener: its address can be pasted (UI-761). */
    relayNote:
      "OAuth sign-ins send the browser back to a loopback address (localhost) on the daemon's host: if the daemon " +
      "runs on another machine, that last page fails to load. Paste its address below and omfp hands it to omp.",
    /** The field of a sign-in's callback address (UI-761). */
    relayLabel: "Paste the address your browser ended on",
    relayPlaceholder: (callback: string) => `${callback}?code=…`,
    relayAction: "Send",
    relayed: "omp got the sign-in",
  },
  composer: {
    label: "Message",
    placeholderIdle: "Message (/ for commands)",
    placeholderBusy: "Steer the agent (ctrl+enter queues, ctrl+c interrupts)",
    /** A touch screen has none of those keys (UI-423). */
    placeholderBusyTouch: "Steer the agent, or queue a follow-up",
    send: { prompt: "Send", "follow-up": "Queue follow-up", steer: "Steer now" },
    /** The send buttons' short labels on a touch screen (UI-423) and in a tight composer (UI-481), all on one row. */
    sendShort: { prompt: "Send", "follow-up": "Queue", steer: "Steer" },
    abort: "Abort",
    activity: {
      thinking: "Thinking…",
      working: "Working…",
      tool: (name: string) => `Running ${name}`,
      running: "Agent working",
    },
    /** Composer bar hints after their key chips (UI-148, UI-172). */
    hint: { newline: "newline", leave: "normal", interrupt: "interrupt", restore: "restore Ctrl-C draft" },
    attachments: "Attached images",
    attachment: (index: number) => `Attached image ${index}`,
    removeAttachment: (index: number) => `Remove image ${index}`,
    /** Large pastes kept as chips (UI-357..UI-360). */
    pastes: "Pasted texts",
    /** A chip's label: `Pasted text #1 · 120 lines`, or the characters for a text of one line. */
    paste: (id: number, lines: number, chars: number) =>
      `Pasted text #${id} · ${lines > 1 ? `${lines} lines` : `${chars} chars`}`,
    pasteText: (id: number) => `Text of pasted text #${id}`,
    removePaste: (id: number) => `Remove pasted text #${id}`,
    pasted: (id: number, lines: number, rawKey: string) =>
      `Pasted ${lines} ${lines === 1 ? "line" : "lines"} as pasted text #${id}; ${rawKey} pastes it raw`,
    pasteRemoved: (id: number) => `Removed pasted text #${id}`,
    commands: "Commands",
    /** The menu of `@` file completions (UI-732). */
    files: "Files",
    focus: "Focus the composer",
    leave: "Leave the composer",
    abortCommand: "Interrupt the agent",
    interruptCommand: "Clear the draft, or abort when it is empty (Ctrl-C)",
    restoreCommand: "Restore this thread's last Ctrl-C-cleared draft, including images and pastes (normal-mode u)",
    /** The readline keys' command titles (UI-760). */
    lineKeys: {
      "composer.lineStart": "Move to the start of the line",
      "composer.lineEnd": "Move to the end of the line",
      "composer.charBack": "Move back one character",
      "composer.charForward": "Move forward one character",
      "composer.wordBack": "Move back one word",
      "composer.wordForward": "Move forward one word",
      "composer.linePrev": "Move to the previous line, or the previous prompt on the first line",
      "composer.lineNext": "Move to the next line, or the next prompt on the last line",
      "composer.deleteForward": "Delete the character after the caret",
      "composer.deleteBack": "Delete the character before the caret",
      "composer.killLineEnd": "Kill to the end of the line",
      "composer.killLineStart": "Kill to the start of the line",
      "composer.killWordBack": "Kill the previous word",
      "composer.killWordForward": "Kill the next word",
      "composer.yank": "Yank the last killed text",
      "composer.transpose": "Swap the characters around the caret",
      "composer.selectAll": "Select the whole draft",
    },
    /** After the `ctrl+c` key chip. */
    interruptHint: "again to abort",
    /** `/` commands over omp's RPC (UI-975..UI-980). */
    slash: {
      /** A routed command's description in the suggestions, by omp's name (UI-976, UI-978). */
      routed: {
        login: "Log in to a provider (:login)",
        logout: "Log out of a provider (:logout)",
        model: "Pick the thread's model in the header (:model)",
        compact:
          "Compact the conversation (:compact), focused on what follows; first soft, remote or snapcompact for that method",
        fork: "Fork this conversation into a new thread",
        copy: "Copy the conversation as Markdown (:copy)",
        export: "Export the conversation as a Markdown file (:export); with a path, omp writes HTML on its host",
        new: "New thread in the current workspace (:new)",
        hotkeys: "Show every key and command (:help)",
        resume: "Pick another thread (:threads)",
      } as Readonly<Record<string, string>>,
      /** The argument hints of the routed commands omp does not announce. */
      routedHints: { login: "[<provider>]", logout: "[<provider>]" } as Readonly<Record<string, string>>,
      tuiOnly: "Needs omp's terminal UI",
      /** The omfp way, after `tuiOnly`, by omp's name. */
      hints: {
        setup: ":login logs in to a provider",
        branch: "omfp:tree navigates the session tree",
        tree: "omfp:tree navigates the session tree",
        delete: ":delete deletes the thread",
        exit: ":stop stops the thread's process",
        quit: ":stop stops the thread's process",
        restart: ":stop then :restart restarts the thread's process",
        live: "the composer's microphone starts live voice",
      } as Readonly<Record<string, string>>,
      refused: (name: string, reason: string) => `/${name} was not sent: ${reason}`,
      noArguments: (name: string) => `/${name} was not sent: omfp runs it without arguments`,
      /** A routed command whose window command nothing registered or enabled (no header in the view). */
      unavailable: (name: string) => `/${name}: nothing in this window runs it now`,
      /** omp answered a `/` prompt with an error (UI-979). */
      failed: (name: string, error: string) => `/${name} failed: ${error}`,
    },
  },
  /** Live voice (UI-180..UI-189). */
  live: {
    start: "Start live voice",
    end: "End live voice",
    panel: "Live voice",
    level: "Voice level",
    status: {
      idle: "off",
      connecting: "connecting",
      listening: "listening",
      working: "working",
      speaking: "speaking",
      muted: "muted",
      error: "error",
    },
    /** Status line hints after their key chips. */
    hint: { mute: "mute", unmute: "unmute", end: "end", dismiss: "dismiss" },
    elsewhere: (client: string) => `in another window (${client})`,
    /** A prompt was typed in the chat since the session started: the voice does not know it (UI-199). */
    behind: "chat changed since start",
    endButton: "End",
    /** The own panel's mute toggle (UI-182, UI-183): one name, its pressed state says whether it is muted. */
    mute: "Mute",
    dismiss: "Dismiss",
    /** Why the mic button cannot start a session (UI-180). */
    insecure:
      "live voice needs a secure page: the browser gives the microphone only to localhost, 127.0.0.1 or HTTPS; open omfp on one of those",
    unsupported:
      "live voice needs WebRTC and a microphone, which this window does not offer; live voice in the desktop app needs the daemon on this machine",
    microphone: (reason: string) => `live voice could not open the microphone: ${reason}`,
    peer: (reason: string) => `live voice could not set up the call: ${reason}`,
    stopFailed: (reason: string) => `live voice could not end the session: ${reason}`,
  },
  queue: {
    label: "Queued messages",
    kind: { steering: "steer", "follow-up": "follow-up" },
    remove: "Remove",
    promote: "Steer now",
    gone: (message: string) => `\u201c${message}\u201d is no longer queued`,
  },
  nav: {
    next: "Next message (a long one scrolls down to its end first)",
    prev: "Previous message (a long one scrolls up to its top first)",
    top: "First message",
    bottom: "Last message",
    halfDown: "Scroll half a page down",
    halfUp: "Scroll half a page up",
  },
  interactions: {
    label: "Requests",
    dismiss: "Dismiss",
    yes: "Yes",
    no: "No",
    submit: "Submit",
    sendAnswers: "Send answers",
    otherAnswer: "Other answer",
    otherAnswerTo: (question: string) => `Other answer to ${question}`,
    ruleMulti: " (any number)",
    ruleSingle: " (one)",
    recommended: "recommended",
    answerRequired: (question: string) => `Answer \u201c${question}\u201d`,
    answeredElsewhere: (client: string) => `Answered elsewhere by ${client}`,
    closedByOmp: "Closed by omp (timed out or cancelled)",
    timeLeft: "Time left",
    dismissClosed: "Dismiss",
  },
  announce: {
    working: "Agent working",
    settled: "Agent finished",
    request: (title: string) => `Request: ${title}`,
    aborted: "Turn aborted",
    abortFailed: "Abort failed",
    draftCleared: "Draft cleared",
    queued: "Message queued",
  },
} as const;
