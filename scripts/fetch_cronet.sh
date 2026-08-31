#!/usr/bin/env bash
# Place libcronet.so next to the core: the naive outbound loads it on first
# use. It is not linked statically — the supplied archive
# libcronet.a contains objects for several architectures, and GNU ld
# refuses to read it.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
module_dir="$(go env GOMODCACHE)/github.com/parhelia512/cronet-go/lib"

library="$(find "$module_dir" -maxdepth 2 -name libcronet.so -path "*linux_amd64*" 2>/dev/null | head -1)"
if [[ -z "$library" ]]; then
    echo "libcronet.so не найдена в кэше модулей — naive работать не будет" >&2
    exit 0
fi

install -Dm644 "$library" "$root/build/libcronet.so"
echo "libcronet.so → build/"
