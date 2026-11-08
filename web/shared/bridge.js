// IPC bridge between pages and the Rust main process (ipcRenderer equivalent).
// Injected at document start in every webview, before page scripts.
(function () {
  if (window.__ytmdIpc || !window.ipc) return;
  const listeners = new Map();
  const pending = new Map();
  let seq = 0;
  const post = message => window.ipc.postMessage(JSON.stringify(message));

  const ipc = {
    send(channel, ...args) {
      post({ k: "send", c: channel, a: args });
    },
    invoke(channel, ...args) {
      const id = ++seq;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        post({ k: "invoke", c: channel, a: args, i: id });
      });
    },
    on(channel, callback) {
      if (!listeners.has(channel)) listeners.set(channel, []);
      listeners.get(channel).push(callback);
    },
    _emit(channel, args) {
      for (const cb of (listeners.get(channel) || []).slice()) {
        try {
          cb(...args);
        } catch (err) {
          console.error("[ytmd-ipc]", channel, err);
        }
      }
    },
    _resolve(id, ok, value) {
      const p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(value);
      else p.reject(new Error(`Error invoking remote method: ${value}`));
    }
  };
  Object.defineProperty(window, "__ytmdIpc", { value: Object.freeze(ipc), writable: false, configurable: false });

  if (window.top !== window) return;

  const onReady = () => ipc.send("__ytmd:domReady", location.href);
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", onReady, { once: true });
  else onReady();

  const platform = window.__ytmdPlatform || {};

  // -webkit-app-region replacement: [data-app-region="drag" | "no-drag"]
  document.addEventListener(
    "mousedown",
    event => {
      if (event.button !== 0) return;
      const target = event.target instanceof Element ? event.target.closest("[data-app-region]") : null;
      if (!target || target.getAttribute("data-app-region") !== "drag") return;
      if (event.target.closest("button, input, select, a, [data-app-region='no-drag']") && event.target.closest("[data-app-region]") !== target) return;
      event.preventDefault();
      if (event.detail === 2) ipc.send("windowControls:toggleMaximize");
      else ipc.send("windowControls:startDrag");
    },
    true
  );

  // Resize handles for frameless windows (Linux): the webview covers the window edges
  const edges = (platform.resizeEdges || "").toLowerCase();
  if (edges) {
    const BORDER = 5;
    const direction = event => {
      let d = "";
      if (edges.includes("n") && event.clientY < BORDER) d += "n";
      else if (edges.includes("s") && event.clientY > window.innerHeight - BORDER) d += "s";
      if (edges.includes("w") && event.clientX < BORDER) d += "w";
      else if (edges.includes("e") && event.clientX > window.innerWidth - BORDER) d += "e";
      return d;
    };
    let style = null;
    let current = "";
    const setCursor = d => {
      if (d === current) return;
      current = d;
      if (!style) {
        style = document.createElement("style");
        (document.head || document.documentElement).appendChild(style);
      }
      style.textContent = d ? `*{cursor:${d}-resize !important}` : "";
    };
    document.addEventListener("mousemove", event => setCursor(event.buttons ? current : direction(event)), true);
    document.addEventListener("mouseleave", () => setCursor(""), true);
    document.addEventListener(
      "mousedown",
      event => {
        if (event.button !== 0) return;
        const d = direction(event);
        if (!d) return;
        event.preventDefault();
        event.stopPropagation();
        ipc.send("windowControls:startResize", d);
      },
      true
    );
  }
})();
