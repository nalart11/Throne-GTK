#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${THRONE_PACKAGE_OUT:-$root/dist}"
app="$out/Throne GTK.app"
pkg="$out/throne-gtk-local-installer.pkg"
manual_plist="$root/resources/macos/dev.nalart.ThroneGtk.manual-helper.plist"
postinstall="$root/resources/macos/installer/postinstall"
components="$root/resources/macos/installer/components.plist"
export COPYFILE_DISABLE=1
export CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-${THRONE_CODESIGN_IDENTITY:--}}"
mode="${1:-}"
case "$mode" in
    --dev-manual) mode=manual ;;
    --release) mode=release ;;
    *) echo 'Usage: package-macos-installer.sh --dev-manual | --release' >&2; exit 1 ;;
esac
if [[ "$mode" = release && ( -z "${INSTALLER_SIGN_IDENTITY:-}" || -z "${NOTARY_PROFILE:-}" || "${CODESIGN_IDENTITY:--}" = - ) ]]; then
    echo 'Release requires CODESIGN_IDENTITY, INSTALLER_SIGN_IDENTITY and NOTARY_PROFILE' >&2
    exit 1
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "PKG можно собрать только на macOS" >&2
    exit 1
fi
for tool in pkgbuild plutil ditto codesign python3 clang; do
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
python3 "$root/scripts/validate-macos-bundle.py" "$app"
if [[ "$mode" = release ]]; then
    codesign -d -r- "$app" 2>&1 | grep -q 'anchor apple generic' || {
        echo 'Release requires a Developer ID signed app, not ad-hoc signing' >&2; exit 1;
    }
fi

mkdir -p "$out"
work="$(mktemp -d "$out/pkg.XXXXXX")"
# Retain uniquely scoped staging for inspection; never execute its postinstall.

payload="$work/payload"
pkg_scripts="$work/scripts"
base="$payload/Library/Application Support/Throne GTK"
mkdir -p "$base" "$pkg_scripts"
ditto --noextattr --noacl "$app" "$base/Throne GTK.app"
printf '%s\n' "$mode" > "$base/.throne-gtk-package"
if [[ "$mode" = manual ]]; then
    mkdir -p "$payload/Library/LaunchDaemons"
    install -m644 "$manual_plist" "$payload/Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist"
    install -m644 "$manual_plist" "$base/manual-helper.plist"
    touch "$base/.throne-gtk-manual"
fi
install -m755 "$root/resources/macos/installer/preinstall" "$pkg_scripts/preinstall"
clang -O2 -Wall -Wextra -I "$root/core/helpertrust" \
    "$root/scripts/check-macos-install-acl.c" -o "$pkg_scripts/check-install-acl"
acl_sign_args=(--force --sign "${CODESIGN_IDENTITY:--}")
if [[ "${CODESIGN_IDENTITY:--}" != - ]]; then
    acl_sign_args+=(--timestamp --options runtime)
fi
codesign "${acl_sign_args[@]}" "$pkg_scripts/check-install-acl"
codesign --verify --strict "$pkg_scripts/check-install-acl"
install -m755 "$pkg_scripts/check-install-acl" "$base/check-install-acl"
printf '%s\n' "$mode" > "$pkg_scripts/mode"
install -m755 "$postinstall" "$pkg_scripts/postinstall"
install -m755 "$root/scripts/uninstall-macos.sh" "$base/uninstall.sh"

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)"
[[ -n "$version" ]] || version="0.1.0"
# Bash 3.2 treats an empty array as unset under nounset. Positional parameters
# expand to zero arguments safely, including on the system macOS Bash.
set --
if [[ "$mode" = release ]]; then
    set -- --sign "$INSTALLER_SIGN_IDENTITY" --timestamp
fi
pkgbuild "$@" \
    --root "$payload" \
    --scripts "$pkg_scripts" \
    --component-plist "$components" \
    --ownership recommended \
    --identifier dev.nalart.ThroneGtk.local-installer \
    --version "$version" \
    --install-location / \
    "$work/throne-gtk-local-installer.pkg"
if [[ "$mode" = release ]]; then
    xcrun notarytool submit "$work/throne-gtk-local-installer.pkg" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$work/throne-gtk-local-installer.pkg"
fi
[[ ! -e "$pkg" ]] || mv "$pkg" "$work/previous.pkg"
mv "$work/throne-gtk-local-installer.pkg" "$pkg"

echo "Готово: $pkg"
echo "Установщик запросит пароль администратора один раз."
