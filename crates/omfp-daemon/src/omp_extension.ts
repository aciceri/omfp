// omfp's omp extension (DMN-107), written by the daemon as `<data-dir>/omp/index.ts`, the module of the extension
// root every omp gets with `--extension`.
// - `/omfp-voice-log <json>` appends the spoken conversation of a live voice session, `{ content, turns }` built by
//   the daemon, to the session as one `custom` message, without a turn (DMN-113).
// - Each request the live voice delegates (omp's `live-delegation` message) reaches the model marked as relayed by the
//   voice, with the session's transcript when it arrived (DMN-116). The daemon keeps the running session's transcript
//   in the file named by `OMFP_LIVE_TRANSCRIPT`; the first model request after a delegation copies it into the session
//   as a `custom` entry (never sent to the model), so the request reads the same on every later model call and after a
//   restart. Only delegations made since this omp started get a copy: a live session ends with its omp, so the file
//   never holds the words of an older delegation's session, and one without a copy gets no transcript.

import { readFileSync } from "node:fs";

interface VoiceLog {
  content: string;
  turns: unknown;
}

type Content = string | ({ type: "text"; text: string } | { type: "image" })[];

interface Message {
  role: string;
  customType?: string;
  content?: Content;
  timestamp?: number;
}

interface Entry {
  type: string;
  customType?: string;
  data?: unknown;
}

interface Context {
  sessionManager: { getSessionId(): string; getEntries(): Entry[] };
}

interface Api {
  registerCommand(
    name: string,
    options: { description: string; handler: (args: string) => Promise<void> },
  ): void;
  sendMessage(
    message: { customType: string; content: string; display: boolean; details: unknown },
    options: { triggerTurn: boolean },
  ): void;
  appendEntry(customType: string, data: unknown): void;
  on(
    event: "context",
    handler: (event: { messages: Message[] }, ctx: Context) => { messages: Message[] } | undefined,
  ): void;
}

/** omp's `customType` of a request the live voice delegated. */
const DELEGATION = "live-delegation";

/** The session entry holding the transcript a delegation was sent with. */
const SNAPSHOT = "omfp-voice-transcript";

const NOTE =
  "Relayed by the realtime voice model of a live voice session, not typed by the user: the request is that " +
  "model's wording of what the user said aloud, and it reads your final answer to the user. The voice " +
  "conversation is the transcript of this session so far (speech recognition, so expect misheard words): " +
  "the user's own words, which win when the request and the conversation disagree.";

interface Snapshot {
  key: string;
  transcript: string;
}

function isSnapshot(data: unknown): data is Snapshot {
  return (
    typeof data === "object" &&
    data !== null &&
    typeof (data as Snapshot).key === "string" &&
    typeof (data as Snapshot).transcript === "string"
  );
}

/** What the daemon wrote for the running session; empty without one. */
function currentTranscript(): string {
  const path = process.env.OMFP_LIVE_TRANSCRIPT;
  if (!path) return "";
  try {
    return readFileSync(path, "utf8").trim();
  } catch {
    return "";
  }
}

function marked(request: string, transcript: string): string {
  const conversation = transcript ? `<voice-conversation>\n${transcript}\n</voice-conversation>\n` : "";
  return `<voice-request>\n${NOTE}\n${conversation}<request>\n${request}\n</request>\n</voice-request>`;
}

export default function omfp(pi: Api): void {
  pi.registerCommand("omfp-voice-log", {
    description: "omfp: keeps the spoken conversation of a live voice session",
    handler: async (args) => {
      const log = JSON.parse(args) as VoiceLog;
      pi.sendMessage(
        { customType: "omfp-voice", content: log.content, display: true, details: { turns: log.turns } },
        { triggerTurn: false },
      );
    },
  });

  // The transcripts of this session's delegations, by the delegation's timestamp.
  const started = Date.now();
  let session: string | undefined;
  const snapshots = new Map<string, string>();
  const isDelegation = (message: Message): boolean =>
    message.role === "custom" && message.customType === DELEGATION;

  pi.on("context", (event, ctx) => {
    if (!event.messages.some(isDelegation)) return undefined;
    const id = ctx.sessionManager.getSessionId();
    if (id !== session) {
      session = id;
      snapshots.clear();
      for (const entry of ctx.sessionManager.getEntries()) {
        if (entry.type === "custom" && entry.customType === SNAPSHOT && isSnapshot(entry.data)) {
          snapshots.set(entry.data.key, entry.data.transcript);
        }
      }
    }
    const messages = event.messages.map((message) => {
      if (!isDelegation(message)) return message;
      const key = String(message.timestamp);
      let transcript = snapshots.get(key);
      if (transcript === undefined) {
        // A delegation of an earlier omp came from another live session: it stays without a transcript.
        transcript = (message.timestamp ?? 0) >= started ? currentTranscript() : "";
        snapshots.set(key, transcript);
        pi.appendEntry(SNAPSHOT, { key, transcript } satisfies Snapshot);
      }
      const content = message.content ?? "";
      if (typeof content === "string") return { ...message, content: marked(content, transcript) };
      const text = content
        .flatMap((part) => (part.type === "text" ? [part.text] : []))
        .join("\n")
        .trim();
      const images = content.filter((part) => part.type !== "text");
      return { ...message, content: [{ type: "text" as const, text: marked(text, transcript) }, ...images] };
    });
    return { messages };
  });
}
