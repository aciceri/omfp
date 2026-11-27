/**
 * The web app's entry: the access token (UI-002), one client over the page's own origin, the host with per-frame
 * snapshots (UI-012), and the frame; or, at `/kit`, the component gallery (UI-173). Styles: the kit's tokens, its built-in themes
 * (UI-510) and components, then the app's and the widgets' own (all token-driven, UI-091), each followed by its boring
 * skin (UI-794); the daemon's user theme comes last (`/theme.css`, see `theme/theme.ts`).
 */

import "@omfp/kit/tokens.css";
import "@omfp/kit/themes.css";
import "@omfp/kit/kit.css";
import "@omfp/kit/boring.css";
import "./styles.css";
import "./widgets/chat/chat.css";
import "./boring.css";

import { type Client, connect, webSocketConnector } from "@omfp/client";
import { INDEX_TOPIC } from "@omfp/protocol";
import { type ReactNode, useEffect, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { TokenScreen } from "./frame/dialogs.tsx";
import { Frame } from "./frame/frame.tsx";
import { browserNotifications } from "./frame/notifications.ts";
import { Gallery } from "./gallery/gallery.tsx";
import { isGalleryPath } from "./gallery/params.ts";
import { createHost, type OmfpHost, OmfpProvider } from "./sdk/index.ts";
import { applyLaunch } from "./sdk/launch.ts";
import { takeLaunch } from "./vm/launch.ts";
import { fragmentLocal, TOKEN_KEY, takeToken } from "./vm/token.ts";

/** The client name recorded as provenance (PRT-033). */
const CLIENT_NAME = "omfp/web";

/** `localStorage`, or none where the page may not use it (private mode). */
function pageStorage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

/** The daemon's WebSocket on the origin that served the page. */
function socketUrl(): string {
  const scheme = window.location.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${window.location.host}/api/ws`;
}

function Root(): ReactNode {
  const [token, setToken] = useState(() =>
    takeToken({ location: window.location, history: window.history, storage: pageStorage() }),
  );
  // Bumped on every submitted token, so the same token can be tried again.
  const [attempt, setAttempt] = useState(0);
  const submit = (value: string) => {
    pageStorage()?.setItem(TOKEN_KEY, value);
    setToken(value);
    setAttempt((count) => count + 1);
  };
  if (token === undefined) {
    return <TokenScreen rejected={false} onSubmit={submit} />;
  }
  return <Connection key={attempt} token={token} onToken={submit} />;
}

/** One connection with `token` while mounted; the token screen again when the daemon refuses it. */
function Connection({ token, onToken }: { token: string; onToken: (token: string) => void }): ReactNode {
  const [{ host, client }] = useState(() => openHost(token));
  const [notifications] = useState(browserNotifications);
  useEffect(() => {
    host.frames.start();
    const release = host.topics.retain(INDEX_TOPIC);
    return () => {
      release();
      host.frames.stop();
      client.close();
    };
  }, [host, client]);
  const rejected = useSyncExternalStore(
    host.frames.subscribe,
    () => host.frames.getSnapshot().connection.kind === "unauthenticated",
  );
  if (rejected) {
    return <TokenScreen rejected onSubmit={onToken} />;
  }
  return (
    <OmfpProvider host={host}>
      <Frame {...(notifications === undefined ? {} : { notifications })} />
    </OmfpProvider>
  );
}

function openHost(token: string): { host: OmfpHost; client: Client } {
  const client = connect({ connector: webSocketConnector(socketUrl()), token, client: CLIENT_NAME });
  const storage = pageStorage();
  // `#local=1`: this window runs on the daemon's machine (`omfp desktop`, UI-188).
  const local = fragmentLocal(window.location.hash);
  const host = createHost(client, storage === undefined ? { local } : { storage, local });
  // `#thread=`, `#workspace=`, `#view=`, `#prompt=`: what the window opens on (`omfp desktop`, UI-214, UI-218).
  const launch = takeLaunch({ location: window.location, history: window.history });
  if (launch !== undefined) {
    applyLaunch(host, launch);
  }
  return { host, client };
}

const container = document.getElementById("root");
if (container === null) {
  throw new Error("index.html has no #root");
}
// `/kit`: the component gallery (UI-173), without a connection.
createRoot(container).render(
  isGalleryPath(window.location.pathname) ? <Gallery search={window.location.search} /> : <Root />,
);
