import Foundation

public struct APIError: LocalizedError, Sendable {
    public var message: String
    public var errorDescription: String? { message }
}

/// Talks to the server's /api endpoints. `base` is the page URL, e.g.
/// http://athlon-server.local/duplex/.
public struct APIClient: Sendable {
    public var base: URL
    let session: URLSession

    public init(base: URL, session: URLSession = .shared) {
        self.base = base
        self.session = session
    }

    func url(_ path: String) -> URL {
        base.appending(path: "api").appending(path: path)
    }

    public func status() async throws -> Snapshot {
        try await get("status")
    }

    public func printer() async throws -> PrinterDetails {
        try await get("printer")
    }

    public func continueJob(_ id: Int) async throws {
        try await post("jobs/\(id)/continue")
    }

    public func reprint(_ id: Int, side: PrintSide, first: Int, last: Int) async throws {
        struct Body: Encodable { var side: PrintSide; var first: Int; var last: Int }
        try await post("jobs/\(id)/reprint", body: Body(side: side, first: first, last: last))
    }

    public func remove(_ id: Int) async throws {
        try await post("jobs/\(id)/remove")
    }

    public func cancelQueueJob(_ id: Int) async throws {
        try await post("queue/\(id)/cancel")
    }

    public func pausePrinter() async throws {
        try await post("printer/pause")
    }

    public func resumePrinter() async throws {
        try await post("printer/resume")
    }

    public func testPage(user: String) async throws {
        struct Body: Encodable { var user: String }
        try await post("test-page", body: Body(user: user))
    }

    // MARK: -

    private func get<T: Decodable>(_ path: String) async throws -> T {
        var req = URLRequest(url: url(path))
        req.timeoutInterval = 15
        let (data, resp) = try await send(req)
        try check(resp, data)
        return try JSONDecoder.duplex.decode(T.self, from: data)
    }

    private func post(_ path: String, body: (some Encodable)? = Optional<String>.none) async throws {
        var req = URLRequest(url: url(path))
        req.httpMethod = "POST"
        req.timeoutInterval = 30
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try JSONEncoder().encode(body)
        }
        let (data, resp) = try await send(req)
        try check(resp, data)
    }

    private func send(_ req: URLRequest) async throws -> (Data, URLResponse) {
        do {
            return try await session.data(for: req)
        } catch let e as URLError {
            throw APIError(message: "連不上列印伺服器（\(e.localizedDescription)）")
        }
    }

    private func check(_ resp: URLResponse, _ data: Data) throws {
        guard let http = resp as? HTTPURLResponse else { return }
        guard (200..<300).contains(http.statusCode) else {
            struct Body: Decodable { var error: String }
            let message = (try? JSONDecoder().decode(Body.self, from: data))?.error
            throw APIError(message: message ?? "伺服器回應 HTTP \(http.statusCode)")
        }
    }
}
