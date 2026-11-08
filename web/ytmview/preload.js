// Port of renderer/ytmview/preload.ts (glue between YTM and the main process)
// NOTE: there is no isolated world in the native webview, so the YTM page cannot reach any
// config/memory store channel: the main process only accepts YTM view scoped channels.
(function () {
  if (window.top !== window) return;
  const ipc = window.__ytmdIpc;
  if (!ipc) return;

  // Context menu (Electron webContents "context-menu"), rendered natively by the main process
  document.addEventListener(
    "contextmenu",
    event => {
      event.preventDefault();
      const target = event.target instanceof Element ? event.target : null;
      const link = target?.closest("a[href]");
      const img = target?.closest("img");
      const tag = target?.tagName;
      const isEditable = !!target && (target.isContentEditable || tag === "TEXTAREA" || (tag === "INPUT" && !["checkbox", "radio", "button", "submit", "range", "file", "color"].includes(target.type)));
      const selectionText = String(window.getSelection() || "");
      ipc.send("ytmView:contextMenu", {
        x: event.clientX,
        y: event.clientY,
        linkURL: link ? link.href : "",
        srcURL: img ? img.currentSrc || img.src : "",
        hasImageContents: !!img,
        isEditable,
        selectionText,
        canUndo: isEditable,
        canRedo: isEditable,
        canCut: isEditable && selectionText.length > 0,
        canCopy: selectionText.length > 0
      });
    },
    true
  );

  // did-navigate / did-navigate-in-page
  const reportNavigation = () => ipc.send("ytmView:navigated", location.href);
  for (const method of ["pushState", "replaceState"]) {
    const original = history[method];
    history[method] = function (...args) {
      const result = original.apply(this, args);
      reportNavigation();
      return result;
    };
  }
  window.addEventListener("popstate", reportNavigation);
  window.addEventListener("load", reportNavigation);

  if (location.hostname !== "music.youtube.com") return;

  function applyVolumeSlider(persist) {
    const volumeSlider = document.querySelector("#volume-slider");
    if (!volumeSlider) return;
    if (persist) volumeSlider.classList.add("ytmd-persist-volume-slider");
    else volumeSlider.classList.remove("ytmd-persist-volume-slider");
  }

  // Sent by the main process after "ytmView:ready"
  ipc.on("ytmView:config", config => {
    if (config.continueWhereYouLeftOff) {
      // The last page the user was on is already a page where it will be playing a song from
      if (!window.location.pathname.startsWith("/watch") && config.lastVideoId) {
        document.dispatchEvent(
          new CustomEvent("yt-navigate", {
            detail: { endpoint: { watchEndpoint: { videoId: config.lastVideoId, playlistId: config.lastPlaylistId } } }
          })
        );
      }
    }
    if (config.alwaysShowVolumeSlider) {
      document.querySelector("ytmusic-app-layout>ytmusic-player-bar #volume-slider")?.classList.add("ytmd-persist-volume-slider");
    }
  });
  ipc.on("ytmView:alwaysShowVolumeSlider", applyVolumeSlider);
})();
