#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app="$root/dist/Throne GTK.app"
pkg="$root/dist/throne-gtk-local-installer.pkg"
manual_plist="$root/resources/macos/dev.nalart.ThroneGtk.manual-helper.plist"
postinstall="$root/resources/macos/installer/postinstall"
components="$root/resources/macos/installer/components.plist"
export COPYFILE_DISABLE=1

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "PKG можно собрать только на macOS" >&2
    exit 1
fi
for tool in pkgbuild plutil ditto codesign; do
    if ! command -v "$tool" >/dev/null; then
        echo "Не найдена команда $tool" >&2
        exit 1
    fi
done
if [[ ! -d "$app" ]]; then
    echo "Нет $app — сначала выполните just dmg" >&2
    exit 1
fi

plutil -lint "$manual_plist"
plutil -lint "$components"
codesign --verify --deep --strict --verbose=2 "$app"

work="$(mktemp -d "${TMPDIR:-/tmp}/throne-gtk-pkg.XXXXXX")"
cleanup() {
    rm -rf "$work"
}
trap cleanup EXIT

payload="$work/payload"
pkg_scripts="$work/scripts"
mkdir -p "$payload/Applications" "$payload/Library/LaunchDaemons" "$pkg_scripts"
ditto --noextattr --noacl "$app" "$payload/Applications/Throne GTK.app"
install -m644 "$manual_plist" \
    "$payload/Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist"
install -m755 "$postinstall" "$pkg_scripts/postinstall"

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)"
[[ -n "$version" ]] || version="0.1.0"
rm -f "$pkg"
pkgbuild \
    --root "$payload" \
    --scripts "$pkg_scripts" \
    --component-plist "$components" \
    --ownership recommended \
    --identifier dev.nalart.ThroneGtk.local-installer \
    --version "$version" \
    --install-location / \
    "$pkg"

echo "Готово: $pkg"
echo "Установщик запросит пароль администратора один раз."
