// Port of components/TitleBar.vue
// createTitleBar(container, { title, icon, iconFile, hasHomeButton, hasSettingsButton,
//   hasMinimizeButton, hasMaximizeButton, centerTitleText, isMainWindow, appButtons: [Element] })
// eslint-disable-next-line no-unused-vars
function createTitleBar(container, props) {
  const ytmd = window.ytmd;
  const platform = window.__ytmdPlatform || {};
  const wcoVisible = !!platform.wcoVisible;
  let windowMaximized = false;
  let windowFullscreen = false;
  let ytmViewUnresponsive = false;
  let appUpdateDownloaded = false;

  const el = (tag, attrs = {}, children = []) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === "class") e.className = v;
      else if (k === "text") e.textContent = v;
      else if (k.startsWith("on")) e.addEventListener(k.substring(2), v);
      else e.setAttribute(k, v);
    }
    for (const c of children) if (c) e.appendChild(c);
    return e;
  };
  const symbol = name => el("span", { class: "material-symbols-outlined", text: name });

  const bar = el("div", { class: "titlebar" + (wcoVisible ? " wco" : ""), "data-app-region": "drag" });
  const titleText = el("p", { class: "title-text" });
  const title = el("div", { class: "title" }, [
    props.icon ? el("span", { class: "icon material-symbols-outlined", text: props.icon }) : null,
    props.iconFile ? el("img", { class: "icon", src: props.iconFile, draggable: "false" }) : null,
    props.title && !props.centerTitleText ? titleText : null
  ]);
  bar.appendChild(el("div", { class: "left" }, [title]));
  if (props.title && props.centerTitleText) bar.appendChild(el("div", { class: "center" }, [titleText]));

  const right = el("div", { class: "right" });
  const updateButtons = el("div", { class: "update-buttons" });
  const updateButton = el("button", { class: "app-button update-button", tabindex: "1", title: "Update ready! Click to restart", "data-app-region": "no-drag", onclick: () => ytmd.restartApplicationForUpdate() }, [symbol("upgrade")]);
  updateButtons.appendChild(updateButton);
  if (props.isMainWindow) right.appendChild(updateButtons);

  const appButtons = el("div", { class: "app-buttons", "data-app-region": "no-drag" });
  for (const b of props.appButtons || []) appButtons.appendChild(b);
  if (props.hasHomeButton) appButtons.appendChild(el("button", { class: "app-button", tabindex: "2", onclick: () => ytmd.ytmViewNavigateDefault() }, [symbol("home")]));
  if (props.hasSettingsButton) appButtons.appendChild(el("button", { class: "app-button", tabindex: "3", onclick: () => ytmd.openSettingsWindow() }, [symbol("settings")]));
  right.appendChild(appButtons);

  const actions = el("div", { class: "windows-action-buttons", "data-app-region": "no-drag" });
  const minimize = el("button", { class: "action-button window-minimize", tabindex: "4", onclick: () => ytmd.minimizeWindow() }, [symbol("remove")]);
  const maximize = el("button", { class: "action-button window-maximize", tabindex: "5", onclick: () => ytmd.maximizeWindow() }, [symbol("square")]);
  const restore = el("button", { class: "action-button window-restore", tabindex: "6", onclick: () => ytmd.restoreWindow() }, [symbol("filter_none")]);
  const close = el("button", { class: "action-button window-close", tabindex: "7", onclick: () => ytmd.closeWindow() }, [symbol("close")]);
  if (props.hasMinimizeButton) actions.appendChild(minimize);
  if (props.hasMaximizeButton) actions.appendChild(maximize);
  if (props.hasMinimizeButton) actions.appendChild(restore);
  actions.appendChild(close);
  if (!wcoVisible) right.appendChild(actions);
  bar.appendChild(right);

  function render() {
    bar.hidden = windowFullscreen;
    titleText.textContent = (props.title || "") + (ytmViewUnresponsive ? " (Unresponsive)" : "");
    maximize.hidden = !(props.hasMaximizeButton && !windowMaximized);
    restore.hidden = !(props.hasMinimizeButton && windowMaximized);
    updateButton.hidden = !appUpdateDownloaded;
  }

  if (ytmd.handleWindowEvents) {
    ytmd.handleWindowEvents(state => {
      windowMaximized = state.maximized;
      windowFullscreen = state.fullscreen;
      render();
    });
  }

  if (props.isMainWindow && ytmd.memoryStore) {
    ytmd.memoryStore.get("ytmViewUnresponsive").then(v => {
      ytmViewUnresponsive = v ?? false;
      render();
    });
    ytmd.memoryStore.get("autoUpdaterDownloaded").then(v => {
      appUpdateDownloaded = v ?? false;
      render();
    });
    ytmd.memoryStore.onStateChanged(newState => {
      ytmViewUnresponsive = newState.ytmViewUnresponsive ?? false;
      appUpdateDownloaded = newState.autoUpdaterDownloaded ?? false;
      render();
    });
  }

  render();
  container.appendChild(bar);
  return bar;
}
