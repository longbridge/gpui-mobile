# iOS URL regression

Run `./run.sh 'platform=iOS Simulator,name=iPhone 17'` on an Apple Silicon Mac.
The test invokes the real `IosPlatform::open_url` implementation, replacing
UIApplication's open method only inside this test process. No external app is
opened. It checks that a slice does not include trailing bytes from its backing
allocation, options is non-nil, and an invalid URL does not reach UIKit.

The baseline fails the exact-URL and non-nil-options assertions. The invalid-URL
case runs only after options is non-nil, to avoid forwarding nil into Swift's
nonoptional URL argument on the baseline.
