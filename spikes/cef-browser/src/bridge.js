// Spike S-16: the bridge the CEF host injects into omfp's page before its scripts run (CDP
// Page.addScriptToEvaluateOnNewDocument). omfp's code is unchanged: a widget marks a placeholder with
// `data-omfp-cef-browser="<id>"` and `data-url`, and this script reports every placeholder to the host through the
// `omfpCefHost` binding (CDP Runtime.addBinding): where it is, whether it shows, whether anything of the page covers it.
// The host draws a native browser over each placeholder and calls `__omfpCef.freeze`/`thaw` to swap it for a capture
// while something covers it.
(() => {
  if (window.__omfpCef) return;

  const SELECTOR = "[data-omfp-cef-browser]";
  // Points sampled inside a placeholder to find whether another element is on top of it: a 5 × 5 grid, inset by 2 px so
  // that a neighbour's border does not count.
  const GRID = 5;
  const INSET = 2;
  // The frame around a widget: its own controls (a floating panel's bar and resize corner) do not cover it; the rest of
  // omfp's UI (the `:` line, dialogs, other panels) does.
  const FRAME = ".omfp-floating-widget, .omfp-floating-file";

  let last = "";

  // The part of `element` that can show: its box cut by the viewport and by every ancestor that clips its overflow
  // (a panel's body, a scrolled tab), so the native view never spills over the panel around it.
  function shown(element) {
    const box = element.getBoundingClientRect();
    let left = Math.max(box.left, 0);
    let top = Math.max(box.top, 0);
    let right = Math.min(box.right, window.innerWidth);
    let bottom = Math.min(box.bottom, window.innerHeight);
    for (let parent = element.parentElement; parent !== null; parent = parent.parentElement) {
      const style = getComputedStyle(parent);
      if (style.overflowX === "visible" && style.overflowY === "visible") continue;
      const clip = parent.getBoundingClientRect();
      const clipLeft = clip.left + parent.clientLeft;
      const clipTop = clip.top + parent.clientTop;
      left = Math.max(left, clipLeft);
      top = Math.max(top, clipTop);
      right = Math.min(right, clipLeft + parent.clientWidth);
      bottom = Math.min(bottom, clipTop + parent.clientHeight);
    }
    return { left, top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
  }

  function covered(element, rect) {
    const frame = element.closest(FRAME) ?? element;
    for (let row = 0; row < GRID; row += 1) {
      for (let column = 0; column < GRID; column += 1) {
        const x = rect.left + INSET + ((rect.width - 2 * INSET) * column) / (GRID - 1);
        const y = rect.top + INSET + ((rect.height - 2 * INSET) * row) / (GRID - 1);
        const top = document.elementFromPoint(x, y);
        if (top !== null && !frame.contains(top)) return true;
      }
    }
    return false;
  }

  function report() {
    const slots = [];
    for (const element of document.querySelectorAll(SELECTOR)) {
      const rect = shown(element);
      const visible =
        rect.width > 2 * INSET &&
        rect.height > 2 * INSET &&
        element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
      slots.push({
        id: element.dataset.omfpCefBrowser,
        url: element.dataset.url ?? "about:blank",
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        w: Math.round(rect.width),
        h: Math.round(rect.height),
        visible,
        covered: visible && covered(element, rect),
      });
    }
    const json = JSON.stringify(slots);
    if (json !== last) {
      last = json;
      window.omfpCefHost(JSON.stringify({ type: "slots", slots }));
    }
    requestAnimationFrame(report);
  }

  function placeholder(id) {
    for (const element of document.querySelectorAll(SELECTOR)) {
      if (element.dataset.omfpCefBrowser === id) return element;
    }
    return null;
  }

  window.__omfpCef = {
    // Shows the capture `src` (a data: URL) where the native view was, the shown part of the placeholder `id`; the
    // host hides the native view right after.
    freeze(id, src) {
      const element = placeholder(id);
      if (element === null) return;
      let capture = element.querySelector(":scope > img[data-omfp-cef-capture]");
      if (capture === null) {
        capture = document.createElement("img");
        capture.dataset.omfpCefCapture = "";
        capture.alt = "";
        element.append(capture);
      }
      const box = element.getBoundingClientRect();
      const rect = shown(element);
      capture.style.cssText = `position:absolute;left:${rect.left - box.left}px;top:${rect.top - box.top}px;width:${rect.width}px;height:${rect.height}px;pointer-events:none;z-index:1`;
      capture.src = src;
    },
    thaw(id) {
      placeholder(id)?.querySelector(":scope > img[data-omfp-cef-capture]")?.remove();
    },
  };

  requestAnimationFrame(report);
})();
