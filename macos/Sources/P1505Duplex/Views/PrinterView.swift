import DuplexKit
import SwiftUI

/// Everything the server knows about the printer, refreshed while visible.
struct PrinterView: View {
    @Environment(AppModel.self) private var model
    @State private var confirmTestPage = false

    var body: some View {
        VStack(spacing: 0) {
            PrinterBanner()
                .padding(.horizontal, 20)
                .padding(.top, 12)
            form
        }
    }

    private var form: some View {
        Form {
            if let p = model.snapshot?.printer {
                Section("狀態") {
                    LabeledContent("印表機", value: p.stateText)
                    LabeledContent("USB") {
                        Label(p.usbConnected ? "已連接" : "未連接",
                              systemImage: p.usbConnected ? "cable.connector" : "cable.connector.slash")
                            .foregroundStyle(p.usbConnected ? Color.primary : Color.red)
                    }
                    LabeledContent("接受新工作", value: p.accepting ? "是" : "否")
                    LabeledContent("排隊中的工作", value: "\(p.queued) 份")
                    LabeledContent("狀態變更", value: p.stateChanged.relativeText)
                }
            }

            if let d = model.details {
                Section("韌體") {
                    if let fw = d.firmware {
                        LabeledContent("最近載入") {
                            Text(fw.time.dateTimeText)
                        }
                        LabeledContent("結果") {
                            Label(fw.ok ? "成功（第 \(fw.attempts) 次）" : "失敗",
                                  systemImage: fw.ok ? "checkmark.circle.fill" : "xmark.circle.fill")
                                .foregroundStyle(fw.ok ? Color.green : Color.red)
                        }
                        if let error = fw.error {
                            Text(error).font(.caption).foregroundStyle(.red)
                        }
                    } else {
                        Text("還沒有載入紀錄。印表機下次開機時會自動載入。")
                            .foregroundStyle(.secondary)
                    }
                }

                Section("碳粉") {
                    let known = d.markers.filter { $0.level != nil }
                    if known.isEmpty {
                        Text("這台印表機不回報碳粉存量。")
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(known, id: \.name) { m in
                            LabeledContent(m.name) {
                                Gauge(value: Double(m.level ?? 0), in: 0...100) { Text("\(m.level ?? 0)%") }
                                    .gaugeStyle(.accessoryLinearCapacity)
                                    .frame(width: 120)
                            }
                        }
                    }
                }

                Section("正在列印與排隊中") {
                    if d.queue.isEmpty {
                        Text("沒有工作").foregroundStyle(.secondary)
                    }
                    ForEach(d.queue) { job in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(job.name).lineLimit(1)
                                Text("\(job.user) · \(job.state.text)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button("取消", role: .destructive) { model.cancelQueueJob(job.id) }
                                .buttonStyle(.borderless)
                                .disabled(model.isBusy("cancel-\(job.id)"))
                        }
                    }
                }

                Section("資訊") {
                    LabeledContent("型號", value: d.makeModel)
                    LabeledContent("位置", value: d.location)
                    if let dq = d.duplexQueue {
                        LabeledContent("手動雙面佇列", value: dq.accepting ? dq.stateText : "不接受工作")
                    }
                    LabeledContent("裝置") {
                        Text(d.deviceUri)
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                            .lineLimit(2)
                    }
                }
            } else if model.connected {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }

            Section {
                HStack {
                    if model.snapshot?.printer?.isPaused == true {
                        Button("恢復列印佇列", systemImage: "play.circle") { model.resumePrinter() }
                            .disabled(model.isBusy("resume"))
                    } else {
                        Button("暫停列印佇列", systemImage: "pause.circle") { model.pausePrinter() }
                            .disabled(model.isBusy("pause"))
                    }
                    Spacer()
                    Button("列印雙面測試頁", systemImage: "doc.text") { confirmTestPage = true }
                        .disabled(model.isBusy("test-page"))
                }
                .disabled(!model.connected)
            } footer: {
                Text("P1505 不會回報卡紙、缺紙、機蓋沒關等狀態，請看印表機面板上的指示燈。")
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
        .task {
            // Refresh while this tab is visible; the job list itself is live.
            while !Task.isCancelled {
                await model.refreshDetails()
                try? await Task.sleep(for: .seconds(5))
            }
        }
        .confirmationDialog("列印一份 5 頁的雙面測試頁？", isPresented: $confirmTestPage) {
            Button("列印") { model.printTestPage() }
        } message: {
            Text("用來確認背面的順序和方向：第 1 張背面應該是正向的 2，第 3 張背面是空白。")
        }
    }
}
