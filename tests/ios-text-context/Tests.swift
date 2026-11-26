import XCTest
import UIKit
@testable import TextContextRegression

@MainActor final class TextContextTests: XCTestCase {
    var input: UITextView!

    override func setUp() async throws {
        continueAfterFailure = true
        for _ in 0..<100 {
            let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
            if let host = windows.first(where: \.isKeyWindow)?.rootViewController as? HostController,
               let gpui = host.gpui, let found = findInput(gpui.view) {
                input = found
                break
            }
            await settle()
        }
        input = try XCTUnwrap(input, "The real upstream GPUI text view must be attached")
    }

    func findInput(_ view: UIView) -> UITextView? {
        if let text = view as? UITextView, NSStringFromClass(type(of: text)) == "GPUITextInputView" { return text }
        return view.subviews.lazy.compactMap(findInput).first
    }
    func settle() async { try? await Task.sleep(for: .milliseconds(100)) }
    func snapshot() -> [String: Any] {
        let pointer = regression_snapshot()!
        defer { regression_free(pointer) }
        return try! JSONSerialization.jsonObject(with: Data(String(cString: pointer).utf8)) as! [String: Any]
    }
    func documentText() -> String { snapshot()["text"] as! String }
    func logState(_ label: String) {
        let json = try! JSONSerialization.data(withJSONObject: snapshot(), options: [.sortedKeys])
        print(label, String(data: json, encoding: .utf8)!)
    }
    func setDocument(_ text: String, caret: Int? = nil) {
        text.withCString { regression_set_document($0, caret ?? text.utf16.count) }
    }
    func reset(_ text: String = "", caret: Int? = nil) async {
        regression_hide_keyboard()
        await settle()
        setDocument(text, caret: caret)
        regression_frame()
        regression_frame()
        XCTAssertTrue(input.becomeFirstResponder())
        await settle()
    }
    func nativeContextBeforeCaret() -> String? {
        guard let selection = input.selectedTextRange,
              let range = input.textRange(from: input.beginningOfDocument, to: selection.start) else { return nil }
        return input.text(in: range)
    }

    func test00FirstResponderBeforeNextHandlerPaint() async {
        setDocument("Context before repaint 🙂")
        let paints = snapshot()["handlerPaints"] as! Int
        XCTAssertTrue(input.becomeFirstResponder())
        await settle()
        XCTAssertEqual(snapshot()["handlerPaints"] as? Int, paints)
        regression_frame()
        await settle()
        XCTAssertEqual(input.text, documentText())
        XCTAssertEqual(nativeContextBeforeCaret(), documentText())
    }

    func test10CommittedDraftIsReplacedInGPUIDocument() async {
        await reset()
        let draft = "原始听写草稿🙂"
        let revision = "优化后的正式表述：明天讨论项目进展。"
        input.insertText(draft)
        XCTAssertEqual(documentText(), draft)
        XCTAssertEqual(input.text, draft, "Committed text must remain available to native text services")
        // Text services can revise a committed UITextView without entering insertText.
        input.text = revision
        input.selectedRange = NSRange(location: revision.utf16.count, length: 0)
        NotificationCenter.default.post(name: UITextView.textDidChangeNotification, object: input)
        XCTAssertEqual(documentText(), revision, "The revision must replace the draft, not append or disappear")
        let changes = snapshot()["changes"] as! [[String: Any]]
        XCTAssertEqual(changes.count, 2)
        XCTAssertEqual(changes.last?["range"] as? [Int], [0, draft.utf16.count])
        logState("REVISION_DOCUMENT")
    }

    func test20SameParagraphReshowRestoresContextWithoutGPUIFrame() async {
        let text = "同一段🙂：继续编辑这里。"
        await reset(text)
        regression_hide_keyboard()
        await settle()
        XCTAssertFalse(input.isFirstResponder)
        let frames = snapshot()["frames"] as! Int
        let paints = snapshot()["handlerPaints"] as! Int
        XCTAssertTrue(input.becomeFirstResponder())
        await settle()
        XCTAssertEqual(snapshot()["frames"] as? Int, frames, "No new GPUI frame may seed the context in this scenario")
        XCTAssertEqual(snapshot()["handlerPaints"] as? Int, paints, "No hidden redraw may reinstall the input handler")
        XCTAssertEqual(input.text, text)
        XCTAssertEqual(nativeContextBeforeCaret(), text)
        let target = (text as NSString).range(of: "这里")
        input.selectedRange = target
        input.insertText("此处")
        XCTAssertEqual(documentText(), text.replacingOccurrences(of: "这里", with: "此处"))
        logState("RESHOW_DOCUMENT")
    }

    func test30ExplicitRangeAndRepeatedCharacters() async {
        let text = "前🙂 aaa aaa 后"
        await reset(text)
        let range = (text as NSString).range(of: "aaa", options: .backwards)
        guard let start = input.position(from: input.beginningOfDocument, offset: range.location),
              let end = input.position(from: start, offset: range.length),
              let replacementRange = input.textRange(from: start, to: end) else {
            XCTFail("The committed document range must be available to UITextInput")
            return
        }
        input.replace(replacementRange, withText: "替换")
        XCTAssertEqual(documentText(), "前🙂 aaa 替换 后")
        let change = (snapshot()["changes"] as! [[String: Any]]).last!
        XCTAssertEqual(change["range"] as? [Int], [range.location, NSMaxRange(range)])
    }

    func test40MarkedCompositionStillCommitsOnce() async {
        await reset("前🙂后", caret: "前🙂".utf16.count)
        input.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        XCTAssertEqual(documentText(), "前🙂ni后")
        input.setMarkedText("你", selectedRange: NSRange(location: 1, length: 0))
        input.unmarkText()
        XCTAssertEqual(documentText(), "前🙂你后")
        XCTAssertTrue(snapshot()["marked"] is NSNull)

        await reset("ABC")
        input.selectedRange = NSRange(location: 1, length: 1)
        input.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        input.setMarkedText("你", selectedRange: NSRange(location: 1, length: 0))
        input.unmarkText()
        XCTAssertEqual(documentText(), "A你C", "Composition must start at the native selection")
    }

    func test50ReturnAndBackspaceKeepUpstreamSemantics() async {
        await reset("A🙂")
        input.deleteBackward()
        XCTAssertEqual(documentText(), "A")
        input.insertText("\n")
        XCTAssertEqual(documentText(), "A\n")
        input.deleteBackward()
        input.deleteBackward()
        XCTAssertEqual(documentText(), "")
        let keys = snapshot()["backspaceKeys"] as! Int
        input.deleteBackward()
        XCTAssertEqual(snapshot()["backspaceKeys"] as? Int, keys + 1, "An empty document must still forward Backspace")

        await reset("ABC")
        input.selectedRange = NSRange(location: 1, length: 1)
        input.insertText("\n")
        XCTAssertEqual(documentText(), "A\nC", "Return must use the native selection while retaining the upstream input path")
    }

    func test60BoundedContextMapsBackToUTF16DocumentRange() async {
        let prefix = String(repeating: "x", count: 2200)
        let text = prefix + "前🙂初稿后文"
        await reset(text)
        XCTAssertLessThanOrEqual(input.text.utf16.count, 2560)
        input.text = input.text.replacingOccurrences(of: "初稿", with: "修订文")
        NotificationCenter.default.post(name: UITextView.textDidChangeNotification, object: input)
        XCTAssertEqual(documentText(), prefix + "前🙂修订文后文")
        let change = (snapshot()["changes"] as! [[String: Any]]).last
        XCTAssertEqual(change?["range"] as? [Int], [2203, 2205])
    }

    func test55BackspaceAtNativeContextStartUsesNativeCaret() async {
        await reset("ABC")
        input.selectedRange = NSRange(location: 0, length: 0)
        input.deleteBackward()
        XCTAssertEqual(documentText(), "ABC", "Moving the native caret to the start must not delete at the old GPUI caret")

        let prefix = String(repeating: "x", count: 2200)
        let suffix = "🙂 document end"
        await reset(prefix + suffix)
        let origin = (prefix + suffix).utf16.count - input.text.utf16.count
        XCTAssertGreaterThan(origin, 0)
        input.selectedRange = NSRange(location: 0, length: 0)
        input.deleteBackward()
        XCTAssertEqual(documentText(), String(repeating: "x", count: 2199) + suffix)
        let change = (snapshot()["changes"] as! [[String: Any]]).last
        XCTAssertEqual(change?["range"] as? [Int], [origin - 1, origin])
    }

    func test70EditableRangeClampsNativeContextWithoutReportedLength() async {
        let prefix = "outside🙂 "
        let editable = "editable 文本"
        let suffix = " outside"
        await reset(prefix + editable + suffix, caret: (prefix + editable).utf16.count)
        regression_set_editable_range(prefix.utf16.count, (prefix + editable).utf16.count, false)
        regression_frame()
        await settle()
        XCTAssertEqual(input.text, editable)
        input.text = "改写后的区域"
        NotificationCenter.default.post(name: UITextView.textDidChangeNotification, object: input)
        XCTAssertEqual(documentText(), prefix + "改写后的区域" + suffix)
    }

    func test80StaleNativeContextDoesNotOverwriteNewDocument() async {
        await reset("Old context")
        setDocument("New GPUI document")
        // Do not repaint the new document before a late native revision arrives.
        input.text = "Obsolete revision"
        NotificationCenter.default.post(name: UITextView.textDidChangeNotification, object: input)
        XCTAssertEqual(documentText(), "New GPUI document")
        regression_frame()
        await settle()
        XCTAssertEqual(input.text, "New GPUI document")
    }
}
