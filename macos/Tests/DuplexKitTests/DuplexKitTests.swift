import Foundation
import Testing
@testable import DuplexKit

// JSON in the shape the server sends (see server/src/status.rs).
let statusJSON = """
{"cups_error":null,
 "jobs":[{"id":92,"title":"報告","user":"dorian","pages":5,"sheets":3,"created":1790749315,
          "front":{"job":93,"state":"completed","sheets_done":3},
          "back":{"job":94,"state":"held","sheets_done":0},
          "stage":"ready_to_flip"}],
 "printer":{"accepting":true,"blocking":false,"message":"","name":"HP_LaserJet_P1505","queued":1,
            "reasons":[{"keyword":"toner-low-warning","text":"碳粉不足","severity":"warning"}],
            "state":"idle","state_changed":1790749768,"usb_connected":true}}
"""

let printerJSON = """
{"cups_error":null,"device_uri":"hp:/usb/X","make_model":"HP LaserJet p1505","info":"HP LaserJet P1505",
 "location":"Home","markers":[{"name":"Black","level":null}],
 "firmware":{"time":1790751005,"ok":true,"attempts":1,"error":null},
 "duplex_queue":null,"printer":null,
 "queue":[],
 "history":[{"id":103,"name":"x (背面)","user":"dorian","state":"completed","created":1790749494,"completed":1790749532,"sheets":3}]}
"""

@Test func decodesStatus() throws {
    let snap = try JSONDecoder.duplex.decode(Snapshot.self, from: Data(statusJSON.utf8))
    let job = try #require(snap.jobs.first)
    #expect(job.stage == .readyToFlip)
    #expect(job.front?.state == .completed)
    #expect(job.back?.job == 94)
    #expect(job.created == Date(timeIntervalSince1970: 1_790_749_315))
    #expect(snap.printer?.usbConnected == true)
    #expect(snap.printer?.reasons.first?.severity == .warning)
    #expect(snap.printer?.stateText == "待機")
}

@Test func decodesPrinterDetails() throws {
    let d = try JSONDecoder.duplex.decode(PrinterDetails.self, from: Data(printerJSON.utf8))
    #expect(d.makeModel == "HP LaserJet p1505")
    #expect(d.markers.first?.level == nil)
    #expect(d.firmware?.ok == true)
    #expect(d.history.first?.completed == Date(timeIntervalSince1970: 1_790_749_532))
}

@Test func unknownStagesAndStatesDecode() throws {
    let json = #"{"id":1,"title":"t","user":"u","pages":1,"sheets":1,"created":0,"front":{"job":1,"state":"weird","sheets_done":0},"back":null,"stage":"something_new"}"#
    let job = try JSONDecoder.duplex.decode(DuplexJob.self, from: Data(json.utf8))
    #expect(job.stage == .unknown)
    #expect(job.front?.state == .unknown)
}

@Test func parsesServerSentEvents() {
    var p = SSEParser()
    var events: [SSEParser.Event] = []
    for line in ["retry: 3000", "", ": keep-alive", "", "event: status", "data: {\"a\":1}", "", "data: x", "data: y", ""] {
        if let e = p.feed(line) { events.append(e) }
    }
    #expect(events == [
        .init(name: "status", data: "{\"a\":1}"),
        .init(name: "message", data: "x\ny"),
    ])
}

// MARK: - Notifications

func job(_ id: Int, _ stage: Stage, user: String = "dorian") -> DuplexJob {
    DuplexJob(id: id, title: "t\(id)", user: user, pages: 4, sheets: 2, created: .now,
              front: nil, back: nil, stage: stage)
}

func snapshot(_ jobs: [DuplexJob], blocking: Bool = false) -> Snapshot {
    let printer = Printer(name: "P", state: "idle", reasons: [], message: "", blocking: blocking,
                          accepting: true, queued: 0, usbConnected: true, stateChanged: .now)
    return Snapshot(printer: printer, cupsError: nil, jobs: jobs)
}

@Test func announcesFlipOnceAndWithdrawsIt() {
    var planner = NotificationPlanner()
    let prefs = NotificationPrefs()
    #expect(planner.update(with: snapshot([job(1, .frontPrinting)]), prefs: prefs).post.isEmpty)

    let flip = planner.update(with: snapshot([job(1, .readyToFlip)]), prefs: prefs)
    #expect(flip.post.count == 1)
    guard case .readyToFlip(let j) = flip.post.first else {
        Issue.record("expected a flip notice")
        return
    }
    #expect(j.id == 1)
    #expect(flip.post.first?.id == "flip-1")
    // Same stage again: nothing new.
    #expect(planner.update(with: snapshot([job(1, .readyToFlip)]), prefs: prefs).post.isEmpty)

    let moved = planner.update(with: snapshot([job(1, .backPrinting)]), prefs: prefs)
    #expect(moved.post.isEmpty)
    #expect(moved.withdraw == ["flip-1"])

    let done = planner.update(with: snapshot([job(1, .done)]), prefs: prefs)
    #expect(done.post.map(\.id) == ["done-1"])
}

@Test func firstSnapshotOnlyAnnouncesPendingFlips() {
    var planner = NotificationPlanner()
    let r = planner.update(with: snapshot([job(1, .readyToFlip), job(2, .done), job(3, .backFailed)], blocking: true),
                           prefs: NotificationPrefs())
    #expect(r.post.map(\.id) == ["flip-1"])
}

@Test func respectsUserFilterAndToggles() {
    var planner = NotificationPlanner()
    let mine = NotificationPrefs(onlyUser: "dorian")
    _ = planner.update(with: snapshot([]), prefs: mine)
    let r = planner.update(with: snapshot([job(1, .readyToFlip, user: "someone"), job(2, .readyToFlip)]), prefs: mine)
    #expect(r.post.map(\.id) == ["flip-2"])

    var quiet = NotificationPlanner()
    let off = NotificationPrefs(flip: false, done: false, failure: false, printer: false)
    _ = quiet.update(with: snapshot([]), prefs: off)
    #expect(quiet.update(with: snapshot([job(1, .readyToFlip)], blocking: true), prefs: off).post.isEmpty)
}

@Test func printerProblemsAndRecovery() {
    var planner = NotificationPlanner()
    let prefs = NotificationPrefs()
    _ = planner.update(with: snapshot([]), prefs: prefs)
    #expect(planner.update(with: snapshot([], blocking: true), prefs: prefs).post.map(\.id) == ["printer"])
    #expect(planner.update(with: snapshot([], blocking: true), prefs: prefs).post.isEmpty)
    #expect(planner.update(with: snapshot([], blocking: false), prefs: prefs).post == [.printerRecovered])
}

@Test func vanishedJobWithdrawsItsFlip() {
    var planner = NotificationPlanner()
    _ = planner.update(with: snapshot([job(1, .readyToFlip)]), prefs: NotificationPrefs())
    let r = planner.update(with: snapshot([]), prefs: NotificationPrefs())
    #expect(r.withdraw == ["flip-1"])
}

@Test func failuresAreAnnounced() {
    var planner = NotificationPlanner()
    _ = planner.update(with: snapshot([job(1, .backPrinting)]), prefs: NotificationPrefs())
    let r = planner.update(with: snapshot([job(1, .backFailed)]), prefs: NotificationPrefs())
    #expect(r.post.map(\.id) == ["failed-1"])
    #expect(r.post.first?.title == "背面沒有印完")
}

@Test func texts() {
    #expect(JobState.processing.text(sheetsDone: 2, sheets: 3) == "列印中 · 已印 2/3 張")
    #expect(Stage.readyToFlip.hint(sheets: 3) == "正面已印完：取出這 3 張紙，放回紙匣後按「繼續」。")
    #expect(Stage.backPrinting.hint(sheets: 3) == nil)
}

@Test func apiURLs() {
    let c = APIClient(base: URL(string: "http://h.local/duplex/")!)
    #expect(c.url("jobs/5/continue").absoluteString == "http://h.local/duplex/api/jobs/5/continue")
}
