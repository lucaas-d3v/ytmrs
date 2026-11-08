// Port of windows/authorize-companion/preload.ts
(function () {
  const ipc = window.__ytmdIpc;
  if (!ipc) return;
  const data = () => window.__ytmdAuthorization || {};
  window.ytmd = Object.freeze({
    sendResult: result => ipc.send("companionAuthorization:result", result),
    getAppName: async () => data().appName,
    getCode: async () => data().code,
    minimizeWindow: () => ipc.send("windowControls:minimize"),
    maximizeWindow: () => ipc.send("windowControls:maximize"),
    restoreWindow: () => ipc.send("windowControls:restore"),
    closeWindow: () => ipc.send("windowControls:close"),
    handleWindowEvents: callback => ipc.on("windowControls:stateChanged", callback)
  });
})();
