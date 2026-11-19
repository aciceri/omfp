// Spike S-16 check, on a virtual display: `xvfb-run -a -s '-screen 0 1400x900x24' bun spikes/cef-browser/check.ts
// <out-dir>` inside omfp's dev shell, with ImageMagick's `import` on PATH. Runs demo.ts on YouTube with CEF's
// DevTools port open, waits for the floating panel's page to load, then shoots the whole display: live; with omfp's
// help dialog (`?`) open over the panel (the page must be frozen: its capture in the slot, the native view hidden, so
// the dialog shows on top); after Escape (live again); then playing a video. Prints the timings and what plays.
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const out = Bun.argv[2] ?? "/tmp/omfp-cef-check";
const url = "https://www.youtube.com/";
const video = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const port = 9333;
mkdirSync(out, { recursive: true });

const demo = Bun.spawn(["bun", join(import.meta.dir, "demo.ts"), url, "--", `--remote-debugging-port=${port}`], {
  stdout: "inherit",
  stderr: "inherit",
});

async function until<T>(what: string, attempt: () => Promise<T | undefined>, seconds = 60): Promise<T> {
  const deadline = Date.now() + seconds * 1000;
  for (;;) {
    const value = await attempt().catch(() => undefined);
    if (value !== undefined) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(100);
  }
}

type Target = { type: string; url: string; webSocketDebuggerUrl: string };

class Cdp {
  #socket: WebSocket;
  #next = 0;
  #waiting = new Map<number, (message: { result?: unknown; error?: unknown }) => void>();

  private constructor(socket: WebSocket) {
    this.#socket = socket;
    socket.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      this.#waiting.get(message.id)?.(message);
    };
  }

  static async open(target: Target): Promise<Cdp> {
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    const opened = Promise.withResolvers<void>();
    socket.onopen = () => opened.resolve();
    socket.onerror = () => opened.reject(new Error(`cannot open ${target.url}`));
    await opened.promise;
    return new Cdp(socket);
  }

  async send(method: string, params: unknown = {}): Promise<unknown> {
    this.#next += 1;
    const answer = Promise.withResolvers<{ result?: unknown; error?: unknown }>();
    this.#waiting.set(this.#next, answer.resolve);
    this.#socket.send(JSON.stringify({ id: this.#next, method, params }));
    const message = await answer.promise;
    if (message.error !== undefined) throw new Error(`${method}: ${JSON.stringify(message.error)}`);
    return message.result;
  }

  async evaluate<T>(expression: string): Promise<T> {
    const result = (await this.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })) as {
      result: { value: T };
    };
    return result.result.value;
  }

  async key(key: string, code: string, text?: string): Promise<void> {
    await this.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, ...(text ? { text } : {}) });
    await this.send("Input.dispatchKeyEvent", { type: "keyUp", key, code });
  }

  close(): void {
    this.#socket.close();
  }
}

async function targets(): Promise<Target[]> {
  return (await fetch(`http://127.0.0.1:${port}/json/list`)).json() as Promise<Target[]>;
}

async function shoot(name: string): Promise<void> {
  const file = join(out, `${name}.png`);
  await Bun.$`import -window root ${file}`.quiet();
  console.log(`shot ${file}`);
}

try {
  const main = await until("omfp's page", async () =>
    (await targets()).find((target) => target.type === "page" && target.url.startsWith("http://127.0.0.1")),
  );
  const page = await Cdp.open(main);
  const host = new URL(url).host;
  const embedded = await until("the embedded page", async () =>
    (await targets()).find((target) => target.type === "page" && target.url.includes(host)),
  );
  const site = await Cdp.open(embedded);
  await until("the embedded page's load", async () =>
    (await site.evaluate<string>("document.readyState")) === "complete" ? true : undefined,
  );
  await Bun.sleep(3000);
  await shoot("1-live");

  const capture = "!!document.querySelector('[data-omfp-cef-browser] > img[data-omfp-cef-capture]')";
  const opened = performance.now();
  await page.key("?", "Slash", "?");
  await until("the capture in the slot", async () => ((await page.evaluate<boolean>(capture)) ? true : undefined), 10);
  console.log(`frozen ${Math.round(performance.now() - opened)} ms after '?'`);
  await Bun.sleep(500);
  await shoot("2-help-over-the-page");

  const closed = performance.now();
  await page.key("Escape", "Escape");
  await until("the capture's removal", async () => ((await page.evaluate<boolean>(capture)) ? undefined : true), 10);
  console.log(`live again ${Math.round(performance.now() - closed)} ms after Escape`);
  await Bun.sleep(1000);
  await shoot("3-live-again");

  console.log(
    "codecs",
    await site.evaluate<unknown>(`({
      h264: MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E"'),
      aac: MediaSource.isTypeSupported('audio/mp4; codecs="mp4a.40.2"'),
      vp9: MediaSource.isTypeSupported('video/webm; codecs="vp9"'),
      av1: MediaSource.isTypeSupported('video/mp4; codecs="av01.0.05M.08"'),
      opus: MediaSource.isTypeSupported('audio/webm; codecs="opus"'),
    })`),
  );
  // EU consent wall: refuse everything, then play a video muted for five seconds.
  await site.evaluate<boolean>(
    `(() => { const b = [...document.querySelectorAll('button')].find((b) => /reject all/i.test(b.innerText)); b?.click(); return !!b; })()`,
  );
  await Bun.sleep(3000);
  await site.send("Page.navigate", { url: video });
  await until("the video element", async () => ((await site.evaluate<boolean>("!!document.querySelector('video')")) ? true : undefined), 30);
  await Bun.sleep(3000);
  console.log(
    "playback",
    await site.evaluate<unknown>(`(async () => {
      const v = document.querySelector('video');
      v.muted = true;
      await v.play();
      const start = v.currentTime;
      await new Promise((done) => setTimeout(done, 5000));
      const q = v.getVideoPlaybackQuality();
      return { seconds: +(v.currentTime - start).toFixed(2), size: v.videoWidth + 'x' + v.videoHeight, frames: q.totalVideoFrames, dropped: q.droppedVideoFrames };
    })()`),
  );
  await shoot("4-video");
  site.close();
  page.close();
} finally {
  demo.kill();
  await demo.exited;
}
