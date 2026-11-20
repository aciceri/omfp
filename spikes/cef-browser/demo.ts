// Spike S-16 demo: `bun spikes/cef-browser/demo.ts [url]` inside omfp's dev shell. Starts an `omfp daemon` on a
// temporary data directory, workspaces root and HOME (nothing of yours is read or written), creates the workspace
// `cef-spike` with one thread and the widget of widget/index.tsx mounted as a floating panel on `url` (default
// YouTube), then opens omfp-cef on that thread. The daemon gets a placeholder `ANTHROPIC_API_KEY` so that omp has a
// model and omfp skips its welcome screen; no prompt is sent. `OMFP_BIN` names the omfp executable (default: `omfp`
// on PATH); extra arguments after `--` go to omfp-cef (e.g. `-- --remote-debugging-port=9333`).
import { mkdirSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const here = import.meta.dir;
const separator = Bun.argv.indexOf("--");
const own = separator === -1 ? Bun.argv.slice(2) : Bun.argv.slice(2, separator);
const extra = separator === -1 ? [] : Bun.argv.slice(separator + 1);
const url = own[0] ?? "https://www.youtube.com/";

const root = mkdtempSync(join(tmpdir(), "omfp-cef-spike-"));
for (const dir of ["data", "ws", "home", "config"]) mkdirSync(join(root, dir));
const probe = Bun.listen({ hostname: "127.0.0.1", port: 0, socket: { data() {} } });
const port = probe.port;
probe.stop(true);

const daemon = Bun.spawn(
  [
    process.env.OMFP_BIN ?? "omfp",
    "daemon",
    "--data-dir",
    join(root, "data"),
    "--workspaces-root",
    join(root, "ws"),
    "--config",
    join(root, "config", "config.toml"),
    "--theme-file",
    join(root, "config", "theme.css"),
    "--bind",
    `127.0.0.1:${port}`,
    "--no-title-generation",
  ],
  {
    env: {
      ...process.env,
      HOME: join(root, "home"),
      XDG_CONFIG_HOME: join(root, "config"),
      ANTHROPIC_API_KEY: "omfp-cef-spike-placeholder",
    },
    stdout: "inherit",
    stderr: "inherit",
  },
);

async function until<T>(what: string, attempt: () => T | undefined | Promise<T | undefined>): Promise<T> {
  const deadline = Date.now() + 30_000;
  for (;;) {
    const value = await attempt();
    if (value !== undefined) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(100);
  }
}

const token = await until("the daemon's token", () => {
  try {
    return readFileSync(join(root, "data", "token"), "utf8").trim() || undefined;
  } catch {
    return undefined;
  }
});
await until("the daemon's port", async () => {
  try {
    return (await fetch(`http://127.0.0.1:${port}/`)).ok ? true : undefined;
  } catch {
    return undefined;
  }
});

const socket = new WebSocket(`ws://127.0.0.1:${port}/api/ws`);
const opened = Promise.withResolvers<void>();
socket.onopen = () => opened.resolve();
socket.onerror = (event) => opened.reject(new Error(`WebSocket: ${event.type}`));
await opened.promise;
let next = 0;
const waiting = new Map<string, (frame: { result?: unknown; error?: unknown }) => void>();
socket.onmessage = (event) => {
  const frame = JSON.parse(String(event.data));
  waiting.get(frame.id)?.(frame);
};
async function call<T>(method: string, params: unknown): Promise<T> {
  next += 1;
  const id = String(next);
  const answer = Promise.withResolvers<{ result?: unknown; error?: unknown }>();
  waiting.set(id, answer.resolve);
  socket.send(JSON.stringify({ type: "request", id, method, params }));
  const frame = await answer.promise;
  if (frame.error !== undefined) throw new Error(`${method}: ${JSON.stringify(frame.error)}`);
  return frame.result as T;
}

await call("system.hello", { token, protocol: 1, client: "omfp/cef-spike" });
const workspace = await call<{ id: string }>("workspaces.create", { name: "cef-spike" });
const thread = await call<{ id: string }>("threads.create", { workspace: { id: workspace.id }, title: "CEF spike" });
await call("views.mount", {
  workspaceId: workspace.id,
  id: "page",
  code: readFileSync(join(here, "widget", "index.tsx"), "utf8"),
  panel: true,
  props: { url, title: "Page" },
});
socket.close();

const window = `http://127.0.0.1:${port}/#token=${token}&local=1&thread=${thread.id}`;
console.log(`omfp-cef-spike: ${window}`);
const host = Bun.spawn([join(here, "run.sh"), `--url=${window}`, `--profile=${join(root, "profile")}`, ...extra], {
  stdout: "inherit",
  stderr: "inherit",
});
// Stopped from outside (check.ts, Ctrl-C): take omfp-cef and the daemon down too.
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    host.kill();
    daemon.kill();
  });
}
const code = await host.exited;
daemon.kill();
await daemon.exited;
process.exit(code);
