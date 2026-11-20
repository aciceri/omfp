// Spike S-16 probe: what replacing Tauri with CEF would have to redo. `xvfb-run -a -s '-screen 0 1400x900x24' bun
// spikes/cef-browser/probe.ts [--sandbox]` inside omfp's dev shell, with xdotool and xprop on PATH. Runs demo.ts on
// YouTube with CEF's DevTools port open and checks, with real X input (xdotool) where focus matters: the window's
// class and size hints, the page's APIs (notifications, WebRTC, colour scheme), keyboard focus between omfp and the
// embedded page, Ctrl+= zoom, a download, the file picker, a popup, the context menu, memory. Prints JSON lines.
import { readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const port = 9333;
const extra = Bun.argv.includes("--sandbox") ? ["--sandbox"] : [];
const demo = Bun.spawn(
  ["bun", join(import.meta.dir, "demo.ts"), "https://www.youtube.com/", "--", `--remote-debugging-port=${port}`, ...extra],
  { stdout: "inherit", stderr: "inherit" },
);

function report(name: string, value: unknown): void {
  console.log(`PROBE ${name} ${JSON.stringify(value)}`);
}

async function until<T>(what: string, attempt: () => Promise<T | undefined>, seconds = 60): Promise<T> {
  const deadline = Date.now() + seconds * 1000;
  for (;;) {
    const value = await attempt().catch(() => undefined);
    if (value !== undefined) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(100);
  }
}

type Target = { id: string; type: string; url: string; webSocketDebuggerUrl: string };

async function targets(): Promise<Target[]> {
  return (await fetch(`http://127.0.0.1:${port}/json/list`)).json() as Promise<Target[]>;
}

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

  /** `gesture`: as if the user had clicked (popups, downloads, file pickers need one). */
  async evaluate<T>(expression: string, gesture = false): Promise<T> {
    const result = (await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
      userGesture: gesture,
    })) as { result: { value: T } };
    return result.result.value;
  }
}

const sh = async (strings: TemplateStringsArray, ...values: unknown[]) =>
  (await Bun.$(strings, ...values).nothrow().quiet().text()).trim();

/** Visible top-level X windows, by id. */
async function xWindows(): Promise<string[]> {
  return (await sh`xdotool search --onlyvisible --name .`).split("\n").filter(Boolean);
}

/** The descendants' resident memory of `pid` and itself, in MiB, with the process count. */
async function memory(pid: number): Promise<{ processes: number; mib: number }> {
  const all = (await sh`ps -eo pid=,ppid=,rss=`).split("\n").map((line) => line.trim().split(/\s+/).map(Number));
  const tree = new Set([pid]);
  for (let grown = true; grown; ) {
    grown = false;
    for (const [p, parent] of all) if (!tree.has(p) && tree.has(parent)) tree.add(p), (grown = true);
  }
  const kib = all.filter(([p]) => tree.has(p)).reduce((sum, [, , rss]) => sum + rss, 0);
  return { processes: tree.size, mib: Math.round(kib / 1024) };
}

try {
  const main = await until("omfp's page", async () =>
    (await targets()).find((target) => target.type === "page" && target.url.startsWith("http://127.0.0.1")),
  );
  const page = await Cdp.open(main);
  const embedded = await until("the embedded page", async () =>
    (await targets()).find((target) => target.type === "page" && target.url.includes("youtube.com")),
  );
  const site = await Cdp.open(embedded);
  await until("YouTube's load", async () =>
    (await site.evaluate<string>("document.readyState")) === "complete" ? true : undefined,
  );
  await Bun.sleep(2000);

  // The window: what the window manager is told.
  // The top-level window has the WM class `omfp` (host.rs); a visible one of that class, else of that name.
  const window = await until(
    "the omfp window",
    async () =>
      (await sh`xdotool search --onlyvisible --class omfp`).split("\n")[0] ||
      (await sh`xdotool search --onlyvisible --name ^omfp$`).split("\n")[0] ||
      undefined,
  );
  report("window", {
    class: await sh`xprop -id ${window} WM_CLASS`,
    sizeHints: await sh`xprop -id ${window} WM_NORMAL_HINTS`,
    actions: await sh`xprop -id ${window} _NET_WM_ALLOWED_ACTIONS`,
  });

  // omfp's page: the APIs the desktop works around today.
  report(
    "main-page",
    await page.evaluate(`(async () => ({
      notification: typeof Notification === "undefined" ? "absent" : Notification.permission,
      requested: typeof Notification === "undefined" ? null : await Promise.race([Notification.requestPermission(), new Promise((r) => setTimeout(() => r("no answer"), 3000))]),
      rtcPeerConnection: typeof RTCPeerConnection,
      getUserMedia: typeof navigator.mediaDevices?.getUserMedia,
      dark: matchMedia("(prefers-color-scheme: dark)").matches,
      secure: isSecureContext,
      clipboard: typeof navigator.clipboard?.writeText,
    }))()`, true),
  );
  report(
    "notification-shown",
    await page.evaluate(`new Promise((done) => {
      try {
        const n = new Notification("omfp-cef probe", { body: "hello" });
        n.onshow = () => done("shown");
        n.onerror = () => done("error");
        setTimeout(() => done("no event"), 3000);
      } catch (e) { done("throws " + e); }
    })`, true),
  );

  // Keyboard focus: a real click in the embedded page, then `?` — omfp's help must not open (the page has the keys);
  // a click on omfp's transcript, then `?` — it opens.
  const slot = await page.evaluate<{ x: number; y: number; w: number; h: number }>(
    `(() => { const r = document.querySelector("[data-omfp-cef-browser]").getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; })()`,
  );
  const geometry = await sh`xdotool getwindowgeometry --shell ${window}`;
  const origin = Object.fromEntries(geometry.split("\n").map((line) => line.split("=")));
  const at = (x: number, y: number) => [String(Number(origin.X) + Math.round(x)), String(Number(origin.Y) + Math.round(y))];
  const helpOpen = `!!document.querySelector(".omfp-help-modal")`;
  const [px, py] = at(slot.x + slot.w / 2, slot.y + slot.h - 30);
  await sh`xdotool mousemove ${px} ${py} click 1`;
  await Bun.sleep(500);
  await sh`xdotool type --delay 50 ?`;
  await Bun.sleep(800);
  const helpFromPage = await page.evaluate<boolean>(helpOpen);
  const pageFocused = await site.evaluate<boolean>("document.hasFocus()");
  const [ox, oy] = at(500, 400);
  await sh`xdotool mousemove ${ox} ${oy} click 1`;
  await Bun.sleep(500);
  await sh`xdotool type --delay 50 ?`;
  await Bun.sleep(800);
  const helpFromOmfp = await page.evaluate<boolean>(helpOpen);
  await sh`xdotool key Escape`;
  await Bun.sleep(300);
  report("focus", { pageFocusedAfterClick: pageFocused, helpOpenedWhilePageFocused: helpFromPage, helpOpenedFromOmfp: helpFromOmfp });

  // Zoom: Ctrl+= on omfp, as in Chrome.
  const ratio = "devicePixelRatio";
  const before = await page.evaluate<number>(ratio);
  await sh`xdotool key ctrl+equal`;
  await Bun.sleep(500);
  const after = await page.evaluate<number>(ratio);
  await sh`xdotool key ctrl+0`;
  report("zoom", { before, afterCtrlEqual: after });

  // A download made by omfp's page itself (`:export`), as a blob link.
  const name = `omfp-cef-probe-${Date.now()}.txt`;
  await page.evaluate(
    `(() => { const a = document.createElement("a"); a.href = URL.createObjectURL(new Blob(["probe"])); a.download = ${JSON.stringify(name)}; document.body.append(a); a.click(); a.remove(); })()`,
    true,
  );
  await Bun.sleep(2000);
  const downloads = join(homedir(), "Downloads");
  let saved = false;
  try {
    saved = readdirSync(downloads).some((file) => file.startsWith(name.replace(".txt", "")) && statSync(join(downloads, file)).size > 0);
  } catch {}
  report("download", { savedInDownloads: saved });

  // The file picker, a popup from the embedded page, the context menu: new X windows?
  let before_ = await xWindows();
  await page.evaluate(
    `(() => { const i = document.createElement("input"); i.type = "file"; document.body.append(i); i.click(); })()`,
    true,
  );
  await Bun.sleep(2000);
  const picker = (await xWindows()).filter((w) => !before_.includes(w));
  report("file-picker", {
    newWindows: await Promise.all(picker.map(async (w) => await sh`xdotool getwindowname ${w}`)),
  });
  for (const w of picker) await sh`xdotool windowclose ${w}`;

  before_ = await xWindows();
  const targetsBefore = (await targets()).length;
  await site.evaluate(`void window.open("https://example.com/", "_blank")`, true);
  await Bun.sleep(3000);
  const popup = (await xWindows()).filter((w) => !before_.includes(w));
  report("popup", {
    newWindows: await Promise.all(popup.map(async (w) => await sh`xdotool getwindowname ${w}`)),
    newTargets: (await targets()).length - targetsBefore,
  });
  for (const w of popup) await sh`xdotool windowclose ${w}`;

  before_ = await xWindows();
  await sh`xdotool mousemove ${ox} ${oy} click 3`;
  await Bun.sleep(1000);
  report("context-menu-on-omfp", { newWindows: (await xWindows()).filter((w) => !before_.includes(w)).length });
  await sh`xdotool key Escape`;

  const host = Number(await sh`pgrep -f -o 'debug/omfp-cef-spike --url'`);
  report("memory", await memory(host));
} catch (error) {
  report("error", String(error));
} finally {
  demo.kill();
  await demo.exited;
}
