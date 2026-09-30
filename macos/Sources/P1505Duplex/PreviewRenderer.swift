#if DEBUG
import AppKit
import DuplexKit
import SwiftUI

/// `P1505Duplex --render-previews DIR`: draws each tab with sample data into
/// PNGs, offscreen, to check layouts without a screen.
@MainActor
enum PreviewRenderer {
    static func run(into dir: URL) {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let model = AppModel(settings: AppSettings(), notifier: Notifier())
        model.settings.onlyMine = false
        model.loadPreview(snapshot: sampleSnapshot, details: sampleDetails)

        for (tab, name) in [(Tab.jobs, "jobs"), (.printer, "printer"), (.history, "history")] {
            model.tab = tab
            write(MainView(openSettings: {}).environment(model), to: dir.appending(path: "\(name).png"))
        }
        write(SettingsView().environment(model), to: dir.appending(path: "settings.png"))

        var paused = sampleSnapshot
        paused.printer?.state = "stopped"
        paused.printer?.reasons = [Reason(keyword: "paused", text: "列印佇列已暫停", severity: .warning)]
        paused.printer?.message = "由手動雙面暫停"
        paused.printer?.blocking = true
        model.loadPreview(snapshot: paused, details: sampleDetails)
        for (tab, name) in [(Tab.jobs, "jobs-paused"), (.printer, "printer-paused")] {
            model.tab = tab
            write(MainView(openSettings: {}).environment(model), to: dir.appending(path: "\(name).png"))
        }
        for appearance in [NSAppearance.Name.darkAqua] {
            model.tab = .jobs
            write(MainView(openSettings: {}).environment(model), to: dir.appending(path: "jobs-dark.png"),
                  appearance: appearance)
        }
    }

    private static func write(_ view: some View, to url: URL, appearance: NSAppearance.Name = .aqua) {
        let host = NSHostingView(rootView: view)
        host.appearance = NSAppearance(named: appearance)
        let size = host.fittingSize
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless],
                              backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: appearance)
        window.backgroundColor = appearance == .aqua ? .windowBackgroundColor : .black
        window.contentView = host
        host.frame = NSRect(origin: .zero, size: size)
        // Let SwiftUI lay out and AppKit-backed controls (Form, List) draw.
        for _ in 0..<5 { RunLoop.main.run(until: Date().addingTimeInterval(0.1)) }
        host.layoutSubtreeIfNeeded()
        guard let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return }
        host.cacheDisplay(in: host.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: url)
        print(url.path)
    }

    static var sampleSnapshot: Snapshot {
        let now = Date()
        func side(_ job: Int, _ state: JobState, _ done: Int = 0) -> Side {
            Side(job: job, state: state, sheetsDone: done)
        }
        let printer = Printer(name: "HP_LaserJet_P1505", state: "processing", reasons: [],
                              message: "cfFilterGhostscript: Rendering completed",
                              blocking: false, accepting: true, queued: 1, usbConnected: true,
                              stateChanged: now.addingTimeInterval(-300))
        return Snapshot(printer: printer, cupsError: nil, jobs: [
            DuplexJob(id: 12, title: "期末報告.pdf", user: "dorian", pages: 7, sheets: 4, created: now,
                      front: side(13, .completed, 4), back: side(14, .held), stage: .readyToFlip),
            DuplexJob(id: 9, title: "租約草稿.pdf", user: "sinclairverlaine", pages: 4, sheets: 2,
                      created: now.addingTimeInterval(-900),
                      front: side(10, .processing, 1), back: side(11, .held), stage: .frontPrinting),
            DuplexJob(id: 5, title: "statement-2026-09.pdf", user: "dorian", pages: 6, sheets: 3,
                      created: now.addingTimeInterval(-3600),
                      front: side(6, .completed, 3), back: side(7, .completed, 3), stage: .done),
        ])
    }

    static var sampleDetails: PrinterDetails {
        let now = Date()
        let printer = sampleSnapshot.printer
        return PrinterDetails(
            printer: printer, cupsError: nil,
            makeModel: "HP LaserJet p1505, hpcups 3.24.4, requires proprietary plugin",
            info: "HP LaserJet P1505", location: "Athlon Server",
            deviceUri: "hp:/usb/HP_LaserJet_P1505?serial=XXXXXXX",
            markers: [], firmware: FirmwareRecord(time: now.addingTimeInterval(-7200), ok: true, attempts: 1, error: nil),
            duplexQueue: printer,
            queue: [QueueJob(id: 10, name: "租約草稿.pdf (正面)", user: "sinclairverlaine", state: .processing,
                             created: now, completed: nil, sheets: 1)],
            history: [
                QueueJob(id: 13, name: "期末報告.pdf (正面)", user: "dorian", state: .completed,
                         created: now.addingTimeInterval(-60), completed: now.addingTimeInterval(-30), sheets: 4),
                QueueJob(id: 7, name: "statement-2026-09.pdf (背面)", user: "dorian", state: .completed,
                         created: now.addingTimeInterval(-3000), completed: now.addingTimeInterval(-2900), sheets: 3),
                QueueJob(id: 4, name: "door test", user: "dorian", state: .canceled,
                         created: now.addingTimeInterval(-5000), completed: now.addingTimeInterval(-4990), sheets: 0),
            ])
    }
}
#endif
