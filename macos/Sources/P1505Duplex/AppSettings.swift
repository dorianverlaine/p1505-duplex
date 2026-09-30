import DuplexKit
import Foundation
import Observation
import ServiceManagement

/// User preferences, stored in UserDefaults.
@MainActor
@Observable
final class AppSettings {
    static let defaultServer = "http://athlon-server.local/duplex/"

    private let defaults = UserDefaults.standard

    var serverURL: String { didSet { defaults.set(serverURL, forKey: "serverURL") } }
    var onlyMine: Bool { didSet { defaults.set(onlyMine, forKey: "onlyMine") } }
    var notifyFlip: Bool { didSet { defaults.set(notifyFlip, forKey: "notifyFlip") } }
    var notifyDone: Bool { didSet { defaults.set(notifyDone, forKey: "notifyDone") } }
    var notifyFailure: Bool { didSet { defaults.set(notifyFailure, forKey: "notifyFailure") } }
    var notifyPrinter: Bool { didSet { defaults.set(notifyPrinter, forKey: "notifyPrinter") } }

    init() {
        defaults.register(defaults: [
            "serverURL": Self.defaultServer,
            "onlyMine": true,
            "notifyFlip": true,
            "notifyDone": true,
            "notifyFailure": true,
            "notifyPrinter": true,
        ])
        serverURL = defaults.string(forKey: "serverURL") ?? Self.defaultServer
        onlyMine = defaults.bool(forKey: "onlyMine")
        notifyFlip = defaults.bool(forKey: "notifyFlip")
        notifyDone = defaults.bool(forKey: "notifyDone")
        notifyFailure = defaults.bool(forKey: "notifyFailure")
        notifyPrinter = defaults.bool(forKey: "notifyPrinter")
    }

    /// The Mac account name; CUPS records the same name as the job's user.
    var userName: String { NSUserName() }

    /// Server base URL, normalized to end with a slash.
    var serverBase: URL? { Self.normalize(serverURL) }

    /// "host/duplex" → "http://host/duplex/"; nil when unusable.
    nonisolated static func normalize(_ input: String) -> URL? {
        var text = input.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return nil }
        if !text.contains("://") { text = "http://" + text }
        if !text.hasSuffix("/") { text += "/" }
        guard let url = URL(string: text), url.host() != nil else { return nil }
        return url
    }

    var notificationPrefs: NotificationPrefs {
        NotificationPrefs(flip: notifyFlip, done: notifyDone, failure: notifyFailure,
                          printer: notifyPrinter, onlyUser: onlyMine ? userName : nil)
    }

    var launchAtLogin: Bool {
        get { SMAppService.mainApp.status == .enabled }
        set {
            do {
                if newValue { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            } catch {
                NSLog("launch at login: \(error)")
            }
        }
    }
}
