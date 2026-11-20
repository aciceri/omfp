/** Texts of the widget runtime: the slot every widget of a view renders in (loading, errors in place, AGT-014). */

export const widgets = {
  loading: (reference: string) => `Loading ${reference}…`,
  missing: (reference: string) => `Unknown widget ${reference}`,
  compileFailed: (reference: string) => `Widget ${reference} does not compile`,
  loadFailed: (reference: string) => `Widget ${reference} could not be loaded`,
  renderFailed: (reference: string) => `Widget ${reference} failed`,
  noEntry: "The widget folder has no index.tsx, index.ts, index.jsx or index.js.",
  noDefaultExport: "The widget's entry has no default export (a React component).",
  /** `file:line:column`, with what is known of the position. */
  location: (file: string, line: number | null, column: number | null) =>
    line === null ? file : column === null ? `${file}:${line}` : `${file}:${line}:${column}`,
  /** `useAnswer().submit`/`cancel` of a card that does not wait for an answer (UI-921). */
  notWaiting: "This card does not wait for an answer.",
  noWorkspace: "This widget has no workspace to keep its state in.",
} as const;
