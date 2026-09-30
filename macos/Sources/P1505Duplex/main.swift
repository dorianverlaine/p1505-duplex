import AppKit
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var model: AppModel!
    private var statusItem: StatusItemController!
    private var settingsWindow: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let notifier = Notifier()
        model = AppModel(settings: AppSettings(), notifier: notifier)
        statusItem = StatusItemController(model: model)
        statusItem.openSettings = { [weak self] in self?.showSettings() }
        model.onChange = { [weak self] in self?.statusItem.updateIcon() }

        notifier.onContinue = { [weak self] id in self?.model.continueJob(id) }
        notifier.onOpen = { [weak self] in self?.statusItem.showPopover(tab: .jobs) }
        notifier.setUp()

        model.start()
    }

    private func showSettings() {
        if settingsWindow == nil {
            let host = NSHostingController(rootView: SettingsView().environment(model))
            let window = NSWindow(contentViewController: host)
            window.title = "P1505 手動雙面 設定"
            window.styleMask = [.titled, .closable]
            window.isReleasedWhenClosed = false
            window.center()
            settingsWindow = window
        }
        NSApp.activate()
        settingsWindow?.makeKeyAndOrderFront(nil)
    }
}

let app = NSApplication.shared
#if DEBUG
if let i = CommandLine.arguments.firstIndex(of: "--render-previews"), i + 1 < CommandLine.arguments.count {
    let dir = URL(fileURLWithPath: CommandLine.arguments[i + 1])
    MainActor.assumeIsolated { PreviewRenderer.run(into: dir) }
    exit(0)
}
#endif
let delegate = MainActor.assumeIsolated { AppDelegate() }
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
