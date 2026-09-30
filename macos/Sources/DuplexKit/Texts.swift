import Foundation

// User-facing wording, kept in line with the web page.

extension JobState {
    /// No page progress: CUPS' sheet counters depend on the driver (foomatic
    /// drivers count every page twice), so they are not shown.
    public var text: String {
        switch self {
        case .pending: "排隊中"
        case .held: "等待繼續"
        case .processing: "列印中"
        case .stopped: "已停止"
        case .canceled: "已取消"
        case .aborted: "失敗"
        case .completed: "已完成"
        case .unknown: "狀態不明"
        }
    }
}

extension Side {
    public var text: String { state.text }
}

extension Stage {
    /// One-line summary for menus and lists.
    public var summary: String {
        switch self {
        case .frontPrinting: "正面列印中"
        case .frontFailed: "正面沒有印完"
        case .blocked: "等印表機恢復"
        case .readyToFlip: "該翻面了"
        case .backPrinting: "背面列印中"
        case .backFailed: "背面沒有印完"
        case .done: "雙面列印完成"
        case .unknown: "狀態不明"
        }
    }

    /// The hint under a job, as on the web page.
    public func hint(sheets: Int) -> String? {
        switch self {
        case .readyToFlip: "正面已印完：取出這 \(sheets) 張紙，放回紙匣後按「繼續」。"
        case .frontPrinting: "正面還在列印，印完後再繼續。"
        case .blocked: "印表機目前無法列印，排除問題後再繼續。"
        case .frontFailed: "正面沒有印完。請用「出問題了？」補印正面，再繼續。"
        case .backFailed: "背面沒有印完。如果有紙卡住或一次進了兩張，用「出問題了？」補印。"
        case .done: "雙面列印完成。"
        case .backPrinting, .unknown: nil
        }
    }
}

extension Printer {
    public var stateText: String {
        switch state {
        case "processing": "列印中"
        case "stopped": "已停止"
        default: "待機"
        }
    }
}

extension Notice {
    public var title: String {
        switch self {
        case .readyToFlip: "該翻面了"
        case .done: "雙面列印完成"
        case .failed(let j): j.stage == .frontFailed ? "正面沒有印完" : "背面沒有印完"
        case .printerProblem: "印表機無法列印"
        case .printerRecovered: "印表機恢復正常"
        }
    }

    public var body: String {
        switch self {
        case .readyToFlip(let j): "「\(j.title)」的正面已印完。取出這 \(j.sheets) 張紙放回紙匣，再按「繼續列印背面」。"
        case .done(let j): "「\(j.title)」\(j.pages) 頁已經印好了。"
        case .failed(let j): "「\(j.title)」需要補印，打開視窗看看。"
        case .printerProblem(let reasons): reasons.isEmpty ? "請檢查印表機。" : reasons.joined(separator: "、")
        case .printerRecovered: "可以繼續列印了。"
        }
    }
}
