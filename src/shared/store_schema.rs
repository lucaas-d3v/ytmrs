use serde_json::{json, Value};

/// Default values of the persistent config store (StoreSchema)
pub fn defaults(app_version: &str) -> Value {
    json!({
        "metadata": {
            "version": 1,
            "appVersion": app_version
        },
        "general": {
            "disableHardwareAcceleration": false,
            "hideToTrayOnClose": false,
            "showNotificationOnSongChange": false,
            "startOnBoot": false,
            "startMinimized": false
        },
        "appearance": {
            "alwaysShowVolumeSlider": false,
            "customCSSEnabled": false,
            "customCSSPath": null,
            "zoom": 100,
            "trayIconStyle": 0
        },
        "playback": {
            "continueWhereYouLeftOff": true,
            "continueWhereYouLeftOffPaused": true,
            "enableSpeakerFill": false,
            "progressInTaskbar": false,
            "ratioVolume": false
        },
        "integrations": {
            "companionServerEnabled": false,
            "companionServerAuthTokens": null,
            "companionServerCORSWildcardEnabled": true,
            "discordPresenceEnabled": false,
            "lastFMEnabled": false,
            "enhancedMediaServiceEnabled": true
        },
        "shortcuts": {
            "playPause": "",
            "next": "",
            "previous": "",
            "thumbsUp": "",
            "thumbsDown": "",
            "volumeUp": "",
            "volumeDown": "",
            "volumeMute": "",
            "toggleShuffle": "",
            "toggleRepeat": ""
        },
        "state": {
            "lastUrl": "https://music.youtube.com/",
            "lastPlaylistId": "",
            "lastVideoId": "",
            "windowBounds": null,
            "windowMaximized": false,
            "miniplayerWindowBounds": null
        },
        "lastfm": {
            "sessionKey": null,
            "scrobblePercent": 50
        },
        "developer": {
            "enableDevTools": false
        }
    })
}
