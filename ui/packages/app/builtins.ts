/** Built-in widgets shipped as folders: `<name>` ← `src/widgets/<folder>`, with `part` as the entry's default export
 * when the folder's own entry is another widget. */
export const BUILTINS: readonly { name: string; folder: string; part?: { file: string; exportName: string } }[] = [
  { name: "header", folder: "header" },
  { name: "workspace", folder: "workspace" },
  { name: "changes", folder: "changes" },
  { name: "background", folder: "background" },
  { name: "todos", folder: "todos" },
  { name: "tree", folder: "tree" },
  { name: "activity", folder: "activity" },
  { name: "web", folder: "web" },
  { name: "mcp", folder: "mcp" },
  { name: "file", folder: "file" },
  { name: "chat", folder: "chat" },
  { name: "transcript", folder: "chat", part: { file: "transcript.tsx", exportName: "TranscriptWidget" } },
  { name: "interactions", folder: "chat", part: { file: "interactions.tsx", exportName: "InteractionsWidget" } },
  { name: "composer", folder: "chat", part: { file: "composer.tsx", exportName: "ComposerWidget" } },
  { name: "terminal", folder: "terminal" },
  { name: "jobs", folder: "jobs" },
];
