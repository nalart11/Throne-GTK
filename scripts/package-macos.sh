#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_name="Throne GTK"
app="$root/dist/${app_name}.app"
dmg="$root/dist/throne-gtk-macos.dmg"
identity="${CODESIGN_IDENTITY:--}"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "DMG можно собрать только на macOS" >&2
    exit 1
fi

for tool in brew dylibbundler rsvg-convert iconutil hdiutil codesign plutil otool file perl; do
    if ! command -v "$tool" >/dev/null; then
        echo "Не найдена команда $tool" >&2
        echo "Установите зависимости: brew install dylibbundler librsvg" >&2
        exit 1
    fi
done

for file in throne-gtk throne-gtk-core; do
    if [[ ! -f "$root/build/$file" ]]; then
        echo "Нет build/$file — сначала выполните RELEASE=1 just build" >&2
        exit 1
    fi
done

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)"
[[ -n "$version" ]] || version="0.1.0"

rm -rf "$app" "$dmg"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks" \
    "$app/Contents/Library/LaunchDaemons"
install -m755 "$root/build/throne-gtk" "$app/Contents/MacOS/throne-gtk"
install -m755 "$root/build/throne-gtk-core" "$app/Contents/MacOS/throne-gtk-core"
install -m644 "$root/resources/macos/dev.nalart.ThroneGtk.helper.plist" \
    "$app/Contents/Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist"
sed "s/@VERSION@/$version/g" "$root/resources/macos/Info.plist.in" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist"

icon_work="$(mktemp -d "${TMPDIR:-/tmp}/throne-gtk-icon.XXXXXX")"
iconset="$icon_work/AppIcon.iconset"
mkdir -p "$iconset"
stage="$(mktemp -d "${TMPDIR:-/tmp}/throne-gtk-dmg.XXXXXX")"
cleanup() {
    rm -rf "$icon_work" "$stage"
}
trap cleanup EXIT

svg="$root/resources/icons/dev.nalart.ThroneGtk.svg"
for spec in "16:16x16" "32:16x16@2x" "32:32x32" "64:32x32@2x" \
            "128:128x128" "256:128x128@2x" "256:256x256" \
            "512:256x256@2x" "512:512x512" "1024:512x512@2x"; do
    pixels="${spec%%:*}"
    name="${spec#*:}"
    rsvg-convert -w "$pixels" -h "$pixels" "$svg" -o "$iconset/icon_${name}.png"
done
if ! iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"; then
    # Some macOS releases reject a valid PNG iconset. ICNS is a simple chunked
    # container; build the same lossless PNG chunks directly as a fallback.
    echo "iconutil отклонил iconset, создаю ICNS напрямую" >&2
    perl -e '
        use strict;
        my $output = shift;
        my (@chunks, $total) = ((), 8);
        while (@ARGV) {
            my ($type, $path) = splice(@ARGV, 0, 2);
            open my $input, "<", $path or die "$path: $!\n";
            binmode $input;
            local $/;
            my $data = <$input>;
            my $chunk = $type . pack("N", length($data) + 8) . $data;
            push @chunks, $chunk;
            $total += length($chunk);
        }
        open my $result, ">", $output or die "$output: $!\n";
        binmode $result;
        print {$result} "icns", pack("N", $total), @chunks;
    ' "$app/Contents/Resources/AppIcon.icns" \
        icp4 "$iconset/icon_16x16.png" \
        ic11 "$iconset/icon_16x16@2x.png" \
        icp5 "$iconset/icon_32x32.png" \
        ic12 "$iconset/icon_32x32@2x.png" \
        ic07 "$iconset/icon_128x128.png" \
        ic13 "$iconset/icon_128x128@2x.png" \
        ic08 "$iconset/icon_256x256.png" \
        ic14 "$iconset/icon_256x256@2x.png" \
        ic09 "$iconset/icon_512x512.png" \
        ic10 "$iconset/icon_512x512@2x.png"
fi

brew_lib="$(brew --prefix)/lib"
dylibbundler -od -b \
    -x "$app/Contents/MacOS/throne-gtk" \
    -x "$app/Contents/MacOS/throne-gtk-core" \
    -d "$app/Contents/Frameworks" \
    -p "@executable_path/../Frameworks" \
    -s "$brew_lib"

unbundled=0
while IFS= read -r binary; do
    if ! file "$binary" | grep -q 'Mach-O'; then
        continue
    fi
    dependencies="$(otool -L "$binary" | tail -n +2 | awk '{print $1}' | \
        grep -E '^(/opt/homebrew|/usr/local/(Cellar|opt))/' || true)"
    if [[ -n "$dependencies" ]]; then
        echo "В bundle остались Homebrew-зависимости у $binary:" >&2
        echo "$dependencies" >&2
        unbundled=1
    fi
done < <(find "$app/Contents" -type f)
if [[ "$unbundled" -ne 0 ]]; then
    exit 1
fi

sign_args=(--force --sign "$identity")
if [[ "$identity" != "-" ]]; then
    sign_args+=(--timestamp --options runtime)
fi
while IFS= read -r -d '' library; do
    codesign "${sign_args[@]}" "$library"
done < <(find "$app/Contents/Frameworks" -type f -name '*.dylib' -print0)
codesign "${sign_args[@]}" "$app/Contents/MacOS/throne-gtk-core"
codesign "${sign_args[@]}" "$app/Contents/MacOS/throne-gtk"
codesign "${sign_args[@]}" "$app"
codesign --verify --deep --strict --verbose=2 "$app"

cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "$app_name" -srcfolder "$stage" -ov -format UDZO "$dmg"

if [[ "$identity" != "-" ]]; then
    codesign --force --sign "$identity" --timestamp "$dmg"
fi
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$dmg"
fi

echo "Готово: $dmg"
