#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/build" "$work/dist"
for binary in throne-gtk throne-gtk-core; do
    printf '#!/bin/sh\nexit 99\n' > "$work/build/$binary"
    chmod 755 "$work/build/$binary"
done
printf 'fixture\n' > "$work/build/libcronet.so"
export THRONE_VERSION=0.2.0 THRONE_BUILD_DIR="$work/build" THRONE_PACKAGE_OUT="$work/dist"
bash "$root/scripts/package-linux.sh"
python3 - "$work/dist/throne-gtk-0.2.0-linux-x86_64.tar.gz" <<'PY'
import sys, tarfile
prefix = 'throne-gtk-0.2.0-linux-x86_64/'
expected = {
    'lib/throne-gtk/throne-gtk': 0o755,
    'lib/throne-gtk/throne-gtk-core': 0o755,
    'lib/throne-gtk/libcronet.so': 0o644,
    'share/icons/hicolor/scalable/apps/dev.nalart.ThroneGtk.svg': 0o644,
    'share/icons/hicolor/symbolic/apps/dev.nalart.ThroneGtk-symbolic.svg': 0o644,
    'share/applications/dev.nalart.ThroneGtk.desktop.in': 0o644,
}
with tarfile.open(sys.argv[1]) as archive:
    files = {}
    for member in archive.getmembers():
        assert not member.name.startswith('/') and '..' not in member.name.split('/')
        assert member.name == prefix[:-1] or member.name.startswith(prefix)
        assert member.isdir() or member.isfile()
        if member.isfile():
            files[member.name[len(prefix):]] = member.mode
    assert files == expected, files
    assert b'@EXEC@' in archive.extractfile(prefix + 'share/applications/dev.nalart.ThroneGtk.desktop.in').read()
PY
if THRONE_VERSION=../escape bash "$root/scripts/package-linux.sh"; then exit 1; fi
rm "$work/build/libcronet.so"
if bash "$root/scripts/package-linux.sh"; then exit 1; fi
ln -s throne-gtk "$work/build/libcronet.so"
if bash "$root/scripts/package-linux.sh"; then exit 1; fi
echo 'Linux packaging fixtures passed'
