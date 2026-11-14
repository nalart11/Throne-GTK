#!/usr/bin/env bash
# Use the native assembler, but publish only after full bundle validation.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export THRONE_GUI_PATH="${THRONE_GUI_PATH:-$root/build/throne-gtk}"
export THRONE_CORE_PATH="${THRONE_CORE_PATH:-$root/build/throne-gtk-core}"
export THRONE_RESOURCES_PATH="${THRONE_RESOURCES_PATH:-$root/build/Resources}"
out="${THRONE_PACKAGE_OUT:-$root/dist}"
export CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-${THRONE_CODESIGN_IDENTITY:--}}"
identity="$CODESIGN_IDENTITY"
if [[ "$(uname -s)" != Darwin ]]; then
    echo 'DMG можно собрать только на macOS' >&2
    exit 1
fi
if [[ -n "${NOTARY_PROFILE:-}" && "$identity" = - ]]; then
    echo 'Notarization requires a Developer ID signing identity' >&2
    exit 1
fi
for tool in python3 codesign hdiutil ditto file otool; do
    command -v "$tool" >/dev/null || { echo "Missing command: $tool" >&2; exit 1; }
done
if [[ -z "${THRONE_VERSION:-}" ]]; then
    THRONE_VERSION="$(python3 - "$root/Cargo.toml" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
print(re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)[1])
PY
    )"
    export THRONE_VERSION
fi
mkdir -p "$out"
work="$(mktemp -d "$out/package.XXXXXX")"
# Keep uniquely scoped staging and native assembly logs for inspection.
THRONE_PACKAGE_OUT="$work" bash "$root/native/macos/package.sh" --no-dmg
app="$work/Throne GTK.app"
python3 "$root/scripts/validate-macos-bundle.py" "$app"
codesign --verify --deep --strict --verbose=2 "$app"
mkdir -p "$work/dmg-root"
ditto "$app" "$work/dmg-root/Throne GTK.app"
ln -s /Applications "$work/dmg-root/Applications"
dmg="$work/throne-gtk-macos.dmg"
hdiutil create -volname 'Throne GTK' -srcfolder "$work/dmg-root" -format UDZO "$dmg"
if [[ "$identity" != - ]]; then
    codesign --force --sign "$identity" --timestamp "$dmg"
fi
hdiutil verify "$dmg"
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$dmg"
fi
[[ ! -e "$out/Throne GTK.app" ]] || mv "$out/Throne GTK.app" "$work/previous.app"
[[ ! -e "$out/throne-gtk-macos.dmg" ]] || mv "$out/throne-gtk-macos.dmg" "$work/previous.dmg"
mv "$app" "$out/Throne GTK.app"
mv "$dmg" "$out/throne-gtk-macos.dmg"
echo "Готово: $out/throne-gtk-macos.dmg"
