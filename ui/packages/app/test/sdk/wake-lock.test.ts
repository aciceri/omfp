/** L3: the screen wake lock a live session holds while the window has the microphone (UI-531). */

import { describe, expect, test } from "bun:test";
import { holdScreenAwake, type WakePage } from "../../src/sdk/wake-lock.ts";

class FakeSentinel extends EventTarget {
  readonly type = "screen";
  onrelease = null;
  released = false;

  async release(): Promise<void> {
    this.released = true;
  }
}

/** The browser's wake lock: each request waits until the test grants or refuses it. */
class FakeLock {
  readonly requests: { grant(): FakeSentinel; refuse(): void }[] = [];
  readonly sentinels: FakeSentinel[] = [];

  request(): Promise<WakeLockSentinel> {
    const { promise, resolve, reject } = Promise.withResolvers<WakeLockSentinel>();
    this.requests.push({
      grant: () => {
        const sentinel = new FakeSentinel();
        this.sentinels.push(sentinel);
        resolve(sentinel as unknown as WakeLockSentinel);
        return sentinel;
      },
      refuse: () => reject(new DOMException("refused", "NotAllowedError")),
    });
    return promise;
  }
}

class FakePage implements WakePage {
  shown = true;
  readonly #listeners = new Set<() => void>();

  visible(): boolean {
    return this.shown;
  }

  onVisibility(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /** The page is hidden or shown again; a hidden page loses its lock, as browsers release it. */
  show(shown: boolean, lock: FakeLock): void {
    this.shown = shown;
    if (!shown) {
      for (const sentinel of lock.sentinels) {
        sentinel.released = true;
      }
    }
    for (const listener of this.#listeners) {
      listener();
    }
  }

  get listeners(): number {
    return this.#listeners.size;
  }
}

/** Lets the lock's promise callbacks, queued before it, run. */
const settle = () => Promise.resolve();

describe("screen wake lock", () => {
  // spec: UI-531
  test("the screen stays on until the session lets it go", async () => {
    const lock = new FakeLock();
    const page = new FakePage();
    const release = holdScreenAwake(lock, page);
    expect(lock.requests.length).toBe(1);
    const sentinel = lock.requests[0]?.grant();
    await settle();
    expect(sentinel?.released).toBe(false);

    release();
    expect(sentinel?.released).toBe(true);
    expect(page.listeners).toBe(0);
    page.show(true, lock);
    expect(lock.requests.length).toBe(1);
  });

  // spec: UI-531
  test("the lock the browser dropped on hide is asked for again when the page shows, never while hidden", async () => {
    const lock = new FakeLock();
    const page = new FakePage();
    const release = holdScreenAwake(lock, page);
    lock.requests[0]?.grant();
    await settle();

    page.show(false, lock);
    expect(lock.requests.length).toBe(1);
    page.show(true, lock);
    expect(lock.requests.length).toBe(2);
    // A second visibility event while that request waits asks nothing more.
    page.show(true, lock);
    expect(lock.requests.length).toBe(2);
    const again = lock.requests[1]?.grant();
    await settle();
    expect(again?.released).toBe(false);
    release();
    expect(again?.released).toBe(true);
  });

  // spec: UI-531
  test("a lock granted after the session ended is let go at once; a refusal is asked again on the next show", async () => {
    const lock = new FakeLock();
    const page = new FakePage();
    const release = holdScreenAwake(lock, page);
    lock.requests[0]?.refuse();
    await settle();
    page.show(true, lock);
    expect(lock.requests.length).toBe(2);

    release();
    const late = lock.requests[1]?.grant();
    await settle();
    await settle();
    expect(late?.released).toBe(true);
  });

  // spec: UI-531
  test("without the API nothing is held", () => {
    const page = new FakePage();
    const release = holdScreenAwake(undefined, page);
    expect(page.listeners).toBe(0);
    release();
  });
});
