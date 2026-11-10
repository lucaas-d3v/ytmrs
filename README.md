# YtmRs

YouTube Music desktop app for Linux written in Rust, based on [YTMDesktop](https://github.com/ytmdesktop/ytmdesktop) (Electron). The interface is native GTK; the only web content is the YouTube Music page, running in the system WebKitGTK instead of a bundled Chromium.

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

Needs: `webkit2gtk-4.1`, `gtk+-3.0`, `libsoup-3.0`, `dbus-1`, `openssl`, GStreamer plugins (good/bad/libav) and a StatusNotifier host for the tray.

Optional build-time variables: `LASTFM_API_KEY`, `LASTFM_API_SECRET`.

Runtime variables: `YTMD_USER_DATA_DIR` (alternative data dir), `YTMD_LOG_TRACE=1`, `YTMD_OPEN_DEVTOOLS=1`, `YTMD_SKIP_SYSTEM_INTEGRATION=1` (skips `ytmrs://` protocol registration).

## Memory

Almost all memory is the YouTube Music page itself. Low memory mode (Settings > General, on by default) disables the JavaScript JIT of the web process: about 445 MiB in total instead of about 630 MiB, for roughly twice the web process CPU time during playback.

## Layout

- `src/ui/`: native GTK windows (main window, settings, miniplayer, lyrics, companion authorization, changelog)
- `src/services/`: port of the original services (config store, player state, YouTube Music view, shortcuts, protocol)
- `src/integrations/`: companion server, Discord presence, Last.fm, MPRIS, notifications, custom CSS, volume ratio
- `web/ytm/scripts.js`: the single script injected into YouTube Music (player hooks and remote control)

Compatibility identifiers kept on purpose so existing companion apps work: API `/api/v1`, port 9863, socket `ytmdesktop-ipc`, mDNS `_ytmdesktop._tcp`.

## Known differences

- Linux only; there is no auto updater.
- "Speaker fill" has no WebKitGTK equivalent; global shortcuts need X11.
- The lyrics window highlights the current line only when YouTube provides timings.
