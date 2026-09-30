import Foundation

// Mirrors the server's JSON (server/src/status.rs, ipp.rs). Keys arrive in
// snake_case and are decoded with `.convertFromSnakeCase`.

/// Where a duplex job stands.
public enum Stage: String, Codable, Sendable {
    case frontPrinting = "front_printing"
    case frontFailed = "front_failed"
    case blocked
    case readyToFlip = "ready_to_flip"
    case backPrinting = "back_printing"
    case backFailed = "back_failed"
    case done
    case unknown

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = Stage(rawValue: raw) ?? .unknown
    }

    /// Something still has to happen before the job is finished.
    public var isActive: Bool {
        switch self {
        case .frontPrinting, .blocked, .readyToFlip, .backPrinting: true
        case .frontFailed, .backFailed, .done, .unknown: false
        }
    }

    public var isFailure: Bool { self == .frontFailed || self == .backFailed }
}

public enum JobState: String, Codable, Sendable {
    case pending, held, processing, stopped, canceled, aborted, completed, unknown

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = JobState(rawValue: raw) ?? .unknown
    }

    public var isFinal: Bool { self == .canceled || self == .aborted || self == .completed }
}

public struct Side: Codable, Sendable, Equatable {
    public var job: Int
    public var state: JobState
    public var sheetsDone: Int

    public init(job: Int, state: JobState, sheetsDone: Int) {
        self.job = job
        self.state = state
        self.sheetsDone = sheetsDone
    }
}

public struct DuplexJob: Codable, Sendable, Equatable, Identifiable {
    public var id: Int
    public var title: String
    public var user: String
    public var pages: Int
    public var sheets: Int
    public var created: Date
    public var front: Side?
    public var back: Side?
    public var stage: Stage

    public init(id: Int, title: String, user: String, pages: Int, sheets: Int, created: Date,
                front: Side?, back: Side?, stage: Stage) {
        self.id = id
        self.title = title
        self.user = user
        self.pages = pages
        self.sheets = sheets
        self.created = created
        self.front = front
        self.back = back
        self.stage = stage
    }
}

public enum Severity: String, Codable, Sendable {
    case error, warning, report
}

public struct Reason: Codable, Sendable, Equatable, Hashable {
    public var keyword: String
    public var text: String
    public var severity: Severity
}

public struct Printer: Codable, Sendable, Equatable {
    public var name: String
    /// idle, processing or stopped.
    public var state: String
    public var reasons: [Reason]
    public var message: String
    public var blocking: Bool
    public var accepting: Bool
    public var queued: Int
    public var usbConnected: Bool
    public var stateChanged: Date

    public init(name: String, state: String, reasons: [Reason], message: String, blocking: Bool,
                accepting: Bool, queued: Int, usbConnected: Bool, stateChanged: Date) {
        self.name = name
        self.state = state
        self.reasons = reasons
        self.message = message
        self.blocking = blocking
        self.accepting = accepting
        self.queued = queued
        self.usbConnected = usbConnected
        self.stateChanged = stateChanged
    }

    public var isPaused: Bool { state == "stopped" || reasons.contains { $0.keyword == "paused" } }
}

public struct Snapshot: Codable, Sendable, Equatable {
    public var printer: Printer?
    public var cupsError: String?
    /// Newest first.
    public var jobs: [DuplexJob]

    public init(printer: Printer?, cupsError: String?, jobs: [DuplexJob]) {
        self.printer = printer
        self.cupsError = cupsError
        self.jobs = jobs
    }
}

public struct QueueJob: Codable, Sendable, Equatable, Identifiable {
    public var id: Int
    public var name: String
    public var user: String
    public var state: JobState
    public var created: Date
    public var completed: Date?
    public var sheets: Int

    public init(id: Int, name: String, user: String, state: JobState, created: Date, completed: Date?, sheets: Int) {
        self.id = id
        self.name = name
        self.user = user
        self.state = state
        self.created = created
        self.completed = completed
        self.sheets = sheets
    }
}

public struct Marker: Codable, Sendable, Equatable {
    public var name: String
    public var level: Int?

    public init(name: String, level: Int?) {
        self.name = name
        self.level = level
    }
}

public struct FirmwareRecord: Codable, Sendable, Equatable {
    public var time: Date
    public var ok: Bool
    public var attempts: Int
    public var error: String?

    public init(time: Date, ok: Bool, attempts: Int, error: String?) {
        self.time = time
        self.ok = ok
        self.attempts = attempts
        self.error = error
    }
}

public struct PrinterDetails: Codable, Sendable, Equatable {
    public var printer: Printer?
    public var cupsError: String?
    public var makeModel: String
    public var info: String
    public var location: String
    public var deviceUri: String
    public var markers: [Marker]
    public var firmware: FirmwareRecord?
    public var duplexQueue: Printer?
    public var queue: [QueueJob]
    public var history: [QueueJob]

    public init(printer: Printer?, cupsError: String?, makeModel: String, info: String, location: String,
                deviceUri: String, markers: [Marker], firmware: FirmwareRecord?, duplexQueue: Printer?,
                queue: [QueueJob], history: [QueueJob]) {
        self.printer = printer
        self.cupsError = cupsError
        self.makeModel = makeModel
        self.info = info
        self.location = location
        self.deviceUri = deviceUri
        self.markers = markers
        self.firmware = firmware
        self.duplexQueue = duplexQueue
        self.queue = queue
        self.history = history
    }
}

public enum PrintSide: String, Codable, Sendable {
    case front, back
}

extension JSONDecoder {
    /// Decoder for the server's JSON: snake_case keys, Unix-second dates.
    public static var duplex: JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        d.dateDecodingStrategy = .secondsSince1970
        return d
    }
}
