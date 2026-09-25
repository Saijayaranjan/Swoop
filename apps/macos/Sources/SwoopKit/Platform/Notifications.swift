import AppKit
import Foundation
import UserNotifications

/// Posts engine notifications to Notification Center with Open / Reveal / Retry actions.
@MainActor
public final class NotificationService: NSObject, UNUserNotificationCenterDelegate {
    public enum Action: String { case open = "OPEN", reveal = "REVEAL", retry = "RETRY", show = "SHOW" }

    /// Called when the user picks an action: (action, taskId, path).
    public var onAction: ((Action, String?, String?) -> Void)?
    private var authorized = false
    private var center: UNUserNotificationCenter? {
        // UNUserNotificationCenter requires a real bundle; unit tests run without one.
        Bundle.main.bundleIdentifier == nil ? nil : UNUserNotificationCenter.current()
    }

    public override init() { super.init() }

    public func configure() {
        guard let center else { return }
        center.delegate = self
        let open = UNNotificationAction(identifier: Action.open.rawValue, title: L10n.tr("Open"), options: [.foreground])
        let reveal = UNNotificationAction(identifier: Action.reveal.rawValue, title: L10n.tr("Show in Finder"), options: [.foreground])
        let retry = UNNotificationAction(identifier: Action.retry.rawValue, title: L10n.tr("Retry"), options: [])
        let show = UNNotificationAction(identifier: Action.show.rawValue, title: L10n.tr("Show"), options: [.foreground])
        center.setNotificationCategories([
            UNNotificationCategory(identifier: "completed", actions: [open, reveal], intentIdentifiers: []),
            UNNotificationCategory(identifier: "failed", actions: [retry, show], intentIdentifiers: []),
            UNNotificationCategory(identifier: "general", actions: [show], intentIdentifiers: []),
        ])
        center.requestAuthorization(options: [.alert, .sound, .badge]) { granted, _ in
            Task { @MainActor in self.authorized = granted }
        }
    }

    /// Posts an engine notification. `quiet` suppresses delivery while the window is focused
    /// (the engine already filtered by per-kind settings).
    public func post(_ n: EngineNotification, sound: Bool) {
        guard let center else { return }
        let content = UNMutableNotificationContent()
        content.title = n.title
        content.body = n.body
        if sound { content.sound = .default }
        switch n.kind {
        case "completed", "torrent_finished": content.categoryIdentifier = "completed"
        case "failed", "checksum_mismatch": content.categoryIdentifier = "failed"
        default: content.categoryIdentifier = "general"
        }
        var info: [String: String] = ["kind": n.kind]
        if let t = n.taskId { info["task_id"] = t }
        if let p = n.path { info["path"] = p }
        content.userInfo = info
        content.threadIdentifier = n.kind
        let req = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        center.add(req)
    }

    public nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .sound, .list]
    }

    public nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        let taskId = info["task_id"] as? String
        let path = info["path"] as? String
        let id = response.actionIdentifier
        await MainActor.run {
            let action: Action
            if id == UNNotificationDefaultActionIdentifier { action = .show } else { action = Action(rawValue: id) ?? .show }
            self.onAction?(action, taskId, path)
        }
    }
}
