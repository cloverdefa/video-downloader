#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

BIN="$HOME/.local/bin"
ICONS="$HOME/.local/share/icons/hicolor"
APPS="$HOME/.local/share/applications"

install -Dm755 video_downloader "$BIN/video-downloader"
install -Dm644 app-icon.svg "$ICONS/scalable/apps/video-downloader.svg"
install -Dm644 app-icon.png "$ICONS/256x256/apps/video-downloader.png"
mkdir -p "$APPS"
sed "s|@EXEC@|$BIN/video-downloader|" video-downloader.desktop >"$APPS/video-downloader.desktop"
chmod 644 "$APPS/video-downloader.desktop"

gtk-update-icon-cache -f -t "$ICONS" 2>/dev/null || true
update-desktop-database "$APPS" 2>/dev/null || true
echo "Installed."
