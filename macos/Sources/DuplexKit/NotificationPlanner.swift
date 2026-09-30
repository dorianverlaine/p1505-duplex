import Foundation

/// A notification to show.
public enum Notice: Equatable, Sendable {
    /// Fronts are done: flip the stack and continue.
    case readyToFlip(DuplexJob)
    case done(DuplexJob)
    case failed(DuplexJob)
    /// The printer started blocking; its reasons.
    case printerProblem([String])
    case printerRecovered

    /// Identifier, so a later notice can replace or withdraw an earlier one.
    public var id: String {
        switch self {
        case .readyToFlip(let j): "flip-\(j.id)"
        case .done(let j): "done-\(j.id)"
        case .failed(let j): "failed-\(j.id)"
        case .printerProblem, .printerRecovered: "printer"
        }
    }
}

public struct NotificationPrefs: Sendable, Equatable {
    public var flip = true
    public var done = true
    public var failure = true
    public var printer = true
    /// Only jobs sent by this user; nil for everyone's.
    public var onlyUser: String?

    public init(flip: Bool = true, done: Bool = true, failure: Bool = true, printer: Bool = true, onlyUser: String? = nil) {
        self.flip = flip
        self.done = done
        self.failure = failure
        self.printer = printer
        self.onlyUser = onlyUser
    }

    public func includes(_ job: DuplexJob) -> Bool {
        onlyUser.map { $0 == job.user } ?? true
    }
}

/// Decides which notifications a new snapshot calls for, compared with the
/// previous one. On the first snapshot only pending flips are announced, so
/// launching the app points out a stack that is waiting.
public struct NotificationPlanner: Sendable {
    private var stages: [Int: Stage] = [:]
    private var blocking: Bool?

    public init() {}

    /// Notices to post, and identifiers of earlier notices to withdraw.
    public mutating func update(with snap: Snapshot, prefs: NotificationPrefs) -> (post: [Notice], withdraw: [String]) {
        let first = blocking == nil
        var post: [Notice] = []
        var withdraw: [String] = []

        for job in snap.jobs where prefs.includes(job) {
            let before = stages[job.id]
            if before == job.stage { continue }
            if before == .readyToFlip {
                withdraw.append("flip-\(job.id)")
            }
            switch job.stage {
            case .readyToFlip where prefs.flip:
                post.append(.readyToFlip(job))
            case .done where prefs.done && !first:
                post.append(.done(job))
            case .frontFailed, .backFailed:
                if prefs.failure && !first { post.append(.failed(job)) }
            default:
                break
            }
        }
        // Jobs that vanished (removed elsewhere) take their flip notice along.
        let current = Set(snap.jobs.map(\.id))
        for (id, stage) in stages where !current.contains(id) && stage == .readyToFlip {
            withdraw.append("flip-\(id)")
        }
        stages = Dictionary(uniqueKeysWithValues: snap.jobs.map { ($0.id, $0.stage) })

        let nowBlocking = snap.printer?.blocking ?? false
        if !first, prefs.printer, nowBlocking != blocking {
            if nowBlocking {
                let reasons = snap.printer?.reasons.map(\.text) ?? []
                post.append(.printerProblem(reasons))
            } else {
                post.append(.printerRecovered)
            }
        }
        blocking = nowBlocking
        return (post, withdraw)
    }
}
