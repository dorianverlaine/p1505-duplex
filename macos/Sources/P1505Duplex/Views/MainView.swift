import DuplexKit
import SwiftUI

/// The popover under the menu bar icon.
struct MainView: View {
    @Environment(AppModel.self) private var model
    var openSettings: () -> Void

    var body: some View {
        @Bindable var model = model
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "printer.fill")
                    .foregroundStyle(.secondary)
                Text("P1505 手動雙面")
                    .font(.headline)
                Spacer()
                ConnectionBadge()
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)
            .padding(.bottom, 10)

            Picker("分頁", selection: $model.tab) {
                Text("工作").tag(Tab.jobs)
                Text("印表機").tag(Tab.printer)
                Text("紀錄").tag(Tab.history)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 16)
            .padding(.bottom, 10)

            Divider()

            Group {
                switch model.tab {
                case .jobs: JobsView()
                case .printer: PrinterView()
                case .history: HistoryView()
                }
            }
            .frame(height: 460)

            Divider()

            HStack {
                Button("網頁版", systemImage: "safari") { model.openWebPage() }
                Spacer()
                Button("設定", systemImage: "gearshape", action: openSettings)
            }
            .buttonStyle(.borderless)
            .labelStyle(.titleAndIcon)
            .font(.callout)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .frame(width: 400)
        .alert("無法完成", isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.errorMessage = nil } }
        )) {
            Button("好") { model.errorMessage = nil }
        } message: {
            Text(model.errorMessage ?? "")
        }
    }
}

struct ConnectionBadge: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(model.connected ? Color.green : Color.orange)
                .frame(width: 7, height: 7)
            Text(model.connected ? "已連線" : "重新連線中")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .help(model.connectionError ?? "即時更新中")
    }
}

/// Printer problems and connection trouble, above the job list.
struct PrinterBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if !model.connected {
            Banner(style: .error, title: "連不上列印伺服器",
                   lines: [model.connectionError ?? "", "伺服器：\(model.settings.serverURL)"])
        } else if let snap = model.snapshot {
            if let p = snap.printer {
                let lines = p.reasons.map(\.text) + (p.message.isEmpty ? [] : [p.message])
                    + (p.usbConnected ? [] : ["印表機沒有接上（USB 未連接）"])
                if !lines.isEmpty || p.state == "stopped" {
                    Banner(style: p.blocking || !p.usbConnected ? .error : .warning, title: "印表機狀態",
                           lines: lines.isEmpty ? ["印表機已停止"] : lines)
                }
            } else {
                Banner(style: .error, title: "連不上 CUPS", lines: [snap.cupsError ?? ""])
            }
        }
    }
}

struct Banner: View {
    enum Style { case error, warning }
    var style: Style
    var title: String
    var lines: [String]

    var body: some View {
        let color: Color = style == .error ? .red : .orange
        VStack(alignment: .leading, spacing: 4) {
            Label(title, systemImage: "exclamationmark.triangle.fill")
                .font(.callout.weight(.semibold))
            ForEach(lines.filter { !$0.isEmpty }, id: \.self) { line in
                Text(line).font(.callout)
            }
        }
        .foregroundStyle(color)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(color.opacity(0.1), in: .rect(cornerRadius: 10))
    }
}
