import XCTest
import UIKit
import ObjectiveC.runtime
@testable import URLRegression

private var openedURL: URL?
private var openedOptions: [UIApplication.OpenExternalURLOptionsKey: Any]?
private var openCount = 0
extension UIApplication {
    @objc func regressionOpen(_ url: URL, options: [UIApplication.OpenExternalURLOptionsKey: Any]?, completionHandler: ((Bool) -> Void)?) {
        openedURL = url
        openedOptions = options
        openCount += 1
    }
}
@MainActor final class PlatformTests: XCTestCase {
    func testURLUsesExactSliceAndNonNilOptions() {
        let original = class_getInstanceMethod(UIApplication.self, #selector(UIApplication.open(_:options:completionHandler:)))!
        let substitute = class_getInstanceMethod(UIApplication.self, #selector(UIApplication.regressionOpen(_:options:completionHandler:)))!
        method_exchangeImplementations(original, substitute)
        defer { method_exchangeImplementations(original, substitute) }
        openCount = 0
        platformOpenURL(false)
        XCTAssertEqual(openCount, 1)
        XCTAssertEqual(openedURL?.absoluteString, "https://example.invalid/exact?text=hello")
        XCTAssertNotNil(openedOptions)
        if openedOptions != nil {
            platformOpenURL(true)
            XCTAssertEqual(openCount, 1, "Invalid URLs must not be passed to UIKit")
        }
    }
}
