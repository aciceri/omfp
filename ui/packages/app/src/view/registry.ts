/** The built-in `omfp:*` widgets a view can hold, by reference (AGT-021); the frame renders the threads column itself
 * (UI-100). */

import type { WidgetComponent } from "../sdk/index.ts";
import { ActivityWidget } from "../widgets/activity/index.ts";
import { BackgroundWidget } from "../widgets/background/index.ts";
import { BrowserWidget } from "../widgets/browser/index.ts";
import { ChangesWidget } from "../widgets/changes/index.ts";
import { ChatWidget, ComposerWidget, InteractionsWidget, TranscriptWidget } from "../widgets/chat/index.ts";
import { DebugWidget } from "../widgets/debug/index.ts";
import { FileWidget } from "../widgets/file/index.ts";
import { HeaderWidget } from "../widgets/header/index.ts";
import { JobsWidget } from "../widgets/jobs/index.ts";
import { McpWidget } from "../widgets/mcp/index.ts";
import { TerminalWidget } from "../widgets/terminal/index.tsx";
import { TodosWidget } from "../widgets/todos/index.ts";
import { TreeWidget } from "../widgets/tree/index.ts";
import { WebWidget } from "../widgets/web/index.ts";
import { WorkspaceWidget } from "../widgets/workspace/index.ts";
import type { WidgetRegistry } from "./resolve.ts";

export const BUILTIN_WIDGETS: WidgetRegistry = new Map<string, WidgetComponent>([
  ["omfp:header", HeaderWidget],
  ["omfp:workspace", WorkspaceWidget],
  ["omfp:changes", ChangesWidget],
  ["omfp:background", BackgroundWidget],
  ["omfp:todos", TodosWidget],
  ["omfp:debug", DebugWidget],
  ["omfp:tree", TreeWidget],
  ["omfp:activity", ActivityWidget],
  ["omfp:mcp", McpWidget],
  ["omfp:chat", ChatWidget],
  ["omfp:transcript", TranscriptWidget],
  ["omfp:interactions", InteractionsWidget],
  ["omfp:composer", ComposerWidget],
  ["omfp:jobs", JobsWidget],
  ["omfp:file", FileWidget],
  ["omfp:terminal", TerminalWidget],
  ["omfp:web", WebWidget],
  ["omfp:browser", BrowserWidget],
]);

/** The built-ins that render the selected thread's pending interactions; without one of them in the view (a workspace
 * override does not count), the frame's safety net does (AGT-015). */
export const INTERACTION_WIDGETS: readonly string[] = ["omfp:chat", "omfp:interactions"];
