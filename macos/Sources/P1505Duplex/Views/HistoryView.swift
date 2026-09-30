import DuplexKit
import SwiftUI

/// Recently finished jobs on the printer.
struct HistoryView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if let history = model.details?.history {
                if history.isEmpty {
                    ContentUnavailableView("還沒有列印紀錄", systemImage: "clock.arrow.circlepath")
                } else {
                    // A plain stack rather than List: List draws its own
                    // separators, which turn bright white on the popover's
                    // material; Divider adapts to it.
                    ScrollView {
                        let sheets = history.filter { $0.state == .completed }.map(\.sheets).reduce(0, +)
                        LazyVStack(alignment: .leading, spacing: 0) {
                            Text("最近 \(history.count) 份工作，共印了 \(sheets) 張")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .padding(.vertical, 10)
                            ForEach(history) { job in
                                Divider()
                                HistoryRow(job: job)
                                    .padding(.vertical, 8)
                            }
                        }
                        .padding(.horizontal, 16)
                        .padding(.bottom, 8)
                    }
                }
            } else if model.connected {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ContentUnavailableView("連不上列印伺服器", systemImage: "wifi.slash",
                                       description: Text(model.connectionError ?? ""))
            }
        }
        .task {
            while !Task.isCancelled {
                await model.refreshDetails()
                try? await Task.sleep(for: .seconds(10))
            }
        }
    }
}

struct HistoryRow: View {
    let job: QueueJob

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: icon)
                .foregroundStyle(color)
            VStack(alignment: .leading, spacing: 2) {
                Text(job.name).lineLimit(1)
                Text("\(job.user) · \(job.sheets) 張 · \(job.state.text(sheetsDone: job.sheets, sheets: job.sheets))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Text((job.completed ?? job.created).dateTimeText)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var icon: String {
        switch job.state {
        case .completed: "checkmark.circle.fill"
        case .canceled: "xmark.circle"
        case .aborted, .stopped: "exclamationmark.triangle.fill"
        default: "circle"
        }
    }

    private var color: Color {
        switch job.state {
        case .completed: .green
        case .aborted, .stopped: .red
        default: .secondary
        }
    }
}
