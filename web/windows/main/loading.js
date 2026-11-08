// Port of windows/main/Index.vue -> components/YTMViewLoading.vue
(function () {
  const ytmd = window.ytmd;
  const $ = id => document.getElementById(id);
  // ---- YTMViewLoading ----
  const Status = { Loading: 0, Hooking: 1, Ready: 2, LoadFailed: 3 };
  const container = $("ytmview-loading-container");
  const loading = $("ytmview-loading");
  const unresponsiveEl = $("ytmview-unresponsive");
  let hide = false;
  let status = Status.Loading;
  let unresponsive = false;
  let loadError = null;
  let timedOut = false;

  // Vue <Transition name="fade" @after-leave="onHide"> equivalent
  function transition(element, visible, afterLeave) {
    if (visible) {
      if (!element.hidden && !element.classList.contains("faded")) return;
      clearTimeout(element.__leaveTimer);
      element.hidden = false;
      requestAnimationFrame(() => element.classList.remove("faded"));
    } else {
      if (element.hidden || element.__leaving) return;
      element.__leaving = true;
      element.classList.add("faded");
      element.__leaveTimer = setTimeout(() => {
        element.__leaving = false;
        element.hidden = true;
        if (afterLeave) afterLeave();
      }, 250);
    }
  }
  const onHide = () => ytmd.appViewHide();

  function statusMessage() {
    switch (status) {
      case Status.Loading:
        return "Loading YouTube Music...";
      case Status.Hooking:
        return "Waiting for YtmRs hooks...";
      case Status.Ready:
        return "Ready";
      default:
        return "";
    }
  }

  function render() {
    transition(container, !hide, onHide);
    transition(loading, status !== Status.Ready, onHide);
    transition(unresponsiveEl, unresponsive, onHide);
    $("loading-status").textContent = statusMessage();
    $("loading-status").hidden = !!loadError;
    $("load-error-container").hidden = !loadError;
    if (loadError) $("load-error-text").textContent = `Failed to load YouTube Music: ${loadError.description} (${loadError.code})`;
    $("loading-timeout").hidden = !(timedOut && !loadError);
  }

  let timeout = setTimeout(() => {
    timedOut = true;
    render();
  }, 5000);

  ytmd.ytmViewStatusChanged(newStatus => {
    if (newStatus !== Status.Ready) {
      timedOut = false;
      clearTimeout(timeout);
      timeout = setTimeout(() => {
        timedOut = true;
        render();
      }, 5000);
    }
    status = newStatus;
    render();
  });
  ytmd.ytmViewLoadError(error => {
    loadError = error;
    render();
  });
  ytmd.memoryStore.get("ytmViewUnresponsive").then(value => {
    unresponsive = value ?? false;
    render();
  });
  ytmd.memoryStore.onStateChanged(newState => {
    unresponsive = newState.ytmViewUnresponsive ?? false;
    render();
  });
  ytmd.appViewHiding(() => {
    hide = true;
    render();
  });
  ytmd.appViewShowing(() => {
    hide = false;
    render();
  });
  $("refresh-ytm").addEventListener("click", () => {
    loadError = null;
    ytmd.ytmViewReload();
    render();
  });
  render();
})();
