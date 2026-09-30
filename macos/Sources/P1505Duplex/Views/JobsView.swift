import DuplexKit
import SwiftUI

struct JobsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            VStack(spacing: 12) {
                PrinterBanner()
                if model.connected && model.jobs.isEmpty {
                    ContentUnavailableView {
                        Label("沒有進行中的雙面列印工作", systemImage: "printer")
                    } description: {
                        Text(model.settings.onlyMine
                             ? "列印到「P1505 手動雙面」後，工作會出現在這裡。只顯示 \(model.settings.userName) 送出的工作。"
                             : "列印到「P1505 手動雙面」後，工作會出現在這裡。")
                    }
                    .padding(.top, 40)
                }
                ForEach(model.jobs) { job in
                    JobCard(job: job)
                }
            }
            .padding(16)
        }
    }
}

struct JobCard: View {
    @Environment(AppModel.self) private var model
    let job: DuplexJob
    @State private var showTrouble = false
    @State private var confirmRemove = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(job.title)
                    .font(.headline)
                    .lineLimit(2)
                Text("\(job.pages) 頁 · \(job.sheets) 張紙 · \(job.user) · \(job.created.timeText)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 3) {
                GridRow {
                    Text("正面").foregroundStyle(.secondary)
                    Text(job.front?.text ?? "狀態不明")
                }
                GridRow {
                    Text("背面").foregroundStyle(.secondary)
                    Text(job.back?.text ?? "狀態不明")
                }
            }
            .font(.callout)

            if let hint = job.stage.hint(sheets: job.sheets) {
                Text(hint)
                    .font(.callout)
                    .foregroundStyle(hintColor)
                    .fixedSize(horizontal: false, vertical: true)
            }

            primaryButton

            DisclosureGroup("出問題了？", isExpanded: $showTrouble) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("卡紙、一次進了兩張紙，或某幾張印壞了，都可以只補印那幾張。紙張依正面出紙順序編號，第 1 張的正面是第 1 頁。")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    ReprintRow(job: job, side: .front)
                    ReprintRow(job: job, side: .back)
                    Button("取消並移除這份工作", role: .destructive) { confirmRemove = true }
                        .buttonStyle(.borderless)
                        .disabled(model.isBusy("remove-\(job.id)"))
                }
                .padding(.top, 6)
            }
            .font(.callout)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.05), in: .rect(cornerRadius: 12))
        .confirmationDialog("取消還沒印的部分並移除這份工作？", isPresented: $confirmRemove) {
            Button("取消並移除", role: .destructive) { model.remove(job.id) }
        }
    }

    private var hintColor: Color {
        switch job.stage {
        case .frontFailed, .backFailed, .blocked: .red
        case .done: .green
        default: .primary
        }
    }

    @ViewBuilder
    private var primaryButton: some View {
        switch job.stage {
        case .readyToFlip, .frontPrinting, .blocked, .frontFailed:
            Button {
                model.continueJob(job.id)
            } label: {
                Label("繼續列印背面", systemImage: "arrow.2.squarepath")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(job.stage != .readyToFlip || model.isBusy("continue-\(job.id)"))
        case .done:
            Button {
                model.remove(job.id)
            } label: {
                Text("完成，移除這份工作").frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
            .controlSize(.large)
            .disabled(model.isBusy("remove-\(job.id)"))
        case .backPrinting, .backFailed, .unknown:
            EmptyView()
        }
    }
}

/// "Reprint sheets first…last of one side".
struct ReprintRow: View {
    @Environment(AppModel.self) private var model
    let job: DuplexJob
    let side: PrintSide
    @State private var first = 1
    @State private var last = 1
    @State private var confirm = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if side == .back {
                Text("先把要補印背面的紙，照原本的方式放回紙匣再按。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 6) {
                Text("第")
                Stepper(value: $first, in: 1...job.sheets) { Text("\(first)").monospacedDigit() }
                Text("到第")
                Stepper(value: $last, in: 1...job.sheets) { Text("\(last)").monospacedDigit() }
                Text("張")
                Spacer()
                Button(side == .front ? "重印正面" : "重印背面") { confirm = true }
                    .disabled(first > last || model.isBusy("reprint-\(job.id)-\(side.rawValue)"))
            }
        }
        .onAppear { last = job.sheets }
        .confirmationDialog(
            side == .front ? "重印第 \(first)–\(last) 張的正面？" : "重印第 \(first)–\(last) 張的背面？",
            isPresented: $confirm
        ) {
            Button(side == .front ? "重印正面" : "重印背面") {
                model.reprint(job.id, side: side, first: first, last: last)
            }
        } message: {
            Text("目前這一面還沒印完的部分會先取消。")
        }
    }
}
