#!/usr/bin/env bash
# Кладёт libcronet.so рядом с ядром: выхлоп naive грузит её при первом
# использовании. Статически она не линкуется — в поставляемом архиве
# libcronet.a лежат объекты сразу под несколько архитектур, и GNU ld
# отказывается его читать.
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
