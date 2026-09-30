import AppKit
import DuplexKit
import Foundation
import Observation

enum Tab: Hashable {
    case jobs, printer, history
}

/// What the menu bar icon shows.
enum IconState: Equatable {
    case offline, idle, printing, readyToFlip, problem
}

@MainActor
@Observable
final class AppModel {
    let settings: AppSettings
    let notifier: Notifier

    private(set) var snapshot: Snapshot?
    private(set) var details: PrinterDetails?
    private(set) var connected = false
    private(set) var connectionError: String?
    /// Actions in flight, by key, to disable their buttons.
    private(set) var busy: Set<String> = []
    var errorMessage: String?
    var tab: Tab = .jobs

    /// Called after anything the menu bar icon depends on changes.
    @ObservationIgnored var onChange: (() -> Void)?

    @ObservationIgnored private var streamTask: Task<Void, Never>?
    @ObservationIgnored private var planner = NotificationPlanner()

    init(settings: AppSettings, notifier: Notifier) {
        self.settings = settings
        self.notifier = notifier
    }

    var client: APIClient? { settings.serverBase.map { APIClient(base: $0) } }

    var jobs: [DuplexJob] {
        guard let jobs = snapshot?.jobs else { return [] }
        return settings.onlyMine ? jobs.filter { $0.user == settings.userName } : jobs
    }

    var readyJobs: [DuplexJob] { jobs.filter { $0.stage == .readyToFlip } }

    var iconState: IconState {
        guard connected, let snap = snapshot else { return .offline }
        if snap.printer?.blocking == true || jobs.contains(where: { $0.stage.isFailure }) { return .problem }
        if !readyJobs.isEmpty { return .readyToFlip }
        if jobs.contains(where: { $0.stage.isActive }) || snap.printer?.state == "processing" { return .printing }
        return .idle
    }

    /// One line for tooltips and the menu header.
    var summary: String {
        guard connected else { return "連不上列印伺服器" }
        guard let p = snapshot?.printer else { return "連不上 CUPS" }
        var parts = ["印表機\(p.stateText)"]
        if !p.usbConnected { parts.append("USB 未連接") }
        if let reason = p.reasons.first { parts.append(reason.text) }
        if !readyJobs.isEmpty { parts.append("\(readyJobs.count) 份等你翻面") }
        return parts.joined(separator: " · ")
    }

    // MARK: - Event stream

    func start() {
        restartStream()
    }

    func restartStream() {
        streamTask?.cancel()
        planner = NotificationPlanner()
        snapshot = nil
        details = nil
        connected = false
        connectionError = nil
        onChange?()
        guard let base = settings.serverBase else {
            connectionError = "伺服器網址不正確"
            return
        }
        streamTask = Task { [weak self] in
            for await update in snapshotStream(base: base) {
                self?.handle(update)
            }
        }
    }

    private func handle(_ update: StreamUpdate) {
        switch update {
        case .snapshot(let snap):
            snapshot = snap
            connected = true
            connectionError = nil
            let (post, withdraw) = planner.update(with: snap, prefs: settings.notificationPrefs)
            notifier.withdraw(withdraw)
            post.forEach(notifier.post)
        case .disconnected(let message):
            connected = false
            connectionError = message
        }
        onChange?()
    }

    // MARK: - Printer details

    func refreshDetails() async {
        guard let client else { return }
        do {
            details = try await client.printer()
        } catch {
            // Shown through the connection state; keep the last details.
            NSLog("printer details: \(error.localizedDescription)")
        }
    }

    // MARK: - Actions

    func isBusy(_ key: String) -> Bool { busy.contains(key) }

    private func perform(_ key: String, refreshDetails alsoDetails: Bool = false,
                         _ op: @escaping @Sendable (APIClient) async throws -> Void) {
        guard let client, !busy.contains(key) else { return }
        busy.insert(key)
        Task {
            defer { busy.remove(key) }
            do {
                try await op(client)
                if alsoDetails { await refreshDetails() }
            } catch {
                errorMessage = error.localizedDescription
                NSApp.activate()
            }
        }
    }

    func continueJob(_ id: Int) {
        notifier.withdraw(["flip-\(id)"])
        perform("continue-\(id)") { try await $0.continueJob(id) }
    }

    func reprint(_ id: Int, side: PrintSide, first: Int, last: Int) {
        perform("reprint-\(id)-\(side.rawValue)") { try await $0.reprint(id, side: side, first: first, last: last) }
    }

    func remove(_ id: Int) {
        perform("remove-\(id)") { try await $0.remove(id) }
    }

    func cancelQueueJob(_ id: Int) {
        perform("cancel-\(id)", refreshDetails: true) { try await $0.cancelQueueJob(id) }
    }

    func pausePrinter() {
        perform("pause", refreshDetails: true) { try await $0.pausePrinter() }
    }

    func resumePrinter() {
        perform("resume", refreshDetails: true) { try await $0.resumePrinter() }
    }

    func printTestPage() {
        let user = settings.userName
        perform("test-page") { try await $0.testPage(user: user) }
    }

    #if DEBUG
    /// Show fixed data instead of the live stream (for rendering previews).
    func loadPreview(snapshot: Snapshot, details: PrinterDetails?) {
        streamTask?.cancel()
        self.snapshot = snapshot
        self.details = details
        connected = true
        connectionError = nil
    }
    #endif

    func openWebPage() {
        if let url = settings.serverBase { NSWorkspace.shared.open(url) }
    }
}
