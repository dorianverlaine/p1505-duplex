import Foundation

/// Incremental parser for text/event-stream: feed it lines, get events.
public struct SSEParser: Sendable {
    public struct Event: Sendable, Equatable {
        public var name: String
        public var data: String
    }

    private var name = ""
    private var data: [String] = []

    public init() {}

    /// Feed one line (without its line break). Returns an event when the
    /// line is the blank line that ends one.
    public mutating func feed(_ line: String) -> Event? {
        if line.isEmpty {
            defer { name = ""; data = [] }
            guard !data.isEmpty else { return nil }
            return Event(name: name.isEmpty ? "message" : name, data: data.joined(separator: "\n"))
        }
        if line.hasPrefix(":") { return nil }  // comment / keep-alive
        let (field, value): (Substring, Substring)
        if let colon = line.firstIndex(of: ":") {
            field = line[..<colon]
            var v = line[line.index(after: colon)...]
            if v.hasPrefix(" ") { v = v.dropFirst() }
            value = v
        } else {
            field = Substring(line)
            value = ""
        }
        switch field {
        case "event": name = String(value)
        case "data": data.append(String(value))
        default: break  // id, retry
        }
        return nil
    }
}

public enum StreamUpdate: Sendable {
    case snapshot(Snapshot)
    /// The connection dropped or could not be made; retrying.
    case disconnected(String)
}

/// Snapshots from /api/events, reconnecting with backoff until cancelled.
public func snapshotStream(base: URL, session: URLSession = .shared) -> AsyncStream<StreamUpdate> {
    AsyncStream { continuation in
        let task = Task {
            var delay: Duration = .seconds(1)
            while !Task.isCancelled {
                do {
                    var req = URLRequest(url: base.appending(path: "api/events"))
                    req.timeoutInterval = 60  // the server sends a keep-alive every 15 s
                    req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    let (bytes, resp) = try await session.bytes(for: req)
                    if let http = resp as? HTTPURLResponse, http.statusCode != 200 {
                        throw APIError(message: "伺服器回應 HTTP \(http.statusCode)")
                    }
                    var parser = SSEParser()
                    // `lines` drops empty lines, so detect event ends ourselves.
                    var line = Data()
                    for try await byte in bytes {
                        if byte == UInt8(ascii: "\n") {
                            var text = String(decoding: line, as: UTF8.self)
                            if text.hasSuffix("\r") { text.removeLast() }
                            line.removeAll(keepingCapacity: true)
                            guard let event = parser.feed(text), event.name == "status" else { continue }
                            if let snap = try? JSONDecoder.duplex.decode(Snapshot.self, from: Data(event.data.utf8)) {
                                delay = .seconds(1)
                                continuation.yield(.snapshot(snap))
                            }
                        } else {
                            line.append(byte)
                        }
                    }
                    continuation.yield(.disconnected("連線中斷"))
                } catch is CancellationError {
                    break
                } catch {
                    if Task.isCancelled { break }
                    continuation.yield(.disconnected(error.localizedDescription))
                }
                try? await Task.sleep(for: delay)
                delay = min(delay * 2, .seconds(30))
            }
            continuation.finish()
        }
        continuation.onTermination = { _ in task.cancel() }
    }
}
