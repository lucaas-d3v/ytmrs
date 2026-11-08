// Port of windows/changelog/preload.ts
(function () {
  const ipc = window.__ytmdIpc;
  if (!ipc) return;
  window.ytmd = Object.freeze({
    minimizeWindow: () => ipc.send("windowControls:minimize"),
    maximizeWindow: () => ipc.send("windowControls:maximize"),
    restoreWindow: () => ipc.send("windowControls:restore"),
    closeWindow: () => ipc.send("windowControls:close"),
    handleWindowEvents: callback => ipc.on("windowControls:stateChanged", callback),
    requestWindowState: () => ipc.send("windowControls:requestState"),
    store: {
      set: (key, value) => ipc.send("configStore:set", key, value),
      get: key => ipc.invoke("configStore:get", key),
      reset: key => ipc.send("configStore:reset", key),
      onDidAnyChange: callback => ipc.on("configStore:stateChanged", callback)
    },
    getAppVersion: () => ipc.invoke("app:getVersion"),
    getReleaseMetadata: () => ipc.invoke("changelog:getReleaseMetadata")
  });
})();
