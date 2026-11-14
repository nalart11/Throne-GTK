#!/usr/bin/env bash
# Stage only: never install, execute payloads, or grant capabilities.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="${THRONE_VERSION:?Set THRONE_VERSION to the release version (without v)}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Expected numeric MAJOR.MINOR.PATCH version' >&2; exit 1; }
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || { echo 'Packaging requires Linux x86_64' >&2; exit 1; }
build="${THRONE_BUILD_DIR:-$root/build}"
out="${THRONE_PACKAGE_OUT:-$root/dist}"
for binary in throne-gtk throne-gtk-core; do
    [[ -f "$build/$binary" && -x "$build/$binary" && ! -L "$build/$binary" ]] || { echo "Missing regular executable: $build/$binary" >&2; exit 1; }
done
[[ -f "$build/libcronet.so" && ! -L "$build/libcronet.so" ]] || { echo 'Missing regular libcronet.so (run just fetch-cronet)' >&2; exit 1; }
mkdir -p "$out"
work="$(mktemp -d "$out/linux-package.XXXXXX")"
trap 'rm -rf -- "$work"' EXIT
name="throne-gtk-${version}-linux-x86_64"
stage="$work/$name"
mkdir -p "$stage/lib/throne-gtk" "$stage/share/applications"
for binary in throne-gtk throne-gtk-core; do
    install -m755 "$build/$binary" "$stage/lib/throne-gtk/$binary"
done
install -m644 "$build/libcronet.so" "$stage/lib/throne-gtk/libcronet.so"
install -Dm644 "$root/resources/icons/dev.nalart.ThroneGtk.svg" "$stage/share/icons/hicolor/scalable/apps/dev.nalart.ThroneGtk.svg"
install -Dm644 "$root/resources/icons/dev.nalart.ThroneGtk-symbolic.svg" "$stage/share/icons/hicolor/symbolic/apps/dev.nalart.ThroneGtk-symbolic.svg"
# Keep the template: the installer must substitute its chosen absolute prefix.
install -m644 "$root/resources/throne-gtk.desktop" "$stage/share/applications/dev.nalart.ThroneGtk.desktop.in"
tar -czf "$work/$name.tar.gz" -C "$work" "$name"
mv "$work/$name.tar.gz" "$out/$name.tar.gz"
echo "$out/$name.tar.gz"
