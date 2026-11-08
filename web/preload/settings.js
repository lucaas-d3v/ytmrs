// Port of windows/settings/preload.ts
(function () {
  const ipc = window.__ytmdIpc;
  if (!ipc) return;
  const platform = window.__ytmdPlatform || {};
  window.ytmd = Object.freeze({
    minimizeWindow: () => ipc.send("windowControls:minimize"),
    maximizeWindow: () => ipc.send("windowControls:maximize"),
    restoreWindow: () => ipc.send("windowControls:restore"),
    closeWindow: () => ipc.send("windowControls:close"),
    handleWindowEvents: callback => ipc.on("windowControls:stateChanged", callback),
    requestWindowState: () => ipc.send("windowControls:requestState"),
    isDarwin: platform.os === "macos",
    isLinux: platform.os === "linux",
    isWindows: platform.os === "windows",
    memoryStore: {
      set: (key, value) => ipc.send("memoryStore:set", key, value),
      get: key => ipc.invoke("memoryStore:get", key),
      onStateChanged: callback => ipc.on("memoryStore:stateChanged", callback)
    },
    store: {
      set: (key, value) => ipc.send("configStore:set", key, value),
      get: key => ipc.invoke("configStore:get", key),
      reset: key => ipc.send("configStore:reset", key),
      onDidAnyChange: callback => ipc.on("configStore:stateChanged", callback)
    },
    safeStorage: {
      decryptString: value => ipc.invoke("safeStorage:decryptString", value),
      encryptString: value => ipc.invoke("safeStorage:encryptString", value)
    },
    restartApplication: () => ipc.send("app:relaunch"),
    restartApplicationForUpdate: () => ipc.send("app:restartApplicationForUpdate"),
    getAppVersion: () => ipc.invoke("app:getVersion"),
    getBuildInfo: () => ipc.invoke("app:getBuildInfo"),
    checkForUpdates: () => ipc.send("app:checkForUpdates"),
    isAppUpdateAvailable: () => ipc.invoke("autoUpdater:isUpdateAvailable"),
    isAppUpdateDownloaded: () => ipc.invoke("autoUpdater:isUpdateDownloaded"),
    // Native file picker replaces <input type="file"> + webUtils.getPathForFile
    selectCustomCSSFile: () => ipc.send("dialog:selectCustomCss"),
    lastFMStepAuthenticate: () => ipc.send("lastfm:authenticate")
  });
})();
