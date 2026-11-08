// Port of renderer/ytmview/scripts/* (bundled as one script, runs in the YTM main world)
(function () {
  "use strict";
  const YTMViewSetupCompletionFlags = { LocationNotApplicable: 1, Early: 2, Styles: 4, Navigation: 8, Hooks: 16, Remote: 32, Extras: 64 };
  const ipc = window.__ytmdIpc;
  if (!ipc) return;

  // ==== protectedapimanager.ts (MessagePorts replaced by the IPC bridge, same op codes)
  const ProtectedAPIOpCode = { ProvideAPIPort: 0, Message: 1, Invoke: 2, InvokeResponse: 3 };
  class ProtectedAPI extends EventTarget {
    constructor(name) {
      super();
      this.name = name;
      this.invokeHandlers = new Map();
    }
    postMessage(name, ...args) {
      ipc.send("protectedApi:message", this.name, name, args);
    }
    handleMessage(name, callback) {
      this.invokeHandlers.set(name, callback);
    }
    async _receive(packet) {
      if (packet.op === ProtectedAPIOpCode.Message) {
        this.dispatchEvent(new CustomEvent(packet.name, { detail: { args: packet.args } }));
      }
      if (packet.op === ProtectedAPIOpCode.Invoke) {
        let response;
        const handler = this.invokeHandlers.get(packet.name);
        if (handler) {
          try {
            response = await handler(new CustomEvent(packet.name, { detail: { args: packet.args } }));
          } catch (err) {
            console.error("[ytmd-err(protectedapi)]", err);
          }
        }
        ipc.send("protectedApi:invokeResponse", this.name, packet.id, response ?? []);
      }
    }
  }
  const protectedapimanager = {
    apis: new Map(),
    init() {
      const apis = this.apis;
      Object.defineProperty(window, "__ytmdProtectedApi", {
        value: Object.freeze({ _receive: packet => apis.get(packet.api)?._receive(packet) }),
        writable: false,
        configurable: false
      });
      ipc.send("protectedApi:bindPort");
    },
    createOrGetAPI(name) {
      if (this.apis.has(name)) return this.apis.get(name);
      const api = new ProtectedAPI(name);
      this.apis.set(name, api);
      ipc.send("protectedApi:providePort", name);
      return api;
    }
  };

// ==== ytmhook.ts
class YTMHook {
  _ytmStateStore;
  _ytmPlayerController;
  get ytmStateStore() {
    return this._ytmStateStore;
  }
  get ytmPlayerController() {
    return this._ytmPlayerController;
  }

  init() {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const self = this;

    // This hook is designed to capture addProvider calls which contain the dependency injection into elements within YTM
    const mapSet = Map.prototype.set;
    Object.defineProperty(Map.prototype, "set", {
      value: function (key, value) {
        if (key.name && key.name === "PLAYER_CONTROLLER_TOKEN") {
          if (value.useClass) {
            const OriginalClass = value.useClass;

            value.useClass = new Proxy(OriginalClass, {
              construct(target, args, newTarget) {
                const instance = Reflect.construct(target, args, newTarget);
                self._ytmPlayerController = instance;
                return instance;
              },
              get(target, prop, receiver) {
                return Reflect.get(target, prop, receiver);
              }
            });
          }
        }

        if (key.name && key.name === "STATE_STORE_TOKEN") {
          if (value.useFactory) {
            const originalFactory = value.useFactory;

            value.useFactory = new Proxy(originalFactory, {
              apply(target, thisArg, args) {
                const instance = Reflect.apply(target, thisArg, args);
                self._ytmStateStore = instance;
                return instance;
              }
            });
          }
        }

        return mapSet.call(this, key, value);
      }
    });
  }

  async ready() {
    if (this._ytmStateStore && this._ytmPlayerController) return Promise.resolve();

    return new Promise(resolve => {
      const interval = setInterval(() => {
        if (this._ytmStateStore && this._ytmPlayerController) {
          resolve();
          clearInterval(interval);
        }
      });
    });
  }
}




// ==== playerstatestore.ts

function getLikeStatus() {
  try {
    const ytmStore = ytmhook.ytmStateStore.store;
    const state = ytmStore.getState();

    let status = "INDIFFERENT";
    if (state.playerPage && state.playerPage.playerOverlay && state.playerPage.playerOverlay.playerOverlayRenderer) {
      if (state.playerPage.playerOverlay.playerOverlayRenderer.actions) {
        status = state.playerPage.playerOverlay.playerOverlayRenderer.actions[0].likeButtonRenderer.likeStatus;
      } else if (state.playerPage.playerOverlay.playerOverlayRenderer.videoActionBar) {
        // New player bar
        const actionBar = state.playerPage.playerOverlay.playerOverlayRenderer.videoActionBar;
        const viewModel = actionBar.videoActionBarViewModel.buttons[0].buttonViewModel.segmentedLikeDislikeButtonViewModel;
        status = viewModel.likeButtonViewModel.likeButtonViewModel.likeStatusEntity.likeStatus;
      }
    }
    return status;
  } catch (err) {
    console.error("[ytmd-err(getLikeStatus)]", err);
    return null;
  }
}

async function remapThumbnailsToDataUrl(thumbnails) {
  for (const thumbnail of thumbnails) {
    if (thumbnail.url) {
      try {
        // YTM utilizes CacheStorage and more specifically for thumbnails it's from `caches.open("yt-player-local-img:${yt.config_.DATASYNC_ID}")`
        const response = await caches.match(thumbnail.url);
        if (response && response.ok) {
          const blob = await response.blob();
          const dataUrl = await new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onloadend = () => resolve(reader.result);
            reader.onerror = reject;
            reader.readAsDataURL(blob);
          });
          thumbnail.url = dataUrl;
        }
      } catch(err) {
        console.error("[ytmd-err(sendVideoData)] Could not update thumbnail url to data url", err);
      }
    }
  }
}

function initPlayerStateStore() {
  const playerStateApi = protectedapimanager.createOrGetAPI("PlayerState");
  const ytmStore = ytmhook.ytmStateStore.store;
  //const playerBar = ytmhook.ytmPlayerController;
  const playerApi = ytmhook.ytmPlayerController.playerApi;

  async function sendStoreState() {
    // We don't want to see everything in the store as there can be some sensitive data so we only send what's necessary to operate
    const state = ytmStore.getState();

    const videoId = playerApi.getPlayerResponse()?.videoDetails?.videoId;
    const defaultLikeStatus = getLikeStatus() ?? "UNKNOWN";
    const storeLikeStatus = state.likeStatus.videos[videoId];

    const likeStatus = storeLikeStatus ? state.likeStatus.videos[videoId] : defaultLikeStatus;
    const volume = state.player.volume;
    const adPlaying = state.player.adPlaying;
    const muted = state.player.muted;
    
    // This feature which can remap queue item thumbnails into data url is extremely intensive both memory and for companion api responses
    // 
    // We opt for a tradeoff below of just remapping the currently playing videos thumbnails to data url
    /*
    const queue = structuredClone(state.queue);
    // Check if offline then convert the thumbnail url by requesting it
    const networkStatusManager = window["yt"]?.networkStatusManager?.instance?.sharedNetworkStatusManager;
    if (networkStatusManager && !networkStatusManager.isNetworkAvailable()) {
      for (const item of queue.items) {
        let playlistPanelVideoRenderer;
        if (item.playlistPanelVideoRenderer) {
          playlistPanelVideoRenderer = item.playlistPanelVideoRenderer;
        } else if (item.playlistPanelVideoWrapperRenderer) {
          playlistPanelVideoRenderer = item.playlistPanelVideoWrapperRenderer.primaryRenderer.playlistPanelVideoRenderer
        }
        await remapThumbnailsToDataUrl(playlistPanelVideoRenderer.thumbnail?.thumbnails ?? []);
      }
    }
    */

    playerStateApi.postMessage("updateFromStore", state.queue, likeStatus, volume, muted, adPlaying);
  }

  async function sendVideoData() {
    const state = ytmStore.getState();
    const videoDetails = structuredClone(playerApi.getPlayerResponse().videoDetails);
    const playlistId = playerApi.getPlaylistId();
    let album = null;
    let hasFullMetadata = false;

    const selectedItemIndex = state.queue.items.findIndex(item => {
      return item.playlistPanelVideoRenderer?.selected ?? item.playlistPanelVideoWrapperRenderer.primaryRenderer.playlistPanelVideoRenderer.selected;
    });
    let currentItem = state.queue.items[selectedItemIndex];
    currentItem = currentItem?.playlistPanelVideoRenderer ?? currentItem?.playlistPanelVideoWrapperRenderer?.primaryRenderer?.playlistPanelVideoRenderer;
    if (currentItem !== null && currentItem !== undefined) {
      hasFullMetadata = true;

      // Fill out video details with better information
      videoDetails.title = currentItem.title.runs.map(v => v.text).join(""); // Can contain featuring text which isn't in player response
      videoDetails.thumbnail = structuredClone(currentItem.thumbnail); // Can contain more thumbnails than player response

      // Maybe obtain the album information from a text run
      for (let i = 0; i < currentItem.longBylineText.runs.length; i++) {
        const item = currentItem.longBylineText.runs[i];
        if (item.navigationEndpoint) {
          if (
            item.navigationEndpoint.browseEndpoint.browseEndpointContextSupportedConfigs.browseEndpointContextMusicConfig.pageType === "MUSIC_PAGE_TYPE_ALBUM"
          ) {
            album = {
              id: item.navigationEndpoint.browseEndpoint.browseId,
              text: item.text
            };
          }
        }
      }
    }

    const defaultLikeStatus = getLikeStatus() ?? "UNKNOWN";
    const storeLikeStatus = state.likeStatus.videos[videoDetails.videoId];

    const likeStatus = storeLikeStatus ? state.likeStatus.videos[videoDetails.videoId] : defaultLikeStatus;

    // Check if offline then convert the thumbnail url to data url if possible
    //
    // This provides a quality of life where at least when something is currently playing and we're offline the media service
    // or anything that may present the currently playing media thumbnail has something tangible to work with
    const networkStatusManager = window["yt"]?.networkStatusManager?.instance?.sharedNetworkStatusManager;
    if (networkStatusManager && !networkStatusManager.isNetworkAvailable()) {
      await remapThumbnailsToDataUrl(videoDetails.thumbnail?.thumbnails ?? []);
    }

    playerStateApi.postMessage("updateVideoDetails", videoDetails, playlistId, album, likeStatus, hasFullMetadata);
  }

  function hydrateApplicationState() {
    sendStoreState();

    const progressState = playerApi.getProgressState();
    playerStateApi.postMessage("updateVideoProgress", progressState.current);

    const videoState = playerApi.getPlayerState();
    playerStateApi.postMessage("updateVideoState", videoState);

    sendVideoData();
  }

  // It is mandatory that try catch be used within these so that we handle errors for YTMD and not throw them back to YTM
  playerApi.addEventListener("onVideoProgress", progress => {
    try {
      playerStateApi.postMessage("updateVideoProgress", progress);
    } catch (err) {
      console.error("[ytmd-err(playerApi.onVideoProgress)]", err);
    }
  });
  playerApi.addEventListener("onStateChange", state => {
    try {
      playerStateApi.postMessage("updateVideoState", state);
    } catch (err) {
      console.error("[ytmd-err(playerApi.onStateChange)]", err);
    }
  });
  playerApi.addEventListener("onVideoDataChange", event => {
    try {
      if (event.playertype === 1) {
        if (event.type === "newdata") {
          playerStateApi.postMessage("playerSignal", "NEW_PLAYBACK");
        }

        if (event.type === "dataloaded" || event.type === "dataupdated") {
          sendVideoData();
        }
      }
    } catch (err) {
      console.error("[ytmd-err(playerApi.onVideoDataChange)]", err);
    }
  });
  ytmStore.subscribe(() => {
    try {
      sendStoreState();
    } catch (err) {
      console.error("[ytmd-err(stateStore.subscribe)]", err);
    }
  });
  window.addEventListener("yt-action", (e) => {
    if (e.detail.actionName === "yt-service-request") {
      if (e.detail.args[1].createPlaylistServiceEndpoint) {
        const title = e.detail.args[2].create_playlist_title;
        const returnValue = e.detail.returnValue;
        returnValue[0].ajaxPromise.then(response => {
          const id = response.data.playlistId;

          playerStateApi.postMessage("playlistCreated", {
            title,
            id
          });
        });
      }
    } else if (e.detail.actionName === "yt-handle-playlist-deletion-command") {
      const playlistId = e.detail.args[0].handlePlaylistDeletionCommand.playlistId;
      playerStateApi.postMessage("playlistDeleted", playlistId);
    }
  });

  try {
    hydrateApplicationState();
  } catch {
    /* empty */
  }
}

async function waitForYTMPlayerApiReady() {
  const playerApi = ytmhook.ytmPlayerController.playerApi;

  await new Promise(resolve => {
    const interval = setInterval(async () => {
      const playerApiReady = playerApi.isReady();

      if (playerApiReady) {
        clearInterval(interval);
        resolve();
      }
    }, 250);
  });
}


// ==== setup/styles.ts
function createStyleSheet() {
  const css = document.createElement("style");
  css.appendChild(
    document.createTextNode(`
      .ytmd-history-back {
        margin: 0 2px 0 18px;
      }

      .ytmd-history-forward {
        margin: 0 18px 0 2px;
      }

      .ytmd-history-back, .ytmd-history-forward {
        cursor: pointer;
        font-size: 24px;
        color: rgba(255, 255, 255, 0.5);
      }

      .ytmd-history-back.pivotbar, .ytmd-history-forward.pivotbar {
        padding-top: 12px;
      }

      .ytmd-history-back.disabled, .ytmd-history-forward.disabled {
        cursor: not-allowed;
      }

      .ytmd-history-back:hover:not(.disabled), .ytmd-history-forward:hover:not(.disabled) {
        color: #FFFFFF;
      }

      .ytmd-hidden {
        display: none;
      }

      .ytmd-persist-volume-slider {
        opacity: 1 !important;
        pointer-events: initial !important;
      }
      
      .ytmd-player-bar-control.library-button {
        margin-left: 8px;
      }

      .ytmd-player-bar-control.library-button.hidden {
        display: none;
      }

      .ytmd-player-bar-control.playlist-button {
        margin-left: 8px;
      }

      .ytmd-player-bar-control.playlist-button.hidden {
        display: none;
      }

      .ytmd-player-bar-control.sleep-timer-button.active {
        color: #FFFFFF;
      }

      .ytmd-lyrics {
        margin-top: 16px;
      }

      .ytmd-lyrics .ytmd-lyric-line {
        font-size: 24px;
        color: rgba(255, 255, 255, 0.7);
        cursor: pointer;
      }
        
      .ytmd-lyrics .ytmd-lyric-line.active {
        color: rgba(255, 255, 255, 1);
      }

      .ytmd-lyrics .ytmd-lyric-line {
        margin-bottom: 16px;
      }

      .ytmd-lyrics .ytmd-lyric-line:last-child {
        margin-bottom: 24px;
      }

      .ytmd-lyrics-source {
        font-size: 14px;
        color: rgba(255, 255, 255, 0.7);
        margin-bottom: 16px;
      }

      .ytmd-lyrics-ytmdnote {
        font-size: 14px;
        color: rgba(255, 255, 255, 0.7);
        margin-bottom: 128px;
      }

      /* This makes sure the return live button is positioned properly */
      .ytmusic-tab-renderer[page-type='MUSIC_PAGE_TYPE_TRACK_LYRICS'] > #contents {
        position: relative;
      }

      .ytmd-lyrics-return-live-container {
        position: sticky;
        bottom: 24px;
        display: flex;
        justify-content: center;
        align-items: center;
      }
    `)
  );
  document.head.appendChild(css);
}

function createMaterialSymbolsLink() {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = "https://fonts.googleapis.com/css2?family=Material+Symbols+Outlined:opsz,wght,FILL,GRAD@24,100,0,0";
  return link;
}



// ==== setup/remote.ts

function initRemote() {
  const remoteApi = protectedapimanager.createOrGetAPI("RemoteControl");

  remoteApi.addEventListener("execute", (event) => {
    const args = event.detail.args;
    const command = args.shift();

    const ytmStore = ytmhook.ytmStateStore.store;
    const playerApi = ytmhook.ytmPlayerController.playerApi;

    switch (command) {
      case "playPause": {
        // 1 = Playing
        if (playerApi.getPlayerState() === 1) {
          playerApi.pauseVideo();
        } else {
          playerApi.playVideo();
        }

        break;
      }

      case "play": {
        playerApi.playVideo();

        break;
      }

      case "pause": {
        playerApi.pauseVideo();

        break;
      }

      case "next": {
        playerApi.nextVideo();

        break;
      }

      case "previous": {
        playerApi.previousVideo();

        break;
      }

      case "toggleLike": {
        const state = ytmStore.getState();
        const videoId = playerApi.getPlayerResponse().videoDetails.videoId;

        let serviceEndpoint = null;
        let toggledServiceEndpoint = null;

        let defaultLikeStatus = "INDIFFERENT";
        if (state.playerPage.playerOverlay.playerOverlayRenderer.actions) {
          const buttonData =
            state.playerPage.playerOverlay.playerOverlayRenderer.actions[0];
          defaultLikeStatus = buttonData.likeButtonRenderer.likeStatus;

          for (const endpoint of buttonData.likeButtonRenderer
            .serviceEndpoints) {
            if (endpoint.likeEndpoint.status === "LIKE") {
              serviceEndpoint = endpoint;
            } else if (endpoint.likeEndpoint.status === "INDIFFERENT") {
              toggledServiceEndpoint = endpoint;
            }
          }
        } else if (
          state.playerPage.playerOverlay.playerOverlayRenderer.videoActionBar
        ) {
          // New player bar
          const actionBar =
            state.playerPage.playerOverlay.playerOverlayRenderer.videoActionBar;
          const viewModel =
            actionBar.videoActionBarViewModel.buttons[0].buttonViewModel
              .segmentedLikeDislikeButtonViewModel;
          defaultLikeStatus =
            viewModel.likeButtonViewModel.likeButtonViewModel.likeStatusEntity
              .likeStatus;

          const toggleButtonViewModel =
            viewModel.likeButtonViewModel.likeButtonViewModel
              .toggleButtonViewModel.toggleButtonViewModel;
          serviceEndpoint =
            toggleButtonViewModel.defaultButtonViewModel.buttonViewModel.onTap
              .innertubeCommand;
          toggledServiceEndpoint =
            toggleButtonViewModel.toggledButtonViewModel.buttonViewModel.onTap
              .innertubeCommand;
        }

        const storeLikeStatus = state.likeStatus.videos[videoId];
        const likeStatus = storeLikeStatus
          ? state.likeStatus.videos[videoId]
          : defaultLikeStatus;

        let serviceEvent = null;

        const ytmusicAppElement = document.querySelector("ytmusic-app");
        if (likeStatus === "LIKE") {
          serviceEvent = {
            bubbles: true,
            cancelable: false,
            composed: true,
            detail: {
              actionName: "yt-service-request",
              args: [ytmusicAppElement, toggledServiceEndpoint],
              optionalAction: false,
              returnValue: [],
            },
          };
        } else {
          serviceEvent = {
            bubbles: true,
            cancelable: false,
            composed: true,
            detail: {
              actionName: "yt-service-request",
              args: [ytmusicAppElement, serviceEndpoint],
              optionalAction: false,
              returnValue: [],
            },
          };
        }

        if (serviceEvent)
          ytmusicAppElement?.dispatchEvent(
            new CustomEvent("yt-action", serviceEvent),
          );

        break;
      }

      case "toggleDislike": {
        const state = ytmStore.getState();
        const videoId = playerApi.getPlayerResponse().videoDetails.videoId;

        let serviceEndpoint = null;
        let toggledServiceEndpoint = null;

        let defaultLikeStatus = "INDIFFERENT";
        if (state.playerPage.playerOverlay.playerOverlayRenderer.actions) {
          const buttonData =
            state.playerPage.playerOverlay.playerOverlayRenderer.actions[0];
          defaultLikeStatus = buttonData.likeButtonRenderer.likeStatus;

          for (const endpoint of buttonData.likeButtonRenderer
            .serviceEndpoints) {
            if (endpoint.likeEndpoint.status === "DISLIKE") {
              serviceEndpoint = endpoint;
            } else if (endpoint.likeEndpoint.status === "INDIFFERENT") {
              toggledServiceEndpoint = endpoint;
            }
          }
        } else if (
          state.playerPage.playerOverlay.playerOverlayRenderer.videoActionBar
        ) {
          // New player bar
          const actionBar =
            state.playerPage.playerOverlay.playerOverlayRenderer.videoActionBar;
          const viewModel =
            actionBar.videoActionBarViewModel.buttons[0].buttonViewModel
              .segmentedLikeDislikeButtonViewModel;
          defaultLikeStatus =
            viewModel.likeButtonViewModel.likeButtonViewModel.likeStatusEntity
              .likeStatus;

          const toggleButtonViewModel =
            viewModel.dislikeButtonViewModel.dislikeButtonViewModel
              .toggleButtonViewModel.toggleButtonViewModel;
          serviceEndpoint =
            toggleButtonViewModel.defaultButtonViewModel.buttonViewModel.onTap
              .innertubeCommand;
          toggledServiceEndpoint =
            toggleButtonViewModel.toggledButtonViewModel.buttonViewModel.onTap
              .innertubeCommand;
        }

        const storeLikeStatus = state.likeStatus.videos[videoId];
        const likeStatus = storeLikeStatus
          ? state.likeStatus.videos[videoId]
          : defaultLikeStatus;

        let serviceEvent = null;

        const ytmusicAppElement = document.querySelector("ytmusic-app");
        if (likeStatus === "DISLIKE") {
          serviceEvent = {
            bubbles: true,
            cancelable: false,
            composed: true,
            detail: {
              actionName: "yt-service-request",
              args: [ytmusicAppElement, toggledServiceEndpoint],
              optionalAction: false,
              returnValue: [],
            },
          };
        } else {
          serviceEvent = {
            bubbles: true,
            cancelable: false,
            composed: true,
            detail: {
              actionName: "yt-service-request",
              args: [ytmusicAppElement, serviceEndpoint],
              optionalAction: false,
              returnValue: [],
            },
          };
        }

        if (serviceEvent)
          ytmusicAppElement?.dispatchEvent(
            new CustomEvent("yt-action", serviceEvent),
          );

        break;
      }

      case "volumeUp": {
        const currentVolumeUp = playerApi.getVolume();

        let newVolumeUp = currentVolumeUp + 10;
        if (currentVolumeUp > 100) {
          newVolumeUp = 100;
        }

        playerApi.setVolume(newVolumeUp);
        ytmStore.dispatch({ type: "SET_VOLUME", payload: newVolumeUp });

        break;
      }

      case "volumeDown": {
        const currentVolumeDown = playerApi.getVolume();

        let newVolumeDown = currentVolumeDown - 10;
        if (currentVolumeDown < 0) {
          newVolumeDown = 0;
        }

        playerApi.setVolume(newVolumeDown);
        ytmStore.dispatch({ type: "SET_VOLUME", payload: newVolumeDown });

        break;
      }

      case "setVolume": {
        const valueInt = parseInt(args[0]);
        // Check if Volume is a number and between 0 and 100
        if (isNaN(valueInt) || valueInt < 0 || valueInt > 100) {
          return;
        }

        playerApi.setVolume(valueInt);
        ytmStore.dispatch({ type: "SET_VOLUME", payload: valueInt });

        break;
      }

      case "mute": {
        playerApi.mute();
        ytmStore.dispatch({ type: "SET_MUTED", payload: true });

        break;
      }

      case "unmute": {
        playerApi.unMute();
        ytmStore.dispatch({ type: "SET_MUTED", payload: false });

        break;
      }

      case "toggleMute": {
        const isMuted = playerApi.isMuted();
        if (isMuted) {
          playerApi.unMute();
          ytmStore.dispatch({ type: "SET_MUTED", payload: false });
        } else {
          playerApi.mute();
          ytmStore.dispatch({ type: "SET_MUTED", payload: true });
        }

        break;
      }

      case "repeatMode": {
        ytmStore.dispatch({ type: "SET_REPEAT", payload: args[0] });

        break;
      }

      case "cycleRepeatMode": {
        const currentRepeatMode = ytmStore.getState().queue.repeatMode;

        if (currentRepeatMode === "NONE") {
          ytmStore.dispatch({ type: "SET_REPEAT", payload: "ALL" });
        }

        if (currentRepeatMode === "ALL") {
          ytmStore.dispatch({ type: "SET_REPEAT", payload: "ONE" });
        }

        if (currentRepeatMode === "ONE") {
          ytmStore.dispatch({ type: "SET_REPEAT", payload: "NONE" });
        }

        break;
      }

      case "seekTo": {
        playerApi.seekTo(args[0]);

        break;
      }

      case "shuffle": {
        ytmhook.ytmPlayerController.queue.shuffle();

        break;
      }

      case "playQueueIndex": {
        let index = args[0];

        const state = ytmStore.getState();
        const queue = state.queue;

        const maxQueueIndex = state.queue.items.length - 1;
        //const maxAutoMixQueueIndex = Math.max(state.queue.automixItems.length - 1, 0);

        let useAutoMix = false;
        if (index > maxQueueIndex) {
          index = index - state.queue.items.length;
          useAutoMix = true;
        }

        let song = null;
        if (!useAutoMix) {
          song = queue.items[index];
        } else {
          song = queue.automixItems[index];
        }

        if (song) {
          let playlistPanelVideoRenderer;
          if (song.playlistPanelVideoRenderer) {
            playlistPanelVideoRenderer = song.playlistPanelVideoRenderer;
          } else if (song.playlistPanelVideoWrapperRenderer) {
            playlistPanelVideoRenderer =
              song.playlistPanelVideoWrapperRenderer.primaryRenderer
                .playlistPanelVideoRenderer;
          }

          document.dispatchEvent(
            new CustomEvent("yt-navigate", {
              detail: {
                endpoint: {
                  watchEndpoint:
                    playlistPanelVideoRenderer.navigationEndpoint.watchEndpoint,
                },
              },
            }),
          );
        }

        break;
      }

      case "navigate": {
        const endpoint = args[0];
        document.dispatchEvent(
          new CustomEvent("yt-navigate", {
            detail: {
              endpoint,
            },
          }),
        );

        break;
      }

      case "queueAdd": {
        const videoId = args[0].videoId;
        const playlistId = args[0].playlistId;
        const index = args[0].index;

        const ytmusicAppElement = document.querySelector("ytmusic-app");
        const returnValue = [];
        const serviceRequestEvent = {
          bubbles: true,
          cancelable: false,
          composed: true,
          detail: {
            actionName: "yt-service-request",
            args: [
              ytmusicAppElement,
              {
                queueAddEndpoint: {
                  queueTarget: {
                    videoId: videoId,
                    playlistId: playlistId,
                  },
                },
              },
            ],
            optionalAction: false,
            returnValue,
          },
        };
        ytmusicAppElement?.dispatchEvent(
          new CustomEvent("yt-action", serviceRequestEvent),
        );
        returnValue[0].ajaxPromise.then(
          (response) => {
            const items = response.data.queueDatas.map((data) => data.content);
            ytmStore.dispatch({
              type: "ADD_ITEMS",
              payload: {
                nextQueueItemId: ytmStore.getState().queue.nextQueueItemId,
                index,
                items,
                shuffleEnabled: ytmStore.getState().queue.shuffleEnabled,
                shouldAssignIds: true,
              },
            });
          },
          () => {},
        );

        break;
      }

      case "queueRemove": {
        const index = parseInt(args[0]);

        ytmStore.dispatch({
          type: "REMOVE_ITEM",
          payload: index,
        });
        break;
      }

      case "queueMove": {
        const fromIndex = parseInt(args[0].fromIndex);
        const toIndex = parseInt(args[0].toIndex);

        ytmStore.dispatch({
          type: "MOVE_ITEM",
          payload: {
            fromIndex,
            toIndex,
          },
        });
        break;
      }
    }
  });

  remoteApi.handleMessage("getPlaylists", () => {
    const playerApi = ytmhook.ytmPlayerController.playerApi;

    return new Promise((resolve, reject) => {
      const ytmusicAppElement = document.querySelector("ytmusic-app");
      const returnValue = [];
      const serviceRequestEvent = {
        bubbles: true,
        cancelable: false,
        composed: true,
        detail: {
          actionName: "yt-service-request",
          args: [
            ytmusicAppElement,
            {
              addToPlaylistEndpoint: {
                videoId: playerApi.getPlayerResponse().videoDetails.videoId,
              },
            },
          ],
          optionalAction: false,
          returnValue,
        },
      };
      ytmusicAppElement?.dispatchEvent(
        new CustomEvent("yt-action", serviceRequestEvent),
      );
      returnValue[0].ajaxPromise.then(
        (response) => {
          try {
            const rawPlaylists =
              response.data.contents[0].addToPlaylistRenderer.playlists;
            const playlists = [];
            if (rawPlaylists) {
              for (const rawPlaylist of rawPlaylists) {
                const playlist = rawPlaylist.playlistAddToOptionRenderer;
                playlists.push({
                  id: playlist.playlistId,
                  title: getYTMTextRun(playlist.title.runs),
                });
              }
            }
            resolve([playlists]);
          } catch (err) {
            console.error("[ytmd-err(getPlaylists)]", err);
            resolve([[]]);
          }
        },
        () => {
          reject();
        },
      );
    });
  });
}


// ==== setup/extras.ts

function overrideHistoryButtonDisplay() {
  const historyButton = document.querySelector(
    "#history-link .history-button",
  );
  if (historyButton) historyButton.style = "display: inline-block !important;";
  else
    console.warn(
      "Could not override history button display. Element not found",
    );
}

async function hideChromecastButton() {
  const ytmStore = ytmhook.ytmStateStore.store;

  ytmStore.dispatch({ type: "SET_CAST_AVAILABLE", payload: false });
  // YTM has independent code which sets the cast available so we're subscribing to ensure it's overriden if they do later call it
  ytmStore.subscribe(() => {
    try {
      const ytmStore = ytmhook.ytmStateStore.store;
      if (ytmStore.getState().castStatus.castAvailable) {
        ytmStore.dispatch({ type: "SET_CAST_AVAILABLE", payload: false });
      }
    } catch (err) {
      console.error("[ytmd-err(hideChromecastButton)]", err);
    }
  });
}

async function createAdditionalPlayerBarControls() {
  const ytmStore = ytmhook.ytmStateStore.store;
  const playerApi = ytmhook.ytmPlayerController.playerApi;

  let currentVideoId = "";

  let libraryFeedbackDefaultToken = "";
  let libraryFeedbackToggledToken = "";

  let sleepTimerTimeout = null;

  const libraryButton = document.createElement("yt-button-shape");
  libraryButton.classList.add("ytmd-player-bar-control");
  libraryButton.classList.add("library-button");
  const libraryButtonData = {
    focused: false,
    iconPosition: "icon-only",
    onTap: function () {
      const closePopupEvent = {
        bubbles: true,
        cancelable: false,
        composed: true,
        detail: {
          actionName: "yt-close-popups-action",
          args: [["ytmusic-menu-popup-renderer"]],
          optionalAction: false,
          returnValue: [],
        },
      };
      const feedbackEvent = {
        bubbles: true,
        cancelable: false,
        composed: true,
        detail: {
          actionName: "yt-service-request",
          args: [
            this,
            {
              feedbackEndpoint: {
                feedbackToken: libraryButtonData.toggled
                  ? libraryFeedbackToggledToken
                  : libraryFeedbackDefaultToken,
              },
            },
          ],
          optionalAction: false,
          returnValue: [],
        },
      };
      this.dispatchEvent(new CustomEvent("yt-action", closePopupEvent));
      this.dispatchEvent(new CustomEvent("yt-action", feedbackEvent));
      ytmhook.ytmStateStore.store.dispatch({
        type: "SET_FEEDBACK_TOGGLE_STATE",
        payload: {
          defaultEndpointFeedbackToken: libraryFeedbackDefaultToken,
          isToggled: !libraryButtonData.toggled,
        },
      });
    }.bind(libraryButton),
    style: "mono",
    toggled: false,
    toggleable: true,
    type: "text",
  };
  libraryButton.rawProps = {
    iconName: "yt-sys-icons:library_add",
    data: libraryButtonData,
  };
  document
    .querySelector("ytmusic-app-layout>ytmusic-player-bar")
    ?.querySelector("ytmusic-like-button-renderer")
    ?.insertAdjacentElement("afterend", libraryButton);

  const playlistButton = document.createElement("yt-button-shape");
  playlistButton.classList.add("ytmd-player-bar-control");
  playlistButton.classList.add("playlist-button");
  const playlistButtonData = {
    focused: false,
    iconPosition: "icon-only",
    onTap: function () {
      const closePopupEvent = {
        bubbles: true,
        cancelable: false,
        composed: true,
        detail: {
          actionName: "yt-close-popups-action",
          args: [["ytmusic-menu-popup-renderer"]],
          optionalAction: false,
          returnValue: [],
        },
      };
      const returnValue = [];
      const serviceRequestEvent = {
        bubbles: true,
        cancelable: false,
        composed: true,
        detail: {
          actionName: "yt-service-request",
          args: [
            this,
            {
              addToPlaylistEndpoint: {
                videoId: currentVideoId,
              },
            },
          ],
          optionalAction: false,
          returnValue,
        },
      };
      this.dispatchEvent(new CustomEvent("yt-action", closePopupEvent));
      this.dispatchEvent(new CustomEvent("yt-action", serviceRequestEvent));
      returnValue[0].ajaxPromise.then(
        (response) => {
          const addToPlaylistEvent = {
            bubbles: true,
            cancelable: false,
            composed: true,
            detail: {
              actionName: "yt-open-popup-action",
              args: [
                {
                  openPopupAction: {
                    popup: {
                      addToPlaylistRenderer:
                        response.data.contents[0].addToPlaylistRenderer,
                    },
                    popupType: "DIALOG",
                  },
                },
                this,
              ],
              optionalAction: false,
              returnValue: [],
            },
          };
          this.dispatchEvent(new CustomEvent("yt-action", addToPlaylistEvent));
          this.dispatchEvent(new CustomEvent("yt-action", closePopupEvent));
        },
        () => {
          // service request errored
        },
        this,
      );
    }.bind(playlistButton),
    style: "mono",
    toggled: false,
    type: "text",
  };
  playlistButton.rawProps = {
    iconName: "yt-sys-icons:playlist_add",
    data: playlistButtonData,
  };
  libraryButton.insertAdjacentElement("afterend", playlistButton);

  playerApi.addEventListener("onVideoDataChange", (event) => {
    if (
      event.playertype === 1 &&
      (event.type === "dataloaded" || event.type === "dataupdated")
    ) {
      currentVideoId = playerApi.getPlayerResponse().videoDetails.videoId;
    }
  });

  const rightControls = document
    ?.querySelector("ytmusic-app-layout>ytmusic-player-bar")
    ?.querySelector(".right-controls-buttons");
  const sleepTimerButton = document.createElement("yt-icon-button");

  const sleepTimerIcon = document.createElement("yt-icon");
  sleepTimerIcon.set("icon", "TIMER");
  sleepTimerButton.appendChild(sleepTimerIcon);

  sleepTimerButton.setAttribute("title", "Sleep timer off");
  sleepTimerButton.classList.add("ytmusic-player-bar");
  sleepTimerButton.classList.add("ytmd-player-bar-control");
  sleepTimerButton.classList.add("sleep-timer-button");
  sleepTimerButton.onclick = () => {
    sleepTimerButton.dispatchEvent(
      new CustomEvent("yt-action", {
        bubbles: true,
        cancelable: false,
        composed: true,
        detail: {
          actionName: "yt-open-popup-action",
          args: [
            {
              openPopupAction: {
                popup: {
                  menuPopupRenderer: {
                    accessibilityData: {
                      label: "Action menu",
                    },
                    items: [
                      {
                        menuServiceItemRenderer: {
                          icon: {
                            iconType: "CLOCK",
                          },
                          serviceEndpoint: {
                            ytmdSleepTimerServiceEndpoint: {
                              time: 5,
                            },
                          },
                          text: {
                            runs: [
                              {
                                text: "5 minutes",
                              },
                            ],
                          },
                        },
                      },
                      {
                        menuServiceItemRenderer: {
                          icon: {
                            iconType: "CLOCK",
                          },
                          serviceEndpoint: {
                            ytmdSleepTimerServiceEndpoint: {
                              time: 10,
                            },
                          },
                          text: {
                            runs: [
                              {
                                text: "10 minutes",
                              },
                            ],
                          },
                        },
                      },
                      {
                        menuServiceItemRenderer: {
                          icon: {
                            iconType: "CLOCK",
                          },
                          serviceEndpoint: {
                            ytmdSleepTimerServiceEndpoint: {
                              time: 15,
                            },
                          },
                          text: {
                            runs: [
                              {
                                text: "15 minutes",
                              },
                            ],
                          },
                        },
                      },
                      {
                        menuServiceItemRenderer: {
                          icon: {
                            iconType: "CLOCK",
                          },
                          serviceEndpoint: {
                            ytmdSleepTimerServiceEndpoint: {
                              time: 30,
                            },
                          },
                          text: {
                            runs: [
                              {
                                text: "30 minutes",
                              },
                            ],
                          },
                        },
                      },
                      {
                        menuServiceItemRenderer: {
                          icon: {
                            iconType: "CLOCK",
                          },
                          serviceEndpoint: {
                            ytmdSleepTimerServiceEndpoint: {
                              time: 45,
                            },
                          },
                          text: {
                            runs: [
                              {
                                text: "45 minutes",
                              },
                            ],
                          },
                        },
                      },
                      {
                        menuServiceItemRenderer: {
                          icon: {
                            iconType: "CLOCK",
                          },
                          serviceEndpoint: {
                            ytmdSleepTimerServiceEndpoint: {
                              time: 60,
                            },
                          },
                          text: {
                            runs: [
                              {
                                text: "1 hour",
                              },
                            ],
                          },
                        },
                      },
                      sleepTimerTimeout !== null
                        ? {
                            menuServiceItemRenderer: {
                              icon: {
                                iconType: "DELETE",
                              },
                              serviceEndpoint: {
                                ytmdSleepTimerServiceEndpoint: {
                                  time: 0,
                                },
                              },
                              text: {
                                runs: [
                                  {
                                    text: "Clear sleep timer",
                                  },
                                ],
                              },
                            },
                          }
                        : {},
                    ],
                  },
                },
                popupType: "DROPDOWN",
              },
            },
            sleepTimerButton,
          ],
          optionalAction: false,
          returnValue: [],
        },
      }),
    );
  };
  rightControls
    ?.querySelector(".shuffle")
    ?.insertAdjacentElement("afterend", sleepTimerButton);

  const humanizeTime = (time) => {
    // This is just a hacked together function to provide a humanization for the sleep timer. It serves no purpose outside that and isn't some complicated humanizer
    if (time === 1) return `${time} minute`;
    if (time > 1 && time < 60) return `${time} minutes`;
    if (time >= 60 && time < 120) return `${time / 60} hour`;
    if (time >= 120) return `${time / 60} hours`;
  };

  window.addEventListener("yt-action", (e) => {
    if (e.detail.actionName === "yt-service-request") {
      if (e.detail.args[1].ytmdSleepTimerServiceEndpoint) {
        if (sleepTimerTimeout !== null) {
          clearTimeout(sleepTimerTimeout);
          sleepTimerTimeout = null;
          if (sleepTimerButton.classList.contains("active")) {
            sleepTimerButton.classList.remove("active");
            sleepTimerButton.setAttribute("title", "Sleep timer off");
          }
        }

        if (e.detail.args[1].ytmdSleepTimerServiceEndpoint.time > 0) {
          if (!sleepTimerButton.classList.contains("active")) {
            sleepTimerButton.classList.add("active");
            sleepTimerButton.setAttribute(
              "title",
              `Sleep timer ${humanizeTime(e.detail.args[1].ytmdSleepTimerServiceEndpoint.time)}`,
            );
          }

          document.body.dispatchEvent(
            new CustomEvent("yt-action", {
              bubbles: true,
              cancelable: false,
              composed: true,
              detail: {
                actionName: "yt-open-popup-action",
                args: [
                  // Endpoint details
                  {
                    openPopupAction: {
                      popup: {
                        notificationActionRenderer: {
                          responseText: {
                            runs: [
                              {
                                text: `Sleep timer set to ${humanizeTime(e.detail.args[1].ytmdSleepTimerServiceEndpoint.time)}`,
                              },
                            ],
                          },
                        },
                      },
                      popupType: "TOAST",
                      uniqueId: crypto.randomUUID(),
                    },
                  },
                  document.querySelector("ytmusic-app"),
                ],
                optionalAction: false,
                returnValue: [],
              },
            }),
          );

          sleepTimerTimeout = setTimeout(
            () => {
              sleepTimerTimeout = null;
              sleepTimerButton.classList.remove("active");
              sleepTimerButton.setAttribute("title", "Sleep timer off");

              if (
                (
                  document.querySelector(
                    "ytmusic-app-layout>ytmusic-player-bar",
                  )
                )?.playing
              ) {
                playerApi.pauseVideo();

                document.body.dispatchEvent(
                  new CustomEvent("yt-action", {
                    bubbles: true,
                    cancelable: false,
                    composed: true,
                    detail: {
                      actionName: "yt-open-popup-action",
                      args: [
                        {
                          openPopupAction: {
                            popup: {
                              dismissableDialogRenderer: {
                                title: {
                                  runs: [
                                    {
                                      text: "Music paused",
                                    },
                                  ],
                                },
                                dialogMessages: [
                                  {
                                    runs: [
                                      {
                                        text: "Sleep timer expired and your music has been paused",
                                      },
                                    ],
                                  },
                                ],
                              },
                            },
                            popupType: "DIALOG",
                          },
                        },
                        document.querySelector("ytmusic-app"),
                      ],
                      optionalAction: false,
                      returnValue: [],
                    },
                  }),
                );
              }
            },
            e.detail.args[1].ytmdSleepTimerServiceEndpoint.time * 1000 * 60,
          );
        } else {
          document.body.dispatchEvent(
            new CustomEvent("yt-action", {
              bubbles: true,
              cancelable: false,
              composed: true,
              detail: {
                actionName: "yt-open-popup-action",
                args: [
                  // Endpoint details
                  {
                    openPopupAction: {
                      popup: {
                        notificationActionRenderer: {
                          responseText: {
                            runs: [
                              {
                                text: `Sleep timer cleared`,
                              },
                            ],
                          },
                        },
                      },
                      popupType: "TOAST",
                      uniqueId: crypto.randomUUID(),
                    },
                  },
                  document.querySelector("ytmusic-app"),
                ],
                optionalAction: false,
                returnValue: [],
              },
            }),
          );
        }
      }
    }
  });

  ytmStore.subscribe(() => {
    const state = ytmStore.getState();

    // Update library button for current data
    const currentMenu = (
      document.querySelector("ytmusic-app-layout>ytmusic-player-bar")
    )?.getMenuRenderer();
    if (currentMenu) {
      if (playlistButton.classList.contains("hidden")) {
        playlistButton.classList.remove("hidden");
      }

      let foundLibraryButton = false;
      for (let i = 0; i < currentMenu.items.length; i++) {
        const item = currentMenu.items[i];
        if (item.toggleMenuServiceItemRenderer) {
          if (
            item.toggleMenuServiceItemRenderer.defaultIcon.iconType ===
              "LIBRARY_SAVED" ||
            item.toggleMenuServiceItemRenderer.defaultIcon.iconType ===
              "LIBRARY_ADD"
          ) {
            foundLibraryButton = true;
            libraryFeedbackDefaultToken =
              item.toggleMenuServiceItemRenderer.defaultServiceEndpoint
                .feedbackEndpoint.feedbackToken;
            libraryFeedbackToggledToken =
              item.toggleMenuServiceItemRenderer.toggledServiceEndpoint
                .feedbackEndpoint.feedbackToken;

            if (
              state.toggleStates.feedbackToggleStates[
                libraryFeedbackDefaultToken
              ] !== undefined &&
              state.toggleStates.feedbackToggleStates[
                libraryFeedbackDefaultToken
              ] !== null
            ) {
              libraryButtonData.toggled =
                state.toggleStates.feedbackToggleStates[
                  libraryFeedbackDefaultToken
                ];
              libraryButton.setters.data(libraryButtonData);
            } else {
              libraryButtonData.toggled = false;
              libraryButton.setters.data(libraryButtonData);
            }

            if (
              item.toggleMenuServiceItemRenderer.defaultIcon.iconType ===
              "LIBRARY_SAVED"
            ) {
              // Default value is saved to library (false == remove from library, true == add to library)
              if (libraryButtonData.toggled) {
                libraryButton.setters.iconName("yt-sys-icons:library_add");
              } else {
                libraryButton.setters.iconName("yt-sys-icons:library_saved");
              }
            } else if (
              item.toggleMenuServiceItemRenderer.defaultIcon.iconType ===
              "LIBRARY_ADD"
            ) {
              // Default value is add to library (false == add to library, true == remove from library)
              if (libraryButtonData.toggled) {
                libraryButton.setters.iconName("yt-sys-icons:library_saved");
              } else {
                libraryButton.setters.iconName("yt-sys-icons:library_add");
              }
            }
            break;
          }
        }
      }

      if (!foundLibraryButton) {
        if (!libraryButton.classList.contains("hidden")) {
          libraryButton.classList.add("hidden");
        }
      } else {
        if (libraryButton.classList.contains("hidden")) {
          libraryButton.classList.remove("hidden");
        }
      }
    } else {
      if (!libraryButton.classList.contains("hidden")) {
        libraryButton.classList.add("hidden");
      }
      if (!playlistButton.classList.contains("hidden")) {
        playlistButton.classList.add("hidden");
      }
    }
  });
}

async function addTimedLyrics() {
  const ytmStore = ytmhook.ytmStateStore.store;
  const playerApi = ytmhook.ytmPlayerController.playerApi;

  let currentLyricBrowseId = "";
  let currentTimedLyrics = null;
  let autoScrolling = false;
  let autoScrollPaused = false;
  let autoScrollRafId = null;
  let viewingLyricsTab = false;
  let ytmLyricTabContents = null;

  const timedLyricsContainer = document.createElement("div");
  timedLyricsContainer.classList.add("ytmd-lyrics");
  const timedLyricsSource = document.createElement("p");
  timedLyricsSource.classList.add("ytmd-lyrics-source");
  const timedLyricsYTMDNote = document.createElement("p");
  timedLyricsYTMDNote.classList.add("ytmd-lyrics-ytmdnote");
  timedLyricsYTMDNote.innerText = "Timed lyrics system provided by YTMDesktop";

  const returnToLiveContainer = document.createElement("div");
  returnToLiveContainer.classList.add("ytmd-lyrics-return-live-container");
  const returnToLive = document.createElement("yt-button-renderer");
  returnToLive.classList.add("ytmd-lyrics-return-live");
  returnToLive.data = {
    text: {
      runs: [
        {
          text: "Sync to video time",
        },
      ],
    },
    style: "STYLE_OVERLAY",
  };

  function enableAutoScroll() {
    autoScrollPaused = false;
    returnToLive.hidden = true;
  }

  function disableAutoScroll() {
    autoScrollPaused = true;
    returnToLive.hidden = false;
  }

  returnToLive.onClick = () => {
    enableAutoScroll();
    autoScrolling = true;
    timedLyricsContainer.querySelector(".active")?.scrollIntoView({
      behavior: "instant",
      block: "center",
      inline: "center",
    });
  };
  returnToLiveContainer.appendChild(returnToLive);

  const tabRenderer = document.querySelector("#player-page #tab-renderer");
  function onUserScroll() {
    if (!viewingLyricsTab) return;
    if (autoScrolling) {
      //cancelAnimationFrame(autoScrollRafId);
      autoScrolling = false;
    }
    disableAutoScroll();
  }
  tabRenderer?.addEventListener("wheel", onUserScroll, { passive: true });
  tabRenderer?.addEventListener("touchstart", onUserScroll, { passive: true });

  async function getTimedLyrics() {
    try {
      // This is an anonymous request and could be broken by Google at any time
      const browseRes = await fetch("/youtubei/v1/browse", {
        method: "POST",
        body: JSON.stringify({
          browseId: currentLyricBrowseId,
          context: {
            client: {
              clientName: "ANDROID_MUSIC",
              clientVersion: "7.12.5",
            },
          },
        }),
      });
      const json = await browseRes.json();

      // This is likely a timed lyrics response
      if (json.contents && json.contents.elementRenderer) {
        const timedLyrics =
          json.contents.elementRenderer.newElement.type.componentType.model
            .timedLyricsModel.lyricsData.timedLyricsData;
        const source =
          json.contents.elementRenderer.newElement.type.componentType.model
            .timedLyricsModel.lyricsData.sourceMessage;

        timedLyricsSource.innerText = source;

        const lyricElements = [];
        for (const lyric of timedLyrics) {
          const lyricElement = document.createElement("p");
          lyricElement.innerText = lyric.lyricLine;
          lyricElement.classList.add("ytmd-lyric-line");
          lyricElement.setAttribute(
            "data-start-ms",
            lyric.cueRange.startTimeMilliseconds,
          );
          lyricElement.setAttribute(
            "data-end-ms",
            lyric.cueRange.endTimeMilliseconds,
          );
          lyricElement.onclick = () => {
            enableAutoScroll();
            playerApi.seekTo(
              parseInt(lyric.cueRange.startTimeMilliseconds) / 1000,
            );
          };
          lyricElements.push(lyricElement);
        }
        timedLyricsContainer.replaceChildren(...lyricElements);

        currentTimedLyrics = {
          timedLyrics,
          source,
        };
      } else {
        currentTimedLyrics = null;
      }
    } catch {
      /* empty */
    }
  }

  function waitForElement(root, selector) {
    return new Promise((resolve) => {
      if (root.querySelector(selector)) {
        return resolve(root.querySelector(selector));
      }

      const observer = new MutationObserver(() => {
        if (root.querySelector(selector)) {
          observer.disconnect();
          resolve(root.querySelector(selector));
        }
      });

      observer.observe(root, {
        childList: true,
        subtree: true,
      });
    });
  }

  async function updateLyricsTab() {
    if (currentTimedLyrics) {
      const tabRenderer = document.querySelector("#player-page #tab-renderer");

      const contents = (await waitForElement(
        tabRenderer,
        ".ytmusic-tab-renderer[page-type='MUSIC_PAGE_TYPE_TRACK_LYRICS'] > #contents",
      ));
      if (!ytmLyricTabContents)
        ytmLyricTabContents = Array.from(contents.children);
      contents.replaceChildren(
        timedLyricsContainer,
        timedLyricsSource,
        timedLyricsYTMDNote,
        returnToLiveContainer,
      );
    }
  }

  function easeOutQuad(x) {
    return 1 - (1 - x) * (1 - x);
  }

  function smoothScrollToCenter(container, element, duration = 250, easing = easeOutQuad) {
    const containerRect = container.getBoundingClientRect();
    const elementRect = element.getBoundingClientRect();

    const targetScrollTop =
      container.scrollTop +
      (elementRect.top - containerRect.top) -
      (container.clientHeight / 2 - elementRect.height / 2);

    const start = container.scrollTop;
    const change = targetScrollTop - start;
    const startTime = performance.now();

    return new Promise((resolve, reject) => {
      let lastKnownRafId;
      function animate(now) {
        // If we're not auto scrolling (interruption) or the last known raf id is not the same (another scroll operation) we cancel this one
        if (!autoScrolling || lastKnownRafId != autoScrollRafId) {
          reject();
          return;
        }

        const t = Math.min(1, (now - startTime) / duration);
        container.scrollTop = start + change * easing(t);

        if (t < 1) {
          autoScrollRafId = lastKnownRafId = requestAnimationFrame(animate);
        } else {
          resolve();
        }
      }
      autoScrollRafId = lastKnownRafId = requestAnimationFrame(animate);
    });
  }

  // This could definitely be optimized far better
  async function updateLyricLines(progress) {
    const msProgress = progress * 1000;
    for (const lyric of timedLyricsContainer.children) {
      const lyricStart = parseInt(lyric.getAttribute("data-start-ms"));
      const lyricEnd = parseInt(lyric.getAttribute("data-end-ms"));

      if (msProgress >= lyricStart && msProgress < lyricEnd) {
        if (!lyric.classList.contains("active")) {
          lyric.classList.add("active");
          if (!autoScrollPaused && viewingLyricsTab) {
            autoScrolling = true;
            try {
              await smoothScrollToCenter(tabRenderer, lyric, 500);
            } catch {}
            autoScrolling = false;
          }
        }
      } else {
        if (lyric.classList.contains("active"))
          lyric.classList.remove("active");
      }
    }
  }

  ytmStore.subscribe(async () => {
    const state = ytmStore.getState();
    if (state.playerPage && state.playerPage.playerPageTabs) {
      let lyricsTab = -1;
      for (let i = 0; i < state.playerPage.playerPageTabs.length; i++) {
        const tab = state.playerPage.playerPageTabs[i];
        // Check if this is the Music Page Lyrics tab
        if (
          tab.tabRenderer?.endpoint?.browseEndpoint?.browseId.startsWith("MPLY")
        ) {
          lyricsTab = i;
          break;
        }
      }

      const lyricBrowseId =
        state.playerPage.playerPageTabs[lyricsTab].tabRenderer.endpoint
          .browseEndpoint.browseId;
      if (currentLyricBrowseId !== lyricBrowseId) {
        currentLyricBrowseId = lyricBrowseId;
        enableAutoScroll();
        await getTimedLyrics();
      }

      if (state.playerPage.playerPageTabSelectedIndex === lyricsTab) {
        viewingLyricsTab = true;
        if (
          state.player.playerResponse.videoDetails.musicVideoType !==
          "MUSIC_VIDEO_TYPE_OMV"
        ) {
          await updateLyricsTab();
          enableAutoScroll();
          autoScrolling = true;
          timedLyricsContainer.querySelector(".active")?.scrollIntoView({
            behavior: "instant",
            block: "center",
            inline: "center",
          });
        } else {
          if (currentTimedLyrics) {
            const contents = (await waitForElement(
              tabRenderer,
              ".ytmusic-tab-renderer[page-type='MUSIC_PAGE_TYPE_TRACK_LYRICS'] > #contents",
            ));
            if (ytmLyricTabContents)
              contents.replaceChildren(...ytmLyricTabContents);
          }
        }
      } else if (state.playerPage.playerPageTabSelectedIndex !== lyricsTab) {
        viewingLyricsTab = false;
        ytmLyricTabContents = null;
      }
    }
  });

  playerApi.addEventListener("onVideoProgress", (progress) => {
    updateLyricLines(progress);
  });
}


// ==== utils.ts
function getYTMTextRun(runs) {
  let final = "";
  for (const run of runs) {
    final += run.text;
  }
  return final;
}

  // ==== setup/navigation.ts (falls back to a history tracker when the Navigation API is unavailable)
  function createNavigationMenuArrows() {
    const historyBackElement = document.createElement("span");
    historyBackElement.classList.add("material-symbols-outlined", "ytmd-history-back", "disabled");
    historyBackElement.innerText = "west";
    const historyForwardElement = document.createElement("span");
    historyForwardElement.classList.add("material-symbols-outlined", "ytmd-history-forward", "disabled");
    historyForwardElement.innerText = "east";

    const nav = window.navigation;
    let index = 0;
    let maxIndex = 0;
    let pendingDirection = 0;
    const canGoBack = () => (nav && "canGoBack" in nav ? nav.canGoBack : index > 0);
    const canGoForward = () => (nav && "canGoForward" in nav ? nav.canGoForward : index < maxIndex);

    historyBackElement.addEventListener("click", function () {
      if (!historyBackElement.classList.contains("disabled")) {
        pendingDirection = -1;
        history.back();
      }
    });
    historyForwardElement.addEventListener("click", function () {
      if (!historyForwardElement.classList.contains("disabled")) {
        pendingDirection = 1;
        history.forward();
      }
    });

    function reconcileNavigationButtons() {
      historyBackElement.classList.toggle("disabled", !canGoBack());
      historyForwardElement.classList.toggle("disabled", !canGoForward());
    }

    if (nav && "canGoBack" in nav) {
      nav.addEventListener("navigate", () => reconcileNavigationButtons());
      nav.addEventListener("currententrychange", () => reconcileNavigationButtons());
    } else {
      const originalPush = history.pushState;
      history.pushState = function (...args) {
        const result = originalPush.apply(this, args);
        index++;
        maxIndex = index;
        reconcileNavigationButtons();
        return result;
      };
      window.addEventListener("popstate", () => {
        index = Math.max(0, Math.min(maxIndex, index + (pendingDirection || -1)));
        pendingDirection = 0;
        reconcileNavigationButtons();
      });
    }

    const pivotBar = document.querySelector("ytmusic-pivot-bar-renderer");
    if (!pivotBar) {
      // New YTM UI
      const leftContent = document.querySelector("ytmusic-nav-bar ytmusic-logo");
      leftContent?.after(historyBackElement, historyForwardElement);
    } else {
      historyForwardElement.classList.add("pivotbar");
      historyBackElement.classList.add("pivotbar");
      pivotBar.prepend(historyForwardElement);
      pivotBar.prepend(historyBackElement);
    }
    reconcileNavigationButtons();
  }

  // ==== index.ts
  const ytmhook = new YTMHook();
  protectedapimanager.init();
  ytmhook.init();

  window.addEventListener("load", async () => {
    let setupCompletions = 0;
    try {
      {
        await ytmhook.ready();
        window.__YTMD_HOOK__ = Object.freeze({
          ytmPlayerController: ytmhook.ytmPlayerController,
          ytmStateStore: ytmhook.ytmStateStore
        });
        setupCompletions |= YTMViewSetupCompletionFlags.Early;
      }
      {
        createStyleSheet();
        const materialSymbols = createMaterialSymbolsLink();
        let materialSymbolsLoaded = false;
        materialSymbols.onload = () => {
          materialSymbolsLoaded = true;
        };
        materialSymbols.onerror = () => {
          materialSymbolsLoaded = true;
        };
        document.head.appendChild(materialSymbols);
        await new Promise(resolve => {
          const interval = setInterval(() => {
            if (materialSymbolsLoaded) {
              clearInterval(interval);
              resolve();
            }
          }, 250);
        });
        setupCompletions |= YTMViewSetupCompletionFlags.Styles;
      }
      {
        createNavigationMenuArrows();
        setupCompletions |= YTMViewSetupCompletionFlags.Navigation;
      }
      {
        await waitForYTMPlayerApiReady();
        initPlayerStateStore();
        setupCompletions |= YTMViewSetupCompletionFlags.Hooks;
      }
      {
        initRemote();
        setupCompletions |= YTMViewSetupCompletionFlags.Remote;
      }
      {
        overrideHistoryButtonDisplay();
        hideChromecastButton();
        createAdditionalPlayerBarControls(); // Sort of broken as of new YTM player bar (10/03/26)
        addTimedLyrics();
        setupCompletions |= YTMViewSetupCompletionFlags.Extras;
      }
    } catch (error) {
      ipc.send("ytmView:errored", { name: error?.name, message: error?.message, stack: error?.stack ?? String(error) });
    } finally {
      ipc.send("ytmView:ready", setupCompletions);
    }
  });
})();
