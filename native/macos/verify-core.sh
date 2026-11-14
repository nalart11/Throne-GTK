#!/bin/bash
# Explicit opt-in real unprivileged core execution, configuration validation only.
# Never rebuilds Rust, installs anything, or launches the GUI/helper.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/native/macos/.build/core-validation"
LIB="$ROOT/native/macos/.build/cargo/release/libnolurix_macos_bridge.a"
CORE="$ROOT/build/nolurix-core"
[[ "$(uname -s)" == Darwin ]] || { echo 'Requires macOS' >&2; exit 1; }
[[ -f "$LIB" ]] || { echo 'Missing built staticlib; build native frontend first.' >&2; exit 1; }
[[ -x "$CORE" ]] || { echo 'Missing built core; run just build-core first.' >&2; exit 1; }
mkdir -p "$OUT"
swiftc -swift-version 6 -strict-concurrency=complete -parse-as-library \
  -target "$(uname -m)-apple-macosx13.0" \
  -import-objc-header "$ROOT/crates/macos-bridge/include/nolurix_macos.h" \
  "$ROOT/native/macos/Sources/Backend.swift" \
  "$ROOT/native/macos/Verification/CoreSmoke.swift" "$LIB" \
  -framework Foundation -framework Combine -framework ServiceManagement \
  -liconv -lSystem -lc -lm -o "$OUT/nolurix"
# Copy only: never sign, modify, or execute the original build artifact.
install -m555 "$CORE" "$OUT/nolurix-core"
"$OUT/nolurix" 2>&1 | tee "$OUT/core-validation.log"
