/**
 * DOM for component tests (`bun run test` preloads it for `packages/kit`, `packages/app` and `packages/site` only):
 * happy-dom replaces `WebSocket`, `fetch` and timers, which the client's transport suite needs from Bun itself.
 */
import { GlobalRegistrator } from "@happy-dom/global-registrator";

GlobalRegistrator.register({ url: "http://127.0.0.1:7421/" });

// happy-dom: `document instanceof window.Document` is false (the window's `Document` is a per-window subclass of the
// class `document` was made from). React Aria's focus checks (`isFocusWithin`) need it to hold.
const BaseDocument: typeof Document = Object.getPrototypeOf(window.Document);
Object.defineProperty(window.Document, Symbol.hasInstance, {
  value: (value: unknown) => value instanceof BaseDocument,
});
