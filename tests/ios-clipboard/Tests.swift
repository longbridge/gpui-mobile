import XCTest
import UIKit
import ObjectiveC.runtime
@testable import ClipboardRegression

private var isolatedPasteboard: UIPasteboard!
extension UIPasteboard {
    @objc class func regressionGeneral() -> UIPasteboard { isolatedPasteboard }
}
@MainActor final class ClipboardTests: XCTestCase {
    func testClipboardPreservesEmbeddedNULAndUnicode() {
        let name = UIPasteboard.Name("dev.gpui.regression.\(UUID().uuidString)")
        isolatedPasteboard = UIPasteboard(name: name, create: true)!
        let original = class_getClassMethod(UIPasteboard.self, NSSelectorFromString("generalPasteboard"))!
        let substitute = class_getClassMethod(UIPasteboard.self, #selector(UIPasteboard.regressionGeneral))!
        method_exchangeImplementations(original, substitute)
        defer {
            method_exchangeImplementations(original, substitute)
            UIPasteboard.remove(withName: name)
            isolatedPasteboard = nil
        }
        clipboardWrite()
        XCTAssertEqual(isolatedPasteboard.string, "Rust\0中文🙂")
        isolatedPasteboard.string = "Rust\0中文🙂"
        XCTAssertTrue(clipboardCheck())
    }
}
