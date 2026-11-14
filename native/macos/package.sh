#!/bin/bash
# Assemble/sign only. Never installs, registers, or executes the app/core/helper.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MAKE_DMG=0
case "${1:-}" in --dmg) MAKE_DMG=1;; --no-dmg|'') ;; *) echo 'Usage: package.sh [--dmg|--no-dmg]' >&2; exit 2;; esac
[[ "$(uname -s)" == Darwin ]] || { echo 'Packaging requires macOS' >&2; exit 1; }
BIN="${THRONE_GUI_PATH:-$ROOT/native/macos/.build/throne-gtk}"
CORE="${THRONE_CORE_PATH:-$ROOT/build/throne-gtk-core}"
[[ -f "$BIN" && -x "$BIN" ]] || { echo "Missing native executable: $BIN (run build.sh first)" >&2; exit 1; }
[[ -f "$CORE" && -x "$CORE" ]] || { echo "Missing existing core: $CORE (set THRONE_CORE_PATH; this script never builds it)" >&2; exit 1; }
OUT="${THRONE_PACKAGE_OUT:-$ROOT/native/macos/.build/package}"
RESOURCES="${THRONE_RESOURCES_PATH:-$ROOT/native/macos/Resources}"
[[ -d "$RESOURCES" ]] || { echo "Missing resources: $RESOURCES" >&2; exit 1; }
mkdir -p "$OUT"
WORK="$(mktemp -d "$OUT/run.XXXXXX")"
exec > >(tee "$WORK/package.log") 2>&1
APP="$WORK/Throne GTK.app"
mkdir -p "$APP/Contents/"{MacOS,Resources,Library/LaunchDaemons} "$WORK/AppIcon.iconset"
install -m755 "$BIN" "$APP/Contents/MacOS/throne-gtk"
install -m755 "$CORE" "$APP/Contents/MacOS/throne-gtk-core"
install -m644 "$ROOT/resources/macos/dev.nalart.ThroneGtk.helper.plist" "$APP/Contents/Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist"
cp -R "$RESOURCES/." "$APP/Contents/Resources/"
VERSION="${THRONE_VERSION:-0.1.0}"
python3 - "$ROOT/resources/macos/Info.plist.in" "$APP/Contents/Info.plist" "$VERSION" "$BIN" "$CORE" <<'PY'
import plistlib,re,subprocess,sys
with open(sys.argv[1],'rb') as f: p=plistlib.load(f)
for key in ('CFBundleVersion','CFBundleShortVersionString'): p[key]=sys.argv[3]
minimums=['13.0']
for binary in sys.argv[4:]:
    build=subprocess.check_output(['xcrun','vtool','-show-build',binary],text=True)
    versions=re.findall(r'\bminos\s+([0-9.]+)',build)
    if not versions: raise SystemExit(f'Cannot determine minimum macOS version: {binary}')
    minimums.extend(versions)
p['LSMinimumSystemVersion']=max(minimums,key=lambda v:tuple(map(int,v.split('.'))))
print('Bundle minimum macOS:',p['LSMinimumSystemVersion'])
assert p['CFBundleIdentifier']=='dev.nalart.ThroneGtk'
assert p['CFBundleExecutable']=='throne-gtk'
with open(sys.argv[2],'wb') as f: plistlib.dump(p,f)
PY
plutil -lint "$APP/Contents/Info.plist" "$APP/Contents/Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist"
ICON="${THRONE_ICON_PNG:-}"
if [[ -z "$ICON" ]]; then
  for candidate in "$ROOT/native/macos/Resources/AppIcon.png" "$ROOT/native/macos/Resources/Throne.png"; do
    if [[ -f "$candidate" ]]; then ICON="$candidate"; break; fi
  done
fi
[[ -f "$ICON" ]] || { echo 'Icon conversion failed; provide THRONE_ICON_PNG pointing at the existing logo PNG.' >&2; exit 1; }
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$ICON" --out "$WORK/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$ICON" --out "$WORK/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$WORK/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"
# Fail closed on third-party dependencies: no GTK/Homebrew bundling or rewriting.
python3 - "$APP/Contents/MacOS/throne-gtk" "$APP/Contents/MacOS/throne-gtk-core" <<'PY'
import subprocess,sys
for binary in sys.argv[1:]:
    output=subprocess.check_output(['otool','-L',binary],text=True)
    print(output)
    for line in output.splitlines()[1:]:
        dep=line.strip().split(' (')[0]
        if not dep.startswith(('/System/Library/','/usr/lib/')):
            raise SystemExit(f'Non-system dependency rejected: {dep} in {binary}')
    loads=subprocess.check_output(['otool','-l',binary],text=True)
    if '/opt/homebrew/' in loads or '/usr/local/' in loads:
        raise SystemExit(f'Homebrew/local load path rejected: {binary}')
PY
IDENTITY="${CODESIGN_IDENTITY:-${THRONE_CODESIGN_IDENTITY:--}}"
sign_args=(--force --sign "$IDENTITY")
if [[ "$IDENTITY" != '-' ]]; then sign_args+=(--timestamp --options runtime); fi
codesign "${sign_args[@]}" "$APP/Contents/MacOS/throne-gtk-core"
codesign "${sign_args[@]}" "$APP/Contents/MacOS/throne-gtk"
codesign "${sign_args[@]}" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
if [[ "$MAKE_DMG" == 1 ]]; then
  mkdir -p "$WORK/dmg-root"
  ditto "$APP" "$WORK/dmg-root/Throne GTK.app"
  ln -s /Applications "$WORK/dmg-root/Applications"
  DMG="$WORK/throne-gtk-macos.dmg"
  hdiutil create -volname 'Throne GTK' -srcfolder "$WORK/dmg-root" -format UDZO "$DMG"
  dmg_sign_args=(--force --sign "$IDENTITY")
  if [[ "$IDENTITY" != '-' ]]; then dmg_sign_args+=(--timestamp); fi
  codesign "${dmg_sign_args[@]}" "$DMG"
  hdiutil verify "$DMG"
  if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
  fi
  [[ ! -e "$OUT/throne-gtk-macos.dmg" ]] || mv "$OUT/throne-gtk-macos.dmg" "$WORK/previous.dmg"
  mv "$DMG" "$OUT/throne-gtk-macos.dmg"
fi
[[ -x "$APP/Contents/MacOS/throne-gtk" && -x "$APP/Contents/MacOS/throne-gtk-core" ]]
[[ ! -e "$OUT/Throne GTK.app" ]] || mv "$OUT/Throne GTK.app" "$WORK/previous.app"
mv "$APP" "$OUT/Throne GTK.app"
echo "Verified app: $OUT/Throne GTK.app"
echo 'No installer, helper registration, or executable launch performed.'
