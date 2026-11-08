// Port of windows/settings/Settings.vue (+ YTMDSetting.vue and KeybindInput.vue)
(async function () {
  const ytmd = window.ytmd;
  const store = ytmd.store;
  const memoryStore = ytmd.memoryStore;
  const safeStorage = ytmd.safeStorage;
  const $ = id => document.getElementById(id);

  createTitleBar($("titlebar-container"), { title: "Settings", icon: "settings" });
  ytmd.requestWindowState();

  // ---------- state ----------
  const s = {
    currentTab: 1,
    requiresRestart: false,
    checkingForUpdate: false,
    updateAvailable: await ytmd.isAppUpdateAvailable(),
    updateNotAvailable: false,
    updateDownloaded: await ytmd.isAppUpdateDownloaded(),
    safeStorageAvailable: !!(await memoryStore.get("safeStorageAvailable")),
    config: {},
    memory: {},
    companionServerAuthTokens: []
  };

  async function decryptTokens(encrypted) {
    if (!s.safeStorageAvailable) return [];
    try {
      return JSON.parse(await safeStorage.decryptString(encrypted)) ?? [];
    } catch {
      return [];
    }
  }

  async function loadConfig(config) {
    s.config = config;
    s.companionServerAuthTokens = await decryptTokens(config.integrations.companionServerAuthTokens);
  }

  const sections = ["general", "appearance", "playback", "integrations", "shortcuts", "lastfm"];
  const initial = {};
  for (const section of sections) initial[section] = await store.get(section);
  await loadConfig(initial);
  for (const key of [
    "discordPresenceConnectionFailed",
    "shortcutsPlayPauseRegisterFailed",
    "shortcutsNextRegisterFailed",
    "shortcutsPreviousRegisterFailed",
    "shortcutsThumbsUpRegisterFailed",
    "shortcutsThumbsDownRegisterFailed",
    "shortcutsVolumeUpRegisterFailed",
    "shortcutsVolumeDownRegisterFailed",
    "shortcutsVolumeMuteRegisterFailed",
    "shortcutsToggleShuffleRegisterFailed",
    "shortcutsToggleRepeatRegisterFailed",
    "companionServerAuthWindowEnabled",
    "autoUpdaterDisabled",
    "lastFMAuthenticationStep"
  ]) {
    s.memory[key] = await memoryStore.get(key);
  }

  const get = path => path.split(".").reduce((o, k) => (o == null ? undefined : o[k]), s.config);
  const set = (path, value) => {
    const parts = path.split(".");
    let o = s.config;
    for (const p of parts.slice(0, -1)) o = o[p] = o[p] ?? {};
    o[parts[parts.length - 1]] = value;
    store.set(path, value);
  };

  store.onDidAnyChange(async newState => {
    await loadConfig(newState);
    render();
  });
  memoryStore.onStateChanged(newState => {
    s.memory = newState;
    s.safeStorageAvailable = !!newState.safeStorageAvailable;
    s.checkingForUpdate = !!newState.autoUpdaterChecking;
    s.updateAvailable = !!newState.autoUpdaterAvailable;
    s.updateNotAvailable = !!newState.autoUpdaterNotAvailable;
    s.updateDownloaded = !!newState.autoUpdaterDownloaded;
    render();
  });

  // ---------- element helpers ----------
  const el = (tag, attrs = {}, children = []) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === "class") e.className = v;
      else if (k === "text") e.textContent = v;
      else if (k.startsWith("on")) e.addEventListener(k.substring(2), v);
      else e.setAttribute(k, v);
    }
    for (const c of children) if (c) e.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
    return e;
  };
  const updaters = [];
  const visibility = (element, predicate) => updaters.push(() => (element.hidden = !predicate()));

  function nameBlock(opts) {
    const extras = () => [
      opts.restartRequired ? el("span", { class: "reload-required material-symbols-outlined", text: "autorenew" }) : null,
      opts.beta ? el("span", { class: "beta-tag", title: "This is a beta feature and may not work correctly yet.", text: "BETA" }) : null
    ];
    const plain = el("p", {}, [opts.name + " ", ...extras()]);
    const withDescription = el("div", { class: "name-description" }, [el("p", { class: "name" }, [opts.name + " ", ...extras()]), el("p", { class: "description", text: opts.description || "" })]);
    const disabled = el("div", { class: "disabled-name-message" }, [
      el("p", { class: "name" }, [el("span", { class: "disabled-tag", text: "DISABLED" }), " " + opts.name + " ", ...extras()]),
      el("p", { class: "message", text: opts.disabledMessage || "" })
    ]);
    const isDisabled = () => (opts.disabled ? opts.disabled() : false);
    const hasDescription = !!(opts.description && opts.description.trim() !== "");
    visibility(plain, () => !isDisabled() && !hasDescription);
    visibility(withDescription, () => !isDisabled() && hasDescription);
    visibility(disabled, isDisabled);
    return [plain, withDescription, disabled];
  }

  // YTMDSetting
  function setting(opts) {
    const row = el("div", { class: "ytmd-setting" + (opts.indented ? " indented" : "") + (opts.flexColumn ? " flex-column" : "") }, nameBlock(opts));
    const isDisabled = () => (opts.disabled ? opts.disabled() : false);
    if (opts.type === "checkbox") {
      const input = el("input", { type: "checkbox" });
      input.addEventListener("change", () => {
        opts.set(input.checked);
        opts.onChange?.();
      });
      updaters.push(() => {
        input.checked = !!opts.get();
        input.disabled = isDisabled();
      });
      row.appendChild(input);
    } else if (opts.type === "range") {
      const value = el("span", { class: "range-value" });
      const input = el("input", { type: "range", min: opts.min, max: opts.max, step: opts.step });
      input.addEventListener("input", () => (value.textContent = input.value));
      input.addEventListener("change", () => {
        opts.set(Number(input.value));
        opts.onChange?.();
      });
      updaters.push(() => {
        if (document.activeElement !== input) input.value = opts.get();
        value.textContent = input.value;
        input.disabled = isDisabled();
      });
      row.appendChild(el("div", { class: "range-selector" }, [value, input]));
    } else if (opts.type === "file") {
      const path = el("input", { type: "text", readonly: "", class: "path", placeholder: "No file chosen" });
      const remove = el("button", { class: "remove", onclick: () => opts.onClear() }, [el("span", { class: "material-symbols-outlined", text: "delete" })]);
      const choose = el("button", { class: "choose", onclick: () => opts.onChoose() }, [el("span", { class: "material-symbols-outlined", text: "file_open" })]);
      updaters.push(() => {
        path.value = opts.get() ?? "";
        remove.hidden = !opts.get();
      });
      row.appendChild(el("div", { class: "file-picker" }, [el("div", { class: "file-input-button" }, [choose, path, remove])]));
    } else if (opts.type === "select") {
      const select = el("div", { class: "select" });
      const text = el("p", { class: "text" });
      const arrow = el("span", { class: "material-symbols-outlined", text: "arrow_drop_down" });
      const selected = el("div", { class: "selected", onclick: () => {
        select.classList.toggle("open");
        arrow.textContent = select.classList.contains("open") ? "arrow_drop_up" : "arrow_drop_down";
      } }, [text, arrow]);
      const options = el("div", { class: "options" });
      for (const [key, label] of Object.entries(opts.optionsMap)) {
        options.appendChild(el("div", { class: "option", text: label, onclick: () => {
          opts.set(Number.parseInt(key));
          select.classList.remove("open");
          arrow.textContent = "arrow_drop_down";
          opts.onChange?.();
        } }));
      }
      select.append(selected, options);
      updaters.push(() => (text.textContent = opts.optionsMap[opts.get()] ?? ""));
      row.appendChild(select);
    } else if (opts.type === "custom") {
      for (const child of opts.children) row.appendChild(child);
    }
    if (opts.showIf) visibility(row, opts.showIf);
    return row;
  }

  // KeybindInput
  function keybindInput(path) {
    const isDisallowedKey = key =>
      ["Meta", "Command", "Control", "Alt", "Shift", "AltGraph", "Pause", "MediaPlayPause", "MediaTrackPrevious", "MediaTrackNext", "MediaStop", "Tab", "AudioVolumeUp", "AudioVolumeDown", "AudioVolumeMute", "ContextMenu", "Cancel"].includes(key);
    const codeMap = {
      NumpadEnter: "Enter", NumpadAdd: "NumAdd", NumpadSubtract: "NumSub", NumpadDecimal: "NumDec", NumpadMultiply: "NumMult", NumpadDivide: "NumDiv",
      Numpad0: "Num0", Numpad1: "Num1", Numpad2: "Num2", Numpad3: "Num3", Numpad4: "Num4", Numpad5: "Num5", Numpad6: "Num6", Numpad7: "Num7", Numpad8: "Num8", Numpad9: "Num9",
      ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right"
    };
    const validateKey = event => {
      if (event.key === " ") return "Space";
      if (codeMap[event.code]) return codeMap[event.code];
      if (event.shiftKey && event.code === "Equal") return "Plus";
      if (event.keyCode >= 65 && event.keyCode <= 90) return event.key.toUpperCase();
      return event.key;
    };
    const wrapper = el("div", { class: "keybind" });
    const text = el("p", { class: "keybind-text" });
    const input = el("input", { class: "keybind-input", type: "text" });
    const remove = el("button", { class: "remove" }, [el("span", { class: "material-symbols-outlined", text: "delete" })]);
    const commit = value => {
      set(path, value);
      render();
    };
    text.addEventListener("click", () => input.focus());
    input.addEventListener("focus", () => wrapper.classList.add("is-editing"));
    input.addEventListener("blur", () => wrapper.classList.remove("is-editing"));
    input.addEventListener("keydown", event => {
      if (isDisallowedKey(event.key)) return;
      event.preventDefault();
      if (event.key === "Escape") {
        commit("");
        input.blur();
        return;
      }
      let keybind = "";
      if (event.metaKey) keybind += "Meta+";
      if (event.ctrlKey) keybind += "CmdOrCtrl+";
      if (event.altKey) keybind += "Alt+";
      if (event.shiftKey) keybind += "Shift+";
      keybind += validateKey(event);
      input.blur();
      commit(keybind);
    });
    remove.addEventListener("click", () => commit(""));
    updaters.push(() => {
      text.textContent = get(path) || "None";
      remove.disabled = !get(path);
    });
    wrapper.append(text, input, remove);
    return wrapper;
  }

  function restart() {
    s.requiresRestart = true;
    render();
  }

  // ---------- tabs ----------
  const tab = n => document.querySelector(`[data-tab-content="${n}"]`);
  const cb = (path, name, extra = {}) => setting({ type: "checkbox", name, get: () => get(path), set: v => set(path, v), ...extra });
  const isDarwin = ytmd.isDarwin;
  const isLinux = ytmd.isLinux;

  // General
  const general = tab(1);
  const hideToTray = cb("general.hideToTrayOnClose", "Hide to tray on close");
  hideToTray.hidden = isDarwin;
  if (!isDarwin) general.appendChild(hideToTray);
  general.appendChild(cb("general.showNotificationOnSongChange", "Show notification on song change"));
  general.appendChild(cb("general.startOnBoot", "Start on boot"));
  general.appendChild(cb("general.disableHardwareAcceleration", "Disable hardware acceleration", { restartRequired: true, onChange: restart }));

  // Appearance
  const appearance = tab(2);
  appearance.appendChild(cb("appearance.alwaysShowVolumeSlider", "Always show volume slider", { description: "This feature only works on the old player bar and may not work for you" }));
  appearance.appendChild(cb("appearance.customCSSEnabled", "Custom CSS"));
  appearance.appendChild(
    setting({
      type: "file",
      indented: true,
      name: "Custom CSS file path",
      get: () => get("appearance.customCSSPath"),
      onChoose: () => ytmd.selectCustomCSSFile(),
      onClear: () => set("appearance.customCSSPath", null),
      showIf: () => !!get("appearance.customCSSEnabled")
    })
  );
  appearance.appendChild(setting({ type: "range", name: "Zoom", min: 30, max: 300, step: 10, get: () => get("appearance.zoom"), set: v => set("appearance.zoom", v) }));
  if (isLinux) {
    appearance.appendChild(setting({ type: "select", name: "Tray icon style", optionsMap: { 0: "Auto", 1: "White", 2: "Black" }, get: () => get("appearance.trayIconStyle"), set: v => set("appearance.trayIconStyle", v) }));
  }

  // Playback
  const playback = tab(3);
  playback.appendChild(cb("playback.continueWhereYouLeftOff", "Continue where you left off", { description: "If you're signed in this feature can be turned off" }));
  playback.appendChild(cb("playback.continueWhereYouLeftOffPaused", "Pause on application launch", { indented: true, showIf: () => !!get("playback.continueWhereYouLeftOff") }));
  playback.appendChild(cb("playback.progressInTaskbar", "Show track progress on taskbar"));
  playback.appendChild(cb("playback.enableSpeakerFill", "Enable speaker fill", { restartRequired: true, onChange: restart }));
  playback.appendChild(cb("playback.ratioVolume", "Ratio volume"));

  // Integrations
  const integrations = tab(4);
  const companionEnabled = () => !!get("integrations.companionServerEnabled") && s.safeStorageAvailable;
  integrations.appendChild(cb("integrations.companionServerEnabled", "Companion server", { disabled: () => !s.safeStorageAvailable, disabledMessage: "This integration cannot be enabled due to safeStorage being unavailable" }));
  integrations.appendChild(cb("integrations.companionServerCORSWildcardEnabled", "Allow browser communication", { indented: true, description: "This setting could be dangerous as it allows any website you visit to communicate with the companion server", showIf: companionEnabled }));
  integrations.appendChild(
    setting({
      type: "checkbox",
      indented: true,
      name: "Enable companion authorization",
      description: "Automatically disables after the first successful authorization or 5 minutes has passed",
      get: () => s.memory.companionServerAuthWindowEnabled,
      set: v => {
        s.memory.companionServerAuthWindowEnabled = v;
        memoryStore.set("companionServerAuthWindowEnabled", v);
      },
      showIf: companionEnabled
    })
  );
  const tokensBody = el("tbody");
  const noCompanions = el("div", { class: "no-authorized-companions" }, [el("span", { text: "No authorized companions" })]);
  updaters.push(() => {
    tokensBody.replaceChildren(
      ...s.companionServerAuthTokens.map(token =>
        el("tr", {}, [
          el("td", { class: "companion" }, [el("span", { class: "name", text: token.appName }), el("br"), el("span", { class: "id", text: token.appId })]),
          el("td", { class: "version", text: token.appVersion }),
          el("td", { class: "controls" }, [el("button", { onclick: () => deleteCompanionAuthToken(token.appId) }, [el("span", { class: "material-symbols-outlined", text: "delete" })])])
        ])
      )
    );
    noCompanions.hidden = s.companionServerAuthTokens.length !== 0;
  });
  integrations.appendChild(
    setting({
      type: "custom",
      flexColumn: true,
      indented: true,
      name: "Authorized companions",
      description: "This is a list of companions that currently have access to the companion server",
      showIf: companionEnabled,
      children: [
        el("table", { class: "authorized-companions-table" }, [
          el("thead", {}, [el("tr", {}, [el("th", { class: "companion", text: "Companion" }), el("th", { class: "version", text: "Version" }), el("th", { class: "controls" })])]),
          tokensBody
        ]),
        noCompanions
      ]
    })
  );
  async function deleteCompanionAuthToken(appId) {
    const index = s.companionServerAuthTokens.findIndex(token => token.appId === appId);
    if (index > -1) s.companionServerAuthTokens.splice(index, 1);
    if (s.safeStorageAvailable) store.set("integrations.companionServerAuthTokens", await safeStorage.encryptString(JSON.stringify(s.companionServerAuthTokens)));
    render();
  }

  integrations.appendChild(cb("integrations.discordPresenceEnabled", "Discord rich presence"));
  const discordFailure = el("div", { class: "setting indented" }, [
    el("p", { class: "discord-failure", text: "Discord connection could not be established after 30 attempts" }),
    el("button", { text: "Retry", onclick: () => {
      set("integrations.discordPresenceEnabled", false);
      set("integrations.discordPresenceEnabled", true);
    } })
  ]);
  visibility(discordFailure, () => !!get("integrations.discordPresenceEnabled") && !!s.memory.discordPresenceConnectionFailed);
  integrations.appendChild(discordFailure);

  integrations.appendChild(cb("integrations.lastFMEnabled", "Last.fm scrobbling", { disabled: () => !s.safeStorageAvailable, disabledMessage: "This integration cannot be enabled due to safeStorage being unavailable" }));
  const authYes = el("span", { style: "color: #4caf50", text: "Yes" });
  const authNo = el("span", { style: "color: #ff1100", text: "No" });
  const logout = el("button", { text: "Logout", onclick: () => {
    set("lastfm.sessionKey", null);
    set("integrations.lastFMEnabled", false);
    render();
  } });
  const lastFMStatus = el("div", { class: "setting indented" }, [el("div", { class: "name-with-description" }, [el("p", { class: "description" }, ["User is Authenticated: ", authYes, authNo])]), logout]);
  visibility(lastFMStatus, () => !!get("integrations.lastFMEnabled"));
  visibility(authYes, () => !!get("lastfm.sessionKey"));
  visibility(authNo, () => !get("lastfm.sessionKey"));
  visibility(logout, () => !!get("lastfm.sessionKey"));
  integrations.appendChild(lastFMStatus);
  const step0 = el("div", { class: "setting indented" }, [el("p", { text: "Continue setup in your browser then come back here when you're done" }), el("button", { text: "Authorize Last.fm in browser", onclick: () => ytmd.lastFMStepAuthenticate() })]);
  visibility(step0, () => !!get("integrations.lastFMEnabled") && s.memory.lastFMAuthenticationStep === 0);
  integrations.appendChild(step0);
  const step1 = el("div", { class: "setting indented" }, [el("p", { text: 'After you\'re done in the browser click "Finish Last.fm authentication"' }), el("button", { text: "Finish Last.fm authentication", onclick: () => ytmd.lastFMStepAuthenticate() })]);
  visibility(step1, () => !!get("integrations.lastFMEnabled") && s.memory.lastFMAuthenticationStep === 1);
  integrations.appendChild(step1);
  integrations.appendChild(
    setting({ type: "range", indented: true, name: "Scrobble percent", description: "Determines when a song is scrobbled", min: 50, max: 95, step: 5, get: () => get("lastfm.scrobblePercent"), set: v => set("lastfm.scrobblePercent", v), showIf: () => !!get("integrations.lastFMEnabled") })
  );
  integrations.appendChild(
    cb("integrations.enhancedMediaServiceEnabled", "Enhanced media service", {
      restartRequired: true,
      description: "An enhanced media service that can provide better media button controls",
      disabled: () => isDarwin,
      disabledMessage: "This integration is not available on your platform",
      beta: true,
      onChange: restart
    })
  );

  // Shortcuts
  const shortcuts = tab(5);
  for (const [key, title, flag] of [
    ["playPause", "Play/Pause", "shortcutsPlayPauseRegisterFailed"],
    ["next", "Next", "shortcutsNextRegisterFailed"],
    ["previous", "Previous", "shortcutsPreviousRegisterFailed"],
    ["thumbsUp", "Thumbs Up", "shortcutsThumbsUpRegisterFailed"],
    ["thumbsDown", "Thumbs Down", "shortcutsThumbsDownRegisterFailed"],
    ["volumeUp", "Increase Volume", "shortcutsVolumeUpRegisterFailed"],
    ["volumeDown", "Decrease Volume", "shortcutsVolumeDownRegisterFailed"],
    ["volumeMute", "Mute Volume", "shortcutsVolumeMuteRegisterFailed"],
    ["toggleShuffle", "Toggle Shuffle", "shortcutsToggleShuffleRegisterFailed"],
    ["toggleRepeat", "Toggle Repeat", "shortcutsToggleRepeatRegisterFailed"]
  ]) {
    const error = el("span", { class: "material-symbols-outlined register-error", title: "Failed to register keybind. Does another application have this keybind?", text: "error" });
    visibility(error, () => !!s.memory[flag]);
    shortcuts.appendChild(el("div", { class: "setting" }, [el("p", { class: "shortcut-title" }, [title, error]), keybindInput(`shortcuts.${key}`)]));
  }

  // About
  const version = await ytmd.getAppVersion();
  const build = await ytmd.getBuildInfo();
  $("version").textContent = `Version: ${version}`;
  $("branch").textContent = `Branch: ${build.branch ?? "None"}`;
  $("commit").textContent = `Commit: ${build.commit ?? "None"}`;
  $("webview").textContent = `Webview: ${build.webview || "Unknown"}`;
  $("check-updates").addEventListener("click", () => {
    ytmd.checkForUpdates();
    s.checkingForUpdate = true;
    render();
  });
  $("restart-update").addEventListener("click", () => ytmd.restartApplicationForUpdate());
  $("restart-button").addEventListener("click", () => ytmd.restartApplication());
  updaters.push(() => {
    const disabled = !!s.memory.autoUpdaterDisabled;
    $("updater-enabled").hidden = disabled;
    $("updater-disabled").hidden = !disabled;
    $("check-updates").hidden = s.updateDownloaded;
    $("check-updates").disabled = !(!s.checkingForUpdate && !s.updateAvailable && !s.updateDownloaded);
    $("restart-update").hidden = !s.updateDownloaded;
    $("checking").hidden = !(s.checkingForUpdate && !s.updateAvailable && !s.updateDownloaded);
    $("downloading").hidden = !(s.updateAvailable && !s.updateDownloaded);
    $("no-update").hidden = !s.updateNotAvailable;
  });
  for (const a of document.querySelectorAll(".links a")) {
    a.addEventListener("click", event => {
      event.preventDefault();
      window.open(a.href, "_blank");
    });
  }

  // Sidebar
  for (const li of document.querySelectorAll(".sidebar li")) {
    li.addEventListener("click", () => {
      s.currentTab = Number(li.dataset.tab);
      render();
    });
  }

  function render() {
    for (const li of document.querySelectorAll(".sidebar li")) li.classList.toggle("active", Number(li.dataset.tab) === s.currentTab);
    for (const t of document.querySelectorAll("[data-tab-content]")) t.hidden = Number(t.dataset.tabContent) !== s.currentTab;
    $("restart-banner").hidden = !s.requiresRestart;
    for (const u of updaters) u();
  }
  render();
})();
