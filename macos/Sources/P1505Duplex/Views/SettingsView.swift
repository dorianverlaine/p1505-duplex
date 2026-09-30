import AppKit
import DuplexKit
import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var server = ""
    @State private var testResult: String?
    @State private var testing = false
    @State private var launchAtLogin = false

    var body: some View {
        @Bindable var settings = model.settings
        Form {
            Section {
                TextField("網址", text: $server, prompt: Text(AppSettings.defaultServer))
                    .onSubmit(apply)
                HStack {
                    Button("套用", action: apply)
                        .disabled(server == model.settings.serverURL)
                    Button("測試連線", action: test)
                        .disabled(testing)
                    if testing { ProgressView().controlSize(.small) }
                    Spacer()
                    if let testResult {
                        Text(testResult).font(.caption).foregroundStyle(.secondary)
                    }
                }
            } header: {
                Text("列印伺服器")
            } footer: {
                Text("網頁版「P1505 手動雙面」的網址，例如 \(AppSettings.defaultServer)")
            }

            Section("顯示") {
                Toggle("只顯示我送出的工作（\(model.settings.userName)）", isOn: $settings.onlyMine)
            }

            Section {
                Toggle("該翻面時", isOn: $settings.notifyFlip)
                Toggle("雙面列印完成時", isOn: $settings.notifyDone)
                Toggle("沒有印完、需要補印時", isOn: $settings.notifyFailure)
                Toggle("印表機無法列印或恢復時", isOn: $settings.notifyPrinter)
                Button("打開系統通知設定…") {
                    if let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension") {
                        NSWorkspace.shared.open(url)
                    }
                }
            } header: {
                Text("通知")
            } footer: {
                Text("「該翻面」的通知上有「繼續列印背面」按鈕，放好紙後直接按就行。")
            }

            Section("一般") {
                Toggle("登入時自動打開", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, on in model.settings.launchAtLogin = on }
                LabeledContent("版本", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—")
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear {
            server = model.settings.serverURL
            launchAtLogin = model.settings.launchAtLogin
        }
    }

    private func apply() {
        model.settings.serverURL = server
        testResult = nil
        model.restartStream()
    }

    private func test() {
        testing = true
        testResult = nil
        let candidate = AppSettings.normalize(server)
        Task {
            defer { testing = false }
            guard let base = candidate else {
                testResult = "網址不正確"
                return
            }
            do {
                let snap = try await APIClient(base: base).status()
                testResult = "連線成功 · 印表機\(snap.printer?.stateText ?? "狀態不明")"
            } catch {
                testResult = error.localizedDescription
            }
        }
    }
}
