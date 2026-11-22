/**
 * The screen wake lock of a live session (UI-531): a phone's screen stays on while the window holds the microphone, so
 * the conversation does not stop when the screen would turn off. The browser releases the lock whenever the page is
 * hidden; it is asked for again each time the page shows.
 */

/** What the lock needs of the page: whether it shows, and its visibility changes. */
export interface WakePage {
  visible(): boolean;
  /** Calls `listener` on every visibility change, until the returned function is called. */
  onVisibility(listener: () => void): () => void;
}

/**
 * Keeps the screen on through `lock` while the page shows, until the returned function is called; nothing where the
 * browser has no wake lock (`undefined`) or refuses it.
 */
export function holdScreenAwake(lock: Pick<WakeLock, "request"> | undefined, page: WakePage): () => void {
  if (lock === undefined) {
    return () => {};
  }
  let held: WakeLockSentinel | undefined;
  let asking = false;
  let done = false;
  const ask = () => {
    if (done || asking || !page.visible() || (held !== undefined && !held.released)) {
      return;
    }
    asking = true;
    lock.request("screen").then(
      (sentinel) => {
        asking = false;
        if (done) {
          void sentinel.release().catch(() => undefined);
        } else {
          held = sentinel;
        }
      },
      () => {
        // Refused (no permission, low battery, hidden meanwhile): the screen follows its settings.
        asking = false;
      },
    );
  };
  const stop = page.onVisibility(ask);
  ask();
  return () => {
    if (done) {
      return;
    }
    done = true;
    stop();
    if (held !== undefined && !held.released) {
      void held.release().catch(() => undefined);
    }
    held = undefined;
  };
}

/** The browser's screen wake lock and page visibility; `undefined` lock where the API is missing. */
export function holdBrowserScreenAwake(): () => void {
  const lock = "wakeLock" in navigator ? navigator.wakeLock : undefined;
  return holdScreenAwake(lock, {
    visible: () => document.visibilityState === "visible",
    onVisibility: (listener) => {
      document.addEventListener("visibilitychange", listener);
      return () => document.removeEventListener("visibilitychange", listener);
    },
  });
}
