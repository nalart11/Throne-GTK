#!/usr/bin/env bash
# Remove the installation; settings and profiles in ~/.config/throne-gtk remain.
set -euo pipefail
prefix="${PREFIX:-$HOME/.local}"
rm -rf "$prefix/lib/throne-gtk"
rm -f "$prefix/bin/throne-gtk"
rm -f "$prefix/share/applications/dev.nalart.ThroneGtk.desktop"
rm -f "$prefix/share/icons/hicolor/scalable/apps/dev.nalart.ThroneGtk.svg"
rm -f "$prefix/share/icons/hicolor/symbolic/apps/dev.nalart.ThroneGtk-symbolic.svg"
command -v update-desktop-database >/dev/null && \
    update-desktop-database "$prefix/share/applications" || true
echo "Удалено. Профили и настройки остались в ~/.config/throne-gtk"
