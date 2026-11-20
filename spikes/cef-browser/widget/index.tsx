// Spike S-16: a web page in a widget. The CEF host (spikes/cef-browser) draws a native browser on `url` over this
// placeholder; anywhere else the placeholder stays as it is. Props: `url`, `instance` (the mount id, the browser's id).
// The placeholder leaves a strip below it: a floating panel's resize corner sits there, and the native browser would
// otherwise cover it.
export default function CefBrowser({
  url = "https://www.youtube.com/",
  instance = "browser",
}: {
  url?: string;
  instance?: string;
}) {
  return (
    <div style={{ boxSizing: "border-box", height: "100%", paddingBlockEnd: "var(--omfp-s3, 16px)" }}>
      <div data-omfp-cef-browser={instance} data-url={url} style={{ position: "relative", height: "100%" }}>
        <p style={{ margin: 16, opacity: 0.7 }}>
          {url} shows here in omfp-cef (spike S-16); this window has no browser to draw it.
        </p>
      </div>
    </div>
  );
}
