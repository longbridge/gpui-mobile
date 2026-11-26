import UIKit

@main final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication, configurationForConnecting session: UISceneSession, options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let config = UISceneConfiguration(name: "Regression", sessionRole: session.role)
        config.delegateClass = SceneDelegate.self
        return config
    }
}

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        let window = UIWindow(windowScene: scene as! UIWindowScene)
        window.rootViewController = HostController()
        window.makeKeyAndVisible()
        self.window = window
    }
}

final class HostController: UIViewController {
    private(set) var gpui: UIViewController!
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard gpui == nil else { return }
        gpui = Unmanaged<UIViewController>.fromOpaque(regression_start()!).takeUnretainedValue()
        addChild(gpui)
        gpui.view.frame = view.bounds
        gpui.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(gpui.view)
        gpui.didMove(toParent: self)
        // Tests explicitly drive GPUI frames to control first-responder timing.
    }
}
