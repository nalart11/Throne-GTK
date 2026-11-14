#!/usr/bin/env bash
# Use the native assembler, but publish only after full bundle validation.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export NOLURIX_GUI_PATH="${NOLURIX_GUI_PATH:-$root/native/macos/.build/nolurix}"
export NOLURIX_CORE_PATH="${NOLURIX_CORE_PATH:-$root/build/nolurix-core}"
export NOLURIX_RESOURCES_PATH="${NOLURIX_RESOURCES_PATH:-$root/native/macos/.build/Resources}"
out="${NOLURIX_PACKAGE_OUT:-$root/dist}"
export CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-${NOLURIX_CODESIGN_IDENTITY:--}}"
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
if [[ -z "${NOLURIX_VERSION:-}" ]]; then
    NOLURIX_VERSION="$(python3 - "$root/Cargo.toml" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
print(re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)[1])
PY
    )"
    export NOLURIX_VERSION
fi
mkdir -p "$out"
work="$(mktemp -d "$out/package.XXXXXX")"
# Keep uniquely scoped staging and native assembly logs for inspection.
NOLURIX_PACKAGE_OUT="$work" bash "$root/native/macos/package.sh" --no-dmg
app="$work/Nolurix.app"
python3 "$root/scripts/validate-macos-bundle.py" "$app"
codesign --verify --deep --strict --verbose=2 "$app"
mkdir -p "$work/dmg-root"
ditto "$app" "$work/dmg-root/Nolurix.app"
ln -s /Applications "$work/dmg-root/Applications"
dmg="$work/nolurix-macos.dmg"
hdiutil create -volname 'Nolurix' -srcfolder "$work/dmg-root" -format UDZO "$dmg"
if [[ "$identity" != - ]]; then
    codesign --force --sign "$identity" --timestamp "$dmg"
fi
hdiutil verify "$dmg"
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$dmg"
fi
[[ ! -e "$out/Nolurix.app" ]] || mv "$out/Nolurix.app" "$work/previous.app"
[[ ! -e "$out/nolurix-macos.dmg" ]] || mv "$out/nolurix-macos.dmg" "$work/previous.dmg"
mv "$app" "$out/Nolurix.app"
mv "$dmg" "$out/nolurix-macos.dmg"
echo "Готово: $out/nolurix-macos.dmg"
