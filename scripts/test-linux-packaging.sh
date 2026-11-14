#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/build" "$work/dist"
for binary in nolurix nolurix-core; do
    printf '#!/bin/sh\nexit 99\n' > "$work/build/$binary"
    chmod 755 "$work/build/$binary"
done
printf 'fixture\n' > "$work/build/libcronet.so"
export NOLURIX_VERSION=0.2.0 NOLURIX_BUILD_DIR="$work/build" NOLURIX_PACKAGE_OUT="$work/dist"
bash "$root/scripts/package-linux.sh"
python3 - "$work/dist/nolurix-0.2.0-linux-x86_64.tar.gz" <<'PY'
import sys, tarfile
prefix = 'nolurix-0.2.0-linux-x86_64/'
expected = {
    'lib/nolurix/nolurix': 0o755,
    'lib/nolurix/nolurix-core': 0o755,
    'lib/nolurix/libcronet.so': 0o644,
    'share/icons/hicolor/scalable/apps/dev.nalart.Nolurix.svg': 0o644,
    'share/icons/hicolor/symbolic/apps/dev.nalart.Nolurix-symbolic.svg': 0o644,
    'share/applications/dev.nalart.Nolurix.desktop.in': 0o644,
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
    assert b'@EXEC@' in archive.extractfile(prefix + 'share/applications/dev.nalart.Nolurix.desktop.in').read()
PY
if NOLURIX_VERSION=../escape bash "$root/scripts/package-linux.sh"; then exit 1; fi
rm "$work/build/libcronet.so"
if bash "$root/scripts/package-linux.sh"; then exit 1; fi
ln -s nolurix "$work/build/libcronet.so"
if bash "$root/scripts/package-linux.sh"; then exit 1; fi
python3 "$root/scripts/test-source-install.py"
echo 'Linux packaging fixtures passed'
