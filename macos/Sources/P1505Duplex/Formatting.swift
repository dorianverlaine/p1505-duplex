import Foundation

extension Locale {
    /// The app is Traditional Chinese only; format dates the same way
    /// whatever the system region is.
    static let app = Locale(identifier: "zh-Hant")
}

extension Date {
    /// 下午2:58
    var timeText: String { formatted(.dateTime.hour().minute().locale(.app)) }

    /// 9月30日 下午2:50
    var dateTimeText: String { formatted(.dateTime.month().day().hour().minute().locale(.app)) }

    /// 5 分鐘前
    var relativeText: String { formatted(.relative(presentation: .named).locale(.app)) }
}
