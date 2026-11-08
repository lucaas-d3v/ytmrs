// Port of windows/miniplayer/Index.vue + Miniplayer.vue + components/RangeInput.vue
(async function () {
  const ytmd = window.ytmd;
  const $ = id => document.getElementById(id);
  const VideoState = { Unknown: -1, Paused: 0, Playing: 1, Buffering: 2 };
  const RepeatMode = { Unknown: -1, None: 0, All: 1, One: 2 };

  // ---- Index.vue ----
  const isWayland = ytmd.isWayland();
  let alwaysOnTop = true;
  const pin = $("pin");
  pin.disabled = isWayland;
  pin.title = !isWayland ? "Pin/Unpin Miniplayer" : "Use your native window manager to pin the miniplayer";
  const renderPin = () => ($("pin-icon").textContent = alwaysOnTop ? "keep" : "keep_off");
  ytmd.handleWindowEvents(windowState => {
    alwaysOnTop = windowState.alwaysOnTop;
    renderPin();
  });
  pin.addEventListener("click", () => ytmd.setAlwaysOnTopWindow(!alwaysOnTop));
  $("pip-exit").addEventListener("click", () => ytmd.closeWindow());

  // ---- Miniplayer.vue ----
  let state = await ytmd.playerStore.getState();
  let volume = state?.volume ?? 0;
  let videoProgress = 0;
  let videoLength = 0;
  let videoId = "";
  let seekbarDragging = false;
  let seekHandleVisible = false;
  const seekbarContainer = $("seekbar-container");
  const infoContainer = $("info-container");
  const infoTitle = $("info-title");

  function getHighestResThumbnail(thumbnails) {
    if (!thumbnails || thumbnails.length === 0) return null;
    return thumbnails.reduce((acc, cur) => (cur.height * cur.width <= acc.height * acc.width ? acc : cur), thumbnails[0]).url;
  }

  function reconcileMarquee() {
    if (infoTitle.scrollWidth > infoContainer.clientWidth) infoTitle.classList.add("marquee");
    else infoTitle.classList.remove("marquee");
  }

  function render() {
    const hasVideo = state?.videoDetails != null;
    $("player").hidden = !hasVideo;
    $("unavailable").hidden = hasVideo;
    if (!hasVideo) return;
    infoTitle.textContent = `${state.videoDetails.title} • ${state.videoDetails.author}`;
    const pct = videoLength ? (videoProgress / videoLength) * 100 : 0;
    $("progress").style.width = `${pct}%`;
    $("handle").style.left = `${pct}%`;
    $("handle").hidden = !seekHandleVisible;
    let icon = "play_arrow";
    if (state.trackState == VideoState.Playing) icon = "pause";
    else if (state.trackState == VideoState.Buffering) icon = "data_saver_off";
    $("play-pause-icon").textContent = icon;
    $("mute-icon").textContent = state.muted ? "volume_off" : volume > 50 ? "volume_up" : "volume_down";
    const repeat = state.queue?.repeatMode;
    $("repeat-icon").textContent = repeat === RepeatMode.All ? "repeat_on" : repeat === RepeatMode.One ? "repeat_one_on" : "repeat";
    const vpct = Math.min(Math.max(volume, 0), 100);
    $("volume-fill").style.width = `${vpct}%`;
    $("volume-handle").style.left = `${vpct}%`;
  }

  function stateChanged(newState) {
    state = newState;
    if (newState?.videoDetails && newState.videoDetails.id != videoId) {
      const thumbnailUrl = getHighestResThumbnail(newState.videoDetails.thumbnails);
      document.body.style.backgroundImage = thumbnailUrl ? `url("${thumbnailUrl}")` : "";
      videoLength = newState.videoDetails.durationSeconds;
      videoId = newState.videoDetails.id;
      render();
      requestAnimationFrame(reconcileMarquee);
    }
    if (!seekbarDragging) videoProgress = newState?.videoProgress ?? 0;
    if (!volumeDragging) volume = newState?.volume ?? volume;
    render();
  }

  function seek(event) {
    const rect = seekbarContainer.getBoundingClientRect();
    const offsetX = Math.max(0, Math.min(event.clientX - rect.left, rect.width));
    const seekToValue = (offsetX * videoLength) / rect.width;
    videoProgress = seekToValue;
    ytmd.executeCommandInYTMView("seekTo", seekToValue);
    render();
  }
  seekbarContainer.addEventListener("mouseenter", () => { seekHandleVisible = true; render(); });
  seekbarContainer.addEventListener("mouseleave", () => { seekHandleVisible = false; render(); });
  seekbarContainer.addEventListener("mousedown", event => {
    seekbarDragging = true;
    seek(event);
    document.body.style.userSelect = "none";
  });
  document.addEventListener("mousemove", event => {
    if (!seekbarDragging) return;
    if (event.clientX > -1) seek(event);
  });
  document.addEventListener("mouseup", () => {
    if (seekbarDragging) {
      seekbarDragging = false;
      document.body.style.userSelect = "";
    }
  });

  $("previous").addEventListener("click", () => ytmd.executeCommandInYTMView("previous"));
  $("play-pause").addEventListener("click", () => ytmd.executeCommandInYTMView("playPause"));
  $("next").addEventListener("click", () => ytmd.executeCommandInYTMView("next"));
  $("repeat").addEventListener("click", () => ytmd.executeCommandInYTMView("cycleRepeatMode"));
  $("mute").addEventListener("click", () => ytmd.executeCommandInYTMView(!state?.muted ? "mute" : "unmute"));

  // RangeInput (min 0, max 100, step 1, scrollStep 10)
  let volumeDragging = false;
  const track = $("volume-track");
  const setVolume = v => {
    volume = Math.min(100, Math.max(0, v));
    ytmd.executeCommandInYTMView("setVolume", volume);
    render();
  };
  const updateFromEvent = event => {
    const rect = track.getBoundingClientRect();
    const x = event.touches ? event.touches[0].clientX : event.clientX;
    setVolume(((x - rect.left) / rect.width) * 100);
  };
  const stopDrag = () => {
    if (!volumeDragging) return;
    volumeDragging = false;
    window.removeEventListener("mousemove", updateFromEvent);
    window.removeEventListener("mouseup", stopDrag);
  };
  $("volume-range").addEventListener("mousedown", event => {
    event.preventDefault();
    volumeDragging = true;
    updateFromEvent(event);
    window.addEventListener("mousemove", updateFromEvent);
    window.addEventListener("mouseup", stopDrag);
  });
  $("volume-range").addEventListener(
    "wheel",
    event => {
      event.preventDefault();
      const delta = event.deltaY > 0 ? -1 : 1;
      setVolume(Math.round(volume + delta * 10));
    },
    { passive: false }
  );

  ytmd.playerStore.handleStateChanged(newState => stateChanged(newState));
  ytmd.requestWindowState();
  window.addEventListener("resize", reconcileMarquee);
  renderPin();
  stateChanged(state);
})();
