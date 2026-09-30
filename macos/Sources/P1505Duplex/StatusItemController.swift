import AppKit
import DuplexKit
import SwiftUI

/// The menu bar icon: left click toggles the main popover, right click (or
/// control-click) opens a menu with the common actions.
@MainActor
final class StatusItemController: NSObject {
    private let model: AppModel
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    private let popover = NSPopover()
    var openSettings: () -> Void = {}

    init(model: AppModel) {
        self.model = model
        super.init()

        popover.behavior = .transient
        popover.animates = false
        let host = NSHostingController(rootView: MainView(openSettings: { [weak self] in self?.showSettings() })
            .environment(model))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host

        if let button = item.button {
            button.target = self
            button.action = #selector(clicked)
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        updateIcon()
    }

    func updateIcon() {
        guard let button = item.button else { return }
        let (symbol, label): (String, String) = switch model.iconState {
        case .offline: ("printer", "連不上列印伺服器")
        case .idle: ("printer", "待機")
        case .printing: ("printer.fill.and.paper.fill", "列印中")
        case .readyToFlip: ("arrow.2.squarepath", "該翻面了")
        case .problem: ("exclamationmark.triangle.fill", "需要處理")
        }
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: "P1505 手動雙面：\(label)")
        image?.isTemplate = true
        button.image = image
        button.appearsDisabled = model.iconState == .offline
        let count = model.readyJobs.count
        button.title = count > 1 ? " \(count)" : ""
        button.imagePosition = count > 1 ? .imageLeading : .imageOnly
        item.length = count > 1 ? NSStatusItem.variableLength : NSStatusItem.squareLength
        button.toolTip = "P1505 手動雙面 — \(model.summary)"
    }

    @objc private func clicked(_ sender: NSStatusBarButton) {
        let event = NSApp.currentEvent
        if event?.type == .rightMouseUp || event?.modifierFlags.contains(.control) == true {
            showMenu()
        } else {
            togglePopover()
        }
    }

    // MARK: - Popover

    func togglePopover() {
        if popover.isShown {
            popover.performClose(nil)
        } else {
            showPopover()
        }
    }

    func showPopover(tab: Tab? = nil) {
        if let tab { model.tab = tab }
        guard let button = item.button else { return }
        if !popover.isShown {
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        }
        NSApp.activate()
        popover.contentViewController?.view.window?.makeKey()
    }

    private func showSettings() {
        popover.performClose(nil)
        openSettings()
    }

    // MARK: - Menu

    private func showMenu() {
        popover.performClose(nil)
        item.menu = buildMenu()
        item.button?.performClick(nil)
        item.menu = nil
    }

    private func buildMenu() -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false

        let header = NSMenuItem(title: model.summary, action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)
        menu.addItem(.separator())

        let ready = model.readyJobs
        for job in ready {
            menu.addItem(ActionItem("繼續列印背面：\(job.title)", symbol: "arrow.2.squarepath",
                                    enabled: !model.isBusy("continue-\(job.id)")) { [model] in
                model.continueJob(job.id)
            })
        }
        let others = model.jobs.filter { ($0.stage.isActive && $0.stage != .readyToFlip) || $0.stage.isFailure }
        for job in others.prefix(5) {
            let row = NSMenuItem(title: "\(job.title) — \(job.stage.summary)", action: nil, keyEquivalent: "")
            row.isEnabled = false
            menu.addItem(row)
        }
        if ready.isEmpty && others.isEmpty {
            let none = NSMenuItem(title: model.connected ? "沒有進行中的雙面工作" : "等待連線…", action: nil, keyEquivalent: "")
            none.isEnabled = false
            menu.addItem(none)
        }

        menu.addItem(.separator())
        menu.addItem(ActionItem("打開主視窗", symbol: "macwindow") { [weak self] in self?.showPopover(tab: .jobs) })
        menu.addItem(ActionItem("印表機狀態", symbol: "printer") { [weak self] in self?.showPopover(tab: .printer) })
        menu.addItem(ActionItem("列印紀錄", symbol: "clock.arrow.circlepath") { [weak self] in self?.showPopover(tab: .history) })
        menu.addItem(ActionItem("在瀏覽器打開網頁版", symbol: "safari") { [model] in model.openWebPage() })

        menu.addItem(.separator())
        let connected = model.connected
        menu.addItem(ActionItem("列印雙面測試頁", symbol: "doc.text", enabled: connected) { [model] in
            model.printTestPage()
        })
        if model.snapshot?.printer?.isPaused == true {
            menu.addItem(ActionItem("恢復列印佇列", symbol: "play.circle", enabled: connected) { [model] in
                model.resumePrinter()
            })
        } else {
            menu.addItem(ActionItem("暫停列印佇列", symbol: "pause.circle", enabled: connected) { [model] in
                model.pausePrinter()
            })
        }

        menu.addItem(.separator())
        menu.addItem(ActionItem("設定…", symbol: "gearshape", key: ",") { [weak self] in self?.openSettings() })
        menu.addItem(ActionItem("結束", symbol: nil, key: "q") { NSApp.terminate(nil) })
        return menu
    }
}

/// A menu item that runs a closure.
private final class ActionItem: NSMenuItem {
    private let run: () -> Void

    init(_ title: String, symbol: String?, key: String = "", enabled: Bool = true, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(fire), keyEquivalent: key)
        target = self
        isEnabled = enabled
        if let symbol { image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) }
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError() }

    @objc private func fire() { run() }
}
