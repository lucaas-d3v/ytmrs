// YtmRs: the only script running inside YouTube Music (page main world, top frame).
// It hooks the YTM player/state store, reports the player state and executes remote commands.
// Everything else (styles, navigation, context menu, lyrics view, sleep timer) is native.
(function () {
  "use strict";
  const handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.ytmd;
  if (!handler || window.__ytmdProtectedApi) return;
  const ipc = {
    send(channel, ...args) {
      handler.postMessage(JSON.stringify({ c: channel, a: args }));
    }
  };
  const YTMViewSetupCompletionFlags = { LocationNotApplicable: 1, Early: 2, Hooks: 16, Remote: 32, Extras: 64 };

  // The YouTube Music service worker (PWA install/offline shell) is useless in the app and
  // WebKit runs it in an extra web process: behave like a browser without service workers
  try {
    delete Navigator.prototype.serviceWorker;
  } catch (err) {
    console.warn("[ytmd] could not disable service workers", err);
  }

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

  let lastStoreState = null;
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
    

    if (
      lastStoreState &&
      lastStoreState.queue === state.queue &&
      lastStoreState.likeStatus === likeStatus &&
      lastStoreState.volume === volume &&
      lastStoreState.muted === muted &&
      lastStoreState.adPlaying === adPlaying
    ) {
      return;
    }
    lastStoreState = { queue: state.queue, likeStatus, volume, muted, adPlaying };

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

// ==== Timed lyrics (data only, rendered by the native lyrics window)
function reportTimedLyrics() {
  const ytmStore = ytmhook.ytmStateStore.store;
  let currentLyricBrowseId = "";

  async function getTimedLyrics(browseId) {
    try {
      // This is an anonymous request and could be broken by Google at any time
      const browseRes = await fetch("/youtubei/v1/browse", {
        method: "POST",
        body: JSON.stringify({
          browseId,
          context: {
            client: {
              clientName: "ANDROID_MUSIC",
              clientVersion: "7.12.5"
            }
          }
        })
      });
      const json = await browseRes.json();
      const lyricsData = json?.contents?.elementRenderer?.newElement?.type?.componentType?.model?.timedLyricsModel?.lyricsData;
      if (!lyricsData) return null;
      return {
        source: lyricsData.sourceMessage ?? "",
        lines: (lyricsData.timedLyricsData ?? []).map(lyric => ({
          text: lyric.lyricLine ?? "",
          start: parseInt(lyric.cueRange?.startTimeMilliseconds ?? "0"),
          end: parseInt(lyric.cueRange?.endTimeMilliseconds ?? "0")
        }))
      };
    } catch {
      return null;
    }
  }

  ytmStore.subscribe(async () => {
    try {
      const state = ytmStore.getState();
      const tabs = state.playerPage?.playerPageTabs;
      if (!tabs) return;
      const lyricsTab = tabs.find(tab => tab.tabRenderer?.endpoint?.browseEndpoint?.browseId?.startsWith("MPLY"));
      const browseId = lyricsTab?.tabRenderer.endpoint.browseEndpoint.browseId ?? "";
      if (browseId === currentLyricBrowseId) return;
      currentLyricBrowseId = browseId;
      const lyrics = browseId ? await getTimedLyrics(browseId) : null;
      // A newer track may have started while fetching
      if (browseId !== currentLyricBrowseId) return;
      const videoId = state.player?.playerResponse?.videoDetails?.videoId ?? null;
      ipc.send("ytmView:timedLyrics", lyrics ? { ...lyrics, browseId, videoId } : null);
    } catch (err) {
      console.error("[ytmd-err(reportTimedLyrics)]", err);
    }
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
      await waitForYTMPlayerApiReady();
      initPlayerStateStore();
      setupCompletions |= YTMViewSetupCompletionFlags.Hooks;
    }
    {
      initRemote();
      setupCompletions |= YTMViewSetupCompletionFlags.Remote;
    }
    {
      hideChromecastButton();
      reportTimedLyrics();
      setupCompletions |= YTMViewSetupCompletionFlags.Extras;
    }
  } catch (error) {
    ipc.send("ytmView:errored", { name: error?.name, message: error?.message, stack: error?.stack ?? String(error) });
  } finally {
    ipc.send("ytmView:ready", setupCompletions);
  }
});
})();
