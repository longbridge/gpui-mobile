#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
cargo build --target aarch64-apple-ios-sim
xcodegen generate
xcodebuild -project TextContextRegression.xcodeproj -scheme TextContextRegression \
  -destination "${1:?Pass an explicit iOS simulator destination}" \
  -derivedDataPath build/DerivedData \
  "REGRESSION_STATIC_LIB=$CARGO_TARGET_DIR/aarch64-apple-ios-sim/debug/libgpui_ios_text_context_regression.a" test
