#!/bin/sh
# Installs YtmRs for the current user (~/.local). Run with PREFIX=/usr/local (as root) for a system wide install.
set -e
cd "$(dirname "$0")"
PREFIX="${PREFIX:-$HOME/.local}"
install -Dm755 ytmrs "$PREFIX/bin/ytmrs"
install -Dm644 ytmrs.desktop "$PREFIX/share/applications/app.ytmrs.YtmRs.desktop"
for icon in icons/*.png; do
  size="$(basename "$icon" .png)"
  install -Dm644 "$icon" "$PREFIX/share/icons/hicolor/$size/apps/ytmrs.png"
done
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" || true
echo "YtmRs installed to $PREFIX/bin/ytmrs"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: add $PREFIX/bin to your PATH" ;; esac
