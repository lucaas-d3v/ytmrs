// Port of windows/updater/preload.ts
(function () {
  const ipc = window.__ytmdIpc;
  if (!ipc) return;
  window.ytmd = Object.freeze({
    autoUpdater: {
      onChecking: callback => ipc.on("autoUpdater:checking", () => callback()),
      onNotAvailable: callback => ipc.on("autoUpdater:not-available", () => callback()),
      onAvailable: callback => ipc.on("autoUpdater:available", () => callback()),
      onDownloaded: callback => ipc.on("autoUpdater:downloaded", () => callback()),
      onError: callback => ipc.on("autoUpdater:error", () => callback())
    }
  });
})();
