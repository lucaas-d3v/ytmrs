// Port of windows/titlebar/preload.ts + windows/main/preload.ts (the main window hosts both)
(function () {
  const ipc = window.__ytmdIpc;
  if (!ipc) return;
  const memoryStore = {
    set: (key, value) => ipc.send("memoryStore:set", key, value),
    get: key => ipc.invoke("memoryStore:get", key),
    onStateChanged: callback => ipc.on("memoryStore:stateChanged", callback)
  };
  window.ytmd = Object.freeze({
    minimizeWindow: () => ipc.send("windowControls:minimize"),
    maximizeWindow: () => ipc.send("windowControls:maximize"),
    restoreWindow: () => ipc.send("windowControls:restore"),
    closeWindow: () => ipc.send("windowControls:close"),
    handleWindowEvents: callback => ipc.on("windowControls:stateChanged", callback),
    requestWindowState: () => ipc.send("windowControls:requestState"),
    isMainWindow: true,
    openSettingsWindow: () => ipc.send("window:openSettings"),
    openMiniplayerWindow: () => ipc.send("window:openMiniplayer"),
    switchFocus: context => ipc.send("ytmView:switchFocus", context),
    ytmViewNavigateDefault: () => ipc.send("ytmView:navigateDefault"),
    ytmViewRecreate: () => ipc.send("ytmView:recreate"),
    restartApplicationForUpdate: () => ipc.send("app:restartApplicationForUpdate"),
    memoryStore,
    // windows/main/preload.ts (the loading overlay now lives in this page)
    ytmViewStatusChanged: callback => ipc.on("ytmView:statusChanged", callback),
    ytmViewLoadError: callback => ipc.on("ytmView:loadError", callback),
    ytmViewReload: () => ipc.send("ytmView:reload"),
    appViewHiding: callback => ipc.on("appView:hide", callback),
    appViewShowing: callback => ipc.on("appView:show", callback),
    appViewHide: () => ipc.send("appView:hide")
  });
})();
