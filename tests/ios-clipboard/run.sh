#!/usr/bin/env bash
# Requires Xcode, an arm64 iOS simulator, Rust's iOS simulator target and XcodeGen.
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
cargo build --target aarch64-apple-ios-sim
xcodegen generate
xcodebuild -project ClipboardRegression.xcodeproj -scheme ClipboardRegression \
  -destination "${1:-platform=iOS Simulator,name=iPhone 17}" \
  -derivedDataPath build/DerivedData \
  "REGRESSION_STATIC_LIB=$CARGO_TARGET_DIR/aarch64-apple-ios-sim/debug/libgpui_ios_clipboard_regression.a" test
