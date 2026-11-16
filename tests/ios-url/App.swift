import UIKit

@_silgen_name("platform_open_url") func platformOpenURL(_ invalid: Bool)

@main class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication, configurationForConnecting session: UISceneSession, options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        if ProcessInfo.processInfo.arguments.contains("--exercise-url") { platformOpenURL(false) }
        let configuration = UISceneConfiguration(name: "Test", sessionRole: session.role)
        configuration.delegateClass = SceneDelegate.self
        return configuration
    }
}
class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        let window = UIWindow(windowScene: scene as! UIWindowScene)
        window.rootViewController = UIViewController()
        window.makeKeyAndVisible()
        self.window = window
    }
}
