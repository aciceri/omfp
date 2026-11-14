// Drives real file tabs in the desktop smoke window. State is per origin and survives each window restart.
export default function init(omfp) {
  const files = [
    ["video.mp4", "video"], ["video.webm", "video"], ["video.ogv", "video"], ["unsupported.mkv", "video"],
    ["audio.wav", "audio"], ["audio.mp3", "audio"], ["audio.ogg", "audio"],
    ["audio.opus", "audio"], ["audio.flac", "audio"], ["audio.m4a", "audio"],
    ["guide.pdf", "pdf"], ["image.png", "image"], ["image.jpg", "image"],
    ["image.gif", "image"], ["image.webp", "image"], ["large.bmp", "large"],
    ["guide.md", "markdown"], ["guide.mdx", "markdown"], ["data.csv", "table"],
    ["data.tsv", "table"], ["data.json", "tree"], ["data.jsonc", "tree"], ["data.yaml", "tree"],
  ];
  const state = JSON.parse(localStorage.getItem("test.matrix") || '{"index":0,"phase":"open"}');
  const q = (selector) => document.querySelector(selector);
  let action = 0;
  let original = "";
  let clickedClose = false;
  let completed = false;
  let outcome = "";
  const tick = setInterval(() => {
    if (completed) return;
    const item = files[state.index];
    if (!item) {
      if (!q('[data-testid=file-tab]') && !localStorage.getItem("omfp.openFiles")?.includes("viewers/")) {
        document.body.dataset.matrix = "done";
      }
      return;
    }
    const [name, kind] = item;
    const path = `viewers/${name}`;
    const tab = q(`[data-testid=file-tab][data-file="${path}"]`);
    const active = tab?.getAttribute("aria-selected") === "true";
    const file = q(`[data-testid=file][data-path="${path}"]`);
    document.body.dataset.smoke = JSON.stringify({name, phase: state.phase, action, active, status: file?.dataset.status});
    if ((state.phase === "open" || state.phase === "reopen") && !tab && !clickedClose) {
      omfp.run("file.open", [path]);
      return;
    }
    if (clickedClose) {
      if (!tab && !localStorage.getItem("omfp.openFiles")?.includes(path)) {
        const next = state.phase === "restore" ? {index: state.index, phase: "reopen"} : {index: state.index + 1, phase: "open"};
        localStorage.setItem("test.matrix", JSON.stringify(next));
        document.body.dataset.smoke = JSON.stringify({name, phase: state.phase, outcome: outcome || "rendered"});
        completed = true;
        document.body.dataset.matrix = "ready";
      }
      return;
    }
    if (kind === "markdown" && action === 1 && q('[data-testid=file-tab][data-file="viewers/next.md"][aria-selected="true"]')) {
      q('[data-testid=file-tab][data-file="viewers/next.md"] [data-testid=file-tab-close]')?.click();
      tab?.click();
      action = 2;
      return;
    }
    if (!active || !file) return;
    let ready = false;
    if (kind === "audio" || kind === "video") {
      const player = file.querySelector('[data-testid=file-player]');
      const error = file.querySelector('[data-testid=file-media-error]');
      if (player && player.paused) { player.muted = true; player.play().catch(() => {}); }
      ready = !!((player && player.currentTime > 0 && (kind !== "video" || player.videoWidth > 0)) ||
        error?.textContent?.includes(`This window cannot play this ${kind} file`));
      if (ready) outcome = error ? "fallback" : "played";
    } else if (kind === "pdf") {
      const page = file.querySelector('[data-testid=pdf-page][data-page="1"]');
      const label = file.querySelector('[data-testid=pdf-zoom]');
      if (action === 0 && page?.dataset.rendered === "true" && page.querySelector("canvas")?.width > 0 &&
          page.querySelector(".textLayer")?.textContent?.includes("Hello PDF one") && label) {
        const range = document.createRange();
        range.selectNodeContents(page.querySelector(".textLayer"));
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        if (!selection.toString().includes("Hello PDF one")) return;
        selection.removeAllRanges();
        original = label.textContent;
        file.querySelector('[data-testid=pdf-zoom-in]')?.click();
        action = 1;
      } else if (action === 1 && label?.textContent !== original) {
        const pages = file.querySelector('[data-testid=pdf-pages]');
        pages.scrollTop = pages.scrollHeight;
        action = 2;
      } else if (action === 2 && file.querySelector('[data-testid=pdf-page-number]')?.textContent?.includes("2 / 2")) ready = true;
    } else if (kind === "image" || kind === "large") {
      const image = file.querySelector('[data-testid=image-after]');
      if (image?.complete && image.naturalWidth > 0) {
        if (kind === "image") ready = true;
        else if (action === 0 && image.naturalWidth === 2000 && image.getAttribute("src")?.startsWith("/files/")) {
          file.querySelector('[data-testid=image-actual]')?.click();
          action = 1;
        } else if (action === 1 && file.querySelector('[data-testid=image-zoom]')?.textContent === "100%") {
          const viewport = file.querySelector('[data-testid=image-viewport]');
          const box = viewport.getBoundingClientRect();
          const at = (type, x) => viewport.dispatchEvent(new PointerEvent(type, {bubbles: true, pointerId: 7, button: 0, clientX: box.left + x, clientY: box.top + 100}));
          at("pointerdown", 300); at("pointermove", 100); at("pointerup", 100);
          if (viewport.scrollLeft > 0) { file.querySelector('[data-testid=file-diff]')?.click(); action = 2; }
        } else if (action === 2) {
          const before = file.querySelector('[data-testid=image-before]');
          if (before?.complete && before.naturalWidth === 2000 && before.getAttribute("src")?.includes("rev=HEAD")) {
            const canvas = document.createElement("canvas");
            canvas.width = canvas.height = 1;
            const context = canvas.getContext("2d");
            context.drawImage(before, 0, 0, 1, 1);
            const head = [...context.getImageData(0, 0, 1, 1).data];
            context.drawImage(image, 0, 0, 1, 1);
            const current = [...context.getImageData(0, 0, 1, 1).data];
            ready = head[2] > head[0] && current[0] > current[2];
          }
        }
      }
    } else if (kind === "markdown") {
      const preview = file.querySelector('[data-testid=file-viewer][data-viewer=markdown]');
      const image = preview?.querySelector('img[alt="sample"]');
      if (action === 0 && image?.complete && image.naturalWidth > 0 && preview?.querySelector('h1')?.textContent === "Guide") {
        preview.querySelector('[data-testid=path-link]')?.click();
        action = 1;
      } else if (action === 2 && active) ready = true;
    } else if (kind === "table") {
      ready = !!file.querySelector('[data-testid=file-table] tbody tr[data-index]')?.textContent?.includes("alpha");
    } else if (kind === "tree") {
      ready = !!file.querySelector('[data-testid=file-tree-node][data-path=".name"]')?.textContent?.includes("alpha");
    }
    if (!ready) return;
    if (state.phase === "open") {
      if (!localStorage.getItem("omfp.openFiles")?.includes(path)) return;
      localStorage.setItem("test.matrix", JSON.stringify({index: state.index, phase: "restore"}));
      document.body.dataset.smoke = JSON.stringify({name, phase: state.phase, outcome: outcome || "rendered"});
      completed = true;
      document.body.dataset.matrix = "ready";
    } else {
      tab.querySelector('[data-testid=file-tab-close]')?.click();
      clickedClose = true;
    }
  }, 100);
  return () => clearInterval(tick);
}
