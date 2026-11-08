// Port of windows/titlebar/Index.vue + components/YTMViewLoading.vue
(function () {
  const ytmd = window.ytmd;
  const $ = id => document.getElementById(id);

  // ---- Titlebar ----
  const keyboardFocusZero = $("keyboard-focus-zero");
  const keyboardFocus = $("keyboard-focus");
  const pip = document.createElement("button");
  pip.className = "app-button";
  pip.innerHTML = '<span class="material-symbols-outlined">pip</span>';
  pip.addEventListener("click", () => ytmd.openMiniplayerWindow());
  createTitleBar($("titlebar-container"), {
    isMainWindow: true,
    hasHomeButton: true,
    hasSettingsButton: true,
    hasMinimizeButton: true,
    hasMaximizeButton: true,
    title: "YtmRs",
    iconFile: "/assets/icons/ytmrs_white.png",
    appButtons: [pip]
  });
  window.addEventListener("focus", () => {
    if (document.activeElement !== keyboardFocusZero) {
      // This resets the focus of keyboard navigation
      keyboardFocusZero.focus();
      keyboardFocusZero.blur();
    }
  });
  keyboardFocus.addEventListener("focus", () => ytmd.switchFocus("ytm"));
  ytmd.requestWindowState();

})();
