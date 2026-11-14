#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/native/macos/.build"
export CARGO_TARGET_DIR="$OUT/cargo"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
mkdir -p "$OUT"
exec > >(tee "$OUT/bridge-smoke.log") 2>&1
cargo rustc --manifest-path "$ROOT/crates/macos-bridge/Cargo.toml" --release --lib -- --print native-static-libs 2>&1 | tee "$OUT/bridge-smoke-rust.log"
LIBS=$(grep 'note: native-static-libs:' "$OUT/bridge-smoke-rust.log" | cut -d: -f3- | tail -1 || true)
if [[ -z "$LIBS" ]]; then
  LIBS='-framework Foundation -framework ServiceManagement -liconv -lSystem -lc -lm'
fi
read -ra NATIVE_LIBS <<< "$LIBS"
swiftc -swift-version 6 -strict-concurrency=complete -parse-as-library \
  -target "$(uname -m)-apple-macosx${MACOSX_DEPLOYMENT_TARGET}" \
  -import-objc-header "$ROOT/crates/macos-bridge/include/nolurix_macos.h" \
  "$ROOT/native/macos/Verification/BridgeSmoke.swift" \
  "$CARGO_TARGET_DIR/release/libnolurix_macos_bridge.a" \
  -framework AppKit -framework SwiftUI -framework Security -framework SystemConfiguration \
  "${NATIVE_LIBS[@]}" -o "$OUT/bridge-smoke"
"$OUT/bridge-smoke"
