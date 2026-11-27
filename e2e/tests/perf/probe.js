// The performance probe of `e2e/tests/perf.rs` (TST-052), installed before the page's scripts run
// (`Page.addScriptToEvaluateOnNewDocument`). It stands in for React DevTools to see every commit, and records frame
// times, long tasks, the time spent handling WebSocket messages and the minimap's redraws while
// `window.__omfpPerf.recording` is on.
//
// Renders are counted the way React DevTools does: a commit's tree is walked from the root, entering only subtrees
// React worked on (a fiber whose child list is not the previous one's, or a new fiber); a function or class
// component rendered when it is new or carries the `PerformedWork` flag. Each render is attributed to the transcript
// item around it (`[data-testid=transcript-item]`, by its `data-key`), or to `outside`.
(() => {
  /** Function, class, forwardRef, memo and simple memo components. */
  const COMPONENTS = new Set([0, 1, 11, 14, 15]);
  const PERFORMED_WORK = 1;
  const HOST_COMPONENT = 5;

  const perf = {
    recording: false,
    /** Walk each commit's tree (costly: off while timing). */
    trackRenders: false,
    commits: 0,
    /** Component renders by transcript item key (`outside` for the rest), summed over the recorded commits. */
    renders: {},
    /** Component renders outside the transcript items, by component name (minified in a release bundle). */
    outside: {},
    /** Per recorded commit: how many components rendered. */
    commitSizes: [],
    /** `requestAnimationFrame` timestamps while recording. */
    frames: [],
    /** Long tasks (`{start, duration}`) while recording. */
    longTasks: [],
    /** WebSocket message handling: count, total and longest ms, and the text received (UTF-16 code units). */
    messages: { count: 0, total: 0, max: 0, bytes: 0 },
    /** Per `keydown`: ms from the event's timestamp to the task after the next frame (input to paint). */
    keys: [],
    /** Event timing entries of keyboard and input events (`{name, duration, delay, processing}`), Chromium only. */
    events: [],
    /** Durations of the transcript minimap's canvas redraws (ms), from its `omfp:minimap-draw` User Timing measures. */
    minimap: [],
    start(trackRenders) {
      this.trackRenders = trackRenders;
      this.commits = 0;
      this.renders = {};
      this.outside = {};
      this.commitSizes = [];
      this.frames = [];
      this.longTasks = [];
      this.messages = { count: 0, total: 0, max: 0, bytes: 0 };
      this.keys = [];
      this.events = [];
      this.minimap = [];
      this.recording = true;
      const tick = (time) => {
        if (this.recording) {
          this.frames.push(time);
          requestAnimationFrame(tick);
        }
      };
      requestAnimationFrame(tick);
    },
    stop() {
      this.recording = false;
      return {
        commits: this.commits,
        renders: this.renders,
        outside: this.outside,
        commitSizes: this.commitSizes,
        frames: this.frames,
        longTasks: this.longTasks,
        messages: this.messages,
        keys: this.keys,
        events: this.events,
        minimap: this.minimap,
      };
    },
  };
  window.__omfpPerf = perf;

  function itemKey(fiber) {
    for (let node = fiber.return; node !== null; node = node.return) {
      if (node.tag === HOST_COMPONENT && node.stateNode?.dataset?.testid === "transcript-item") {
        return node.stateNode.dataset.key;
      }
    }
    return "outside";
  }

  function componentName(type) {
    if (typeof type === "function") {
      return type.displayName || type.name || "anonymous";
    }
    if (type !== null && typeof type === "object") {
      return componentName(type.render ?? type.type);
    }
    return String(type);
  }

  function countRenders(root) {
    let rendered = 0;
    const stack = [root.current];
    while (stack.length > 0) {
      const fiber = stack.pop();
      const previous = fiber.alternate;
      if (COMPONENTS.has(fiber.tag) && (previous === null || (fiber.flags & PERFORMED_WORK) !== 0)) {
        rendered += 1;
        const key = itemKey(fiber);
        perf.renders[key] = (perf.renders[key] ?? 0) + 1;
        if (key === "outside") {
          const name = componentName(fiber.type);
          perf.outside[name] = (perf.outside[name] ?? 0) + 1;
        }
      }
      if (previous === null || fiber.child !== previous.child) {
        for (let child = fiber.child; child !== null; child = child.sibling) {
          stack.push(child);
        }
      }
    }
    perf.commitSizes.push(rendered);
  }

  let renderers = 0;
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    isDisabled: false,
    renderers: new Map(),
    inject(renderer) {
      renderers += 1;
      this.renderers.set(renderers, renderer);
      return renderers;
    },
    checkDCE() {},
    onScheduleFiberRoot() {},
    onCommitFiberUnmount() {},
    onPostCommitFiberRoot() {},
    setStrictMode() {},
    onCommitFiberRoot(_renderer, root) {
      if (!perf.recording) {
        return;
      }
      perf.commits += 1;
      if (perf.trackRenders) {
        countRenders(root);
      }
    },
  };

  // Unsupported entry types (WebKit has neither) are ignored.
  const observe = (type, options, record) => {
    if (PerformanceObserver.supportedEntryTypes?.includes(type)) {
      new PerformanceObserver((list) => {
        if (perf.recording) {
          list.getEntries().forEach(record);
        }
      }).observe({ type, ...options });
    }
  };
  observe("longtask", {}, (entry) => perf.longTasks.push({ start: entry.startTime, duration: entry.duration }));
  observe("measure", {}, (entry) => entry.name === "omfp:minimap-draw" && perf.minimap.push(entry.duration));
  observe("event", { durationThreshold: 16 }, (entry) => {
    if (["keydown", "keypress", "keyup", "beforeinput", "input"].includes(entry.name)) {
      perf.events.push({
        name: entry.name,
        duration: entry.duration,
        delay: entry.processingStart - entry.startTime,
        processing: entry.processingEnd - entry.processingStart,
      });
    }
  });

  // Input to paint: from a key's timestamp to the first task after the frame that follows it.
  window.addEventListener(
    "keydown",
    (event) => {
      if (perf.recording) {
        const pressed = event.timeStamp;
        requestAnimationFrame(() => setTimeout(() => perf.keys.push(performance.now() - pressed), 0));
      }
    },
    true,
  );

  // The client sets `socket.onmessage`: time each call (decoding, the reducer, marking the frame dirty).
  const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, "onmessage");
  Object.defineProperty(WebSocket.prototype, "onmessage", {
    configurable: true,
    enumerable: descriptor.enumerable,
    get() {
      return descriptor.get.call(this);
    },
    set(handler) {
      const timed =
        typeof handler === "function"
          ? function (event) {
              const started = performance.now();
              try {
                return handler.call(this, event);
              } finally {
                if (perf.recording) {
                  const spent = performance.now() - started;
                  perf.messages.count += 1;
                  perf.messages.total += spent;
                  perf.messages.max = Math.max(perf.messages.max, spent);
                  perf.messages.bytes += typeof event.data === "string" ? event.data.length : event.data.byteLength;
                }
              }
            }
          : handler;
      descriptor.set.call(this, timed);
    },
  });
})();
