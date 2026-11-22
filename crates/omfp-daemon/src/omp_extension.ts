// omfp's omp extension (DMN-113), written by the daemon as `<data-dir>/omp/index.ts`, the module of the extension
// root every omp gets with `--extension`. `/omfp-voice-log <json>` appends the spoken conversation of a live voice
// session, `{ content, turns }` built by the daemon, to the session as one `custom` message, without a turn.

interface VoiceLog {
  content: string;
  turns: unknown;
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
}
