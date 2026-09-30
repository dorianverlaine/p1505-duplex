import DuplexKit
import Foundation
@preconcurrency import UserNotifications

/// Posts notices as system notifications. The flip notice carries a
/// "continue" button that releases the back sides right from the banner.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    private static let flipCategory = "FLIP"
    private static let continueAction = "CONTINUE"

    // Fetched on use: it needs an app bundle, which previews run without.
    private var center: UNUserNotificationCenter { .current() }

    /// The user pressed "continue" on a flip notice.
    var onContinue: ((Int) -> Void)?
    /// The user clicked a notice itself.
    var onOpen: (() -> Void)?

    func setUp() {
        center.delegate = self
        let action = UNNotificationAction(identifier: Self.continueAction, title: "繼續列印背面", options: [])
        let category = UNNotificationCategory(identifier: Self.flipCategory, actions: [action],
                                              intentIdentifiers: [], options: [])
        center.setNotificationCategories([category])
        Task {
            do {
                _ = try await center.requestAuthorization(options: [.alert, .sound])
            } catch {
                NSLog("notification authorization: \(error)")
            }
        }
    }

    func post(_ notice: Notice) {
        let content = UNMutableNotificationContent()
        content.title = notice.title
        content.body = notice.body
        content.sound = .default
        if case .readyToFlip(let job) = notice {
            content.categoryIdentifier = Self.flipCategory
            content.userInfo = ["job": job.id]
            content.interruptionLevel = .active
        }
        let request = UNNotificationRequest(identifier: notice.id, content: content, trigger: nil)
        center.add(request) { error in
            if let error { NSLog("posting notification: \(error)") }
        }
    }

    func withdraw(_ ids: [String]) {
        guard !ids.isEmpty else { return }
        center.removeDeliveredNotifications(withIdentifiers: ids)
        center.removePendingNotificationRequests(withIdentifiers: ids)
    }

    // A menu bar app counts as frontmost; show banners anyway.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            willPresent notification: UNNotification) async
        -> UNNotificationPresentationOptions
    {
        [.banner, .sound, .list]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            didReceive response: UNNotificationResponse) async
    {
        let action = response.actionIdentifier
        let job = response.notification.request.content.userInfo["job"] as? Int
        await MainActor.run {
            if action == Self.continueAction, let job {
                onContinue?(job)
            } else if action == UNNotificationDefaultActionIdentifier {
                onOpen?()
            }
        }
    }
}
