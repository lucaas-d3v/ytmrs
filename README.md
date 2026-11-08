# YtmRs

YouTube Music desktop app written in Rust: a 1:1 port of [YTMDesktop](https://github.com/ytmdesktop/ytmdesktop) (Electron) that uses the operating system's native webview (WebKitGTK on Linux, WebView2 on Windows, WKWebView on macOS) instead of bundling Chromium. The original source lives at <https://github.com/ytmdesktop/ytmdesktop>.

## Install (Linux x86_64)

Download from [Releases](https://github.com/lucaas-d3v/ytmrs/releases):

- **Debian/Ubuntu:** `sudo apt install ./ytmrs_*_amd64.deb`
- **Any distro:** extract `ytmrs-*-linux-x86_64.tar.gz` and run `./install.sh` (installs to `~/.local`; use `sudo PREFIX=/usr/local ./install.sh` for system wide)

Runtime requirements: WebKitGTK 4.1, GTK 3 and GStreamer plugins (good, bad, libav) for audio playback.

## Build

```sh
cargo build --release          # binary: target/release/ytmrs
cargo test                     # unit tests
```

Linux needs: `webkit2gtk-4.1`, `gtk+-3.0`, `libsoup-3.0`, `dbus-1`, `openssl`, GStreamer plugins (good/bad/libav, for YouTube Music audio/video) and a StatusNotifier host for the tray.

Optional build-time variables (same as the original): `LASTFM_API_KEY`, `LASTFM_API_SECRET`, `YTMD_UPDATE_FEED_OWNER`, `YTMD_UPDATE_FEED_REPOSITORY`.

Runtime variables: `YTMD_USER_DATA_DIR` (alternative data dir), `YTMD_LOG_TRACE=1`, `YTMD_OPEN_DEVTOOLS=1`, `YTMD_SKIP_SYSTEM_INTEGRATION=1` (skips `ytmrs://` protocol registration).

## Architecture (original → port)

| Original (Electron/TS) | Port (Rust) |
|---|---|
| `ServiceHost` / `ServiceCollection` | `services/service_host.rs` (same dependency ordering and lifecycle) |
| `conf` ConfigStore + migrations | `services/config_store.rs` (same `config.json` schema, semver migrations, file watch) |
| `BrowserWindow` / `WebContentsView` | `services/window_manager.rs` (tao windows + wry child webviews) |
| preload + `contextBridge` | `web/shared/bridge.js` + `web/preload/*.js` (same `window.ytmd` APIs) |
| Vue windows | `web/windows/*` (plain HTML/CSS/JS, embedded in the binary) |
| MessagePort ProtectedAPI | `services/protected_api.rs` (same op codes over the IPC bridge) |
| fastify + socket.io companion server | axum + socketioxide, own rate limiter, mdns-sd, local IPC server |
| xosms | MPRIS2 over zbus (Linux) |
| Electron `safeStorage` | AES-256-GCM, key in the OS keyring |
| Tray, globalShortcut, dialogs | tray-icon (StatusNotifier), global-hotkey, GTK dialogs / rfd |

Compatibility identifiers kept on purpose so existing companion apps work: API `/api/v1`, port 9863, socket `ytmdesktop-ipc`, mDNS `_ytmdesktop._tcp`.

## Known differences

- Windows/macOS code paths are written but were not compiled or tested; Windows thumbnail toolbar buttons and media service (SMTC) are not implemented yet; macOS has no enhanced media service (the original disables it there too).
- "Speaker fill" has no WebKitGTK equivalent; spellcheck suggestions are not in the context menu.
- On Linux, frameless window resizing is done through edge handles injected into the webviews.
