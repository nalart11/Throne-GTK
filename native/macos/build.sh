#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/native/macos/.build"
export CARGO_TARGET_DIR="$OUT/cargo"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
mkdir -p "$OUT"
mkdir -p "$OUT/Resources"
cp -R "$ROOT/native/macos/Resources/." "$OUT/Resources/"
cargo rustc --manifest-path "$ROOT/crates/macos-bridge/Cargo.toml" --release --lib -- --print native-static-libs 2>&1 | tee "$OUT/rust-build.log"
# rustc reports platform-native link requirements; split only its trusted output.
LIBS=$(grep 'note: native-static-libs:' "$OUT/rust-build.log" | cut -d: -f3- | tail -1 || true)
if [ -z "$LIBS" ]; then
  # Stable Darwin dependencies reported by rustc for this staticlib. Cargo can
  # omit its cached diagnostic; never mutate sources to force recompilation.
  LIBS='-framework Foundation -framework ServiceManagement -liconv -lSystem -lc -lm'
fi
read -ra NATIVE_LIBS <<< "$LIBS"
swiftc -swift-version 6 -strict-concurrency=complete -parse-as-library "$ROOT"/native/macos/Sources/*.swift \
  -target "$(uname -m)-apple-macosx${MACOSX_DEPLOYMENT_TARGET}" \
  -import-objc-header "$ROOT/crates/macos-bridge/include/throne_macos.h" \
  "$CARGO_TARGET_DIR/release/libthrone_macos_bridge.a" \
  -framework AppKit -framework SwiftUI -framework Security -framework SystemConfiguration \
  "${NATIVE_LIBS[@]}" -o "$OUT/throne-gtk" 2>&1 | tee "$OUT/swift-build.log"
echo "Built $OUT/throne-gtk (not launched)"
