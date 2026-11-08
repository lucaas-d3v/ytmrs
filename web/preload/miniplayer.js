// Port of windows/miniplayer/preload.ts
(function () {
  const ipc = window.__ytmdIpc;
  if (!ipc) return;
  const platform = window.__ytmdPlatform || {};
  window.ytmd = Object.freeze({
    minimizeWindow: () => ipc.send("windowControls:minimize"),
    maximizeWindow: () => ipc.send("windowControls:maximize"),
    restoreWindow: () => ipc.send("windowControls:restore"),
    closeWindow: () => ipc.send("windowControls:close"),
    setAlwaysOnTopWindow: alwaysOnTop => ipc.send("windowControls:setAlwaysOnTop", alwaysOnTop),
    handleWindowEvents: callback => ipc.on("windowControls:stateChanged", callback),
    requestWindowState: () => ipc.send("windowControls:requestState"),
    playerStore: {
      getState: () => ipc.invoke("playerStateStore:getState"),
      handleStateChanged: callback => ipc.on("playerStateStore:stateChanged", callback)
    },
    executeCommandInYTMView: (command, ...args) => ipc.send("remoteControl:execute", command, ...args),
    isWayland: () => !!platform.isWayland
  });
})();
