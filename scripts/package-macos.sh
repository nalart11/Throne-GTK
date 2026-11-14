#!/usr/bin/env bash
# Shared native assembler: never rebuild or launch the inspected binaries.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export THRONE_GUI_PATH="${THRONE_GUI_PATH:-$root/build/throne-gtk}"
export THRONE_CORE_PATH="${THRONE_CORE_PATH:-$root/build/throne-gtk-core}"
export THRONE_RESOURCES_PATH="${THRONE_RESOURCES_PATH:-$root/build/Resources}"
export THRONE_PACKAGE_OUT="${THRONE_PACKAGE_OUT:-$root/dist}"
if [[ -z "${THRONE_VERSION:-}" ]]; then
    THRONE_VERSION="$(python3 - "$root/Cargo.toml" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
print(re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)[1])
PY
    )"
    export THRONE_VERSION
fi
exec bash "$root/native/macos/package.sh" --dmg
