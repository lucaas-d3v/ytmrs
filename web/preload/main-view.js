// Port of windows/main/preload.ts
(function () {
  const ipc = window.__ytmdIpc;
  if (!ipc) return;
  window.ytmd = Object.freeze({
    memoryStore: {
      set: (key, value) => ipc.send("memoryStore:set", key, value),
      get: key => ipc.invoke("memoryStore:get", key),
      onStateChanged: callback => ipc.on("memoryStore:stateChanged", callback)
    },
    ytmViewStatusChanged: callback => ipc.on("ytmView:statusChanged", callback),
    ytmViewLoadError: callback => ipc.on("ytmView:loadError", callback),
    ytmViewReload: () => ipc.send("ytmView:reload"),
    appViewHiding: callback => ipc.on("appView:hide", callback),
    appViewShowing: callback => ipc.on("appView:show", callback),
    appViewHide: () => ipc.send("appView:hide")
  });
})();
