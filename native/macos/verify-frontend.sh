#!/bin/bash
# Exercise the actual frontend transport without creating NSApplication/windows.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/native/macos/.build"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
bash "$ROOT/native/macos/verify-bridge.sh"
swiftc -swift-version 6 -strict-concurrency=complete -parse-as-library \
  -target "$(uname -m)-apple-macosx${MACOSX_DEPLOYMENT_TARGET}" \
  -import-objc-header "$ROOT/crates/macos-bridge/include/nolurix_macos.h" \
  "$ROOT/native/macos/Sources/Backend.swift" \
  "$ROOT/native/macos/Sources/RoutingEditor.swift" \
  "$ROOT/native/macos/Sources/ProfileSearch.swift" \
  "$ROOT/native/macos/Verification/FrontendSmoke.swift" \
  "$OUT/cargo/release/libnolurix_macos_bridge.a" \
  -framework Foundation -framework Combine -framework ServiceManagement \
  -liconv -lSystem -lc -lm -o "$OUT/frontend-smoke" \
  2>&1 | tee "$OUT/frontend-smoke-build.log"
"$OUT/frontend-smoke" 2>&1 | tee "$OUT/frontend-smoke.log"
