# iOS clipboard text regression

Run `./run.sh 'platform=iOS Simulator,name=iPhone 17'` on an Apple Silicon Mac
with Xcode, XcodeGen and Rust's `aarch64-apple-ios-sim` target installed.

The test calls the real platform clipboard methods but substitutes a unique
named UIPasteboard for `generalPasteboard` inside the test process. It restores
the original method and removes the named pasteboard afterward. The general
system clipboard is never read or written.

It independently checks write and read with embedded NUL, Chinese characters
and emoji. Both assertions fail on the unmodified backend, which truncates at
the NUL, and pass after the fix. This test does not cover image or metadata
clipboard support.
