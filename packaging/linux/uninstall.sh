#!/usr/bin/env bash
rm -f ~/.local/bin/video-downloader \
  ~/.local/share/applications/video-downloader.desktop \
  ~/.local/share/icons/hicolor/scalable/apps/video-downloader.svg \
  ~/.local/share/icons/hicolor/256x256/apps/video-downloader.png
update-desktop-database ~/.local/share/applications 2>/dev/null || true
