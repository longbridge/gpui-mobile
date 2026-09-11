import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        let window = UIWindow(frame: UIScreen.main.bounds)
        window.rootViewController = UINavigationController(rootViewController: MarkdownContainerController())
        window.makeKeyAndVisible()
        self.window = window
        return true
    }

    func applicationDidBecomeActive(_ application: UIApplication) {
        gpui_ios_did_become_active(nil)
    }

    func applicationWillResignActive(_ application: UIApplication) {
        gpui_ios_will_resign_active(nil)
    }
}

/// UIKit owns this view's frame; GPUI owns only the content rendered inside it.
final class GPUITextView: UIView {
    private let gpuiWindow: UnsafeMutableRawPointer
    let contentController: UIViewController

    override init(frame: CGRect) {
        gpui_ios_set_embedded()
        gpui_ios_register_app()
        gpui_ios_run_demo()
        guard let window = gpui_ios_get_window(),
              let controller = gpui_ios_view_controller(window) else {
            fatalError("Could not create the GPUI Markdown view")
        }
        gpuiWindow = window
        contentController = Unmanaged<UIViewController>.fromOpaque(controller).takeUnretainedValue()
        super.init(frame: frame)
        clipsToBounds = true
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is unsupported") }

    func attach(to parent: UIViewController) {
        parent.addChild(contentController)
        addSubview(contentController.view)
        contentController.didMove(toParent: parent)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.width > 0, bounds.height > 0,
              contentController.view.frame != bounds else { return }
        // Publish the new geometry and its rendered content together. Otherwise
        // Core Animation can stretch the previous drawable until the next tick.
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        contentController.view.frame = bounds
        gpui_ios_layout_view(gpuiWindow)
        drawFrame()
        CATransaction.commit()
    }

    func drawFrame() { gpui_ios_request_frame(gpuiWindow) }
}

final class MarkdownContainerController: UIViewController {
    private var markdown: GPUITextView!
    private var heightConstraint: NSLayoutConstraint!
    private var displayLink: CADisplayLink?

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Markdown"
        overrideUserInterfaceStyle = .light
        view.backgroundColor = .systemGroupedBackground

        let sizeControl = UISegmentedControl(items: ["Compact", "Expanded"])
        sizeControl.selectedSegmentIndex = 1
        sizeControl.addTarget(self, action: #selector(resizeDocument(_:)), for: .valueChanged)

        let header = UIStackView(arrangedSubviews: [sizeControl])
        header.axis = .vertical
        header.spacing = 12
        header.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(header)

        markdown = GPUITextView(frame: .zero)
        markdown.layer.cornerRadius = 16
        markdown.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(markdown)
        markdown.attach(to: self)

        heightConstraint = markdown.heightAnchor.constraint(equalToConstant: 280)
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            header.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            header.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            markdown.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 16),
            markdown.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            markdown.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            markdown.bottomAnchor.constraint(lessThanOrEqualTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -16),
        ])
        let bottom = markdown.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -16)
        bottom.priority = .defaultHigh
        bottom.isActive = true
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        displayLink = CADisplayLink(target: self, selector: #selector(renderFrame))
        displayLink?.add(to: .main, forMode: .common)
    }

    override func viewWillDisappear(_ animated: Bool) {
        displayLink?.invalidate()
        displayLink = nil
        super.viewWillDisappear(animated)
    }

    @objc private func renderFrame() { markdown.drawFrame() }

    @objc private func resizeDocument(_ sender: UISegmentedControl) {
        UIView.performWithoutAnimation {
            heightConstraint.isActive = sender.selectedSegmentIndex == 0
            view.setNeedsLayout()
            view.layoutIfNeeded()
        }
    }
}
