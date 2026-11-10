# Changelog

## 2.2.1

- Fixed: the YouTube Music page did not react to clicks (the hidden loading overlay swallowed them).

## 2.2.0

- The whole interface is now native GTK: main window, settings, miniplayer, lyrics, companion authorization and changelog. The only web content left is the YouTube Music page itself.
- **Low memory mode** (on by default, Settings > General): runs YouTube Music without the JavaScript JIT. About 30% less memory, more processor time while the page is busy.
- The YouTube Music service worker is disabled, which removes an extra web process.
- Native replacements for the in-page extras: back/forward/home buttons, sleep timer, lyrics window, context menu.
- Taskbar progress through the Unity launcher API, tray icon follows the system color scheme.
- Linux only. The auto updater and the Windows/macOS code paths were removed.
