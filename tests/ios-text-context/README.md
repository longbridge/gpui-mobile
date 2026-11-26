# iOS committed-text context regression

This standalone UIKit/XCTest host uses a real GPUI window, a synthetic
`EntityInputHandler` document and the backend's actual `GPUITextInputView`.
Assertions read the GPUI document back through Rust, rather than checking only
the native buffer or a separate Swift model. It has no files, account, clipboard,
microphone, network service or product signing configuration.

## Run

Requires an Apple Silicon Mac, Xcode with an iOS simulator, XcodeGen, and Rust's
`aarch64-apple-ios-sim` target. Use a dedicated empty simulator and pass its
explicit destination:

```sh
./run.sh 'platform=iOS Simulator,id=YOUR-SIMULATOR-UUID'
```

`CARGO_TARGET_DIR` may select an existing absolute Cargo cache. The test app has
its own `dev.gpui.regression.TextContextRegression` identifier and is unsigned.
The camera/video features only satisfy the baseline's existing unconditional
platform-view factories; the harness never uses those services.

## Scenarios

1. Become first responder before the next GPUI handler paint, then repaint.
2. Commit a draft, then deliver an optimized revision through native text-view
   change notification. Assert that the GPUI document contains exactly the
   revision and that its replacement range covers the previous draft.
3. Hide the keyboard, reopen the same focused paragraph, and read native context
   before the caret without another GPUI frame or handler paint. Select a word
   and edit it, checking the GPUI document.
4. Replace an explicit native range in repeated text containing a surrogate pair.
5. Update marked text and commit once, retaining surrounding GPUI content.
6. Preserve the baseline's Return insertion and Backspace key behavior.
7. Move the native caret to the mirrored context's start and send Backspace at
   the corresponding GPUI position, including a context starting mid-document.
8. Translate a bounded native context back to a long document's UTF-16 offsets.
9. Honor an explicit editable region when the handler does not report a total
   document length.
10. Reject a stale native revision after GPUI content has changed.

The host has no display link: tests explicitly request frames. A counter in the
real handler's paint closure detects hidden handler replacement during the
reopen scenario. The application handle remains alive for the UIKit process
lifetime, including Rust thread-local teardown.

On unmodified `7548ebc`, the revision and reopen scenarios fail: committed text is
cleared from the native scratch buffer, revisions do not replace it in GPUI, and
reopening does not restore context. The initial marked-text and Return/Backspace
sequences are baseline controls; their additional native-selection checks
exercise selection changes in the new mirrored context.

These tests exercise the native editing protocol and notification paths used by
text services. They do not run speech recognition, a third-party keyboard, or a
physical-device dictation service, and do not establish old-iOS compatibility.
