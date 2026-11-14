#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export CARGO_TARGET_DIR="$ROOT/native/macos/.build/cargo"
mkdir -p "$ROOT/native/macos/.build"
cargo test --manifest-path "$ROOT/crates/macos-bridge/Cargo.toml" 2>&1 | tee "$ROOT/native/macos/.build/bridge-test.log"
