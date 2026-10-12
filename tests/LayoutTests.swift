// Lookout — Tests (assert-basiert, laufen unter -Onone: `bash build.sh --tests`).
// Kein Framework: die Layout-Mathematik und die Status-Parser sind reine Funktionen.

import ApplicationServices
import Cocoa

private var ran = 0
/// Optional ein Namensteil als Argument: nur passende Tests laufen (einzeln rot/gruen sehen).
private let only = CommandLine.arguments.dropFirst().first

private func test(_ name: String, _ body: () -> Void) {
    if let only, !name.contains(only) { return }
    body()
    ran += 1
    print("  ok  \(name)")
}

// ---------- Task 3: claude agents --json ----------

private let agentsFixture = """
[
  {"id":"6a9d0a2e","cwd":"/Users/test","kind":"background","startedAt":1782917161034,
   "sessionId":"6a9d0a2e-7f03-46d9-9fa1-9d2f471d2c1a","name":"Review drift alert","state":"blocked"},
  {"pid":99492,"cwd":"/Users/test/Projects/Alpha/Tools/Viewer","kind":"interactive",
   "startedAt":1789671613103,"sessionId":"1d53b4a7-34b1-4cf6-92bf-5425b62cedb5",
   "name":"warpgrid-0c","status":"busy"},
  {"pid":45880,"cwd":"/Users/test/Projects/Alpha","kind":"interactive","startedAt":1789720001713,
   "sessionId":"74d07d7b-5d79-4774-a049-e48fcd031fb2","name":"alpha-6b","status":"idle"},
  {"kind":"interactive","name":"ohne-session-id","status":"idle"}
]
"""

func testParseAgentsJson() {
    let s = parseAgentsJson(Data(agentsFixture.utf8))
    assert(s.count == 3, "Eintrag ohne sessionId muss verworfen werden, nicht raten (bekam \(s.count))")

    let background = s[0]
    assert(background.id == "claude:6a9d0a2e-7f03-46d9-9fa1-9d2f471d2c1a", "id = engine:sessionId")
    assert(background.pid == nil, "Hintergrund-Sessions haben keine pid")
    assert(background.ready, "state=blocked wartet auf Kevin -> ready")
    assert(background.alive)
    assert(background.startedAt == Date(timeIntervalSince1970: 1_782_917_161.034), "Startzeit aus startedAt (ms)")

    let busy = s[1]
    assert(busy.pid == 99492)
    assert(!busy.ready, "status=busy ist nicht ready")
    assert(busy.name == "warpgrid-0c")
    assert(busy.engine == .claude)

    let idle = s[2]
    assert(idle.pid == 45880)
    assert(idle.ready, "status=idle ist ready")

    // Kevin 2026-10-09: `shell` = Zug vorbei, nur Hintergrund-Befehle laufen; `busy` = das Modell arbeitet.
    let shell = parseAgent(["sessionId": "s", "pid": 7, "kind": "interactive", "status": "shell"])
    assert(shell?.backgroundOnly == true && shell?.ready == false, "shell: nicht fertig, nur Hintergrund")
    assert(!busy.backgroundOnly && !idle.backgroundOnly && !background.backgroundOnly, "sonst nie nur Hintergrund")
}

func testParseAgentsJsonRobust() {
    assert(parseAgentsJson(Data("kein json".utf8)).isEmpty, "Muell -> leere Liste, kein Absturz")
    assert(parseAgentsJson(Data("{}".utf8)).isEmpty, "Objekt statt Liste -> leer")
    assert(parseAgentsJson(Data("[]".utf8)).isEmpty)
}

// ---------- Status-Dateien statt Dauer-Abfrage (2026-09-27, Absturz-Diagnose) ----------

/// Schreibt eine Status-Datei, wie Claude sie anlegt (`<dir>/sessions/<pid>.json`).
private func writeSessionFile(_ dir: String, pid: Int32, sid: String, status: String,
                              startedAt: Date = Date(), name: String? = nil) {
    let json = """
    {"pid":\(pid),"sessionId":"\(sid)","cwd":"/tmp","startedAt":\(Int(startedAt.timeIntervalSince1970 * 1000)),\
    "kind":"interactive","entrypoint":"cli","name":"\(name ?? sid)","status":"\(status)"}
    """
    try! FileManager.default.createDirectory(atPath: dir + "/sessions", withIntermediateDirectories: true)
    try! json.write(toFile: "\(dir)/sessions/\(pid).json", atomically: false, encoding: .utf8)
}

func testReadSessionFiles() {
    let root = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    assert(readSessionFiles(root) == nil, "Ordner fehlt -> nil (alte Abfrage)")

    let me = getpid()
    writeSessionFile(root, pid: me, sid: "wartet", status: "waiting")
    writeSessionFile(root, pid: 99_999_990, sid: "fertig", status: "idle")   // Prozess gibt es nicht: bleibt
    writeSessionFile(root, pid: 99_999_991, sid: "laeuft", status: "shell")
    try! "{}".write(toFile: root + "/sessions/.syncthing.1.json.tmp", atomically: false, encoding: .utf8)
    try! "".write(toFile: root + "/sessions/99999992.json", atomically: false, encoding: .utf8)   // halb geschrieben
    let s = readSessionFiles(root) ?? []
    assert(s.count == 3, "drei gueltige Dateien, .tmp und leere Datei zaehlen nicht (bekam \(s.count))")
    let byName = Dictionary(uniqueKeysWithValues: s.map { ($0.name, $0) })
    assert(byName["wartet"]?.waiting == true && byName["wartet"]?.ready == true && byName["wartet"]?.pid == me)
    assert(byName["fertig"]?.ready == true && byName["fertig"]?.waiting == false)
    assert(byName["laeuft"]?.ready == false, "shell = Hintergrund-Befehl laeuft = arbeitet (wie busy)")
    assert(byName["wartet"]?.id == "claude:wartet", "dieselbe Kennung wie aus claude agents")

    // Liegengeblieben: dieselbe pid, aber der Eintrag ist aelter als der Prozess (Absturz, pid neu vergeben).
    writeSessionFile(root, pid: me, sid: "alt", status: "idle", startedAt: Date(timeIntervalSince1970: 1_000))
    assert(!(readSessionFiles(root) ?? []).contains { $0.name == "alt" }, "alter Eintrag auf neuer pid faellt weg")
    // Auch wenn die pid jetzt einem root-Prozess gehoert (launchd): libproc lieferte dafuer keine Startzeit (2026-09-28).
    writeSessionFile(root, pid: 1, sid: "root", status: "idle", startedAt: Date(timeIntervalSince1970: 1_000))
    assert(!(readSessionFiles(root) ?? []).contains { $0.name == "root" }, "alter Eintrag auf root-pid faellt weg")

    let broken = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: broken) }
    try! FileManager.default.createDirectory(atPath: broken + "/sessions", withIntermediateDirectories: true)
    assert(readSessionFiles(broken)?.isEmpty == true, "leerer Ordner = keine Session, KEIN Rueckfall")
    try! #"{"sessionId":"x"}"#.write(toFile: broken + "/sessions/1.json", atomically: false, encoding: .utf8)
    assert(readSessionFiles(broken) == nil, "Dateien da, keine lesbar (Format geaendert?) -> nil")
}

/// Kevin 2026-09-27: Status aus den Dateien, `claude agents` nur alle 5 Min. — dann auch die Hintergrund-Agenten.
func testClaudeStatusAsksRarely() {
    let root = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    let files = root + "/main", noFiles = root + "/work"
    try! FileManager.default.createDirectory(atPath: noFiles, withIntermediateDirectories: true)
    let me = getpid()
    writeSessionFile(files, pid: me, sid: "a", status: "busy")
    let agent = Session(id: "claude:bg", engine: .claude, pid: nil, name: "bg", cwd: "", ready: true, waiting: true,
                        alive: true)
    var calls: [String] = []
    var official: [String: [Session]] = [
        files: [agent, Session(id: "claude:a", engine: .claude, pid: me, name: "a", cwd: "", ready: false, alive: true)],
        noFiles: [Session(id: "claude:u", engine: .claude, pid: me, name: "u", cwd: "", ready: true, alive: true)],
    ]
    let status = ClaudeStatus(dirs: [files, noFiles]) { dir in calls.append(dir); return official[dir] }
    let t0 = Date(timeIntervalSince1970: 1_000_000)

    var s = status.sessions(now: t0)
    assert(calls == [files, noFiles], "erster Takt: offizielle Abfrage je Config-Dir (bekam \(calls))")
    assert(Set(s.map(\.id)) == ["claude:a", "claude:bg", "claude:u"], "Dateien + Hintergrund + Rueckfall-Dir")

    calls = []
    writeSessionFile(files, pid: me, sid: "a", status: "idle")
    // Die Abfrage liest dieselbe Datei — sie sagt ab jetzt ebenfalls "fertig".
    official[files]![1] = Session(id: "claude:a", engine: .claude, pid: me, name: "a", cwd: "", ready: true,
                                  alive: true)
    s = status.sessions(now: t0.addingTimeInterval(6))
    assert(calls == [noFiles], "Session-Takt: nur das Dir ohne Dateien fragt noch (bekam \(calls))")
    assert(s.first { $0.id == "claude:a" }?.ready == true, "Statuswechsel kommt aus der Datei")
    assert(s.contains { $0.id == "claude:bg" }, "Hintergrund-Agent bleibt aus der letzten Abfrage")

    calls = []
    _ = status.sessions(now: t0.addingTimeInterval(ClaudeStatus.officialEvery - 1))
    assert(calls == [noFiles], "vor Ablauf der 5 Min. keine offizielle Abfrage fuer das Dir mit Dateien")
    calls = []
    _ = status.sessions(now: t0.addingTimeInterval(ClaudeStatus.officialEvery))
    assert(calls == [files, noFiles], "nach 5 Min. wieder offiziell")

    // Abgleich: die Abfrage kennt eine laufende Session, die in den Dateien fehlt -> dort die alte Abfrage je Takt.
    official[files]!.append(Session(id: "claude:neu", engine: .claude, pid: me, name: "neu", cwd: "", ready: false,
                                    alive: true))
    s = status.sessions(now: t0.addingTimeInterval(2 * ClaudeStatus.officialEvery))
    assert(s.contains { $0.id == "claude:neu" }, "fehlende Session kommt aus der Abfrage")
    calls = []
    s = status.sessions(now: t0.addingTimeInterval(2 * ClaudeStatus.officialEvery + 6))
    assert(calls.contains(files) && s.contains { $0.id == "claude:neu" }, "bis zum naechsten Abgleich je Takt fragen")
    // Stimmen Dateien und Abfrage wieder ueberein, gilt wieder die Datei.
    writeSessionFile(files, pid: 99_999_993, sid: "neu", status: "busy")
    official[files]!.removeLast()
    official[files]!.append(Session(id: "claude:neu", engine: .claude, pid: 99_999_993, name: "neu", cwd: "",
                                    ready: false, alive: true))
    _ = status.sessions(now: t0.addingTimeInterval(3 * ClaudeStatus.officialEvery))
    calls = []
    _ = status.sessions(now: t0.addingTimeInterval(3 * ClaudeStatus.officialEvery + 6))
    assert(calls == [noFiles], "Abgleich stimmt wieder -> keine Abfrage mehr fuer dieses Dir (bekam \(calls))")
}

/// review quick 2026-09-27 (GLM): ein umbenanntes Status-Feld liesse jede Session dauerhaft "arbeiten" — weicht der
/// Zustand in zwei Abgleichen hintereinander ab, gilt die alte Abfrage; eine einzelne Abweichung (Wechsel zwischen den
/// Lesungen) nicht. Und: scheitert die Abfrage, verschwinden die Hintergrund-Agenten, statt ewig stehen zu bleiben.
func testClaudeStatusDistrustsWrongStates() {
    let dir = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let me = getpid()
    writeSessionFile(dir, pid: me, sid: "a", status: "busy")
    let agent = Session(id: "claude:bg", engine: .claude, pid: nil, name: "bg", cwd: "", ready: true, waiting: true,
                        alive: true)
    var answer: [Session]? = [agent, Session(id: "claude:a", engine: .claude, pid: me, name: "a", cwd: "", ready: true,
                                             alive: true)]
    var calls = 0
    let status = ClaudeStatus(dirs: [dir]) { _ in calls += 1; return answer }
    let t0 = Date(timeIntervalSince1970: 2_000_000)
    let at = { (minutes: Double) in t0.addingTimeInterval(minutes * 60) }

    _ = status.sessions(now: at(0))                 // 1. Abgleich: Datei "busy", Abfrage "idle"
    calls = 0
    var s = status.sessions(now: at(0.1))
    assert(calls == 0 && s.first { $0.id == "claude:a" }?.ready == false, "eine Abweichung: weiter aus der Datei")
    _ = status.sessions(now: at(5))                 // 2. Abgleich: weicht wieder ab
    calls = 0
    s = status.sessions(now: at(5.1))
    assert(calls == 1, "zweimal hintereinander abweichend -> alte Abfrage je Takt (bekam \(calls))")
    assert(s.first { $0.id == "claude:a" }?.ready == true, "Zustand kommt jetzt aus der Abfrage")

    writeSessionFile(dir, pid: me, sid: "a", status: "idle")   // stimmt wieder
    _ = status.sessions(now: at(10))
    calls = 0
    s = status.sessions(now: at(10.1))
    assert(calls == 0 && s.first { $0.id == "claude:a" }?.ready == true, "stimmt wieder -> zurueck zu den Dateien")
    assert(s.contains { $0.id == "claude:bg" }, "Hintergrund-Agent aus der letzten Abfrage")

    answer = nil                                    // Abfrage scheitert (Binary weg, Zeitlimit)
    s = status.sessions(now: at(15))
    assert(!s.contains { $0.id == "claude:bg" }, "gescheiterte Abfrage: keine veralteten Hintergrund-Agenten")
    assert(s.contains { $0.id == "claude:a" }, "Dateien tragen weiter")
}

/// review quick 2026-09-27 (Kimi): meldet `claude agents` einen Hintergrund-Agenten MIT pid, darf er weder als in den
/// Dateien fehlende Session gelten (Dauer-Rueckfall = wieder ~13 Starts je Minute) noch aus "Ohne Fenster" fallen.
func testBackgroundAgentWithPid() {
    let me = getpid()
    let json = #"[{"kind":"background","sessionId":"b2","pid":\#(me),"state":"blocked"},"# +
        #"{"kind":"interactive","sessionId":"a","pid":\#(me),"status":"idle"}]"#
    let parsed = parseAgentsJson(Data(json.utf8))
    assert(parsed.first { $0.id == "claude:b2" }?.pid == nil, "Hintergrund-Agent: seine pid zaehlt nicht")
    assert(parsed.first { $0.id == "claude:a" }?.pid == me, "interaktive Session behaelt ihre pid")

    let dir = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: dir) }
    writeSessionFile(dir, pid: me, sid: "a", status: "idle")
    var calls = 0
    let status = ClaudeStatus(dirs: [dir]) { _ in calls += 1; return parseAgentsJson(Data(json.utf8)) }
    let t0 = Date(timeIntervalSince1970: 3_000_000)
    _ = status.sessions(now: t0)
    calls = 0
    let s = status.sessions(now: t0.addingTimeInterval(6))
    assert(calls == 0, "kein Rueckfall wegen des Hintergrund-Agenten (bekam \(calls) Abfragen)")
    assert(s.contains { $0.id == "claude:b2" && $0.waiting }, "blockierter Hintergrund-Agent steht unter Ohne Fenster")
}

/// sysctl statt libproc (2026-09-28): auch root-Prozesse haben eine Startzeit — gegengeprueft an `ps` als unabhaengiger
/// Quelle (im Register pruefte ein Test, der die Startzeit aus der Funktion selbst ableitet, nichts; review quick 27.09.).
func testProcessStartDate() {
    let launchd = processStartDate(1)
    let ps = runCommand("/bin/ps", ["-o", "lstart=", "-p", "1"]).flatMap { String(data: $0, encoding: .utf8) }
    let fmt = DateFormatter()
    fmt.locale = Locale(identifier: "en_US_POSIX")
    fmt.dateFormat = "EEE MMM d HH:mm:ss yyyy"
    let fromPs = ps.flatMap { fmt.date(from: $0.split(whereSeparator: \.isWhitespace).joined(separator: " ")) }
    assert(launchd != nil, "launchd (root) hat eine Startzeit — libproc lieferte hier nil")
    assert(fromPs != nil && abs(launchd!.timeIntervalSince(fromPs!)) < 1,
           "Startzeit = ps -o lstart (bekam \(String(describing: launchd)), ps \(String(describing: ps)))")
    assert(processStartDate(getpid())! <= Date(), "eigener Prozess: Start liegt in der Vergangenheit")
    assert(processStartDate(99_999_990) == nil, "unbekannte pid -> nil")
    assert(processAlive(1) && processAlive(getpid()), "root-Prozess (kill: EPERM) lebt wie der eigene")
    assert(!processAlive(99_999_990) && !processAlive(0), "unbekannte pid und Prozessgruppe 0 leben nicht")
}

/// Kevin 2026-09-28: der 5-Min.-Abgleich prueft auch die Gegenrichtung — eine lebende Datei-Session, die `claude agents`
/// nicht kennt, heisst: den Dateien nicht trauen, bis der naechste Abgleich stimmt. Kurz vor der Abfrage gestartete
/// zaehlen nicht (koennen danach begonnen haben), beendete auch nicht.
func testClaudeStatusChecksBothWays() {
    let dir = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let me = getpid()
    let started = processStartDate(me)!   // echter Start: besteht die Pruefung auf neu vergebene pid
    writeSessionFile(dir, pid: me, sid: "a", status: "idle", startedAt: started)
    writeSessionFile(dir, pid: 99_999_994, sid: "tot", status: "idle", startedAt: Date(timeIntervalSince1970: 1_000))
    var answer: [Session] = []                      // die Abfrage kennt keine der beiden
    var calls = 0
    let status = ClaudeStatus(dirs: [dir]) { _ in calls += 1; return answer }
    let at = { (seconds: TimeInterval) in started.addingTimeInterval(seconds) }

    _ = status.sessions(now: at(30))                // 30 s nach dem Start: "a" zu jung, "tot" laeuft nicht
    calls = 0
    var s = status.sessions(now: at(36))
    assert(calls == 0 && s.contains { $0.id == "claude:a" }, "junge Session: weiter aus den Dateien (bekam \(calls))")

    _ = status.sessions(now: at(30 + ClaudeStatus.officialEvery))   // naechster Abgleich: "a" alt genug
    calls = 0
    s = status.sessions(now: at(36 + ClaudeStatus.officialEvery))
    assert(calls == 1, "lebende Datei-Session, die claude agents nicht kennt -> alte Abfrage je Takt (bekam \(calls))")
    assert(!s.contains { $0.id == "claude:a" }, "es gilt die Abfrage")

    answer = [Session(id: "claude:a", engine: .claude, pid: me, name: "a", cwd: "", ready: true, alive: true)]
    _ = status.sessions(now: at(30 + 2 * ClaudeStatus.officialEvery))
    calls = 0
    s = status.sessions(now: at(36 + 2 * ClaudeStatus.officialEvery))
    assert(calls == 0 && s.contains { $0.id == "claude:a" }, "stimmt wieder -> zurueck zu den Dateien (bekam \(calls))")
}

/// Eine unbrauchbare Antwort von `claude agents` gilt als gescheitert (nil), nicht als leere Liste — sonst hielte der
/// Gegenabgleich jede Datei-Session fuer unbekannt, und Lookout zeigte gar keine Claude-Session mehr.
func testAgentsReply() {
    assert(agentsReply(Data("kein json".utf8)) == nil, "Muell -> nil")
    assert(agentsReply(Data("{}".utf8)) == nil, "Objekt statt Liste -> nil")
    assert(agentsReply(Data("[]".utf8))?.isEmpty == true, "leere Liste bleibt leere Liste")
    assert(agentsReply(Data(#"[{"kind":"interactive","status":"idle"}]"#.utf8)) == nil,
           "Eintraege da, keiner lesbar (Format geaendert?) -> nil")
    assert(agentsReply(Data(agentsFixture.utf8))?.count == 3, "ein unlesbarer Eintrag unter lesbaren faellt nur weg")
}

/// Rahmen und Schilder nur neu einordnen, wenn ein fremdes Fenster dazwischen liegt (WindowServer-Fehler je Einordnen).
func testSitsAbove() {
    let r = CGRect(x: 0, y: 0, width: 10, height: 10)
    let me = getpid()
    let panel = OnScreenWindow(number: 1, pid: me, rect: r), other = OnScreenWindow(number: 2, pid: me, rect: r)
    let term = OnScreenWindow(number: 9, pid: 500, rect: r), chrome = OnScreenWindow(number: 7, pid: 600, rect: r)
    assert(sitsAbove(1, 9, in: [chrome, panel, term]), "direkt darueber, fremdes Fenster davor stoert nicht")
    assert(sitsAbove(1, 9, in: [panel, other, term]), "dazwischen nur eigene Panels")
    assert(!sitsAbove(1, 9, in: [term, panel]), "Terminal angeklickt: liegt ueber dem Panel")
    assert(!sitsAbove(1, 9, in: [panel, chrome, term]), "fremdes Fenster dazwischen")
    assert(!sitsAbove(1, 9, in: [term]), "Panel noch nicht auf dem Schirm")
    assert(!sitsAbove(1, 9, in: [panel]), "Terminal nicht in der Liste")
    assert(!sitsAbove(1, 9, in: nil), "Liste unbekannt: wie frueher einordnen")
}

// ---------- Task 4: Zug-Ende aus Session-Logs ----------

/// Echte Ereignis-Vokabulare, am 2026-09-18 aus Kevins Sessions erhoben.
private let codexWorking = """
{"type":"event_msg","payload":{"type":"task_complete","completed_at":1}}
{"type":"event_msg","payload":{"type":"task_started"}}
{"type":"response_item","payload":{"type":"reasoning"}}
"""
private let codexDone = """
{"type":"event_msg","payload":{"type":"task_started"}}
{"type":"response_item","payload":{"type":"message"}}
{"type":"event_msg","payload":{"type":"task_complete","completed_at":1789722036}}
{"type":"token_usage_record"}
"""
private let grokDone = """
{"ts":"2026-09-18T09:00:00Z","type":"turn_started","turn_number":0}
{"ts":"2026-09-18T09:00:48Z","type":"phase_changed","phase":"streaming_text"}
{"ts":"2026-09-18T09:00:48Z","type":"turn_ended"}
"""
private let kimiWorking = """
{"type":"agent.turn.ended","agentId":"main"}
{"type":"agent.turn.started","agentId":"main"}
{"type":"llm.request"}
"""

func testTurnStateFromTail() {
    assert(lastTurnState(in: codexDone, engine: .codex) == .done, "letztes Zug-Ereignis ist task_complete")
    assert(lastTurnState(in: codexWorking, engine: .codex) == .working,
           "task_started NACH task_complete heisst: arbeitet wieder")
    assert(lastTurnState(in: grokDone, engine: .grok) == .done)
    assert(lastTurnState(in: kimiWorking, engine: .kimi) == .working)
    assert(lastTurnState(in: "", engine: .codex) == .unknown, "leer -> unbekannt, nicht 'fertig'")
    assert(lastTurnState(in: "{kaputt\n{\"type\":\"turn_ended\"}", engine: .grok) == .done,
           "kaputte Zeile darf die Auswertung nicht kippen")
}

func testLogReadyClassification() {
    let now = Date()
    assert(classifyLogReady(lastWrite: now.addingTimeInterval(-120), now: now,
                            thresholdSec: 90, procActive: false),
           "Ruhe ueber der Schwelle und kein Prozess-Signal -> fertig")
    assert(!classifyLogReady(lastWrite: now.addingTimeInterval(-30), now: now,
                             thresholdSec: 90, procActive: false),
           "noch keine 90 s Ruhe -> nicht fertig")
    assert(!classifyLogReady(lastWrite: now.addingTimeInterval(-600), now: now,
                             thresholdSec: 90, procActive: true),
           "Prozess rechnet -> nicht fertig, egal wie lange die Datei ruht")
}

// ---------- Task 5: Session an Prozess haengen ----------

func testAttachProcesses() {
    let procs = [
        EngineProc(pid: 100, engine: .claude, tty: "ttys001", cwd: "/Users/test/Projects/Alpha"),
        EngineProc(pid: 200, engine: .codex, tty: "ttys003", cwd: "/Users/test/Projects/Alpha"),
    ]
    let sessions = [
        // interaktiv, pid bekannt -> bleibt
        Session(id: "claude:a", engine: .claude, pid: 100, name: "a", cwd: "", ready: true, alive: true),
        // interaktiv, pid laeuft nicht mehr -> raus
        Session(id: "claude:b", engine: .claude, pid: 999, name: "b", cwd: "", ready: true, alive: true),
        // Hintergrundagent ohne pid -> bleibt, aber OHNE erratene pid
        Session(id: "claude:c", engine: .claude, pid: nil, name: "c",
                cwd: "/Users/test/Projects/Alpha", ready: true, alive: true),
        // Codex-Log-Session -> ueber den Arbeitsordner an den Prozess
        Session(id: "codex:d", engine: .codex, pid: nil, name: "d",
                cwd: "/Users/test/Projects/Alpha", ready: true, alive: true),
        // Codex-Log-Session ohne laufenden Prozess -> raus
        Session(id: "codex:e", engine: .codex, pid: nil, name: "e", cwd: "/tmp/weg", ready: true, alive: true),
    ]
    let live = attachProcesses(sessions, procs)
    let ids = live.map(\.id)
    assert(ids == ["claude:a", "claude:c", "codex:d"], "bekam \(ids)")
    assert(live.first { $0.id == "claude:c" }?.pid == nil,
           "Hintergrundagent darf keine geratene pid bekommen — sonst landet er in einem fremden Fenster")
    assert(live.first { $0.id == "codex:d" }?.pid == 200)
}

/// Abnahme 2026-09-18: eine Session, gestartet als nacktes `claude` (ohne Pfad), blieb
/// "Ohne Fenster" — ps rueckt den Namen mit zwei Leerzeichen ein, und " claude" war keine Engine.
/// Plan 1a/1e (2026-09-26): jede Zeile wird gelesen (pid, ppid, tty, Rechenzeit, Name) — die Elternkarte braucht auch
/// Shell und Warp-Helfer; ob es eine KI-CLI mit Terminal ist, entscheidet danach `engine(of:)`.
func testParsePsLine() {
    let bare = parsePsLine(" 7087  6746 ttys003    0:00.12 claude")
    assert(bare == PsRow(pid: 7087, ppid: 6746, tty: "ttys003", cpu: 0.12, comm: "claude"), "nackter Name, bekam \(String(describing: bare))")
    assert(bare.flatMap(engine(of:)) == .claude)
    let path = parsePsLine("29834 29800 ttys000  337:28.94 /Users/test/.local/bin/claude")
    assert(path?.pid == 29834 && path?.ppid == 29800 && path.flatMap(engine(of:)) == .claude, "voller Pfad")
    assert(abs((path?.cpu ?? 0) - 20_248.94) < 0.001, "Rechenzeit in Minuten:Sekunden, bekam \(String(describing: path?.cpu))")
    assert(parsePsLine(" 1234  1200 ttys009    0:01.00 codex").flatMap(engine(of:)) == .codex)
    let noTty = parsePsLine("20894     1 ??         2:10.00 /Applications/ChatGPT.app/Contents/Resources/codex")
    assert(noTty != nil && engine(of: noTty!) == nil, "gelesen, aber ohne Terminal kein Fenster")
    let npm = parsePsLine(" 7174  6746 ttys003    0:00.40 npm exec @playwright/mcp@0.0.79 --browser chrome")
    assert(npm?.comm == "npm exec @playwright/mcp@0.0.79 --browser chrome" && engine(of: npm!) == nil, "keine KI-CLI, Name mit Leerzeichen")
    let shell = parsePsLine(" 6746  6700 ttys003    0:00.10 -zsh")
    assert(shell?.ppid == 6700 && engine(of: shell!) == nil, "Shell: nur fuer die Elternkarte")
    assert(parsePsLine("PID PPID TT TIME COMMAND") == nil && parsePsLine("kaputt") == nil, "unlesbar -> nil")
    assert(parsePsLine("  900   800 ttys004 1-02:03:04 codex")?.cpu == 93_784, "Tage-Format")
}

// ---------- Plan 1a: fremde Terminals in Ruhe lassen (2026-09-26) ----------

/// Live 2026-09-26: claude ← -zsh ← Warp-Helfer ← Warp ← launchd (beide Warp-Prozesse heissen in ps `stable`).
/// Dazu eine Session in iTerm, eine Waise (Eltern-pid fehlt) und eine Schleife in der Elternkarte.
private let psFixture = """
    1     0 ??       337:28.94 /sbin/launchd
  500     1 ??        61:02.10 /Applications/Warp.app/Contents/MacOS/stable
  510   500 ??         4:11.00 /Applications/Warp.app/Contents/MacOS/stable
  520   510 ttys001    0:00.10 -zsh
  530   520 ttys001    3:05.50 /Users/test/.local/bin/claude
  600     1 ??         0:30.00 /Applications/iTerm.app/Contents/MacOS/iTerm2
  610   600 ttys002    0:00.10 -zsh
  620   610 ttys002    0:02.00 claude
  700   999 ttys003    0:00.20 claude
  800   801 ttys004    0:00.20 codex
  801   800 ttys004    0:00.10 -zsh
  900   530 ttys001    0:00.40 npm exec @playwright/mcp@0.0.79 --browser chrome
"""

/// Review-Fund 8 (25.09.): KI-Prozesse in VS Code, iTerm und Terminal.app wurden alle 2 min angepingt — ihr Tab-Titel
/// blinkte und sprang auf den Ordnernamen. `inTerminal`: laeuft der Prozess unter Warp oder Ghostty?
func testProcessTreeInTerminal() {
    guard let list = processList(psFixture, terminalPids: [500], cwd: { _ in "" }) else { return assert(false, "lesbar") }
    let procs = Dictionary(uniqueKeysWithValues: list.procs.map { ($0.pid, $0) })
    assert(Set(procs.keys) == [530, 620, 700, 800], "nur KI-CLIs mit Terminal, bekam \(procs.keys.sorted())")
    assert(procs[530]?.inTerminal == true, "Warp-Ahne: Shell und Warp-Helfer stehen nur in der Elternkarte")
    assert(procs[620]?.inTerminal == false, "iTerm-Ahne")
    assert(procs[700]?.inTerminal == false, "Waise")
    assert(procs[800]?.inTerminal == false, "Schleife bricht ab")
    assert(list.rows.count == 12, "alle Zeilen bleiben fuer Elternkarte und Rechenzeit")

    let parents: [pid_t: pid_t] = [3: 2, 2: 1, 1: 0, 7: 8, 8: 7]
    assert(hasAncestor(3, in: [1], parents: parents))
    assert(!hasAncestor(1, in: [1], parents: parents), "nur Ahnen, nicht der Prozess selbst")
    assert(!hasAncestor(7, in: [1], parents: parents), "Schleife")
    let deep = Dictionary(uniqueKeysWithValues: (1...40).map { (pid_t($0), pid_t($0 - 1)) })
    assert(hasAncestor(40, in: [9], parents: deep) && !hasAncestor(40, in: [1], parents: deep), "hoechstens 32 Stufen")

    // R2-8: unlesbar heisst "unbekannt" — eine leere Liste loeschte jede Zuordnung.
    assert(processList("Fehler: kaputt\nnoch mehr", terminalPids: [500]) == nil, "nicht leer, aber keine Zeile lesbar -> nil")
    assert(processList("", terminalPids: [500])?.procs.isEmpty == true, "leer gelesen -> keine Prozesse")
    assert(processList("    1     0 ??  0:01.00 /sbin/launchd", terminalPids: [])?.procs.isEmpty == true,
           "lesbar, aber keine KI-CLI -> leer")
}

/// Plan 1a (GP 1, R2-8): eine Session in einem fremden Terminal bleibt in der Liste ("Ohne Fenster"), wird nie gepingt
/// und blockiert die Namens-Zuordnung nicht — heute blockierte sie ohne Titel alle Namens-Treffer.
func testForeignTerminalLeftAlone() {
    let procs = processList(psFixture, terminalPids: [500], cwd: { _ in "" })!.procs
    var warp = Session(id: "claude:w", engine: .claude, pid: 530, name: "w", cwd: "", ready: false, alive: true)
    warp.digest = Digest(title: "Lookout bauen")
    let iterm = Session(id: "claude:i", engine: .claude, pid: 620, name: "i", cwd: "", ready: false, alive: true)
    let live = attachProcesses([warp, iterm], procs)
    assert(live.map(\.id) == ["claude:w", "claude:i"], "die fremde Session bleibt gelistet, bekam \(live.map(\.id))")
    let terminal = procs.filter(\.inTerminal)
    let s = pingSchedule(pids: terminal.map(\.pid), mapped: [], pending: [:], newWindow: true, now: Date())
    assert(s.due == [530], "nur Prozesse unter Warp/Ghostty werden angepingt, bekam \(s.due)")
    let wins = [tabWindow([], title: "◐ Lookout bauen")]
    // review quick 2026-09-26 (Kimi): die Funktion selbst laesst fremde Prozesse aussen vor — auch mit der vollen Liste.
    let m = titleMatches(sessions: live, windows: wins, procs: procs)
    assert(m == [530: 0], "fremde Session ohne Titel blockiert die Namens-Zuordnung nicht, bekam \(m)")
}

/// Kevin 2026-09-18 zu "Ohne Fenster (35)": "Nur Echtes". Gezeigt wird, was laeuft und kein
/// Fenster hat, dazu frische Hintergrund-Agenten; beendete Log-Laeufe fallen weg (die hat
/// `attachProcesses` schon verworfen), alte Agenten zaehlen nur noch mit.
func testWindowlessSessions() {
    let now = Date(timeIntervalSince1970: 2_000_000)
    func s(_ id: String, pid: pid_t?, started: TimeInterval? = nil) -> Session {
        Session(id: id, engine: .claude, pid: pid, name: id, cwd: "", ready: true, alive: true,
                startedAt: started.map { now.addingTimeInterval(-$0) })
    }
    let live = [s("tab", pid: 10), s("imFenster", pid: 11), s("frisch", pid: nil, started: 3600),
                s("alt", pid: nil, started: 30 * 86_400), s("ohneZeit", pid: nil)]
    let r = windowlessSessions(live, inWindows: ["imFenster"], now: now)
    assert(r.shown.map(\.id) == ["tab", "frisch"], "bekam \(r.shown.map(\.id))")
    assert(r.staleAgents == 2, "alte und zeitlose Agenten zaehlen nur mit, bekam \(r.staleAgents)")
}

// ---------- Koordinaten (Konvention 1) ----------

func testCoordinateRoundTrip() {
    let screenH: CGFloat = 1269
    let cocoa = CGRect(x: 100, y: 200, width: 800, height: 600)
    let ax = cocoaToAX(cocoa, .topLeft, screenH: screenH)
    assert(ax.origin.y == screenH - cocoa.maxY, "AX-y misst von oben")
    assert(ax.origin.x == cocoa.origin.x, "x bleibt x")
    assert(axToCocoa(ax, .topLeft, screenH: screenH) == cocoa, "Hin und zurueck muss identisch sein")

    let unchanged = cocoaToAX(cocoa, .cocoaBottomLeft, screenH: screenH)
    assert(unchanged == cocoa, "ohne Umrechnung bleibt das Rechteck gleich")
}

func testFramesDiffer() {
    let a = CGRect(x: 0, y: 0, width: 100, height: 100)
    assert(!framesDiffer(a, a.offsetBy(dx: 20, dy: 0)), "20 pt liegen in der Toleranz")
    assert(framesDiffer(a, a.offsetBy(dx: 30, dy: 0)), "30 pt sind eine echte Abweichung")
}

// ---------- Task 6/7: Layout-Mathematik (alles Cocoa, Slot 0 oben-links) ----------

private let vis = CGRect(x: 0, y: 38, width: 1512, height: 982)

func testGrid22() {
    let f = gridLayout(vis, cols: 2, rows: 2, side: .right, ids: ["a", "b", "c", "d"])
    assert(f.frames.count == 4 && f.overflow.isEmpty)
    assert(f.frames["a"] == CGRect(x: 0, y: 529, width: 756, height: 491), "bekam \(f.frames["a"]!)")
    assert(f.frames["a"]!.maxY == 1020, "Slot 0 liegt oben")
    assert(f.frames["d"]!.minY == 38, "letzter Slot schliesst unten buendig ab")
}

func testGrid22OverflowReservesRail() {
    let f = gridLayout(vis, cols: 2, rows: 2, side: .right, ids: ["a", "b", "c", "d", "e"])
    assert(f.overflow == ["e"])
    assert(f.frames["a"]!.width == (vis.width - miniRailWidth(vis)) / 2,
           "erst den Streifen abziehen, dann durch die Spalten teilen")
    assert(miniStack(vis, side: .right, ids: f.overflow)["e"]!.width == miniRailWidth(vis))
}

func testFocusFills() {
    let (m, s) = focusLayout(vis, side: .right)
    assert(m.maxX == s.minX, "kein Spalt zwischen Hauptfenster und Streifen")
    assert(m.minX == vis.minX)
    assert(m.height == vis.height)
}

func testRailOverflowCap() {
    let many = (0..<20).map { "w\($0)" }
    let m = miniStack(vis, side: .right, ids: many)
    assert(m.count < 20, "ueberzaehlige bleiben unberuehrt statt unsichtbar gestapelt")
    for r in m.values { assert(r.height >= MIN_CARD_H, "keine Karte unter der Mindesthoehe") }
}

func testAttentionOrderReadyQueue() {
    // Kevin 2026-09-18: wer SPAETER fertig wird, stellt sich hinten an — nicht vor die,
    // die schon warten.
    let t0 = Date(timeIntervalSince1970: 1000)
    let frueh = fakeWindow(ready: true, firstSeen: t0, readySince: Date(timeIntervalSince1970: 5000))
    let spaet = fakeWindow(ready: true, firstSeen: t0, readySince: Date(timeIntervalSince1970: 9000))
    let arbeitet = fakeWindow(ready: false, firstSeen: t0)
    assert(attentionOrder([spaet, frueh, arbeitet]) == [1, 0, 2],
           "zuerst fertig steht vorn, dann der spaetere, arbeitende hinten")
}

func testFocusLeftSide() {
    let (m, s) = focusLayout(vis, side: .left)
    assert(s.minX == vis.minX && s.maxX == m.minX, "Streifen links, Hauptfenster rechts daneben")
}

// --- Reihenfolge "wer wartet auf Kevin" ---

private func fakeWindow(ready: Bool, firstSeen: Date, readySince: Date? = nil,
                       waiting: Bool = false, hasSession: Bool = true) -> TermWindow {
    let s = Session(id: "claude:x\(firstSeen.timeIntervalSince1970)-\(waiting)", engine: .claude,
                    pid: 1, name: "x", cwd: "", ready: ready, waiting: waiting, alive: true)
    return TermWindow(app: "Warp", bundleId: WARP_BID, ownerPid: 0, ax: AXUIElementCreateApplication(0),
                      frame: .zero, title: "", sessions: hasSession ? [s] : [],
                      firstSeen: firstSeen, readySince: ready ? (readySince ?? firstSeen) : nil,
                      movable: true, minimized: false, fullscreen: false)
}

func testAttentionOrderReadyFirst() {
    let old = Date(timeIntervalSince1970: 1000)
    let new = Date(timeIntervalSince1970: 2000)
    let windows = [fakeWindow(ready: false, firstSeen: old), fakeWindow(ready: true, firstSeen: new)]
    assert(attentionOrder(windows) == [1, 0], "fertiges Fenster rueckt nach vorn")

    let both = [fakeWindow(ready: true, firstSeen: new), fakeWindow(ready: true, firstSeen: old)]
    assert(attentionOrder(both) == [1, 0], "bei Gleichstand entscheidet die Erstsichtung")
}

func testWaitingBeatsEverything() {
    // Kevin 2026-09-18: "wenn eine frage gestellt wird an mich ... das hat prio 1".
    let t0 = Date(timeIntervalSince1970: 1000)
    let laengstFertig = fakeWindow(ready: true, firstSeen: t0,
                                   readySince: Date(timeIntervalSince1970: 2000))
    let fragt = fakeWindow(ready: true, firstSeen: t0,
                           readySince: Date(timeIntervalSince1970: 9000), waiting: true)
    let arbeitet = fakeWindow(ready: false, firstSeen: t0)
    assert(attentionOrder([laengstFertig, fragt, arbeitet]) == [1, 0, 2],
           "die Frage steht vorn, obwohl die andere Session viel laenger fertig ist")
    assert(fragt.isWaiting && !laengstFertig.isWaiting)
}

func testWaitingFromPermissionEvents() {
    // Grok schreibt permission_requested/_resolved (live gemessen 2026-09-18).
    let offen = #"{"type":"turn_started"}"# + "\n" + #"{"type":"permission_requested","tool":"bash"}"#
    let beantwortet = #"{"type":"permission_requested","tool":"bash"}"# + "\n"
        + #"{"type":"permission_resolved","granted":true}"#
    assert(lastTurnState(in: offen, engine: .grok) == .waiting, "offene Rueckfrage")
    assert(lastTurnState(in: beantwortet, engine: .grok) == .working, "beantwortet -> arbeitet weiter")

    // Muster statt fester Namen: auch Codex-artige Benennungen greifen.
    let codex = #"{"type":"event_msg","payload":{"type":"task_started"}}"# + "\n"
        + #"{"type":"event_msg","payload":{"type":"exec_approval_request"}}"#
    assert(lastTurnState(in: codex, engine: .codex) == .waiting)

    // Hintergrund-Agent mit state=blocked ist ebenfalls eine offene Frage.
    let blocked = parseAgentsJson(Data(#"[{"kind":"background","sessionId":"b1","state":"blocked"}]"#.utf8))
    assert(blocked.first?.waiting == true && blocked.first?.ready == true)
    let busy = parseAgentsJson(Data(#"[{"kind":"interactive","sessionId":"i1","status":"busy"}]"#.utf8))
    assert(busy.first?.waiting == false)

    // Interaktive Session mit offener Rueckfrage: eigener Status `waiting`
    // (live gemessen 2026-09-18).
    let fragt = parseAgentsJson(Data(#"[{"kind":"interactive","sessionId":"i2","pid":1,"status":"waiting"}]"#.utf8))
    assert(fragt.first?.waiting == true, "status=waiting ist eine offene Frage")
    assert(fragt.first?.ready == true, "wer wartet, arbeitet nicht — zaehlt also als bereit")
    let idle = parseAgentsJson(Data(#"[{"kind":"interactive","sessionId":"i3","pid":1,"status":"idle"}]"#.utf8))
    assert(idle.first?.ready == true && idle.first?.waiting == false)
}

func testGridShrinksToWindowCount() {
    // Ein Fenster bekommt den ganzen Schirm, nicht ein Viertel.
    let one = gridLayout(vis, cols: 2, rows: 2, side: .right, ids: ["a"])
    assert(one.frames["a"] == vis, "bekam \(one.frames["a"]!)")
    // Zwei Fenster teilen sich nebeneinander, volle Hoehe.
    let two = gridLayout(vis, cols: 2, rows: 2, side: .right, ids: ["a", "b"])
    assert(two.frames["a"]!.height == vis.height && two.frames["b"]!.height == vis.height)
    assert(two.frames["a"]!.maxX == two.frames["b"]!.minX, "keine Luecke dazwischen")
    // Drei in 3x2: eine Reihe, volle Hoehe.
    let three = gridLayout(vis, cols: 3, rows: 2, side: .right, ids: ["a", "b", "c"])
    assert(three.frames["c"]!.height == vis.height)
    assert(three.overflow.isEmpty)
}

// ---------- Bildschirm-Zuordnung (Review-Fund GLM) ----------

func testScreenIndexByOverlap() {
    let haupt = CGRect(x: 0, y: 0, width: 3008, height: 1153)
    let zweit = CGRect(x: 687, y: -1117, width: 1728, height: 1085)
    let screens = [haupt, zweit]

    assert(screenIndex(of: CGRect(x: 100, y: 100, width: 800, height: 600), screens: screens) == 0)
    assert(screenIndex(of: CGRect(x: 800, y: -900, width: 800, height: 600), screens: screens) == 1)
    // Mittelpunkt im Dock-Streifen (nicht Teil von visibleFrame) — darf NICHT auf Schirm 0
    // zurueckfallen, sonst reisst es das Fenster vom Zweitmonitor herueber.
    let imDock = CGRect(x: 800, y: -1150, width: 800, height: 100)
    assert(screenIndex(of: imDock, screens: screens) == 1, "naechster Schirm gewinnt, nicht Index 0")
    // Gar keine Ueberlappung -> naechster Mittelpunkt
    assert(screenIndex(of: CGRect(x: 900, y: -3000, width: 100, height: 100), screens: screens) == 1)
}

// ---------- Kevins zwei Bildschirme (gemessen 2026-09-18) ----------

private let oled = CGRect(x: 0, y: 0, width: 3008, height: 1239)        // LG 45" 21:9
private let macbook = CGRect(x: 687, y: -1037, width: 1728, height: 1005)

func testAutoGridPerScreen() {
    // Drei Sessions auf dem Ultrawide: drei Saeulen ueber die VOLLE Hoehe, keine halben Kacheln.
    let g3 = autoGrid(oled, count: 3)
    assert(g3 == (3, 1), "bekam \(g3)")
    let f3 = gridLayout(oled, cols: g3.cols, rows: g3.rows, side: .right, ids: ["a", "b", "c"])
    assert(f3.frames["a"]!.height == oled.height, "volle Hoehe")
    assert(f3.frames["a"]!.width == 1002, "bekam \(f3.frames["a"]!.width)")

    // Sechs Sessions: erst die Spalten fuellen, dann eine zweite Reihe.
    assert(autoGrid(oled, count: 6) == (3, 2))

    // Derselbe Bestand auf dem MacBook: nur zwei Spalten, sonst wird es zu schmal.
    assert(autoGrid(macbook, count: 3) == (2, 2), "bekam \(autoGrid(macbook, count: 3))")
    assert(autoGrid(macbook, count: 2) == (2, 1))
    assert(autoGrid(macbook, count: 1) == (1, 1))
}

func testReadingWidthOnUltrawide() {
    // Ein Fenster ueber die volle Breite waere unlesbar — Hauptfenster steht mittig.
    let (main, left, right) = readingLayout(oled, side: .right)
    assert(main.width == READING_WIDTH, "bekam \(main.width)")
    assert(left != nil && right != nil, "auf 3008 pt passen Streifen auf BEIDE Seiten")
    assert(left!.maxX == main.minX && main.maxX == right!.minX, "lueckenlos")
    assert(abs(left!.width - right!.width) <= 1, "mittig")
    assert(left!.minX == oled.minX && right!.maxX == oled.maxX)

    // Auf dem Laptop passt das nicht — dort faellt die Ansicht auf einen Streifen zurueck.
    let (m2, l2, r2) = readingLayout(macbook, side: .right)
    assert(l2 == nil && r2 != nil, "nur ein Streifen, rechts")
    assert(m2.maxX == r2!.minX)
}

func testNoGlowOnMaximizedWindow() {
    let t0 = Date(timeIntervalSince1970: 1000)
    var voll = fakeWindow(ready: true, firstSeen: t0)
    voll.frame = macbook                                   // fuellt den Schirm komplett
    var gekachelt = fakeWindow(ready: true, firstSeen: t0)
    gekachelt.frame = CGRect(x: macbook.minX, y: macbook.minY,
                             width: macbook.width / 2, height: macbook.height)
    let screens = [macbook]
    assert(isMaximized(voll, screens: screens), "volle Flaeche zaehlt als maximiert")
    assert(!isMaximized(gekachelt, screens: screens), "eine halbe Kachel nicht")

    let ziele = glowTargets(windows: [voll, gekachelt], onScreen: nil, screenH: 1117, screens: screens)
    assert(ziele.count == 1, "nur das gekachelte Fenster leuchtet, bekam \(ziele.count)")
}

func testNoGlowOnActiveWindow() {
    // Kevin 2026-09-18: das Fenster, in dem er arbeitet, bekommt NIE einen Rahmen —
    // egal wie gross. Zwischen zwei Antworten ist es technisch "fertig".
    let t0 = Date(timeIntervalSince1970: 1000)
    var a = fakeWindow(ready: true, firstSeen: t0)
    var b = fakeWindow(ready: true, firstSeen: t0.addingTimeInterval(1))
    a.frame = CGRect(x: 0, y: 0, width: 1000, height: 600)
    b.frame = CGRect(x: 1000, y: 0, width: 1000, height: 600)
    let ziele = glowTargets(windows: [a, b], onScreen: nil, screenH: 1269, screens: [oled], focusedIndex: 0)
    assert(ziele.map(\.id) == [b.sessions[0].id], "die aktive Session leuchtet nie, bekam \(ziele.map(\.id))")
    // Kein Stapel-Sonderfall mehr: jedes fertige Fenster bekommt seinen Rahmen, die echte
    // Tiefenreihenfolge verdeckt ihn dort, wo andere Fenster davor liegen.
    assert(glowTargets(windows: [a, b], onScreen: nil, screenH: 1269, screens: [oled]).count == 2)
}

func testGlowKnowsItsWindow() {
    let t0 = Date(timeIntervalSince1970: 1000)
    var w = fakeWindow(ready: true, firstSeen: t0)          // ownerPid 0
    w.frame = CGRect(x: 100, y: 200, width: 800, height: 600)
    let ax = cocoaToAX(w.frame, .topLeft, screenH: 1269)
    let ziele = glowTargets(windows: [w], onScreen: [OnScreenWindow(number: 42, pid: 0, rect: ax)],
                            screenH: 1269, screens: [oled])
    assert(ziele.first?.windowNumber == 42, "Rahmen kennt sein Fenster -> Ebene direkt darueber")
    let fremd = [OnScreenWindow(number: 43, pid: 999, rect: ax)]
    assert(glowTargets(windows: [w], onScreen: fremd, screenH: 1269, screens: [oled]).isEmpty,
           "gleiches Rechteck, andere App: das Fenster liegt nicht auf diesem Space")
    let ohneListe = glowTargets(windows: [w], onScreen: nil, screenH: 1269, screens: [oled])
    assert(ohneListe.count == 1 && ohneListe[0].windowNumber == nil,
           "Fensterliste nicht lesbar: nicht faelschlich ausblenden — nur ohne Ebene")
    // Liste gelesen, aber kein Terminal-Fenster auf diesem Schirm (Kevin auf einem Schreibtisch
    // nur mit Chrome): kein Rahmen. Vorher galt leer wie "nicht lesbar", und jedes fertige
    // Fenster bekam einen Rahmen an einer Stelle ohne Fenster.
    assert(glowTargets(windows: [w], onScreen: [], screenH: 1269, screens: [oled]).isEmpty,
           "kein Terminal-Fenster auf dem Schirm -> kein Rahmen")
    // review quick 2026-09-18: zwei Fenster derselben App auf demselben Rechteck sind nicht
    // unterscheidbar — lieber kein Rahmen als einer ueber dem falschen Fenster.
    let doppelt = [OnScreenWindow(number: 42, pid: 0, rect: ax), OnScreenWindow(number: 44, pid: 0, rect: ax)]
    assert(glowTargets(windows: [w], onScreen: doppelt, screenH: 1269, screens: [oled]).isEmpty,
           "mehrdeutige Zuordnung -> kein Rahmen")
    // Fuer die Fensterliste zaehlt nur "liegt auf dem Schirm" — auch mehrdeutig ist sichtbar.
    assert(locateOnScreen(w, onScreen: doppelt, screenH: 1269) == .ambiguous)
    assert(locateOnScreen(w, onScreen: fremd, screenH: 1269) == .none)
    assert(locateOnScreen(w, onScreen: [], screenH: 1269) == .none, "leer gelesen = nicht auf dem Schirm")
    assert(locateOnScreen(w, onScreen: nil, screenH: 1269) == .unknown, "nicht lesbar = unbekannt")
}

/// Kevin 2026-09-18: "kevin-05 sehe ich nicht, aber er zeigt an, dass da ein Fenster sei" —
/// das Ghostty-Fenster lag minimiert im Dock. Die Fensterliste sagt jetzt, wo es ist.
func testWindowWhereabouts() {
    var w = fakeWindow(ready: true, firstSeen: Date(timeIntervalSince1970: 1000))
    assert(whereabouts(w, onScreen: true) == .visible && WindowWhereabouts.visible.label.isEmpty)
    assert(whereabouts(w, onScreen: false) == .otherSpace, "nicht auf dem Schirm -> anderer Schreibtisch")
    w.movable = false
    assert(whereabouts(w, onScreen: true) == .immovable)
    w.fullscreen = true
    assert(whereabouts(w, onScreen: false) == .fullscreen, "Vollbild liegt auf eigenem Schreibtisch")
    w.minimized = true
    assert(whereabouts(w, onScreen: false) == .minimized, "minimiert sticht alles")
    assert(WindowWhereabouts.minimized.label == "minimiert (im Dock)")
    assert(WindowWhereabouts.otherSpace.label == "anderer Schreibtisch")
}

// ---------- Ziel-Knoepfe + Leitstand (Kevin 2026-09-18, ersetzt das Wartezimmer) ----------

private let lgUndMacbook = [ScreenInfo(visible: oled, builtin: false), ScreenInfo(visible: macbook, builtin: true)]

func testResolveTarget() {
    assert(resolveTarget(.external, screens: lgUndMacbook) == .single(0))
    assert(resolveTarget(.builtin, screens: lgUndMacbook) == .single(1))
    assert(resolveTarget(.both, screens: lgUndMacbook) == .leitstand(work: 0, control: 1))
    // Die Reihenfolge der Schirme darf keine Rolle spielen.
    let umgedreht = [lgUndMacbook[1], lgUndMacbook[0]]
    assert(resolveTarget(.external, screens: umgedreht) == .single(1))
    assert(resolveTarget(.both, screens: umgedreht) == .leitstand(work: 1, control: 0))
    // MacBook allein: jede Wahl landet auf dem MacBook, die Wahl selbst bleibt erhalten.
    let nurMacbook = [lgUndMacbook[1]]
    assert(resolveTarget(.external, screens: nurMacbook) == .single(0), "fehlender LG -> vorhandener Schirm")
    assert(resolveTarget(.both, screens: nurMacbook) == .single(0), "Leitstand braucht zwei Schirme")
    // Zugeklappt, nur der LG.
    assert(resolveTarget(.builtin, screens: [lgUndMacbook[0]]) == .single(0))
    assert(resolveTarget(.external, screens: []) == nil)
}

/// Fenster ueber BEIDE Schirme verteilt: 0 arbeitet (LG), 1 fertig (MacBook), 2 fragt (LG),
/// 3 arbeitet (MacBook), 4 ist eine normale Shell ohne KI-Session (LG).
private func spreadWindows() -> [TermWindow] {
    let t0 = Date(timeIntervalSince1970: 1000)
    var w = [fakeWindow(ready: false, firstSeen: t0),
             fakeWindow(ready: true, firstSeen: t0.addingTimeInterval(1)),
             fakeWindow(ready: true, firstSeen: t0.addingTimeInterval(2), waiting: true),
             fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(3)),
             fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(4), hasSession: false)]
    w[0].frame = CGRect(x: 100, y: 100, width: 800, height: 600)
    w[1].frame = CGRect(x: 900, y: -900, width: 800, height: 600)
    w[2].frame = CGRect(x: 1500, y: 300, width: 800, height: 600)
    w[3].frame = CGRect(x: 700, y: -700, width: 600, height: 500)
    w[4].frame = CGRect(x: 2000, y: 200, width: 600, height: 400)
    return w
}

func testTargetMovesAllToOneScreen() {
    let w = spreadWindows()
    for (target, ziel, anderer) in [(Target.single(1), macbook, oled), (Target.single(0), oled, macbook)] {
        for mode in LayoutMode.allCases {
            let plan = planLayout(mode: mode, target: target, windows: w, screens: [oled, macbook], side: .right)
            assert(Set(plan.keys) == [0, 1, 2, 3],
                   "\(mode): alle KI-Fenster, die normale Shell nicht (bekam \(plan.keys.sorted()))")
            for r in plan.values {
                assert(ziel.contains(r), "\(mode): \(r) liegt nicht auf dem Ziel-Schirm")
                assert(!anderer.intersects(r), "\(mode): der andere Schirm bleibt frei")
            }
        }
    }
}

func testFocusMainIsActiveSession() {
    let w = spreadWindows()
    let plan = planLayout(mode: .reading, target: .single(0), windows: w, screens: [oled, macbook],
                          side: .right, focusedIndex: 2)
    assert(plan[2] == readingLayout(oled, side: .right).main, "die aktive Session steht in der Mitte")
    let ohne = planLayout(mode: .reading, target: .single(0), windows: w, screens: [oled, macbook], side: .right)
    assert(ohne[0] == readingLayout(oled, side: .right).main, "ohne aktive Session: die zuerst gesehene")
}

// ---------- Fokus 2 (Kevin 2026-09-18, Design v3) ----------

/// Mitte = aktive Session, LINKS wartet auf Kevin (Frage zuerst, dann fertig nach
/// Fertig-Zeitpunkt), RECHTS laeuft.
func testFocus2SidesByStatus() {
    let w = spreadWindows()                       // 0 laeuft, 1 fertig, 2 Frage, 3 laeuft
    let plan = planLayout(mode: .focus2, target: .single(0), windows: w, screens: [oled, macbook],
                          side: .right, focusedIndex: 0)
    let (main, left, right) = readingLayout(oled, side: .right)
    assert(plan[0] == main, "die aktive Session steht in der Mitte")
    for i in [1, 2] { assert(left!.contains(plan[i]!), "Fenster \(i) wartet auf Kevin -> links") }
    assert(right!.contains(plan[3]!), "Fenster 3 laeuft -> rechts")
    assert(plan[2]!.maxY > plan[1]!.maxY, "die Frage steht ueber dem nur fertigen")
    assert(plan[4] == nil, "normale Shell bleibt liegen")
}

/// Wird eine Session fertig, gleitet ihr Fenster nach links — und zurueck, sobald sie wieder
/// arbeitet. Die aktive Session bleibt dabei in der Mitte, egal was sie gerade tut.
func testFocus2FollowsStatus() {
    var w = spreadWindows()
    let (main, left, right) = readingLayout(oled, side: .right)
    let before = planLayout(mode: .focus2, target: .single(0), windows: w, screens: [oled, macbook],
                            side: .right, focusedIndex: 1)
    assert(before[1] == main, "auch eine fertige aktive Session steht in der Mitte")
    assert(right!.contains(before[3]!), "3 laeuft -> rechts")

    let frame = w[3].frame
    w[3] = fakeWindow(ready: true, firstSeen: w[3].firstSeen, readySince: Date(timeIntervalSince1970: 9000))
    w[3].frame = frame
    let after = planLayout(mode: .focus2, target: .single(0), windows: w, screens: [oled, macbook],
                           side: .right, focusedIndex: 1)
    assert(left!.contains(after[3]!), "3 ist fertig -> jetzt links")
    assert(after[2]!.maxY > after[3]!.maxY, "die offene Frage bleibt oben")
    assert(right!.contains(after[0]!), "0 laeuft weiter rechts")
}

/// MacBook (zu schmal fuer zwei Seiten): Hauptfenster links, rechte Spalte oben wartend. Seit Plan 1d (2026-09-26)
/// sind Laufende dort nie Fenster: sie liegen geparkt hinter der Mitte und stehen als Zeilen in der ARBEITET-Liste.
func testFocus2Narrow() {
    let w = spreadWindows()
    let a = planArrangement(mode: .focus2, target: .single(1), windows: w, screens: [oled, macbook],
                            side: .right, focusedIndex: 0)
    let main = readingLayout(macbook, side: .right).main
    assert(a.frames[0] == main, "Hauptfenster links gross")
    for i in [1, 2] { assert(a.frames[i]!.minX >= main.maxX, "Fenster \(i) wartet -> rechte Spalte") }
    assert(a.frames[2]!.maxY > a.frames[1]!.maxY, "die Frage steht oben")
    assert(a.frames[3] == main && a.parked == [3], "das laufende Fenster liegt geparkt hinter der Mitte")
    assert(a.lists.map(\.entries) == [[.header("ARBEITET (1)"), .row(3)]], "bekam \(a.lists)")
    assert(a.lists[0].rect.maxY <= min(a.frames[1]!.minY, a.frames[2]!.minY), "die Liste steht unter den Wartenden")
}

// ---------- Raster mit fester Auswahl (Kevin 2026-09-18: "Fest") ----------

func testGridChoicesFitScreen() {
    assert(GRID_CHOICES.map(\.key) == ["2x2", "3x2", "4x2", "3x3", "4x3", "5x2"])
    assert(gridChoices(for: oled).count == 6, "auf dem LG passen alle")
    assert(gridChoices(for: macbook).map(\.key) == ["2x2", "3x2", "3x3"], "MacBook hoechstens 3 Spalten")
    for screen in [oled, macbook] {
        for g in gridChoices(for: screen) {
            assert(floor(screen.width / CGFloat(g.cols)) >= MIN_CARD_W, "\(g.key): Spalte zu schmal")
            assert(floor(screen.height / CGFloat(g.rows)) >= MIN_CARD_H, "\(g.key): Zeile zu niedrig")
        }
    }
    // Gespeicherte Wahl passt nicht auf den Ziel-Schirm -> auf das Groesste zurechtgestutzt, das passt.
    assert(fittedGrid(GridSpec(cols: 5, rows: 2), on: macbook).key == "3x2")
    assert(fittedGrid(GridSpec(cols: 4, rows: 3), on: macbook).key == "3x3")
    assert(fittedGrid(GridSpec(cols: 5, rows: 2), on: oled).key == "5x2")
    assert(GridSpec("4x3") == GridSpec(cols: 4, rows: 3) && GridSpec("quatsch") == nil)
}

func testFixedGridKeepsCellSize() {
    let g = GridSpec(cols: 3, rows: 3)
    let cellW = floor(oled.width / 3), cellH = floor(oled.height / 3)
    let four = fixedGrid(oled, g, ids: [7, 8, 9, 10])
    assert(four.count == 4)
    for r in four.values { assert(r.width == cellW && r.height == cellH, "jede Kachel ein Neuntel, bekam \(r)") }
    assert(four[7]!.minX == oled.minX && four[7]!.maxY == oled.maxY, "erster Platz oben links")
    assert(four[10]!.minX == oled.minX && four[10]!.maxY == oled.maxY - cellH, "vierter: zweite Reihe links")

    // Mehr Fenster als Plaetze: die uebrigen bleiben unberuehrt liegen.
    let ten = fixedGrid(oled, g, ids: Array(0..<10))
    assert(ten.count == 9 && ten[9] == nil, "das zehnte Fenster bekommt keinen Platz")
    for (a, ra) in ten { for (b, rb) in ten where a < b { assert(!ra.intersects(rb), "\(a) und \(b) ueberlappen") } }
}

func testGridIgnoresClicks() {
    let w = spreadWindows()
    let g = GridSpec(cols: 3, rows: 3)
    let a = planLayout(mode: .grid, target: .single(0), windows: w, screens: [oled, macbook],
                       side: .right, focusedIndex: 0, grid: g)
    let b = planLayout(mode: .grid, target: .single(0), windows: w, screens: [oled, macbook],
                       side: .right, focusedIndex: 3, grid: g)
    assert(a == b, "ein Klick ordnet im Raster nichts um")
    assert(a[0] == fixedGrid(oled, g, ids: [0])[0], "Reihenfolge nach Erstsichtung, fester Platz")
}

// ---------- Fenster ausschliessen (Kevin 2026-09-18) ----------

func testExcludedWindowsStayPut() {
    let w = spreadWindows()
    let raus: Set<String> = [w[2].sessions[0].id]
    for target in [Target.single(0), .single(1), .leitstand(work: 0, control: 1),
                   .leitstand(work: 0, control: 1, readySide: .right)] {
        for mode in LayoutMode.allCases {
            let plan = planLayout(mode: mode, target: target, windows: w, screens: [oled, macbook],
                                  side: .right, focusedIndex: 2, excluded: raus)
            assert(Set(plan.keys) == [0, 1, 3], "\(mode)/\(target): ausgeschlossenes Fenster bleibt liegen")
        }
    }
    assert(isExcluded(w[2], raus) && !isExcluded(w[1], raus))
}

// ---------- Fehler 2: kein Endlos-Neuversuch ----------

func testNoEndlessRetry() {
    let soll = CGRect(x: 0, y: 1, width: 804, height: 619)
    let ist = CGRect(x: 0, y: -57, width: 804, height: 677)
    assert(needsMove(current: ist, dest: soll, memory: nil), "erster Versuch")
    assert(!needsMove(current: soll, dest: soll, memory: nil), "steht schon richtig")
    let gemerkt = MoveMemory(target: soll, reached: ist)
    assert(!needsMove(current: ist, dest: soll, memory: gemerkt), "gleiches Ziel, gleicher Ist-Rahmen -> nicht erneut")
    assert(needsMove(current: ist, dest: soll.offsetBy(dx: 400, dy: 0), memory: gemerkt), "neues Ziel -> bewegen")
    assert(needsMove(current: ist.offsetBy(dx: 300, dy: 0), dest: soll, memory: gemerkt),
           "Kevin hat das Fenster verschoben -> wieder anordnen")
}

/// Gemessen 2026-09-18 an eigenen Testfenstern: ragt ein Warp-Fenster nach dem Verschieben
/// (noch in alter Groesse) ueber seinen Schirm hinaus, klemmt jede folgende Verkleinerung
/// die Unterkante. Invariante: kein Zwischenstand verlaesst den alten oder den neuen Rahmen.
func testFrameWritesStayInside() {
    func apply(_ writes: [AXWrite], to start: CGRect, check: (CGRect) -> Void) -> CGRect {
        var r = start
        for wr in writes {
            switch wr {
            case .position(let p): r.origin = p
            case .size(let s): r.size = s           // AX: oben links bleibt stehen
            }
            check(r)
        }
        return r
    }
    // AX-Koordinaten (y nach unten): LG-Streifen voll hoch -> untere Karte, MacBook -> LG, Wachsen.
    let pairs: [(CGRect, CGRect)] = [
        (CGRect(x: 804, y: 30, width: 1400, height: 1239), CGRect(x: 0, y: 649, width: 804, height: 619)),
        (CGRect(x: 687, y: 1301, width: 1728, height: 1007), CGRect(x: 0, y: 649, width: 804, height: 619)),
        (CGRect(x: 1935, y: 1805, width: 480, height: 503), CGRect(x: 804, y: 30, width: 1400, height: 1239)),
        (CGRect(x: 687, y: 1301, width: 1728, height: 503), CGRect(x: 1935, y: 1301, width: 480, height: 1007)),
    ]
    for (from, to) in pairs {
        let end = apply(frameWrites(from: from, to: to), to: from) { r in
            assert(from.contains(r) || to.contains(r), "Zwischenstand \(r) verlaesst alt \(from) und neu \(to)")
        }
        assert(end == to, "am Ende steht das Ziel, bekam \(end)")
    }
    // Rettung eines schon geklemmten Fensters: erst in Ist-Groesse unten buendig aufs Ziel.
    let klemmt = CGRect(x: 0, y: 649, width: 804, height: 677)
    let ziel = CGRect(x: 0, y: 649, width: 804, height: 619)
    let end = apply(rescueWrites(from: klemmt, to: ziel), to: klemmt) { r in
        assert(r.maxY <= ziel.maxY, "Zwischenstand \(r) ragt unter das Ziel")
    }
    assert(end == ziel)
}

/// Unterwegs aendert sich die Groesse nicht mehr: am alten Platz verkleinern (oben links
/// bleibt), gleiten, am Ziel wachsen. Die Gleitbahn liegt am Start im alten, am Ende im neuen
/// Rahmen — kein Zwischenbild ragt ueber einen der beiden hinaus.
func testGlidePath() {
    let pairs: [(CGRect, CGRect)] = [
        (CGRect(x: 804, y: 0, width: 1400, height: 1239), CGRect(x: 0, y: 1, width: 804, height: 619)),
        (CGRect(x: 1935, y: -1039, width: 480, height: 503), CGRect(x: 804, y: 0, width: 1400, height: 1239)),
        (CGRect(x: 687, y: -536, width: 1728, height: 503), CGRect(x: 1935, y: -1039, width: 480, height: 1007)),
    ]
    for (from, to) in pairs {
        let (a, b) = glidePath(from: from, to: to)
        assert(a.size == b.size, "unterwegs keine Groessenaenderung")
        assert(from.contains(a) && to.contains(b), "Start im alten, Ende im neuen Rahmen")
        assert(a.minX == from.minX && a.maxY == from.maxY, "verkleinert wird oben links verankert (wie AX)")
        assert(b.minX == to.minX && b.maxY == to.maxY, "am Ziel waechst es von oben links aus")
    }
}

// ---------- Fehler 1: Zuordnung nach herausgezogenem Tab ----------

func testPingSchedule() {
    let t = Date(timeIntervalSince1970: 1_000_000)
    // Neuer Prozess ohne Fenster -> sofort; bekannte Zuordnung -> Ruhe.
    var s = pingSchedule(pids: [1, 2], mapped: [2], pending: [:], newWindow: false, now: t)
    assert(s.due == [1], "nur der unbekannte, bekam \(s.due)")
    // Ein NEUES Fenster (herausgezogener Tab): jede Zuordnung wird neu geprueft.
    s = pingSchedule(pids: [1, 2], mapped: [1, 2], pending: [:], newWindow: true, now: t)
    assert(s.due == [1, 2], "neues Fenster -> alle neu pruefen")
    // Fehlversuche: nie endgueltig aufgeben, nur seltener fragen.
    var pending: [pid_t: PingState] = [:]
    var now = t
    for attempt in 1...8 {
        let r = pingSchedule(pids: [1], mapped: [], pending: pending, newWindow: false, now: now)
        assert(r.due == [1], "Versuch \(attempt) faellig")
        pending = pingResult(due: r.due, found: [], pending: r.pending, now: now)
        let wait = pingRetryInterval(attempts: pending[1]!.attempts)
        assert(wait == (attempt < 4 ? 15 : 120), "Versuch \(attempt): Abstand \(wait)")
        let early = pingSchedule(pids: [1], mapped: [], pending: pending, newWindow: false,
                                 now: now.addingTimeInterval(wait - 1))
        assert(early.due.isEmpty, "vor Ablauf nicht erneut")
        now = now.addingTimeInterval(wait)
    }
    // Ohne Bedienungshilfen ist kein Titel lesbar: gepingt wird dann gar nicht — jeder Versuch
    // schriebe nur Marker und Ordnernamen in Kevins Tab-Titel (live 18:26 nach dem Neubau).
    let blind = pingSchedule(pids: [1, 2], mapped: [], pending: [:], newWindow: true, now: t, trusted: false)
    assert(blind.due.isEmpty, "ohne Freigabe kein Ping, bekam \(blind.due)")
    assert(Set(blind.pending.keys) == [1, 2], "vorgemerkt bleibt, was nach der Freigabe dran ist")
    // Gesperrter Bildschirm: AX liefert kein einziges Terminal-Fenster. Ein Ping koennte nichts finden und setzte
    // nur jeden Tab-Titel auf den Ordnernamen (Log bis 2026-09-25: 3.249 Runden, 15.696 Titel).
    let dunkel = pingSchedule(pids: [1, 2], mapped: [], pending: [:], newWindow: true, now: t, windowsReadable: false)
    assert(dunkel.due.isEmpty, "ohne lesbares Fenster kein Ping, bekam \(dunkel.due)")
    assert(Set(dunkel.pending.keys) == [1, 2], "vorgemerkt fuer danach")
    // Ein neues Fenster prueft sofort alles neu — aber ein Hintergrund-Tab faellt danach in
    // seinen langen Takt zurueck, statt wieder im 15-s-Takt zu blinken (review quick v3).
    let fresh = pingSchedule(pids: [1], mapped: [], pending: pending, newWindow: true, now: now)
    assert(fresh.due == [1], "neues Fenster -> sofort neu pruefen")
    let after = pingResult(due: fresh.due, found: [], pending: fresh.pending, now: now)
    assert(pingRetryInterval(attempts: after[1]!.attempts) == 120, "der Zaehler bleibt, bekam \(after[1]!.attempts)")
    // Gefunden -> Ruhe; verschwundener Prozess -> vergessen.
    pending = pingResult(due: [1], found: [1], pending: pending, now: now)
    assert(pending[1] == nil)
    let gone = pingSchedule(pids: [], mapped: [], pending: [7: PingState(attempts: 2, lastTry: t)],
                            newWindow: false, now: now)
    assert(gone.pending.isEmpty && gone.due.isEmpty)
}

// ---------- Fehler 3: Umrisse blenden nicht animiert ----------

func testOverlayPanelsDoNotAnimate() {
    for level in [NSWindow.Level.normal, .floating] {
        let p = makeOverlayPanel(level: level)
        assert(p.animationBehavior == .none, "macOS-Fensteranimation liesse alte und neue Umrisse ueberlappen")
        assert(p.ignoresMouseEvents && p.level == level)
    }
}

/// Leitstand (Kevin 2026-09-24): wer fertig ist oder fragt, zieht vom MacBook hoch LINKS neben
/// die aktive Session; rechts bleibt frei fuer Chrome, das MacBook zeigt, was laeuft.
func testLeitstand() {
    let w = spreadWindows()                       // 0 laeuft, 1 fertig, 2 Frage, 3 laeuft, 4 Shell
    let target = Target.leitstand(work: 0, control: 1)
    let (main, left, right) = readingLayout(oled, side: .right)
    let plan = planLayout(mode: .reading, target: target, windows: w, screens: [oled, macbook],
                          side: .right, focusedIndex: 3)
    assert(plan[3] == main, "aktive Session mittig auf dem grossen Schirm")
    for i in [1, 2] { assert(left!.contains(plan[i]!), "Fenster \(i) wartet -> links neben aktiv") }
    assert(plan[2]!.maxY > plan[1]!.maxY, "die Frage steht ueber dem nur fertigen")
    assert(macbook.contains(plan[0]!), "Fenster 0 laeuft -> MacBook")
    assert(!plan.values.contains { $0.intersects(right!) }, "rechts bleibt frei fuer Chrome")
    assert(plan[4] == nil, "normale Shell bleibt liegen")
    let ohne = planLayout(mode: .grid, target: target, windows: w, screens: [oled, macbook], side: .right)
    assert(ohne[0] == main, "ohne aktive Session: die zuerst gesehene")

    // Mehr Wartende als die Spalte fasst: der Rest reiht sich LINKS auf dem MacBook ein.
    let t0 = Date(timeIntervalSince1970: 1000)
    let many = (0..<10).map { fakeWindow(ready: $0 > 0, firstSeen: t0.addingTimeInterval(Double($0))) }
    let full = planLayout(mode: .reading, target: target, windows: many, screens: [oled, macbook],
                          side: .right, focusedIndex: 0)
    assert((1...6).allSatisfy { left!.contains(full[$0]!) }, "wer zuerst fertig war, steht neben aktiv")
    for i in [7, 8, 9] {
        assert(macbook.contains(full[i]!) && full[i]!.maxX <= macbook.minX + miniRailWidth(macbook),
               "Fenster \(i) reiht sich links auf dem MacBook ein")
    }
    // Auch die MacBook-Spalte voll (6 + 5): der Rest steht vorn im Raster, statt liegen zu bleiben
    // und dort zu ueberlappen, wo inzwischen ein anderes Fenster hinkommt (review quick 2026-09-24).
    let flood = (0..<15).map { fakeWindow(ready: $0 > 0, firstSeen: t0.addingTimeInterval(Double($0))) }
    let flooded = planLayout(mode: .reading, target: target, windows: flood, screens: [oled, macbook],
                             side: .right, focusedIndex: 0)
    assert(flooded.count == 15, "jedes Fenster bekommt einen Platz (bekam \(flooded.count))")
    let rects = Array(flooded.values)
    for i in rects.indices { for j in rects.indices where j > i {
        assert(!rects[i].intersects(rects[j]), "keine Ueberlappung: \(rects[i]) / \(rects[j])")
    } }
}

/// Leitstand 2 (Kevin 2026-09-24): "fertig sollte immer neben aktiv sein". Grosser Schirm:
/// links frei fuer Chrome, Mitte aktiv, rechts daneben wer auf Kevin wartet; MacBook: was laeuft.
func testLeitstand2() {
    var w = spreadWindows()                       // 0 laeuft, 1 fertig, 2 Frage, 3 laeuft, 4 Shell
    let target = Target.leitstand(work: 0, control: 1, readySide: .right)
    let (main, left, right) = readingLayout(oled, side: .right)
    let plan = planLayout(mode: .reading, target: target, windows: w, screens: [oled, macbook],
                          side: .left, focusedIndex: 0)
    assert(plan[0] == main, "aktive Session mittig auf dem grossen Schirm")
    for i in [1, 2] { assert(right!.contains(plan[i]!), "Fenster \(i) wartet -> rechts neben aktiv") }
    assert(plan[2]!.maxY > plan[1]!.maxY, "die Frage steht ueber dem nur fertigen")
    assert(macbook.contains(plan[3]!), "Fenster 3 laeuft -> MacBook")
    assert(!plan.values.contains { $0.intersects(left!) }, "links bleibt frei fuer Chrome")
    assert(plan[4] == nil, "normale Shell bleibt liegen")

    // Status wechselt: 3 wird fertig -> hoch neben aktiv, 1 arbeitet wieder -> aufs MacBook.
    w[3] = fakeWindow(ready: true, firstSeen: w[3].firstSeen, readySince: Date(timeIntervalSince1970: 9000))
    w[1] = fakeWindow(ready: false, firstSeen: w[1].firstSeen)
    let after = planLayout(mode: .reading, target: target, windows: w, screens: [oled, macbook],
                           side: .right, focusedIndex: 0)
    assert(right!.contains(after[3]!) && macbook.contains(after[1]!), "folgt dem Status")

    // Mehr Wartende als die Spalte fasst: der Rest reiht sich rechts auf dem MacBook ein.
    let t0 = Date(timeIntervalSince1970: 1000)
    let many = (0..<10).map { fakeWindow(ready: $0 > 0, firstSeen: t0.addingTimeInterval(Double($0))) }
    let full = planLayout(mode: .reading, target: target, windows: many, screens: [oled, macbook],
                          side: .right, focusedIndex: 0)
    let inColumn = (1..<10).filter { right!.contains(full[$0]!) }
    assert(inColumn == Array(1...6), "wer zuerst fertig war, steht neben aktiv (bekam \(inColumn))")
    for i in [7, 8, 9] {
        assert(macbook.contains(full[i]!) && full[i]!.minX >= macbook.maxX - miniRailWidth(macbook),
               "Fenster \(i) reiht sich rechts auf dem MacBook ein")
    }

    assert(resolveTarget(.both2, screens: lgUndMacbook) == target)
    assert(resolveTarget(.both2, screens: [lgUndMacbook[1]]) == .single(0), "braucht zwei Schirme")
}

/// Leitstand 3 (Kevin 2026-09-28): MacBook = Arbeitsplatz (Wartende links, aktiv rechts), der LG zeigt die Laufenden
/// am linken und rechten Rand, die Mitte (halber Schirm) bleibt frei fuer Chrome.
func testLeitstand3() {
    let w = spreadWindows()                       // 0 laeuft, 1 fertig, 2 Frage, 3 laeuft, 4 Shell
    let target = Target.leitstand(work: 1, control: 0, flipped: true)
    assert(resolveTarget(.both3, screens: lgUndMacbook) == target)
    assert(resolveTarget(.both3, screens: [lgUndMacbook[1]]) == .single(0), "braucht zwei Schirme")
    let (main, column) = focusLayout(macbook, side: .left)
    let middle = CGRect(x: oled.minX + oled.width / 4, y: oled.minY, width: oled.width / 2, height: oled.height)
    let plan = planLayout(mode: .reading, target: target, windows: w, screens: [oled, macbook],
                          side: .right, focusedIndex: 3)
    assert(plan[3] == main, "aktive Session auf dem MacBook rechts")
    for i in [1, 2] { assert(column.contains(plan[i]!), "Fenster \(i) wartet -> links daneben") }
    assert(plan[2]!.maxY > plan[1]!.maxY, "die Frage steht ueber dem nur fertigen")
    assert(oled.contains(plan[0]!) && plan[0]!.maxX <= middle.minX, "Fenster 0 laeuft -> LG links")
    assert(!plan.values.contains { $0.intersects(middle) }, "Mitte bleibt frei fuer Chrome")
    assert(plan[4] == nil, "normale Shell bleibt liegen")

    // Viele: 5 Wartende passen aufs MacBook, der Rest + die Laufenden verteilen sich links/rechts auf dem LG.
    let t0 = Date(timeIntervalSince1970: 1000)
    let many = (0..<12).map { fakeWindow(ready: $0 > 0 && $0 < 9, firstSeen: t0.addingTimeInterval(Double($0))) }
    let full = planLayout(mode: .reading, target: target, windows: many, screens: [oled, macbook],
                          side: .right, focusedIndex: 0)
    assert((1...5).allSatisfy { column.contains(full[$0]!) }, "wer zuerst fertig war, steht neben aktiv")
    assert((6..<12).allSatisfy { oled.contains(full[$0]!) && !full[$0]!.intersects(middle) }, "Rest am LG-Rand")
    // Flut: jedes Fenster bekommt einen Platz, nichts ueberlappt (notfalls in der Mitte).
    let flood = (0..<25).map { fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(Double($0))) }
    let flooded = planLayout(mode: .reading, target: target, windows: flood, screens: [oled, macbook],
                             side: .right, focusedIndex: 0)
    assert(flooded.count == 25, "jedes Fenster bekommt einen Platz (bekam \(flooded.count))")
    let rects = Array(flooded.values)
    for i in rects.indices { for j in rects.indices where j > i {
        assert(!rects[i].intersects(rects[j]), "keine Ueberlappung: \(rects[i]) / \(rects[j])")
    } }
}

func testClickSwapsOnlyTwoWindows() {
    // Vorschau-Seite 2026-09-18: "Klick auf eine Kachel TAUSCHT sie" — nur die zwei
    // beteiligten Fenster bewegen sich, alle anderen bleiben stehen (vorher sortierte ein
    // Klick alle Seitenfenster neu).
    var w = spreadWindows()                       // KI-Fenster 0…3 in Erstsichtungs-Reihenfolge
    // Alle laufen: im Leitstand zieht der Status Fertige sonst ohnehin auf den grossen Schirm, und in Fokus 1 wird seit
    // Plan 1d nur unter Laufenden getauscht (Frage und fertig ordnet der Fertig-Zeitpunkt, `testClickAcrossGroups`).
    for i in [1, 2] { let f = w[i].frame; w[i] = fakeWindow(ready: false, firstSeen: w[i].firstSeen); w[i].frame = f }
    let ordered = layoutCandidates(w)
    assert(ordered == [0, 1, 2, 3])
    for target in [Target.single(0), .leitstand(work: 0, control: 1), .leitstand(work: 0, control: 1, readySide: .right)] {
        let vorher = planLayout(mode: .reading, target: target, windows: w, screens: [oled, macbook],
                                side: .right, focusedIndex: 0)
        let shown = restSequence(ordered, main: 0, hint: [])
        let rest = target == .single(0) ? statusSwap(shown, oldMain: 0, newMain: 2, windows: w)
                                        : swappedRest(shown, oldMain: 0, newMain: 2)
        assert(rest == [1, 0, 3], "bekam \(rest)")
        let nachher = planLayout(mode: .reading, target: target, windows: w, screens: [oled, macbook],
                                 side: .right, focusedIndex: 2, restHint: rest)
        assert(nachher[2] == vorher[0], "\(target): das angeklickte Fenster steht, wo das alte Hauptfenster stand")
        assert(nachher[0] == vorher[2], "\(target): das alte Hauptfenster nimmt den frei gewordenen Platz")
        assert(nachher[1] == vorher[1] && nachher[3] == vorher[3], "\(target): alle anderen bleiben stehen")
    }
    // Gemerkte Reihenfolge ueberlebt verschwundene und neue Fenster.
    assert(restSequence([0, 1, 2, 3], main: 1, hint: [3, 9, 0]) == [3, 0, 2], "9 gibt es nicht mehr, 2 ist neu")
}

func testLegacyLayoutNamesStillWork() {
    assert(LayoutMode.parse("grid32") == .grid, "alte Konfiguration faellt nicht auf den Default")
    assert(LayoutMode.parse("grid22") == .grid)
    assert(LayoutMode.parse("stack") == .reading, "der Stapel ist raus (Kevin 2026-09-18) -> Fokus")
    assert(LayoutMode.parse("focus") == .reading, "der alte Fokus-Modus ist in der Lesebreite aufgegangen")
    assert(LayoutMode.parse("focus2") == .focus2)
    assert(LayoutMode.parse("quatsch") == .grid)
    assert(LayoutMode.allCases == [.reading, .focus2, .grid], "Ctrl+Opt+1 Fokus, 2 Fokus 2, 3 Raster")
    assert(LayoutMode.allCases.map(\.label) == ["Fokus", "Fokus 2", "Raster"])
}

// ---------- Schilder + Grün-Fix (Design 2026-09-21) ----------

/// Claude schreibt den Fenstertitel selbst: Kreisel = arbeitet, ✳ = fertig/Frage (gemessen 2026-09-21).
func testTitleGlyph() {
    let busy = titleGlyph("◐ Handoff spawn 2026-09-21 beta-pr1-build")
    assert(busy.glyph == .busy && busy.text == "Handoff spawn 2026-09-21 beta-pr1-build",
           "Kreisel = arbeitet, Text ohne Zeichen")
    assert(titleGlyph("◑ Muse code erkunden").glyph == .busy, "beide Kreisel-Hälften")
    let idle = titleGlyph("✳ Rechnungen prüfen")
    assert(idle.glyph == .idle && idle.text == "Rechnungen prüfen")
    let folder = titleGlyph("shop+feature-pr0")
    assert(folder.glyph == .none && folder.text == "shop+feature-pr0",
           "Ordnername nach Ping-Rückschreiben: kein Zeichen")
    assert(titleGlyph("wg-map-4711").glyph == .none, "Ping-Marker ist kein Status")
    assert(titleGlyph("").glyph == .none)
}

/// Echte Eintragsformen aus Claude-Transkripten (Stichprobe 2026-09-21).
private let transcriptFixture = [
    #"e","aiTitle":"abgeschnittene erste Zeile"}"#,
    #"{"type":"ai-title","aiTitle":"Altes Thema"}"#,
    #"{"type":"user","timestamp":"2026-09-21T12:40:00.000Z","message":{"role":"user","content":"Bau die Karten"}}"#,
    #"{"type":"assistant","timestamp":"2026-09-21T12:40:05.000Z","message":{"content":[{"type":"thinking","thinking":""}]}}"#,
    #"{"type":"assistant","timestamp":"2026-09-21T12:40:06.000Z","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test","description":"Tests der Karten laufen lassen"}}]}}"#,
    #"{"type":"user","timestamp":"2026-09-21T12:40:30.000Z","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
    #"{"type":"user","isMeta":true,"timestamp":"2026-09-21T12:40:31.000Z","message":{"content":"<system-reminder>"}}"#,
    #"{"type":"ai-title","aiTitle":"Build queue cards"}"#,
].joined(separator: "\n")

func testDigestTranscript() {
    let d = digestTranscript(transcriptFixture)
    assert(d.title == "Build queue cards", "der letzte ai-title gilt")
    assert(d.step == "Tests der Karten laufen lassen" && d.last == .tool, "letzter Schritt = Bash-Beschreibung")
    assert(d.promptAt == ISO8601DateFormatter().date(from: "2026-09-21T12:40:00Z"),
           "Zugbeginn = Kevins Prompt, nicht Werkzeug-Ergebnis oder isMeta")
    assert(d.answerAt == ISO8601DateFormatter().date(from: "2026-09-21T12:40:06Z"))
    assert(d.text == nil && d.question == nil)

    let fertig = transcriptFixture + "\n"
        + #"{"type":"assistant","timestamp":"2026-09-21T12:41:00.000Z","message":{"content":[{"type":"text","text":"Kevin, die Karten sind gebaut."}]}}"#
    let f = digestTranscript(fertig)
    assert(f.last == .text && f.text == "Kevin, die Karten sind gebaut.")

    let frage = fertig + "\n"
        + #"{"type":"assistant","timestamp":"2026-09-21T12:42:00.000Z","message":{"content":[{"type":"tool_use","id":"toolu_frage","name":"AskUserQuestion","input":{"questions":[{"question":"Welche Variante?","header":"V","options":[]}]}}]}}"#
    assert(digestTranscript(frage).question == "Welche Variante?", "offene Frage erkannt")
    let beantwortet = frage + "\n"
        + #"{"type":"user","timestamp":"2026-09-21T12:43:00.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_frage","content":"A"}]}}"#
    let b = digestTranscript(beantwortet)
    assert(b.question == nil, "die Antwort auf DIESE Frage schließt sie")
    assert(b.promptAt == d.promptAt, "ein Werkzeug-Ergebnis ist kein neuer Prompt")

    // review quick 2026-09-21 (Kimi): das Ergebnis eines PARALLEL gestarteten Werkzeugs beendet die
    // Frage nicht — kam in Kevins Transkripten bei 1 von 211 Fragen vor.
    let parallel = fertig + "\n"
        + #"{"type":"assistant","timestamp":"2026-09-21T12:42:00.000Z","message":{"content":[{"type":"tool_use","id":"toolu_frage","name":"AskUserQuestion","input":{"questions":[{"question":"Welche Variante?"}]}},{"type":"tool_use","id":"toolu_ls","name":"Bash","input":{"command":"ls","description":"Ordner ansehen"}}]}}"#
        + "\n" + #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_ls","content":"a b"}]}}"#
    assert(digestTranscript(parallel).question == "Welche Variante?", "fremdes Werkzeug-Ergebnis lässt die Frage offen")
    let meta = frage + "\n" + #"{"type":"user","isMeta":true,"message":{"content":"<local-command-stdout>"}}"#
    assert(digestTranscript(meta).question == "Welche Variante?", "System-Einblendung lässt die Frage offen")
    let neuerPrompt = frage + "\n" + #"{"type":"user","message":{"content":"Mach weiter"}}"#
    assert(digestTranscript(neuerPrompt).question == nil, "ein neuer Prompt schließt die Frage")

    // U+2028 mitten im JSON-Text darf die Zeile nicht zerteilen (Spike: Pythons splitlines tat es).
    let trenner = #"{"type":"ai-title","aiTitle":"Zeile"# + "\u{2028}" + #"zwei"}"#
    assert(digestTranscript(trenner).title == "Zeile\u{2028}zwei")
    assert(digestTranscript("").title == nil, "leer -> leere Auswertung, kein Absturz")
}

func testStepLabel() {
    assert(stepLabel(tool: "Bash", input: ["command": "git status", "description": "Zeigt den Stand"]) == "Zeigt den Stand")
    assert(stepLabel(tool: "Bash", input: ["command": "git status\n--short"]) == "git status",
           "ohne Beschreibung: erste Befehlszeile")
    assert(stepLabel(tool: "Edit", input: ["file_path": "/a/b/Layout.swift"]) == "ändert Layout.swift")
    assert(stepLabel(tool: "Agent", input: ["description": "Review Shop"]) == "Agent: Review Shop")
    assert(stepLabel(tool: "mcp__mail__listEmails", input: [:]) == "mail: listEmails")
    assert(stepLabel(tool: "WebFetch", input: ["url": "https://docs.warp.dev/x"]) == "liest Webseite docs.warp.dev")
    assert(stepLabel(tool: "AskUserQuestion", input: ["questions": [["question": "Welche?"]]]) == "Frage: Welche?")
}

private func tabSession(_ id: String, ready: Bool, waiting: Bool = false, title: String?) -> Session {
    var s = Session(id: "claude:\(id)", engine: .claude, pid: 1, name: id, cwd: "/Users/test/Projects/Beta",
                    ready: ready, waiting: waiting, alive: true)
    s.digest = Digest(title: title)
    return s
}

private func tabWindow(_ sessions: [Session], title: String) -> TermWindow {
    TermWindow(app: "Warp", bundleId: WARP_BID, ownerPid: 0, ax: AXUIElementCreateApplication(0),
               frame: .zero, title: title, sessions: sessions, firstSeen: Date(timeIntervalSince1970: 1000),
               movable: true, minimized: false, fullscreen: false)
}

/// Kevin 2026-09-21: „das oberste Fenster leuchtet grün, es arbeitet aber". /handoff öffnet den
/// neuen Tab im selben Fenster; der alte Tab (beta-52) war fertig, der sichtbare (beta-e5)
/// arbeitete — „irgendein Tab fertig" machte das ganze Fenster grün.
func testVisibleTabDecides() {
    let alt = tabSession("beta-52", ready: true, title: "Handoff spawn loop closing")
    let neu = tabSession("beta-e5", ready: false, title: "Handoff spawn 2026-09-21 beta-pr1-build")
    let w = tabWindow([alt, neu], title: "◐ Handoff spawn 2026-09-21 beta-pr1-build")
    assert(w.visibleSession?.id == neu.id, "der sichtbare Tab wird am Thema erkannt")
    assert(!w.isReady, "sichtbarer Tab arbeitet -> Fenster arbeitet")
    assert(glowTargets(windows: [w], onScreen: nil, screenH: 1117, screens: [macbook]).isEmpty, "kein Rahmen")
    // Fokus 2: das arbeitende Fenster reiht sich bei den Laufenden ein (nach Erstsichtung).
    let laeuft = fakeWindow(ready: false, firstSeen: Date(timeIntervalSince1970: 500))
    assert(attentionOrder([w, laeuft]) == [1, 0], "kein Vordrängeln zu den Wartenden")
    var zurueck = w
    zurueck.title = "✳ Handoff spawn loop closing"
    assert(zurueck.visibleSession?.id == alt.id && zurueck.isReady, "Kevin schaltet auf den alten Tab: fertig")
}

func testVisibleTabFallbacks() {
    let a = tabSession("a", ready: true, title: "Thema A")
    let b = tabSession("b", ready: false, title: "Thema B")
    // Thema noch nicht im Transkript (6-s-Takt) -> das Zeichen im Titel entscheidet.
    let unbekannt = tabWindow([a, b], title: "◐ Neues Thema")
    assert(unbekannt.visibleSession == nil && !unbekannt.isReady, "Kreisel -> arbeitet")
    let ruhig = tabWindow([a, b], title: "✳ Neues Thema")
    assert(ruhig.isReady && !ruhig.isWaiting, "✳ -> fertig")
    // Kein Zeichen (Ordnername nach Ping-Rückschreiben) -> bisherige Regel.
    assert(tabWindow([a, b], title: "Beta").isReady, "ohne Anhaltspunkt: fertig, sobald ein Tab fertig ist")
    // Eine Session: exakt wie bisher — der Titel spielt keine Rolle.
    assert(!tabWindow([b], title: "✳ irgendwas").isReady, "eine Session entscheidet selbst")
    assert(!tabWindow([], title: "✳ irgendwas").isReady, "Fenster ohne Session ist nie fertig")
}

/// review quick 2026-09-21: ein eindeutiger Themen-Treffer beweist nur etwas, wenn jede Claude-Session
/// im Fenster ihr Thema kennt (GLM); ohne Claude-Zeichen im Titel ist die EINE Nicht-Claude-Session
/// sichtbar — Codex/Grok/Kimi schreiben keine Zeichen, und /handoff startet auch sie im Tab.
func testVisibleTabRobust() {
    let codex = Session(id: "codex:1", engine: .codex, pid: 2, name: "Alpha",
                        cwd: "/Users/test/Projects/Alpha", ready: false, alive: true)
    // GLM-Fund: der versteckte Claude-Tab heißt zufällig wie der Codex-Titel (Ordnername).
    let w = tabWindow([tabSession("alt", ready: true, title: "Alpha"), codex], title: "Alpha")
    assert(w.visibleSession?.id == codex.id && !w.isReady, "ohne Claude-Zeichen ist die Codex-Session sichtbar")
    // /handoff nach Codex: alter Claude-Tab fertig, Codex arbeitet — die alte Regel machte das grün.
    let handoff = tabWindow([tabSession("alt2", ready: true, title: "Alte Aufgabe"), codex], title: "Alpha")
    assert(!handoff.isReady, "Handoff nach Codex: der arbeitende Codex-Tab entscheidet")
    // GLM-Fund, Variante: sichtbare Claude-Session ohne Transkript, versteckte mit gleichem Thema.
    let ohneThema = Session(id: "claude:neu", engine: .claude, pid: 3, name: "neu", cwd: "", ready: false, alive: true)
    let gleich = tabWindow([tabSession("alt3", ready: true, title: "Handoff spawn 2026-09-21"), ohneThema],
                           title: "◐ Handoff spawn 2026-09-21")
    assert(gleich.visibleSession == nil && !gleich.isReady, "Treffer nicht beweisend -> das Zeichen entscheidet")
    // Leerzeichen am Thema stören den Abgleich nicht (Kimi).
    let a = tabSession("a", ready: true, title: "Thema A ")
    let b = tabSession("b", ready: false, title: "Thema B")
    assert(tabWindow([a, b], title: "✳ Thema A").visibleSession?.id == a.id, "Thema mit Leerzeichen am Ende")
}

func testHiddenQuestion() {
    let fragt = tabSession("alt", ready: true, waiting: true, title: "Alte Aufgabe")
    let arbeitet = tabSession("neu", ready: false, title: "Neue Aufgabe")
    let w = tabWindow([fragt, arbeitet], title: "◐ Neue Aufgabe")
    assert(!w.isWaiting && w.hiddenQuestion, "Frage im versteckten Tab: nur Hinweis, kein Bernstein")
    let sichtbar = tabWindow([fragt, arbeitet], title: "✳ Alte Aufgabe")
    assert(sichtbar.isWaiting && !sichtbar.hiddenQuestion, "sichtbare Frage: Rahmen, kein Extra-Hinweis")
    let fertig = tabSession("f", ready: true, title: "Fertig")
    assert(!tabWindow([fertig, arbeitet], title: "◐ Neue Aufgabe").hiddenQuestion,
           "fertige versteckte Tabs werden ignoriert (Kevin 2026-09-21)")
}

func testTranscriptReader() {
    let root = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    let dir = root + "/projects/-Users-test-Projects-Beta"
    try! FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: root) }
    let path = dir + "/sid-1.jsonl"
    try! (#"{"type":"ai-title","aiTitle":"Erstes Thema"}"# + "\n").write(toFile: path, atomically: true, encoding: .utf8)

    let reader = TranscriptReader(configDirs: [root])
    assert(reader.digest(sessionId: "sid-1")?.title == "Erstes Thema", "in irgendeinem Projektordner gefunden")
    assert(reader.digest(sessionId: "fehlt") == nil, "ohne Transkript: nil, kein Absturz")
    let fh = FileHandle(forWritingAtPath: path)!
    fh.seekToEndOfFile()
    fh.write(Data((#"{"type":"ai-title","aiTitle":"Zweites Thema"}"# + "\n").utf8))
    try? fh.close()
    assert(reader.digest(sessionId: "sid-1")?.title == "Zweites Thema", "gewachsene Datei wird neu gelesen")
    reader.forget(except: [])
    assert(reader.digest(sessionId: "sid-1")?.title == "Zweites Thema", "nach dem Vergessen neu gesucht")

    // review quick 2026-10-10 (GLM): die Nachricht der anderen Session faellt nach einem langen Zug aus dem Ausschnitt.
    let small = TranscriptReader(configDirs: [root], tailBytes: 400)
    let peer = #"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/55848.sock","verifiedPeerPid":55848,"name":"Lead"},"message":{"content":"go"}}"#
    let w = FileHandle(forWritingAtPath: path)!
    w.seekToEndOfFile()
    w.write(Data((peer + "\n").utf8))
    assert(small.digest(sessionId: "sid-1")?.peerPid == 55848, "Nachricht im Ausschnitt")
    w.write(Data((String(repeating: #"{"type":"progress"}"# + "\n", count: 40)).utf8))
    try? w.close()
    let later = small.digest(sessionId: "sid-1")
    assert(later?.peerPid == 55848 && later?.peerName == "Lead", "aus dem Ausschnitt gefallen: gemerkt")
}

func testAdvanceSince() {
    let t0 = Date(timeIntervalSince1970: 10_000)
    var laeuft = Session(id: "claude:a", engine: .claude, pid: 1, name: "a", cwd: "", ready: false, alive: true)
    laeuft.digest = Digest(answerAt: t0.addingTimeInterval(-5), promptAt: t0.addingTimeInterval(-600))
    var fertig = Session(id: "claude:b", engine: .claude, pid: 2, name: "b", cwd: "", ready: true, alive: true)
    fertig.digest = Digest(answerAt: t0.addingTimeInterval(-360))
    let first = advanceSince([:], sessions: [laeuft, fertig], now: t0)
    assert(first["claude:a"]?.since == t0.addingTimeInterval(-600), "läuft seit Kevins letztem Prompt")
    assert(first["claude:b"]?.since == t0.addingTimeInterval(-360), "fertig seit der letzten Antwort")
    let same = advanceSince(first, sessions: [laeuft, fertig], now: t0.addingTimeInterval(6))
    assert(same == first, "gleicher Zustand: Beginn bleibt")
    let jetztFertig = Session(id: "claude:a", engine: .claude, pid: 1, name: "a", cwd: "", ready: true, alive: true)
    let t1 = t0.addingTimeInterval(12)
    let changed = advanceSince(same, sessions: [jetztFertig], now: t1)
    assert(changed["claude:a"] == SinceEntry(tone: .ready, since: t1), "Wechsel: ab jetzt")
    assert(changed["claude:b"] == nil, "verschwundene Session fällt raus")
    var ohne = Session(id: "claude:c", engine: .claude, pid: 3, name: "c", cwd: "", ready: false, alive: true)
    ohne.digest = Digest()
    assert(advanceSince([:], sessions: [ohne], now: t0)["claude:c"]?.since == nil, "unbekannt -> nil, nicht geraten")
    var wacht = Session(id: "claude:w", engine: .claude, pid: 4, name: "w", cwd: "", ready: false, alive: true)
    wacht.watching = true
    wacht.digest = Digest(answerAt: t0.addingTimeInterval(-120), promptAt: t0.addingTimeInterval(-900))
    assert(advanceSince([:], sessions: [wacht], now: t0)["claude:w"]
           == SinceEntry(tone: .watching, since: t0.addingTimeInterval(-120)), "überwacht seit der letzten Antwort")
}

func testDurationText() {
    // Kevin 2026-09-26 (Schild B): ausgeschrieben, damit "FERTIG  seit 6 Min." sich wie ein Satz liest
    assert(durationText(30) == "gerade eben")
    assert(durationText(6 * 60 + 20) == "6 Min.")
    assert(durationText(3600 + 5 * 60) == "1 Std. 5 Min.")
    assert(durationText(2 * 3600 + 20) == "2 Std.", "volle Stunde ohne '0 Min.'")
    assert(durationText(-4) == "gerade eben", "Uhr-Zucken: nie negativ")
}

func testFirstSentence() {
    assert(firstSentence("Kevin, die Übersichtsseite ist jetzt in deinem Browser offen. Details folgen.")
           == "Die Übersichtsseite ist jetzt in deinem Browser offen.", "Anrede weg, erster Satz")
    assert(firstSentence("Kevin 👋 **Die Währungsfrage ist geklärt: in Euro buchen.** Damit gilt …")
           == "Die Währungsfrage ist geklärt: in Euro buchen.", "Emoji + Markdown weg")
    assert(firstSentence("Kevins Plan steht.") == "Kevins Plan steht.", "nur die Anrede, nicht 'Kevins'")
    assert(firstSentence("Für 2.1 gibt es noch keine bessere Version") == "Für 2.1 gibt es noch keine bessere Version",
           "Punkt in einer Zahl ist kein Satzende")
    assert(firstSentence("## Ergebnis\n- alles grün") == "Ergebnis alles grün", "Überschrift und Aufzählung geglättet")
    assert(firstSentence(String(repeating: "a", count: 300)).count == 160, "hart gekürzt")
    // Kevin 2026-09-27 (Screenshot): "12 is running." — ein "#" vor einer Zahl ist eine Nummer, keine Überschrift
    assert(firstSentence("#12 is running. Its window opened at 23:27.") == "#12 is running.", "Nummer am Zeilenanfang bleibt")
    assert(firstSentence("Kevin, #12 is running. Its window opened.") == "#12 is running.", "Nummer nach der Anrede bleibt")
    assert(firstSentence("-5 Grad am Morgen. Mehr folgt.") == "-5 Grad am Morgen.", "Minus vor einer Zahl ist keine Aufzählung")
}

func testLabelContent() {
    let now = Date(timeIntervalSince1970: 20_000)
    var s = Session(id: "claude:e5", engine: .claude, pid: 1, name: "beta-e5",
                    cwd: "/Users/test/Projects/Beta/.claude/worktrees/shop+feature-pr1", ready: false, alive: true)
    s.digest = Digest(name: "Build queue cards", step: "Tests der Karten laufen lassen",
                      text: "Kevin, Zwischenstand: die Hälfte steht.", last: .tool)
    let laeuft = labelContent(for: tabWindow([s], title: "◐ Build queue cards"),
                              since: SinceEntry(tone: .running, since: now.addingTimeInterval(-22 * 60)), now: now)
    // Kevin 2026-09-26 (Schild B): Klartext-Status, Zeit immer mit "seit" — liest sich wie ein Satz. Kevin 2026-10-12
    // ("Ruhig"): beim Arbeiten keine dritte Zeile, die mit jedem Schritt wechselt.
    assert(laeuft.lines == ["● ARBEITET  seit 22 Min.", "Build queue cards"], "bekam \(laeuft.lines)")
    let zeile = sessionLine(tabWindow([s], title: "◐ Build queue cards"),
                            since: [s.id: SinceEntry(tone: .running, since: now.addingTimeInterval(-22 * 60))], now: now)
    assert(zeile.title == "● Build queue cards — seit 22 Min.",
           "Menü- und Listenzeile mit derselben Zeitangabe — bekam \(zeile.title)")
    s.digest?.last = .text
    assert(labelContent(for: tabWindow([s], title: "◐ Build queue cards"), since: nil, now: now).detail == nil,
           "schreibt gerade Text: auch dann ruhig")

    var f = Session(id: "claude:14", engine: .claude, pid: 2, name: "alpha-14",
                    cwd: "/Users/test/Projects/Alpha", ready: true, alive: true)
    f.digest = Digest(name: "Rechnungen prüfen",
                      text: "Kevin, die Übersichtsseite ist jetzt in deinem Browser offen. Mehr folgt.", last: .text)
    let fertig = labelContent(for: tabWindow([f], title: "✳ Rechnungen prüfen"),
                              since: SinceEntry(tone: .ready, since: now.addingTimeInterval(-6 * 60)), now: now)
    assert(fertig.lines == ["✓ FERTIG  seit 6 Min.", "Rechnungen prüfen",
                            "Die Übersichtsseite ist jetzt in deinem Browser offen."], "bekam \(fertig.lines)")

    var q = Session(id: "claude:f8", engine: .claude, pid: 3, name: "gamma-f8",
                    cwd: "/Users/test/Projects/Gamma", ready: true, waiting: true, alive: true)
    q.digest = Digest(title: "Handoff spawn 2026-09-21", step: "Frage: in Euro buchen?", question: "in Euro buchen?")
    let frage = labelContent(for: tabWindow([q], title: "✳ Handoff spawn 2026-09-21"),
                             since: SinceEntry(tone: .waiting, since: now.addingTimeInterval(-30)), now: now)
    // "wartet auf dich" statt "Frage an dich": deckt auch die Freigabe eines Schritts ab (unten). Ohne eigenen Namen nie
    // Claudes Auto-Titel, nur der Ordner (Kevin 2026-10-12: der Titel wechselt laufend, "Ja mach das · ClaudeCode").
    assert(frage.lines == ["? WARTET AUF DICH  gerade eben", "Gamma",
                           "„in Euro buchen?“"], "bekam \(frage.lines)")
    q.digest = Digest(title: "Handoff spawn 2026-09-21", step: "Entwurf anlegen")
    assert(labelContent(for: tabWindow([q], title: "✳ Handoff spawn 2026-09-21"), since: nil, now: now).detail
           == "Freigabe: Entwurf anlegen", "Frage ohne AskUserQuestion = Freigabe für den Schritt")

    let codex = Session(id: "codex:1", engine: .codex, pid: 4, name: "Alpha",
                        cwd: "/Users/test/Projects/Alpha", ready: false, alive: true)
    let c = labelContent(for: tabWindow([codex], title: "Alpha"), since: nil, now: now)
    assert(c.lines == ["● ARBEITET", "Alpha"], "ohne Transkript: Status + Fenstertitel, nichts doppelt — bekam \(c.lines)")
    assert(labelContent(for: tabWindow([codex], title: "Alpha"),
                        since: SinceEntry(tone: .ready, since: now), now: now).duration == nil,
           "Dauer eines anderen Zustands wird nicht gezeigt")

    var alt = q
    alt.digest = Digest(title: "Alte Aufgabe")
    assert(labelContent(for: tabWindow([alt, s], title: "◐ Build queue cards"), since: nil, now: now).hint
           == "Tab dahinter hat eine Frage")
    // Sichtbarer Tab nicht erkennbar (Thema noch nicht im Transkript): Thema aus dem Titel, aber
    // kein Schritt — der könnte zum versteckten Tab gehören.
    let unklar = labelContent(for: tabWindow([f, s], title: "◐ Neues Thema"), since: nil, now: now)
    assert(unklar.topic == "Neues Thema" && unklar.detail == nil, "bekam \(unklar.lines)")
}

func testLabelTargets() {
    let t0 = Date(timeIntervalSince1970: 1000)
    var aktiv = fakeWindow(ready: true, firstSeen: t0)
    var laeuft = fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(1))
    var voll = fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(2))
    var shell = fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(3), hasSession: false)
    var winzig = fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(4))
    aktiv.frame = CGRect(x: 0, y: 0, width: 1248, height: 999)
    laeuft.frame = CGRect(x: 1248, y: 884, width: 480, height: 200)
    voll.frame = macbook
    shell.frame = CGRect(x: 1248, y: 684, width: 480, height: 200)
    winzig.frame = CGRect(x: 1248, y: 84, width: 200, height: 90)
    let ziele = labelTargets(windows: [aktiv, laeuft, voll, shell, winzig], onScreen: nil, screenH: 1117,
                             screens: [macbook], focusedIndex: 0, since: [:], now: t0)
    // Kimi 2026-09-25: "voll" ist die einzige Kachel auf dem MacBook, von Lookout selbst so gross gelegt —
    // sie braucht ihr Schild. Die Maximiert-Regel gilt nur dem Rahmen (Kevin 2026-09-18).
    assert(ziele.map(\.id) == [laeuft.sessions[0].id, voll.sessions[0].id],
           "Schild auch auf laufenden und voll gekachelten Fenstern — nicht auf aktivem, Shell, zu kleinem")
    let r = ziele[0].rect
    assert(r.maxY == laeuft.frame.maxY - 40 - 2 && r.minX > laeuft.frame.minX && r.maxX < laeuft.frame.maxX,
           "direkt unter der Warp-Tab-Leiste, innerhalb des Fensters")
    // Kevin 2026-09-26 (Vorschlag B, "größer und klarer"): Status + Name 16 pt auf 22-pt-Zeilen, Schritt 13,5 pt auf
    // 18 pt, Rand 9 — zwei Zeilen 62 pt hoch, drei 80 pt (vorher alles 14 pt: 54 bzw. 73 pt).
    assert(r.height == 9 * 2 + 22 + 22, "zwei Zeilen: Status + Name, bekam Höhe \(r.height)")
    assert(labelRect(window: laeuft.frame, bundleId: WARP_BID, lines: 3).height == 9 * 2 + 22 + 22 + 18,
           "Schritt-Zeile kleiner als Status und Name")
    assert([0, 1, 2, 3].map { labelLineStyle($0).size } == [16, 16, 13.5, 13.5] && labelLineStyle(0).weight == .bold,
           "Status fett, Name groß, Schritt und Hinweis kleiner")
    // Kontrast nachgerechnet 26.09.: bei 58 % Deckkraft über hellem Terminal-Text nur 2,8:1, ab 92 % mindestens 9,5:1
    assert(LABEL_BG_ALPHA >= 0.92, "Hintergrund fast deckend")
}

/// Menü-Kopf mit denselben Wörtern wie das Schild (Kevin 2026-09-26).
func testSessionsHeader() {
    assert(sessionsHeader(asking: 0, ready: 0) == "SESSIONS")
    assert(sessionsHeader(asking: 1, ready: 2) == "SESSIONS (1 wartet auf dich · 2 fertig)")
    assert(sessionsHeader(asking: 2, ready: 0) == "SESSIONS (2 warten auf dich)")
}

// ---------- v4: saubere Namen, Fenster am Namen, verworfene Klicks (Design 2026-09-21) ----------

/// Der Namens-Hook setzt `custom-title` im Transkript (Probe 2026-09-21) — der gewinnt vor dem Auto-Titel.
func testDigestCustomTitle() {
    let t = [#"{"type":"ai-title","aiTitle":"Handoff spawn 2026-09-21 Gamma Umzug"}"#,
             #"{"type":"custom-title","customTitle":"Gamma","sessionId":"s"}"#,
             #"{"type":"custom-title","customTitle":"Gamma · Buchhaltung weiterführen","sessionId":"s"}"#]
        .joined(separator: "\n")
    let d = digestTranscript(t)
    assert(d.name == "Gamma · Buchhaltung weiterführen", "der letzte eigene Name gilt")
    assert(d.title == "Handoff spawn 2026-09-21 Gamma Umzug", "der Auto-Titel bleibt daneben erhalten")
}

func testNameOnLabelAndTab() {
    var s = Session(id: "claude:f8", engine: .claude, pid: 3, name: "gamma-f8",
                    cwd: "/Users/test/Projects/Gamma", ready: false, alive: true)
    s.digest = Digest(title: "Handoff spawn 2026-09-21", name: "Gamma · Buchhaltung weiterführen",
                      step: "liest die Doku", last: .tool)
    let c = labelContent(for: tabWindow([s], title: "◐ Gamma · Buchhaltung weiterführen"), since: nil, now: Date())
    assert(c.topic == "Gamma · Buchhaltung weiterführen", "Name statt Auto-Titel, kein Ordner dahinter — \(c.topic)")
    let alt = tabSession("alt", ready: true, title: "Alte Aufgabe")
    let w = tabWindow([alt, s], title: "◐ Gamma · Buchhaltung weiterführen")
    assert(w.visibleSession?.id == s.id && !w.isReady, "sichtbarer Tab über den eigenen Namen erkannt")
    let uebergang = tabWindow([alt, s], title: "◐ Handoff spawn 2026-09-21")
    assert(uebergang.visibleSession?.id == s.id, "Übergang: der Titel zeigt noch den Auto-Titel")
}

/// Kevin 2026-09-21: ein neues Fenster kam erst spät in die Mitte. Log: arbeitende Sessions übermalen den
/// Ping-Marker ("ohne Marker", viermal in Folge) — jetzt erkennt Lookout Fenster am Namen im Titel.
func testTitleMatchesWindows() {
    var a = Session(id: "claude:a", engine: .claude, pid: 11, name: "a", cwd: "", ready: false, alive: true)
    a.digest = Digest(title: "Auto A", name: "Lookout · Fenster-Übersicht")
    var b = Session(id: "claude:b", engine: .claude, pid: 12, name: "b", cwd: "", ready: true, alive: true)
    b.digest = Digest(title: "Qwen-Image safety check")
    let codex = Session(id: "codex:c", engine: .codex, pid: 13, name: "Alpha",
                        cwd: "/Users/test/Projects/Alpha", ready: false, alive: true)
    let wins = [tabWindow([], title: "◐ Lookout · Fenster-Übersicht"), tabWindow([], title: "✳ Qwen-Image safety check"),
                tabWindow([], title: "Alpha"), tabWindow([], title: "◑ Unbekannt")]
    // Laufende Terminal-Prozesse genau dieser Sessions
    func procs(_ s: [Session]) -> [EngineProc] {
        s.compactMap { x in x.pid.map { EngineProc(pid: $0, engine: x.engine, tty: "ttys0\($0)", cwd: "") } }
    }
    let m = titleMatches(sessions: [a, b, codex], windows: wins, procs: procs([a, b, codex]))
    assert(m == [11: 0, 12: 1], "eindeutige Claude-Paare über Name bzw. Auto-Titel, bekam \(m)")
    var zwilling = Session(id: "claude:a2", engine: .claude, pid: 14, name: "a2", cwd: "", ready: false, alive: true)
    zwilling.digest = a.digest
    assert(titleMatches(sessions: [a, zwilling], windows: wins, procs: procs([a, zwilling])).isEmpty,
           "zwei Sessions mit gleichem Namen -> Ping entscheidet")
    let doppelt = [tabWindow([], title: "◐ Lookout · Fenster-Übersicht"), tabWindow([], title: "◑ Lookout · Fenster-Übersicht")]
    assert(titleMatches(sessions: [a], windows: doppelt, procs: procs([a])).isEmpty, "zwei Fenster mit gleichem Titel -> keine Zuordnung")
    // Ohne Claude-Zeichen zeigt das Fenster keine Claude-Session (Codex-Ordnername, Ping-Rückschreiben)
    var ordner = Session(id: "claude:o", engine: .claude, pid: 15, name: "o", cwd: "", ready: true, alive: true)
    ordner.digest = Digest(title: "Alpha")
    assert(titleMatches(sessions: [ordner], windows: wins, procs: procs([ordner])).isEmpty, "Titel ohne Zeichen beweist nichts")
    // review quick v4: eindeutig ist ein Titel nur, wenn JEDE Claude-Session im Terminal bekannt ist. Eine frische
    // (noch nicht abgefragt oder noch ohne Transkript) trägt womöglich schon denselben Startnamen — sonst hinge eine
    // Hintergrund-Session dauerhaft am falschen Fenster (der Ping läuft für zugeordnete nie wieder).
    let frisch = procs([a, b, codex]) + [EngineProc(pid: 16, engine: .claude, tty: "ttys016", cwd: "")]
    assert(titleMatches(sessions: [a, b, codex], windows: wins, procs: frisch).isEmpty, "Claude-Prozess ohne Session -> Ping entscheidet")
    let ohneTitel = Session(id: "claude:n", engine: .claude, pid: 16, name: "n", cwd: "", ready: false, alive: true)
    assert(titleMatches(sessions: [a, b, codex, ohneTitel], windows: wins, procs: frisch).isEmpty,
           "Session ohne bekannten Namen -> Ping entscheidet")
    let grok = procs([a, b, codex]) + [EngineProc(pid: 17, engine: .grok, tty: "ttys017", cwd: "")]
    assert(titleMatches(sessions: [a, b, codex], windows: wins, procs: grok) == [11: 0, 12: 1], "andere Engines stören nicht")
}

/// Name und Auto-Titel schreibt Claude nur bei jeder Nachricht neu — nach einem langen Zug liegen sie bis
/// ~5,5 MB vor dem Dateiende (gemessen 2026-09-21), weit vor dem gelesenen Ausschnitt (review quick v4).
func testTranscriptTitlesFarBack() {
    let root = NSTemporaryDirectory() + "lookout-test-\(UUID().uuidString)"
    let dir = root + "/projects/-Users-test-Projects-Alpha"
    try! FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: root) }
    let path = dir + "/sid-2.jsonl"
    func append(_ s: String) {
        let fh = FileHandle(forWritingAtPath: path)!
        fh.seekToEndOfFile()
        fh.write(Data(s.utf8))
        try? fh.close()
    }
    let filler = String(repeating: #"{"type":"assistant","message":{"content":[{"type":"text","text":"weiter"}]}}"# + "\n",
                        count: 60)
    try! (#"{"type":"custom-title","customTitle":"Gamma · Buchhaltung weiterführen"}"# + "\n"
          + #"{"type":"ai-title","aiTitle":"Ledger books"}"# + "\n" + filler)
        .write(toFile: path, atomically: true, encoding: .utf8)

    let reader = TranscriptReader(configDirs: [root], tailBytes: 1024)
    let first = reader.digest(sessionId: "sid-2")
    assert(first?.name == "Gamma · Buchhaltung weiterführen" && first?.title == "Ledger books",
           "erstes Lesen sucht rückwärts, bekam \(String(describing: first))")
    append(filler)
    let later = reader.digest(sessionId: "sid-2")
    assert(later?.name == "Gamma · Buchhaltung weiterführen" && later?.title == "Ledger books",
           "gewachsen ohne neuen Namen: der zuletzt gesehene bleibt")
    append(#"{"type":"custom-title","customTitle":"Neu"}"# + "\n")
    assert(reader.digest(sessionId: "sid-2")?.name == "Neu", "neuer Name gewinnt")
    // review quick 2026-10-01 (Codex): nach einem langen Zug steht Kevins Prompt nicht mehr im Dateiende — der zuletzt
    // gesehene Ausloeser bleibt, bis ein neuer kommt.
    append(#"{"type":"user","origin":{"kind":"human"},"turnPosition":{"promptIndex":1,"turnIndex":1},"message":{"content":"und jetzt CI?"}}"# + "\n")
    assert(reader.digest(sessionId: "sid-2")?.kevinTurn == true, "Kevins Prompt im Ausschnitt")
    // Schritt fuer Schritt aus dem Ausschnitt (je Lesung weniger Zuwachs als der Ausschnitt): sein Text bleibt mit.
    let wenig = String(repeating: #"{"type":"assistant","message":{"content":[{"type":"text","text":"weiter"}]}}"# + "\n",
                       count: 8)
    for _ in 0..<3 {
        append(wenig)
        _ = reader.digest(sessionId: "sid-2")
    }
    assert(reader.digest(sessionId: "sid-2")?.kevinTurn == true, "Prompt aus dem Ausschnitt gefallen: bleibt Kevins Zug")
    assert(reader.digest(sessionId: "sid-2")?.kevinPrompt == "und jetzt CI?", "sein Text bleibt mit (Start-Prompt-Prüfung)")
    // review quick 2026-10-01 (Codex): wuchs die Datei zwischen zwei Lesungen um mehr als den Ausschnitt, kann dort Kevins
    // Nachricht mitten im Zug stehen — dann gilt der gemerkte Start-Prompt nicht mehr, im Zweifel Kevins Zug.
    append(filler)
    assert(reader.digest(sessionId: "sid-2")?.kevinTurn == true, "Sprung: bleibt Kevins Zug")
    assert(reader.digest(sessionId: "sid-2")?.kevinPrompt == nil, "Sprung über den Ausschnitt: nie Start-Prompt")
    append(#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/x.sock","verifiedPeerPid":7,"name":"Lead"},"message":{"content":"go"}}"# + "\n")
    assert(reader.digest(sessionId: "sid-2")?.kevinTurn == false, "die andere Session löst den nächsten Zug aus")

    // Eine Namenszeile über einer Blockgrenze geht nicht verloren; gesucht wird nur bis zur Obergrenze.
    for chunk in [16, 37, 100, 4096] {
        let t = lastTitles(path: path, chunk: chunk, maxBytes: 1 << 20)
        assert(t.name == "Neu" && t.title == "Ledger books", "Block \(chunk): \(t)")
    }
    assert(lastTitles(path: path, chunk: 100, maxBytes: 2048).title == nil, "nur bis zur Obergrenze")
    let fehlt = lastTitles(path: root + "/fehlt.jsonl")
    assert(fehlt.name == nil && fehlt.title == nil, "fehlende Datei: nichts, kein Absturz")
}

/// Klicks während Lookout selbst Fenster bewegt, gingen verloren (Kevin 2026-09-21) — jetzt vorgemerkt.
/// Live 2026-09-25 nach dem Neubau: Chrome war vorn, Lookout kannte keine aktive Session — ein beliebiges Fenster stand
/// mit Mint-Rahmen in der Mitte, bis Kevin ein Terminal anklickte. Beim Start zaehlt darum auch das zuletzt fokussierte
/// Fenster eines Terminals, das gerade nicht vorn ist.
func testFocusSourceAtStart() {
    assert(focusSource(frontPid: 5, terminalPids: [5, 6], aiTerminalPids: [6], activeKnown: true) == 5,
           "ein Terminal vorn: dessen Fokus zaehlt")
    assert(focusSource(frontPid: 9, terminalPids: [5, 6], aiTerminalPids: [6], activeKnown: true) == nil,
           "andere App vorn, aktive Session bekannt: nichts aendern")
    assert(focusSource(frontPid: 9, terminalPids: [5, 6], aiTerminalPids: [6], activeKnown: false) == 6,
           "Start mit Chrome vorn: zuletzt fokussiertes Fenster des KI-Terminals")
    assert(focusSource(frontPid: nil, terminalPids: [], aiTerminalPids: [], activeKnown: false) == nil, "ohne Terminal nichts")
}

/// Ein scheiterndes oder haengendes `ps` heisst "unbekannt", nicht "keine Prozesse" (Kevin 2026-09-25, Review Fund 10:
/// 6x nachts im Log — die leere Liste loeschte jede Fenster-Zuordnung, danach wurde alles neu angepingt).
func testProcessListFailureIsUnknown() {
    assert(engineProcesses(psPath: "/usr/bin/false") == nil, "gescheitertes ps -> nil, nicht []")
    assert(engineProcesses() != nil, "echtes ps liefert eine Liste")
}

func testMoveGate() {
    var g = MoveGate()
    assert(g.event(selfMove: false), "ohne Bewegung sofort auswerten")
    assert(!g.event(selfMove: true) && g.pending, "während der Bewegung vormerken")
    assert(g.moveEnded() && !g.pending, "nach der Bewegung einmal nachholen")
    assert(!g.moveEnded(), "nichts vorgemerkt -> nichts nachholen")
}

// ---------- Durchsicht 2026-09-25 (Kevins Screenshot: gruener Rahmen ohne Grund) ----------

/// Kevin 2026-09-25: gruener Rahmen oben links auf dem MacBook, quer ueber einer laufenden Session. AppKit rueckte die
/// Overlay-Panels zurecht (`constrainFrameRect`): unter die Menueleiste — der Rahmen sass oben 7 pt zu tief — und beim
/// Wechsel vom MacBook an die LG-Unterkante zurueck aufs MacBook (Probe: −7,−460 statt −7,−7). Overlay-Panels muessen
/// exakt dort liegen, wo Lookout sie hinsetzt.
func testOverlayPanelsStayWhereTold() {
    _ = NSApplication.shared
    let screens = NSScreen.screens
    guard let first = screens.first else { return }
    let p = makeOverlayPanel(level: .normal)
    defer { p.orderOut(nil) }
    func place(_ r: CGRect) -> CGRect {
        p.setFrame(r, display: false)
        p.orderFrontRegardless()
        return p.frame
    }
    // Oberkante 7 pt in der Menueleiste: das Rahmen-Panel eines Fensters oben am Schirm.
    let oben = CGRect(x: first.visibleFrame.minX + 40, y: first.visibleFrame.maxY - 293, width: 400, height: 300)
    assert(place(oben) == oben, "oben verschoben: \(p.frame) statt \(oben)")
    // Zweiter Schirm (MacBook unter dem LG): von dort auf einen Rahmen, der die Unterkante des ersten beruehrt und
    // 7 pt in den zweiten ragt — der Fall aus dem Screenshot.
    for other in screens.dropFirst() {
        let o = other.frame
        _ = place(CGRect(x: o.midX - 200, y: o.midY - 150, width: 400, height: 300))
        let unten = CGRect(x: max(first.frame.minX, o.minX) - 7, y: first.frame.minY - 7, width: 818, height: 427)
        assert(place(unten) == unten, "Schirmwechsel verschoben: \(p.frame) statt \(unten)")
    }
}

/// Kevin 2026-09-25: der Rahmen gleitet mit seinem Fenster, statt waehrend der Fahrt an der alten Stelle zu stehen
/// (GLM+Kimi). Ein Tick mit veraltetem Rechteck zieht ihn dabei nicht zurueck.
func testGlowFollowsGlidingWindow() {
    _ = NSApplication.shared
    let glow = GlowOverlay()
    let r1 = CGRect(x: 100, y: 100, width: 400, height: 300)
    let r2 = CGRect(x: 600, y: 100, width: 400, height: 300)
    glow.update(targets: [GlowTarget(id: "a", rect: r1, waiting: false, windowNumber: nil)])
    let panel = NSApplication.shared.windows.first { $0.frame == r1.insetBy(dx: -7, dy: -7) }
    assert(panel != nil, "Rahmen-Panel um das Fenster")
    glow.follow(["a": r2])
    assert(panel?.frame == r2.insetBy(dx: -7, dy: -7), "folgt dem gleitenden Fenster, bekam \(String(describing: panel?.frame))")
    glow.update(targets: [GlowTarget(id: "a", rect: r1, waiting: false, windowNumber: nil)], frozen: ["a"])
    assert(panel?.frame == r2.insetBy(dx: -7, dy: -7), "veralteter Tick zieht den gleitenden Rahmen nicht zurueck")
    panel?.orderOut(nil)
}

/// Ein Schnappschuss, waehrend dessen Lookout selbst bewegt hat, zeigt Zwischenstaende: die zuletzt gemessenen Rahmen
/// gelten weiter — sonst sprang ein gerade angekommenes Fenster zurueck und glitt neu los (Variante zu GLM/Kimi
/// "Menue-Klick waehrend der Fahrt", 2026-09-25).
func testSnapshotDuringOwnMove() {
    func win(_ pid: pid_t, _ frame: CGRect) -> TermWindow {
        TermWindow(app: "Warp", bundleId: WARP_BID, ownerPid: 0, ax: AXUIElementCreateApplication(pid), frame: frame,
                   title: "", firstSeen: Date(timeIntervalSince1970: 1000), movable: true, minimized: false,
                   fullscreen: false)
    }
    let angekommen = CGRect(x: 0, y: 0, width: 804, height: 413)
    let unterwegs = CGRect(x: 400, y: -300, width: 804, height: 413)
    let fremd = CGRect(x: 1000, y: 0, width: 500, height: 500)
    let merged = keepMeasuredFrames([win(1, unterwegs), win(2, fremd)], known: [win(1, angekommen)])
    assert(merged[0].frame == angekommen, "gemessener Rahmen gilt, nicht der Zwischenstand")
    assert(merged[1].frame == fremd, "unbekanntes Fenster behaelt seinen Schnappschuss")
    // Plan 1c (R2-4): `bringToFront` holt ein minimiertes Fenster zurueck; ein Tick, der schon lief, bringt noch den
    // alten Schnappschuss mit und darf es nicht zurueck auf "minimiert" setzen.
    var alt = win(3, angekommen)
    alt.minimized = true
    assert(!keepMeasuredFrames([alt], known: [win(3, angekommen)])[0].minimized, "frisch geholt bleibt geholt")
}

/// Codex/Grok/Kimi (GLM+Kimi 2026-09-25): ein Zug, der vor mehr als 6 h fertig wurde, oder eine Codex-Session von
/// vorgestern fiel samt Fenster aus Rahmen, Schild und Anordnung, obwohl der Prozess noch lief. Gesucht wird jetzt vom
/// lebenden Prozess aus: je Arbeitsordner die neueste Log-Datei, ohne Altersgrenze — ohne Prozess gar nicht.
func testLogSessionsFollowLiveProcesses() {
    let home = NSTemporaryDirectory() + "lookout-logs-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: home) }
    let now = Date()
    func write(_ path: String, _ text: String, age: TimeInterval) {
        let fm = FileManager.default
        try! fm.createDirectory(atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        try! text.write(toFile: path, atomically: true, encoding: .utf8)
        try! fm.setAttributes([.modificationDate: now.addingTimeInterval(-age)], ofItemAtPath: path)
    }
    let fmt = DateFormatter()
    fmt.dateFormat = "yyyy/MM/dd"
    let started = now.addingTimeInterval(-3 * 86_400)
    // Codex: vor drei Tagen gestartet, seit 10 h fertig.
    write(home + "/.codex/sessions/\(fmt.string(from: started))/rollout-a.jsonl",
          #"{"type":"session_meta","payload":{"session_id":"cx-1","cwd":"/w/codex"}}"# + "\n" + codexDone, age: 10 * 3600)
    // Grok: zwei Sessions im selben Ordner — die neuere (fertig, vor 8 h) gilt, nicht die aeltere.
    write(home + "/.grok/sessions/%2Fw%2Fgrok/g-alt/events.jsonl", #"{"type":"turn_started"}"#, age: 30 * 3600)
    write(home + "/.grok/sessions/%2Fw%2Fgrok/g-neu/events.jsonl", grokDone, age: 8 * 3600)
    // Kimi: seit 7 h fertig.
    write(home + "/.kimi-code/sessions/wd_kimi_0123456789ab/session_k/state.json",
          #"{"id":"session_k","cwd":"/w/kimi"}"#, age: 7 * 3600)
    write(home + "/.kimi-code/sessions/wd_kimi_0123456789ab/session_k/agents/main/wire.jsonl",
          #"{"type":"agent.turn.ended"}"#, age: 7 * 3600)
    let procs = [EngineProc(pid: 101, engine: .codex, tty: "ttys001", cwd: "/w/codex"),
                 EngineProc(pid: 102, engine: .grok, tty: "ttys002", cwd: "/w/grok"),
                 EngineProc(pid: 103, engine: .kimi, tty: "ttys003", cwd: "/w/kimi"),
                 EngineProc(pid: 104, engine: .codex, tty: "ttys004", cwd: "/w/ohne-log")]
    let finder = LogSessionFinder(home: home, startOf: { _ in started })
    let found = Dictionary(uniqueKeysWithValues: finder.sessions(for: procs, thresholdSec: 90).map { ($0.id, $0) })
    assert(found["codex:cx-1"]?.ready == true, "Codex von vorgestern, seit 10 h fertig — bekam \(found.keys.sorted())")
    assert(found["grok:g-neu"]?.ready == true && found["grok:g-alt"] == nil, "je Prozess die neueste Grok-Session")
    assert(found["kimi:session_k"]?.ready == true, "Kimi seit 7 h fertig")
    assert(found.count == 3, "ohne Log keine Session, bekam \(found.keys.sorted())")
    assert(finder.sessions(for: [], thresholdSec: 90).isEmpty, "ohne Prozess wird nichts gelesen")

    // Zwei Grok-Terminals im selben Ordner (review quick 2026-09-25, GLM): jede Session gehoert zu dem Prozess, der
    // vor ihr gestartet ist — nicht der zuletzt geschriebenen Datei der erste Prozess der Liste. Sonst tauschten die
    // beiden Fenster ihren Status.
    func created(_ path: String, _ age: TimeInterval) {
        try! FileManager.default.setAttributes([.creationDate: now.addingTimeInterval(-age)], ofItemAtPath: path)
    }
    let alt = home + "/.grok/sessions/%2Fw%2Fzwei/z-alt/events.jsonl"
    let neu = home + "/.grok/sessions/%2Fw%2Fzwei/z-neu/events.jsonl"
    write(alt, #"{"type":"turn_started"}"#, age: 60)          // aelterer Prozess arbeitet gerade (zuletzt geschrieben)
    write(neu, grokDone, age: 1800)                          // juengerer Prozess ist seit 30 min fertig
    created(alt, 5 * 3600)
    created(neu, 3600)
    let starts: [pid_t: Date] = [301: now.addingTimeInterval(-5 * 3600 - 60), 302: now.addingTimeInterval(-3600 - 60)]
    let zwei = LogSessionFinder(home: home, startOf: { starts[$0] })
        .sessions(for: [EngineProc(pid: 301, engine: .grok, tty: "ttys005", cwd: "/w/zwei"),
                        EngineProc(pid: 302, engine: .grok, tty: "ttys006", cwd: "/w/zwei")], thresholdSec: 90)
    let pids = Dictionary(uniqueKeysWithValues: zwei.map { ($0.id, $0.pid) })
    assert(pids["grok:z-alt"] == 301 && pids["grok:z-neu"] == 302, "Session je Prozess nach Startzeit, bekam \(pids)")
}

/// Log mit Datum und begrenzter Groesse (review 2026-09-25: eine Endlosschleife "18:26" im Log stammte aus der Zeit
/// vor v3 — ohne Datum sah sie aktuell aus; 1,3 MB je Woche ohne Rotation).
func testLogWithDateAndRotation() {
    let dir = NSTemporaryDirectory() + "lookout-log-\(UUID().uuidString)"
    try! FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let path = dir + "/Lookout.log"
    appendLog("erste Zeile", to: path, rotateAt: 50)
    let first = try! String(contentsOfFile: path, encoding: .utf8)
    assert(first.range(of: #"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}  erste Zeile\n$"#, options: .regularExpression) != nil,
           "Zeile mit Datum, bekam \(first)")
    appendLog("zweite", to: path, rotateAt: 50)
    appendLog("dritte", to: path, rotateAt: 50)
    let alt = (try? String(contentsOfFile: path + ".1", encoding: .utf8)) ?? ""
    let neu = try! String(contentsOfFile: path, encoding: .utf8)
    assert(alt.contains("erste Zeile") && alt.contains("zweite") && !alt.contains("dritte"), "ab 50 Bytes rotiert: \(alt)")
    assert(neu.contains("dritte") && !neu.contains("zweite"), "neues Log beginnt frisch: \(neu)")
    // Scheitert die Rotation (hier laesst sich `.1` nicht entfernen), wird weiter angehaengt — nie ueberschrieben
    // (review quick 2026-09-25, GLM+Kimi: sonst ersetzte die neue Zeile die ganze Historie).
    let stuck = dir + "/Stuck.log"
    try! FileManager.default.createDirectory(atPath: stuck + ".1", withIntermediateDirectories: true)
    try! "x".write(toFile: stuck + ".1/inhalt", atomically: true, encoding: .utf8)
    try! FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: stuck + ".1")
    defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: stuck + ".1") }
    appendLog("alt eins", to: stuck, rotateAt: 50)
    appendLog("alt zwei", to: stuck, rotateAt: 50)
    appendLog("neu", to: stuck, rotateAt: 50)
    let kept = try! String(contentsOfFile: stuck, encoding: .utf8)
    assert(kept.contains("alt eins") && kept.contains("neu"), "Rotation gescheitert: Historie bleibt, bekam \(kept)")
}

// ---------- Plan 1e: fertig trotz Hintergrund-Befehl, wenn der nur wartet (Kevin 2026-09-26 abends) ----------

/// `claude agents` meldet `busy`, solange ein Hintergrund-Befehl laeuft — auch nach der fertigen Antwort. Das Zug-Ende
/// steht im Transkript: `system`/`turn_duration`, nach der letzten Antwort und den Stop-Hooks.
func testTurnEnded() {
    let antwort = #"{"type":"assistant","timestamp":"2026-09-26T19:16:40.000Z","message":{"content":[{"type":"text","text":"Fertig."}]}}"#
    let hook = #"{"type":"system","subtype":"stop_hook_summary","hookCount":6}"#
    let ende = #"{"type":"system","subtype":"turn_duration","durationMs":102576,"timestamp":"2026-09-26T19:16:42.562Z","isMeta":false}"#
    assert(digestTranscript([antwort, hook, ende].joined(separator: "\n")).turnEnded, "turn_duration beendet den Zug")
    assert(!digestTranscript([antwort, hook].joined(separator: "\n")).turnEnded, "ohne turn_duration: Zug offen")
    let prompt = #"{"type":"user","message":{"content":"Mach weiter"}}"#
    assert(!digestTranscript([antwort, ende, prompt].joined(separator: "\n")).turnEnded, "ein neuer Prompt hebt es auf")
    // Hintergrund-Meldungen kommen als Prompt (live: origin task-notification, ohne isMeta) und starten einen Zug.
    let meldung = #"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>fertig</task-notification>"}}"#
    assert(!digestTranscript([antwort, ende, meldung].joined(separator: "\n")).turnEnded, "Hintergrund-Meldung")
    assert(!digestTranscript([ende, antwort].joined(separator: "\n")).turnEnded, "eine spaetere Antwort hebt es auf")
    let meta = #"{"type":"user","isMeta":true,"message":{"content":"<system-reminder>"}}"#
    assert(digestTranscript([antwort, ende, meta].joined(separator: "\n")).turnEnded, "System-Einblendung ist kein Prompt")
    // Kevin 2026-10-10 (23:27, zwei wartende Fenster nach /compact FERTIG): die Zusammenfassung ist kein Prompt.
    let peer = #"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/55848.sock","verifiedPeerPid":55848,"name":"Lead"},"message":{"content":"go"}}"#
    let grenze = #"{"type":"system","subtype":"compact_boundary","isMeta":false}"#
    let summary = #"{"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"content":"This session is being continued from a previous conversation that ran out of context."}}"#
    let d = digestTranscript([peer, antwort, ende, grenze, summary].joined(separator: "\n"))
    assert(d.turnEnded && d.kevinTurn == false && d.peerPid == 55848, "Verdichtung aendert nichts am Zug")
}

/// Live 2026-09-26: eine Session mit `zsh → fnm → node` im Hintergrund (0 % Rechenzeit), daneben die MCP-Server als
/// direkte Kinder und `caffeinate`.
private let backgroundFixture = """
  530   520 ttys001    3:05.50 /Users/test/.local/bin/claude
  540   530 ??         0:00.00 /bin/zsh
  541   540 ??         0:00.01 /opt/homebrew/bin/fnm
  542   541 ??         0:02.00 node
  550   530 ttys001    0:12.00 /Library/Frameworks/Python.framework/Versions/3.13/Resources/Python.app/Contents/MacOS/Python
  551   530 ttys001    0:00.40 npm exec @upstash/context7-mcp@3.2.0
  552   551 ttys001    0:05.00 node
  560   530 ttys001    0:00.00 caffeinate
  570   530 ??         0:00.01 -bash
  580   999 ??         0:00.01 /bin/zsh
"""

/// Kevins Regel: arbeitet der Hintergrund-Befehl, laeuft die Session; ein Beobachter oder etwas Ruhendes heisst fertig.
/// "Arbeitet" = in den letzten 90 s mehr als 1 s Rechenzeit oder mehr als 50 KB Netz (Startwerte).
func testBackgroundActivity() {
    let rows = backgroundFixture.split(separator: "\n").compactMap(parsePsLine)
    assert(backgroundPids(of: 530, rows: rows) == [540, 541, 542, 570],
           "Shell-Kinder samt allem darunter; MCP-Server und caffeinate zaehlen nicht")
    assert(backgroundPids(of: 999, rows: rows) == [580] && backgroundPids(of: 1, rows: rows).isEmpty)

    let t0 = Date(timeIntervalSince1970: 50_000)
    func at(_ dt: TimeInterval, cpu: Double, net: UInt64 = 0) -> UsageSample {
        UsageSample(at: t0.addingTimeInterval(dt), cpu: [542: cpu], net: [542: net])
    }
    let t96 = t0.addingTimeInterval(96)
    assert(backgroundBusy([at(0, cpu: 2), at(60, cpu: 2.3)], now: t0.addingTimeInterval(60)),
           "erst 60 s beobachtet: laeuft noch")
    assert(!backgroundBusy([at(0, cpu: 2), at(60, cpu: 2.3), at(96, cpu: 2.5)], now: t96),
           "90 s mit 0,5 s Rechenzeit und ohne Netz: fertig")
    assert(backgroundBusy([at(0, cpu: 2), at(96, cpu: 3.5)], now: t96), "1,5 s Rechenzeit: arbeitet")
    assert(backgroundBusy([at(0, cpu: 2, net: 1_000), at(96, cpu: 2, net: 80_000)], now: t96),
           "Netz waechst (wartet auf ein Sprachmodell): arbeitet")
    assert(backgroundBusy([at(0, cpu: 2), UsageSample(at: t96, cpu: [543: 4], net: [:])], now: t96),
           "ein neuer Prozess zaehlt mit seiner ganzen Rechenzeit")
    assert(!backgroundBusy([at(0, cpu: 2, net: 90_000), UsageSample(at: t96, cpu: [:], net: [:])], now: t96),
           "beendete Prozesse und geschlossene Verbindungen ziehen nichts ab")

    let net = parseNettop(",bytes_in,bytes_out,\nlaunchd.1,0,0,\n2.1.283.530,1107283,4290220,\nnode.542,6546,1797507,\nkaputt\n")
    assert(net == [1: 0, 530: 5_397_503, 542: 1_804_053], "Name darf Punkte enthalten, bekam \(net)")

    assert(Set(backgroundTasks(of: 530, rows: rows)) == [[540, 541, 542], [570]],
           "je Hintergrund-Shell ein Auftrag samt allem darunter")

    // Die Messreihe ueber mehrere Session-Ticks (Netz und lauschende Ports aus eingesetzten Quellen statt nettop/lsof).
    // Fixture: Shell 540 fuehrt einen Server (node 542 lauscht), Shell 570 ist ein Aufpasser (lauscht nicht).
    var netNow: [pid_t: UInt64] = [:]
    var said: [String] = []
    let server: Set<pid_t> = [542]
    let watch = BackgroundWatch(netUsage: { netNow }, listening: { _ in server }, report: { said.append($0) })
    var busy = Session(id: "claude:b", engine: .claude, pid: 530, name: "lookout-b", cwd: "", ready: false, alive: true,
                       backgroundOnly: true)
    busy.digest = Digest(text: "Waiting for the workers. Nothing needed from you.", last: .text, turnEnded: true)
    func tick(_ dt: TimeInterval, _ s: Session, rows r: [PsRow] = rows) -> Session {
        watch.adjust([s], rows: r, now: t0.addingTimeInterval(dt), window: 90)[0]
    }
    assert(!tick(0, busy).ready && !tick(50, busy).ready, "vor 90 s Ruhe: laeuft")
    // Kevin 2026-09-27 (Lead-Session): wer auf seinen eigenen Aufpasser wartet, braucht Kevin nicht — kein Fertig-Alarm.
    let wacht = tick(96, busy)
    assert(wacht.watching && !wacht.ready, "Aufpasser ruht seit 90 s: ueberwacht, nicht fertig")
    assert(said == ["Hintergrund ruht seit 90 s: lookout-b gilt als überwacht (Aufpasser läuft)"], "bekam \(said)")
    assert(tick(99, busy).watching && said.count == 1, "gleicher Zustand: keine neue Zeile")
    let rechnet = rows.map { $0.pid == 542 ? PsRow(pid: 542, ppid: 541, tty: "??", cpu: 9, comm: "node") : $0 }
    let wieder = tick(102, busy, rows: rechnet)
    assert(!wieder.ready && !wieder.watching, "Rechenzeit waechst: laeuft wieder")
    assert(said.last == "Hintergrund arbeitet wieder: lookout-b", "bekam \(said)")
    // Nur ein ruhender Server (Vorschau fuer Kevin oder vergessen): fertig wie bisher (Plan 1e).
    let nurServer = rows.filter { $0.pid != 570 }
    let serverWatch = BackgroundWatch(netUsage: { [:] }, listening: { _ in server }, report: { said.append($0) })
    for dt in [0.0, 96] { _ = serverWatch.adjust([busy], rows: nurServer, now: t0.addingTimeInterval(dt), window: 90) }
    let fertig = serverWatch.adjust([busy], rows: nurServer, now: t0.addingTimeInterval(99), window: 90)[0]
    assert(fertig.ready && !fertig.watching, "nur ein ruhender Server: fertig")
    assert(said.last == "Hintergrund ruht seit 90 s: lookout-b gilt als fertig (nur ein Server läuft)",
           "bekam \(said.last ?? "-")")
    // Scheitert die Port-Abfrage, raet Lookout nicht: wie gemeldet (laeuft).
    let blind = BackgroundWatch(netUsage: { [:] }, listening: { _ in nil }, report: { _ in })
    for dt in [0.0, 96] { _ = blind.adjust([busy], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    let unklar = blind.adjust([busy], rows: rows, now: t0.addingTimeInterval(99), window: 90)[0]
    assert(!unklar.ready && !unklar.watching, "lsof scheitert: wie gemeldet")
    // review quick 2026-09-27 (Kimi): scheitert eine Messung bei einer Session, die schon ruhte, behaelt sie ihren Stand —
    // sonst "arbeitet wieder" im Log und ein Takt ARBEITET, obwohl der Hintergrund unveraendert ruht. Gilt fuer lsof und
    // (dieselbe Ursache, schon seit Plan 1e) fuer nettop.
    var lsofOk = true, netOk = true
    var wackelLog: [String] = []
    let wackel = BackgroundWatch(netUsage: { netOk ? [:] : nil }, listening: { _ in lsofOk ? server : nil },
                                 report: { wackelLog.append($0) })
    for dt in [0.0, 96] { _ = wackel.adjust([busy], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    lsofOk = false
    let haelt = wackel.adjust([busy], rows: rows, now: t0.addingTimeInterval(102), window: 90)[0]
    assert(haelt.watching && !haelt.ready && wackelLog.count == 1, "lsof scheitert: bleibt überwacht, bekam \(wackelLog)")
    lsofOk = true
    netOk = false
    let haeltNetz = wackel.adjust([busy], rows: rows, now: t0.addingTimeInterval(108), window: 90)[0]
    assert(haeltNetz.watching && wackelLog.count == 1, "nettop scheitert: bleibt überwacht, bekam \(wackelLog)")
    netOk = true
    let nurServerWackel = BackgroundWatch(netUsage: { netOk ? [:] : nil }, listening: { _ in server }, report: { _ in })
    for dt in [0.0, 96] { _ = nurServerWackel.adjust([busy], rows: nurServer, now: t0.addingTimeInterval(dt), window: 90) }
    netOk = false
    let haeltFertig = nurServerWackel.adjust([busy], rows: nurServer, now: t0.addingTimeInterval(102), window: 90)[0]
    assert(haeltFertig.ready && !haeltFertig.watching, "nettop scheitert: fertig bleibt fertig")
    netOk = true
    // review quick 2026-09-26 (GLM + Kimi): endet der Hintergrund-Befehl (claude agents meldet idle) oder die Session,
    // arbeitet nichts wieder — keine falsche Log-Zeile.
    let quietWatch = BackgroundWatch(netUsage: { [:] }, listening: { _ in [] }, report: { said.append($0) })
    for dt in [0.0, 96] { _ = quietWatch.adjust([busy], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    let before = said.count
    let idle = Session(id: busy.id, engine: .claude, pid: 530, name: "lookout-b", cwd: "", ready: true, alive: true)
    _ = quietWatch.adjust([idle], rows: rows, now: t0.addingTimeInterval(102), window: 90)
    assert(said.count == before, "Hintergrund beendet, Session normal fertig: keine Zeile, bekam \(said.suffix(2))")
    for dt in [110.0, 206] { _ = quietWatch.adjust([busy], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    let flipped = said.count
    _ = quietWatch.adjust([], rows: rows, now: t0.addingTimeInterval(212), window: 90)
    assert(said.count == flipped, "Session beendet: keine Zeile, bekam \(said.suffix(2))")
    netNow = [542: 500_000]
    let netz = tick(300, busy, rows: rechnet)
    assert(!netz.ready && !netz.watching, "Netz waechst: laeuft")
    var offen = busy
    offen.digest = Digest(turnEnded: false)
    let offenJetzt = tick(600, offen)
    assert(!offenJetzt.ready && !offenJetzt.watching, "Antwort nicht fertig: bleibt, wie claude agents es meldet")
    // Ohne Hintergrund-Shell haelt etwas anderes die Session busy (etwa ein Hintergrund-Agent) — Lookout raet nicht.
    let ohneShell = rows.filter { ![540, 541, 542, 570].contains($0.pid) }
    for dt in [700.0, 760, 820] {
        let s = tick(dt, busy, rows: ohneShell)
        assert(!s.ready && !s.watching, "ohne Hintergrund-Befehl: wie gemeldet")
    }
    // Kevin 2026-10-09 (Team Lead 12:18:46): eine Nachricht einer anderen Session beginnt einen Zug ohne Prompt-Zeile; das
    // Transkript zeigt bis zur ersten Antwort noch das alte Zug-Ende. Claude meldet aber sofort `busy`: arbeitet.
    var leadLog: [String] = []
    let leadWatch = BackgroundWatch(netUsage: { [:] }, listening: { _ in [] }, report: { leadLog.append($0) })
    for dt in [0.0, 96] { _ = leadWatch.adjust([busy], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    var geweckt = Session(id: busy.id, engine: .claude, pid: 530, name: "lookout-b", cwd: "", ready: false, alive: true)
    geweckt.digest = busy.digest
    let arbeitet = leadWatch.adjust([geweckt], rows: rows, now: t0.addingTimeInterval(99), window: 90)[0]
    assert(!arbeitet.ready && !arbeitet.watching, "busy trotz altem Zug-Ende im Transkript: arbeitet")
    assert(leadLog == ["Hintergrund ruht seit 90 s: lookout-b gilt als überwacht (Aufpasser läuft)",
                       "Hintergrund arbeitet wieder: lookout-b"], "bekam \(leadLog)")
    let frage = Session(id: "claude:q", engine: .claude, pid: 530, name: "q", cwd: "", ready: true, waiting: true, alive: true)
    assert(watch.adjust([frage], rows: rows, now: t0, window: 90)[0].waiting, "eine Frage bleibt Frage")
    let codex = Session(id: "codex:c", engine: .codex, pid: 530, name: "c", cwd: "", ready: false, alive: true)
    assert(!watch.adjust([codex], rows: rows, now: t0.addingTimeInterval(2000), window: 90)[0].ready, "nur Claude")
}

/// Kevin 2026-10-01: wartet eine Session auf ihren Aufpasser, braucht Kevin aber (Frage, Bitte, Fertig-Meldung, oder
/// seine eigene Nachricht hat den Zug ausgeloest), versteckte grau sie — frische Zug-Enden 28.09.-01.10.: 23 von 23.
/// Dann FERTIG wie vor 1e+.
func testWatcherNeedsKevin() {
    let rows = backgroundFixture.split(separator: "\n").compactMap(parsePsLine)
    let t0 = Date(timeIntervalSince1970: 60_000)
    var said: [String] = []
    let watch = BackgroundWatch(netUsage: { [:] }, listening: { _ in [542] }, report: { said.append($0) })
    func session(_ text: String, kevin: Bool = false) -> Session {
        var s = Session(id: "claude:q", engine: .claude, pid: 530, name: "lookout-q", cwd: "", ready: false, alive: true,
                        backgroundOnly: true)
        s.digest = Digest(text: text, last: .text, turnEnded: true, kevinTurn: kevin)
        return s
    }
    let frage = session("CI is red on one flaky test. Retry it or skip it?")
    for dt in [0.0, 96] { _ = watch.adjust([frage], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    let out = watch.adjust([frage], rows: rows, now: t0.addingTimeInterval(99), window: 90)[0]
    assert(out.ready && !out.watching, "Frage in der Antwort: fertig statt überwacht")
    assert(said == ["Hintergrund ruht seit 90 s: lookout-q gilt als fertig (Antwort an dich)"], "bekam \(said)")
    let ruhig = watch.adjust([session("Waiting for CI. Nothing needed from you.")], rows: rows,
                             now: t0.addingTimeInterval(102), window: 90)[0]
    assert(ruhig.watching && !ruhig.ready, "nichts für Kevin: überwacht wie bisher")
    let bitte = session("Die Datei ist hochgeladen. Bitte klick im Shop auf Weiter.")
    assert(watch.adjust([bitte], rows: rows, now: t0.addingTimeInterval(105), window: 90)[0].ready, "Bitte an Kevin: fertig")
    let antwort = session("The three rounds can run at once; the limit is each family's usage window.", kevin: true)
    assert(watch.adjust([antwort], rows: rows, now: t0.addingTimeInterval(108), window: 90)[0].ready,
           "Antwort auf Kevins eigene Nachricht: fertig, er ist dran")
    // review quick 2026-10-01 (Codex): scheitert die Messung, behielt eine ueberwachte Session ihren Stand — auch wenn ihre
    // neue Antwort Kevin braucht.
    var netOk = true
    let wackel = BackgroundWatch(netUsage: { netOk ? [:] : nil }, listening: { _ in [542] }, report: { _ in })
    let still = session("Waiting for CI. Nothing needed from you.")
    for dt in [0.0, 96] { _ = wackel.adjust([still], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    assert(wackel.adjust([still], rows: rows, now: t0.addingTimeInterval(99), window: 90)[0].watching, "erst überwacht")
    netOk = false
    assert(wackel.adjust([still], rows: rows, now: t0.addingTimeInterval(102), window: 90)[0].watching,
           "Messung scheitert, Antwort unverändert: bleibt überwacht")
    // review quick 2026-10-01 (GLM): ohne lesbaren Antworttext im Zweifel fertig, nicht grau.
    var ohneText = session("")
    ohneText.digest?.text = nil
    assert(watch.adjust([ohneText], rows: rows, now: t0.addingTimeInterval(111), window: 90)[0].ready, "kein Text: fertig")
    let nachfrage = wackel.adjust([session("CI is green. Merge now?")], rows: rows, now: t0.addingTimeInterval(105),
                                  window: 90)[0]
    assert(nachfrage.ready && !nachfrage.watching, "Messung scheitert, neue Antwort fragt Kevin: fertig")
    // Kevin 2026-10-01 abends: "nichts noetig" im ersten Satz gewinnt — auch wenn danach etwas "complete" ist oder eine
    // andere Session auf Kevin wartet. Seine eigene Nachricht macht es weiter fertig.
    let lage = session("Nothing needed from you. The handover is complete. #13 waits for your answer by day.")
    assert(watch.adjust([lage], rows: rows, now: t0.addingTimeInterval(114), window: 90)[0].watching,
           "Lagebericht, der mit 'nichts nötig' beginnt: überwacht")
    let lageKevin = session("Nothing needed from you. The handover is complete.", kevin: true)
    assert(watch.adjust([lageKevin], rows: rows, now: t0.addingTimeInterval(117), window: 90)[0].ready,
           "Antwort auf Kevins eigene Nachricht: weiter fertig")
    // Kevin 2026-10-03 (Screenshot 08:58): sagt die Antwort auf Kevins Nachricht zu, sich zu melden, und will sonst
    // nichts von ihm, wartet sie nur auf ihren Aufpasser — ueberwacht.
    let zusage = session("Kevin, alles eingetragen. ✅\n\n- **Merkmalsliste:** freigegeben.\n\n"
                         + "Ich melde mich, sobald v3 durch ist.", kevin: true)
    assert(watch.adjust([zusage], rows: rows, now: t0.addingTimeInterval(120), window: 90)[0].watching,
           "Zusage auf Kevins Nachricht: überwacht")
    let zusageFrage = session("Alles eingetragen. Ich melde mich, sobald v3 durch ist. Soll ich v4 gleich starten?",
                              kevin: true)
    assert(watch.adjust([zusageFrage], rows: rows, now: t0.addingTimeInterval(123), window: 90)[0].ready,
           "Zusage mit Frage: fertig")
    let zusageBitte = session("Der Abruf läuft. Ich melde mich, sobald du den Code im Portal eingegeben hast.", kevin: true)
    assert(watch.adjust([zusageBitte], rows: rows, now: t0.addingTimeInterval(126), window: 90)[0].ready,
           "Zusage mit Bitte an Kevin: fertig")
    assert(promisesReport("Ich melde mich mit der fertigen Zahl.") && promisesReport("Sobald v3 steht, sage ich Bescheid."),
           "deutsche Zusagen")
    assert(promisesReport("I'll report back when CI is green.") && promisesReport("I’ll let you know once it lands."),
           "englische Zusagen, auch mit typografischem Apostroph")
    assert(!promisesReport("Sag Bescheid, dann öffne ich dir das Fenster wieder.") && !promisesReport("Melde dich, wenn es hakt."),
           "Bitte an Kevin ist keine Zusage")
    // review quick 2026-10-03 (GLM): eine Aufforderung ohne Signalwort nach der Zusage bleibt fertig — die Zusage zaehlt nur
    // als letzter Satz.
    let zusageDanachAuftrag = session("Ich melde mich, sobald der Build durch ist. Merge in der Zwischenzeit PR #12.",
                                      kevin: true)
    assert(watch.adjust([zusageDanachAuftrag], rows: rows, now: t0.addingTimeInterval(132), window: 90)[0].ready,
           "Auftrag nach der Zusage: fertig")
    assert(!promisesReport("I'll let you know when it's done. Approve the budget in the dashboard."), "englisch ebenso")
    assert(promisesReport("Ich melde mich, sobald v3 durch ist. ✅"), "Emoji nach der Zusage ist kein neuer Satz")
    // review quick 2026-10-03 (Codex): nur die erste Person ist eine Zusage, und eine Bitte oder Bedingung an Kevin sperrt.
    assert(promisesReport("Sobald v3 durch ist, melde ich mich."), "erste Person, umgestellt")
    assert(!promisesReport("Melde mich im Browser an, damit ich den Export starten kann."), "Imperativ an Kevin")
    assert(!promisesReport("Please approve the deployment now. I'll report back when CI is green."), "please")
    assert(!promisesReport("Ich melde mich, wenn du den Code im Portal eingegeben hast."), "wartet erst auf Kevin")
    assert(!promisesReport("I'll let you know if you need to restart it."), "if you")
    // review quick 2026-10-03 (GLM R2, Kimi R2): ein Auftrag im selben Satzteil-Gefuege bleibt fertig.
    assert(!promisesReport("I'll report back when CI is green. merge PR #12 in the meantime"), "klein nach dem Punkt")
    assert(!promisesReport("Ich melde mich, sobald der Build durch ist — merge in der Zwischenzeit PR #12."), "Gedankenstrich")
    assert(!promisesReport("Ich melde mich, sobald der Build durch ist: merge danach PR #12."), "Doppelpunkt")
    assert(!promisesReport("Ich melde mich, sobald v3 steht.\n- Merge PR #12"), "Aufzählung danach")
    assert(promisesReport("Ich melde mich, sobald v3 durch ist (ca. 16:00)."), "Uhrzeit ist kein Doppelpunkt-Satzteil")
    // Abnahme 2026-10-03 (Kimi): ein Emoji am Satzanfang tarnt den Auftrag danach nicht.
    assert(!promisesReport("Alles eingetragen. Ich melde mich, sobald der Build durch ist. ✅ Merge danach PR #12."),
           "Emoji vor dem Auftrag")
    assert(!promisesReport("Ich melde mich, sobald der Build durch ist. 1) Merge PR #12"), "Nummer vor dem Auftrag")
    let fertigUndZusage = session("Der Auftrag ist fertig. Ich melde mich, sobald der Kontrolllauf durch ist.", kevin: true)
    assert(watch.adjust([fertigUndZusage], rows: rows, now: t0.addingTimeInterval(135), window: 90)[0].watching,
           "Kevin 2026-10-03: erledigt + Zusage = die Session wartet nur noch (wie 'alles eingetragen' im Screenshot-Fenster)")
    let ohneZusage = session("Alles eingetragen. v3 läuft bis etwa 16 Uhr.", kevin: true)
    assert(watch.adjust([ohneZusage], rows: rows, now: t0.addingTimeInterval(129), window: 90)[0].ready,
           "Antwort auf Kevins Nachricht ohne Zusage: weiter fertig")
}

/// Kevin 2026-10-09 (Team Lead, 6 Tage: 211 von 523 Zug-Enden ohne seine Nachricht FERTIG, obwohl er nur wartete): eine
/// Lead-Session (`leadNames`) berichtet ueber andere Fenster. Mit ruhendem Aufpasser macht sie nur eine Frage oder Kevins
/// eigene, ungesehene Nachricht fertig.
func testLeadReportsOnOtherWindows() {
    let rows = backgroundFixture.split(separator: "\n").compactMap(parsePsLine)
    let t0 = Date(timeIntervalSince1970: 80_000)
    let watch = BackgroundWatch(netUsage: { [:] }, listening: { _ in [542] }, leads: ["Beta · Lead"], report: { _ in })
    func session(_ text: String, name: String = "Beta · Lead", kevin: Bool = false) -> Session {
        var s = Session(id: "claude:\(name)", engine: .claude, pid: 530, name: name, cwd: "", ready: false, alive: true,
                        backgroundOnly: true)
        s.digest = Digest(text: text, last: .text, turnEnded: true, kevinTurn: kevin)
        return s
    }
    var dt = 0.0
    func judged(_ s: Session) -> Session {
        dt += 3
        return watch.adjust([s], rows: rows, now: t0.addingTimeInterval(dt), window: 90)[0]
    }
    let andere = "Kevin, two windows wait for you: #8's look at its page and #12's merge. One watcher runs. I wait."
    for t in [0.0, 96] { _ = watch.adjust([session(andere)], rows: rows, now: t0.addingTimeInterval(t), window: 90) }
    dt = 96
    assert(judged(session(andere)).watching, "Lead: Bericht über andere Fenster ist überwacht")
    assert(judged(session("Kevin, #8's handover is complete: the new window runs since 22:13.")).watching,
           "Lead: 'complete' über eine andere Karte ist überwacht")
    assert(judged(session("Kevin, #8 waits on the case page. Shall I hold #12 until then?")).ready, "Lead mit Frage: fertig")
    assert(judged(session("Kevin, noted: the order stays.", kevin: true)).ready, "Kevins eigene Nachricht: fertig")
    assert(judged(session(andere, name: "Beta · Worker")).ready, "andere Session: Wortliste wie bisher")
    assert(judged(session(andere, name: "beta · lead")).ready, "Name muss genau stimmen")
    // review quick 2026-10-09 (Codex + GLM): das Menue zeigt den eigenen Namen aus dem Transkript, nicht den der Status-Datei.
    var umbenannt = session(andere, name: "beta-lead")
    umbenannt.digest?.name = "Beta · Lead"
    assert(judged(umbenannt).watching, "Name wie im Menue (Transkript) zählt auch")
}

/// Kevin 2026-10-04 (Screenshot 22:38): die Antwort auf Kevins eigene Nachricht ist fertig, bis er das Fenster nach
/// ihr einmal vor sich hatte — danach gilt sie wie eine Antwort, die nicht er ausgeloest hat (ueberwacht, wenn sie nichts
/// von ihm will).
func testWatcherAnswerSeen() {
    let rows = backgroundFixture.split(separator: "\n").compactMap(parsePsLine)
    let t0 = Date(timeIntervalSince1970: 70_000)
    let answered = t0.addingTimeInterval(-30)
    var said: [String] = []
    let watch = BackgroundWatch(netUsage: { [:] }, listening: { _ in [542] }, report: { said.append($0) })
    func session(_ text: String, answerAt: Date? = answered) -> Session {
        var s = Session(id: "claude:s", engine: .claude, pid: 530, name: "lookout-s", cwd: "", ready: false, alive: true,
                        backgroundOnly: true)
        s.digest = Digest(text: text, last: .text, answerAt: answerAt, turnEnded: true, kevinTurn: true)
        return s
    }
    let antwort = session("Der Zusatz ist notiert ✅: Er kommt als eigene Zeile in den Bericht, und danach entscheidest du. "
                       + "Die Punkte 1, 2 und 4–6 sind noch offen, das hat bis Mittwoch Zeit.")
    for dt in [0.0, 96] { _ = watch.adjust([antwort], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    assert(watch.adjust([antwort], rows: rows, now: t0.addingTimeInterval(99), window: 90)[0].ready, "ungesehen: fertig")
    let vorher = watch.adjust([antwort], rows: rows, now: t0.addingTimeInterval(102), window: 90,
                              seen: [antwort.id: answered.addingTimeInterval(-5)])[0]
    assert(vorher.ready && !vorher.watching, "nur vor der Antwort vorn gehabt: fertig")
    let gesehen = watch.adjust([antwort], rows: rows, now: t0.addingTimeInterval(105), window: 90,
                               seen: [antwort.id: answered.addingTimeInterval(5)])[0]
    assert(gesehen.watching && !gesehen.ready, "nach der Antwort vorn gehabt: überwacht")
    assert(said.last == "Hintergrund ruht seit 90 s: lookout-s gilt als überwacht (Antwort gesehen, Aufpasser läuft)",
           "bekam \(said.last ?? "-")")
    let fremd = watch.adjust([antwort], rows: rows, now: t0.addingTimeInterval(108), window: 90,
                             seen: ["claude:andere": t0])[0]
    assert(fremd.ready, "eine andere Session gesehen: fertig")
    // Gesehen heisst nicht "will nichts": Frage, Bitte oder Fertig-Meldung halten das Fenster fertig.
    let seen = [antwort.id: t0]
    for text in ["CI ist grün. Soll ich v4 starten?", "Der Export liegt bereit. Bitte lade ihn hoch.",
                 "Card #9 is done."] {
        assert(watch.adjust([session(text)], rows: rows, now: t0.addingTimeInterval(111), window: 90, seen: seen)[0].ready,
               "gesehen, aber '\(text)': fertig")
    }
    // Wie ohne Kevins Nachricht: "nichts noetig" im ersten Satz gewinnt.
    let lage = session("Nothing needed from you. The handover is complete.")
    assert(watch.adjust([lage], rows: rows, now: t0.addingTimeInterval(114), window: 90, seen: seen)[0].watching,
           "gesehen + 'nichts nötig' zuerst: überwacht")
    // Im Zweifel fertig: ohne Zeitpunkt der Antwort ist "danach" nicht pruefbar.
    let ohneZeit = session("Die Punkte sind noch offen.", answerAt: nil)
    assert(watch.adjust([ohneZeit], rows: rows, now: t0.addingTimeInterval(117), window: 90, seen: seen)[0].ready,
           "ohne Antwort-Zeitpunkt: fertig")
    // Scheitert die Messung, behaelt die gesehene Antwort ihren Stand (`keepQuiet`).
    var netOk = true
    let wackel = BackgroundWatch(netUsage: { netOk ? [:] : nil }, listening: { _ in [542] }, report: { _ in })
    for dt in [0.0, 96] { _ = wackel.adjust([antwort], rows: rows, now: t0.addingTimeInterval(dt), window: 90, seen: seen) }
    assert(wackel.adjust([antwort], rows: rows, now: t0.addingTimeInterval(99), window: 90, seen: seen)[0].watching,
           "erst überwacht")
    netOk = false
    assert(wackel.adjust([antwort], rows: rows, now: t0.addingTimeInterval(102), window: 90, seen: seen)[0].watching,
           "Messung scheitert, Antwort gesehen: bleibt überwacht")
    let neu = session("Die Punkte sind noch offen.", answerAt: t0.addingTimeInterval(101))
    assert(wackel.adjust([neu], rows: rows, now: t0.addingTimeInterval(105), window: 90, seen: seen)[0].ready,
           "Messung scheitert, neue ungesehene Antwort: fertig")
}

/// Welche Session hat Kevin vor sich? Die sichtbare des fokussierten Fensters, nur wenn dessen Terminal vorn ist.
func testSessionInFront() {
    func win(_ pid: pid_t, title: String, _ sessions: [Session]) -> TermWindow {
        var w = TermWindow(app: "Warp", bundleId: WARP_BID, ownerPid: 900, ax: AXUIElementCreateApplication(pid),
                           frame: CGRect(x: CGFloat(pid - 900) * 700, y: 100, width: 600, height: 400), title: title,
                           firstSeen: Date(), movable: true, minimized: false, fullscreen: false)
        w.sessions = sessions
        return w
    }
    func shown(_ ws: [TermWindow]) -> [OnScreenWindow] {
        ws.map { OnScreenWindow(number: Int($0.frame.minX), pid: $0.ownerPid,
                                rect: cocoaToAX($0.frame, axOrigin(for: $0.bundleId), screenH: 1000)) }
    }
    func claude(_ id: String, title: String) -> Session {
        var s = Session(id: "claude:\(id)", engine: .claude, pid: 1, name: id, cwd: "", ready: true, alive: true)
        s.digest = Digest(title: title)
        return s
    }
    let eins = win(901, title: "✳ Bericht", [claude("a", title: "Bericht")])
    let zwei = win(902, title: "✳ Plan", [claude("b", title: "Bericht"), claude("c", title: "Plan")])
    let unklar = win(903, title: "✳ Neu", [claude("d", title: "Bericht"), claude("e", title: "Plan")])
    var ohneThema = claude("f", title: "")
    ohneThema.digest = Digest()
    let neu = win(904, title: "✳ Irgendwas", [ohneThema])
    let codex = win(905, title: "codex", [Session(id: "codex:g", engine: .codex, pid: 2, name: "g", cwd: "", ready: true,
                                                   alive: true)])
    let windows = [eins, zwei, unklar, neu, codex]
    let alle = shown(windows)
    func front(_ w: TermWindow, title: String? = nil, pid: pid_t = 900, onScreen: [OnScreenWindow]? = nil) -> String? {
        sessionInFront(windows, focused: w.ax, title: title ?? w.title, frontPid: pid, onScreen: onScreen ?? alle,
                       screenH: 1000)
    }
    assert(sessionInFront(windows, focused: eins.ax, title: nil, frontPid: 900, onScreen: alle, screenH: 1000) == nil,
           "Titel nicht frisch lesbar: keine (review quick 2026-10-04, Codex)")
    assert(front(eins) == "claude:a", "eine Session")
    assert(front(zwei) == "claude:c", "sichtbarer Tab")
    assert(front(unklar) == nil, "sichtbarer Tab unklar: keine")
    assert(front(eins, pid: 77) == nil, "anderes Programm vorn: keine")
    assert(sessionInFront(windows, focused: nil, title: "✳ Bericht", frontPid: 900, onScreen: alle, screenH: 1000) == nil,
           "kein Fokus bekannt: keine")
    assert(sessionInFront(windows, focused: AXUIElementCreateApplication(999), title: "✳ Bericht", frontPid: 900,
                          onScreen: alle, screenH: 1000) == nil, "fremdes Fenster")
    // review quick 2026-10-04 (GLM): minimiert oder auf einem anderen Schreibtisch = nicht vor Kevin.
    assert(front(eins, onScreen: shown([zwei])) == nil, "nicht auf dem sichtbaren Schreibtisch: keine")
    assert(sessionInFront(windows, focused: eins.ax, title: eins.title, frontPid: 900, onScreen: nil, screenH: 1000) == nil,
           "Fensterliste nicht lesbar: keine")
    assert(front(eins, onScreen: shown([eins, eins])) == "claude:a", "deckungsgleich mit einem geparkten: zaehlt")
    // review quick 2026-10-04 (Codex): eine Session, aber Kevin hat den normalen Shell-Tab daneben vor sich.
    assert(front(eins, title: "~/projekt — zsh") == nil, "Shell-Tab ohne Claude-Zeichen: keine")
    assert(front(eins, title: "✳ Anderes Thema") == nil, "Claude-Zeichen, aber fremdes Thema: keine")
    assert(front(eins, title: "◐ Bericht") == "claude:a", "frischer Titel gilt (arbeitet gerade)")
    assert(front(zwei, title: "✳ Bericht") == "claude:b", "frischer Titel waehlt den Tab")
    assert(front(neu) == nil, "Thema unbekannt: keine — der Tab einer beendeten Session traegt auch ✳ (review quick, GLM)")
    assert(front(codex) == nil, "nur Claude-Sessions")
}

/// Kevin 2026-10-01 abends: Beginnt die Antwort ausdruecklich mit "nichts noetig" (erster Satz ohne Frage, Bitte oder
/// Fertig-Meldung, nirgends ein Fragezeichen), zaehlt der Rest beim Aufpasser nicht. Messung an 798 Faellen vom 01.10.:
/// grau 239 -> 282 von 380, versteckt 1 Fertig-Meldung und 1 unsichere Bitte.
func testOpensWithNothingNeeded() {
    let lage = "Nothing needed from you. The handover to the next window is complete and clean. Three windows run into "
        + "the night: #13 (replay), #32 (trial), PR-3 (waits for your answer by day)."
    assert(!asksNothing(lage) && opensWithNothingNeeded(lage), "der erste Satz entscheidet")
    assert(opensWithNothingNeeded("Für dich gerade nichts. #13 wartet auf deine Antwort."), "deutsch")
    assert(opensWithNothingNeeded("  Nothing needed from you!\n\n#13 waits for your merge."), "Ausrufezeichen, Leerzeilen")
    assert(!opensWithNothingNeeded("Nothing needed from you. Should I merge #13 now?"), "Fragezeichen sperrt weiter")
    assert(!opensWithNothingNeeded("Card #9 is finished. Nothing needed from you."), "'nichts nötig' erst im zweiten Satz")
    assert(!opensWithNothingNeeded("Nothing needed from you except the merge."), "Bitte im ersten Satz")
    assert(!opensWithNothingNeeded("Nothing needed from you, I'm done."), "Fertig-Meldung im ersten Satz")
    assert(!opensWithNothingNeeded("All good\nNothing needed from you."), "Zeilenumbruch beendet den ersten Satz")
    assert(!opensWithNothingNeeded(""), "leer")
    // review quick 2026-10-01 (Codex): ein Abkuerzungspunkt beendet keinen Satz, Markdown nach dem Punkt schon.
    assert(!opensWithNothingNeeded("Nothing needed from you for e.g. the logs, but I need your approval for deployment."),
           "Abkürzungspunkt ist kein Satzende")
    assert(!opensWithNothingNeeded("Nothing needed from you for e.g.  the logs, but I need your approval."), "auch mit zwei Leerzeichen")
    assert(opensWithNothingNeeded("**Nothing needed from you.** The handover is complete."), "Markdown nach dem Punkt")
    // review quick 2026-10-01 Runde 3 (Codex): oeffnendes Markdown vor einem Kleinbuchstaben beendet keinen Satz,
    // schliessende deutsche Anfuehrungszeichen und Guillemets gehoeren zum Satzende.
    assert(!opensWithNothingNeeded("Nothing needed from you for e.g. **the logs**, but I need your approval."),
           "Markdown nach dem Abkürzungspunkt")
    assert(opensWithNothingNeeded("„Für dich gerade nichts.“ #13 wartet auf deine Antwort."), "deutsche Anführungszeichen")
    assert(opensWithNothingNeeded("»Für dich gerade nichts.« #13 wartet auf deine Antwort."), "Guillemets")
    // review quick 2026-10-01 Runde 4 (Codex): nach dem Leerraum zaehlt der erste Buchstabe bzw. die erste Ziffer, jedes
    // Zeichen davor (Backticks, Markdown, Klammern) wird uebersprungen.
    assert(!opensWithNothingNeeded("Nothing needed from you for e.g. `the logs`, but I need your approval."), "Inline-Code")
    assert(!opensWithNothingNeeded("Nothing needed from you for e.g. ~~the logs~~, but I need your approval."), "Durchgestrichen")
    assert(!opensWithNothingNeeded("Nothing needed from you. `make test` waits for your merge."),
           "klein beginnender Code nach dem Punkt: im Zweifel derselbe Satz, also fertig")
    assert(opensWithNothingNeeded("Nothing needed from you. `Make` waits for your merge."), "groß beginnender Code: neuer Satz")
    // R11 C (Kevin 2026-10-05): eine noch offene fruehere Frage sperrt auch hinter "nichts noetig" im ersten Satz.
    assert(!opensWithNothingNeeded("Kevin, nothing needed from you. #12 runs.\n\nStill open with you: the planning question."),
           "offene Frage am Ende")
    assert(asksNothing("Part 1 of the check is done. The lead pings me when #12 is through."), "Aufpasser: Teilschritt (A')")
    // Kevin 2026-10-06: "auf deinen Mac" ist ein Ort, keine Bitte; Warten auf Kevin bleibt eine.
    assert(asksNothing("Ich habe sie auf deinen Mac durchgereicht. Nach einem Neustart sag Bescheid."), "Ort, keine Bitte")
    assert(asksNothing("Die Datei liegt auf deinem Mac."), "Ort im Dativ")
    assert(!asksNothing("Kevin, protokolliert. Ich warte auf deinen Stick."), "wartet auf Kevin")
    assert(!asksNothing("Ich warte nur noch auf dein Go."), "wartet auf Kevin, mit nur noch")
    assert(!asksNothing("Das wartet auf deine Freigabe."), "auf deine als ganzes Wort")
}

/// Kevin 2026-10-01 abends: Ein Fenster, das eine andere Session per Uebergabe gestartet hat (`claude "…"`), hat seinen
/// ersten Zug nicht von Kevin — der Prompt steht in den Startargumenten des Prozesses. Getipptes bleibt Kevins.
func testLaunchPromptIsNotKevins() {
    let brief = "Read the brief at /tmp/handover/x.md completely and work through it."
    let args = ["/usr/local/bin/claude", "--model", "opus", brief]
    assert(launchPrompt(brief, arguments: args), "Start-Prompt")
    assert(launchPrompt(brief + "\n", arguments: args), "Zeilenende am Prompt egal")
    assert(!launchPrompt(brief, arguments: ["claude"] + brief.split(separator: " ").map(String.init)),
           "ohne Anführungszeichen getippt: im Zweifel Kevins Zug")
    assert(!launchPrompt("Please also check the logs from yesterday.", arguments: args), "getippt")
    assert(!launchPrompt("work through it", arguments: args), "getippt, steht nur zufällig im Start-Prompt")
    assert(!launchPrompt("the brief at /tmp/handover/x.md", arguments: ["claude"] + brief.split(separator: " ").map(String.init)),
           "getippt, Teil eines ungequoteten Start-Prompts")
    assert(!launchPrompt("opus", arguments: args), "zu kurz, um sicher zu sein")
    // review quick 2026-10-01 (Codex R1 + R3): der Wert einer Freitext-Option (`-n/--name`, `--system-prompt`,
    // `--append-system-prompt`) ist kein Start-Prompt.
    let name = "Prüfe die CI-Läufe jetzt"
    assert(!launchPrompt(name, arguments: ["claude", "--name", name]) && !launchPrompt(name, arguments: ["claude", "-n", name]),
           "Sitzungsname, keine Übergabe")
    assert(launchPrompt(brief, arguments: ["claude", "--name", name, brief]), "Start-Prompt nach dem Namen")
    // review quick 2026-10-01 Runde 4 (Codex): ein Start-Prompt ist ein Satz — Pfade, IDs, Modellnamen (Optionswerte wie
    // `--add-dir`) haben keinen Leerraum.
    let pfad = "/Users/test/Projects/Alpha/Tools"
    assert(!launchPrompt(pfad, arguments: ["claude", "--add-dir", pfad]), "Pfad als erste Nachricht")
    // Der ausloesende Prompt wird gemerkt — Kevins Nachricht mitten im Zug ist immer getippt.
    let start = #"{"type":"user","origin":{"kind":"human"},"turnPosition":{"promptIndex":1,"turnIndex":1},"timestamp":"2026-10-01T19:02:01.288Z","message":{"content":"\#(brief)"}}"#
    let mitten = #"{"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"human"},"prompt":"und #12?"}}"#
    let d = digestTranscript(start)
    assert(d.kevinTurn == true && d.kevinPrompt == brief, "bekam \(String(describing: d.kevinPrompt))")
    assert(digestTranscript(lines(start, mitten)).kevinPrompt == nil, "Nachricht mitten im Zug: getippt")
    // review quick 2026-10-01 (Codex): nur der erste Prompt einer Session kann ein Start-Prompt sein — derselbe Text spaeter
    // erneut abgeschickt (Eingabe-Verlauf) ist getippt; ohne Positionsangabe im Zweifel auch.
    let nochmal = #"{"type":"user","origin":{"kind":"human"},"turnPosition":{"promptIndex":3,"turnIndex":7},"message":{"content":"\#(brief)"}}"#
    assert(digestTranscript(lines(start, nochmal)).kevinPrompt == nil, "später erneut abgeschickt")
    assert(digestTranscript(#"{"type":"user","origin":{"kind":"human"},"message":{"content":"\#(brief)"}}"#).kevinPrompt == nil,
           "ohne Positionsangabe")
    let startedAt = d.kevinPromptAt.map { $0.addingTimeInterval(-3) }      // Prozess 3 s vor dem Prompt gestartet
    assert(launchCorrected(d, arguments: args, started: startedAt).kevinTurn == false, "Übergabe: nicht Kevins Zug")
    assert(launchCorrected(d, arguments: ["claude", "--resume", "abc"], started: startedAt).kevinTurn == true,
           "getippt: Kevins Zug")
    assert(launchCorrected(d, arguments: nil, started: startedAt).kevinTurn == true, "Startargumente nicht lesbar: wie bisher")
    // review quick 2026-10-01 Runde 3 (Codex): nach `/clear` beginnt im selben Prozess eine neue Unterhaltung — tippt Kevin
    // dort denselben Text, ist das sein Zug. Ein Start-Prompt kommt in den ersten 30 s nach dem Prozessstart (gemessen 3 s;
    // Runde 4: 2 Minuten liessen ein fruehes /clear zu).
    assert(launchCorrected(d, arguments: args, started: startedAt.map { $0.addingTimeInterval(-45) }).kevinTurn == true,
           "45 s nach dem Start: getippt")
    assert(launchCorrected(d, arguments: args, started: startedAt.map { $0.addingTimeInterval(-3600) }).kevinTurn == true,
           "eine Stunde nach dem Start (nach /clear): Kevins Zug")
    assert(launchCorrected(d, arguments: args, started: nil).kevinTurn == true, "Startzeit unbekannt: wie bisher")
    var ohneZeit = d
    ohneZeit.kevinPromptAt = nil
    assert(launchCorrected(ohneZeit, arguments: args, started: startedAt).kevinTurn == true, "Prompt ohne Zeit: wie bisher")
    // Die eigenen Startargumente sind lesbar.
    assert(processArguments(getpid()) == CommandLine.arguments, "bekam \(String(describing: processArguments(getpid())))")
}

/// Server oder Aufpasser? (Kevin 2026-09-27) Ein Server lauscht auf einem Port. `lsof -Fp` live gemessen 27.09.: je
/// Prozess "p<pid>", darunter "f<fd>"; Exit 1 schon dann, wenn nichts lauscht oder einer der Prozesse nicht mehr lebt.
func testListeningPids() {
    assert(parseLsofPids("p957\nf14\nf16\np22053\nf21\n") == [957, 22053], "je Prozess eine p-Zeile")
    assert(parseLsofPids("").isEmpty && parseLsofPids("kaputt\n").isEmpty, "nichts lauscht")
}

/// Kevin 2026-09-27 (Screenshots einer Lead-Session): "er hat ja nichts zu tun, er überwacht jetzt nur die Sessions" —
/// grau, ohne Rahmen, bei den Laufenden; die dritte Zeile sagt, worauf er wartet.
func testWatchingLabel() {
    let now = Date(timeIntervalSince1970: 30_000)
    var s = Session(id: "claude:lead", engine: .claude, pid: 1, name: "beta-lead", cwd: "/Users/test/Projects/Beta",
                    ready: false, alive: true)
    s.watching = true
    s.digest = Digest(name: "Beta · Lead", text: "Nothing needed from you, Kevin. CI is running.", last: .text)
    let w = tabWindow([s], title: "✳ Beta · Lead")
    assert(!w.isReady && !w.isWaiting && w.isWatching, "überwacht: kein Rahmen, keine Frage, nicht bei den Wartenden")
    assert(windowTone(w) == .watching && sessionTone(s) == .watching)
    let seit = SinceEntry(tone: .watching, since: now.addingTimeInterval(-8 * 60))
    let c = labelContent(for: w, since: seit, now: now)
    assert(c.lines == ["◌ ÜBERWACHT  seit 8 Min.", "Beta · Lead", "Nothing needed from you, Kevin."],
           "bekam \(c.lines)")
    assert(sessionLine(w, since: [s.id: seit], now: now).title == "◌ Beta · Lead — seit 8 Min.",
           "Menü und Liste mit demselben Zeichen")
    var beides = Session(id: "claude:x", engine: .claude, pid: 2, name: "x", cwd: "", ready: true, alive: true)
    beides.watching = true
    assert(windowTone(tabWindow([beides], title: "✳ X")) == .ready, "fertig geht vor")
    // review quick 2026-09-27 (GLM): zwei Tabs, sichtbarer nicht erkennbar, Titel ✳ — keine Session ist fertig, eine
    // überwacht: der ruhige sichtbare Tab ist die überwachende, kein Rahmen. Ist eine fertig, bleibt die alte Regel.
    var aufpasser = tabSession("w1", ready: false, title: "Thema W")
    aufpasser.watching = true
    let zweiTabs = tabWindow([aufpasser, tabSession("l1", ready: false, title: "Thema L")], title: "✳ Neues Thema")
    assert(zweiTabs.visibleSession == nil && !zweiTabs.isReady && zweiTabs.isWatching, "✳ ohne Fertige: überwacht")
    let mitFertig = tabWindow([aufpasser, tabSession("f1", ready: true, title: "Thema F")], title: "✳ Neues Thema")
    assert(mitFertig.isReady && !mitFertig.isWatching, "✳ mit einer Fertigen: fertig wie bisher")
}

// ---------- Wartet auf andere Session oder Wecker (Kevin 2026-09-27) ----------

private func lines(_ l: String...) -> String { l.joined(separator: "\n") }

/// Formen live aus den Transkripten 27.09. (CLI 2.1.283), auf das Noetige gekuerzt.
func testDigestPeerAndWakeups() {
    let kevin = #"{"type":"user","origin":{"kind":"human"},"message":{"content":"so this session stays open and waits?"}}"#
    let lead = #"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/33134.sock","verifiedPeerPid":33134,"name":"Beta · Lead","fromMode":"prompting"},"message":{"content":"Another Claude session sent a message: …"}}"#
    let handback = #"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"ac2046f78b16995a7","senderTaskId":"ac2046f78b16995a7","handback":true},"message":{"content":"[Subagent hand-back] …"}}"#
    let meldung = #"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>fertig</task-notification>"}}"#
    var d = digestTranscript(lines(kevin, lead))
    assert(d.peerPid == 33134 && d.peerName == "Beta · Lead", "Nachricht einer anderen Session nach Kevins Prompt")
    // R11 (Kevin 2026-10-05): Kevins Frage beendet das Gespraech mit der anderen Session nicht mehr — er fragte, ob er
    // schliessen kann, und sie wartete danach weiter auf die Lead-Session.
    d = digestTranscript(lines(lead, kevin))
    assert(d.peerPid == 33134 && d.peerName == "Beta · Lead", "Kevins neuer Prompt beendet das Gespräch nicht")
    assert(digestTranscript(lines(kevin, handback)).peerName == nil, "Bericht eines eigenen Subagenten ist keine andere Session")
    assert(digestTranscript(lines(lead, meldung)).peerPid == 33134, "Hintergrund-Meldung ist kein Prompt von Kevin")
    // Wer hat den Zug ausgeloest? Kevins Prompt ja, die andere Session und eine Hintergrund-Meldung nicht (Kevin 2026-10-01).
    assert(digestTranscript(lines(lead, kevin)).kevinTurn == true, "Kevins Prompt")
    assert(digestTranscript(lines(kevin, lead)).kevinTurn == false, "Nachricht der anderen Session")
    assert(digestTranscript(lines(kevin, meldung)).kevinTurn == false, "Hintergrund-Meldung")
    assert(digestTranscript(lines(kevin, handback)).kevinTurn == true, "Bericht eines eigenen Subagenten: Kevins Zug läuft weiter")
    // review quick 2026-10-01 (GLM): der Wecker loest den Zug aus; sein Prompt ist eine Systemeinblendung ohne origin.
    let feuer = #"{"type":"system","subtype":"scheduled_task_fire","taskId":"c10455b9","timestamp":"2026-09-27T09:57:00.519Z"}"#
    let weckPrompt = #"{"type":"user","isMeta":true,"message":{"content":"Prüfe den Fortschritt der Prüfer."}}"#
    assert(digestTranscript(lines(kevin, feuer, weckPrompt)).kevinTurn == false, "Wecker löst den Zug aus")
    // Kevin schreibt mitten in einen Zug, den die andere Session ausgeloest hat (Anhang statt Prompt).
    let mitten = #"{"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"human"},"prompt":"und #12?"}}"#
    assert(digestTranscript(lines(kevin, lead, mitten)).kevinTurn == true, "Kevins Nachricht mitten im Zug")
    let leadMitten = #"{"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"peer"},"prompt":"ok"}}"#
    assert(digestTranscript(lines(kevin, lead, leadMitten)).kevinTurn == false, "Nachricht der anderen Session mitten im Zug")

    // Diese Session schreibt selbst einer anderen (SendMessage; das Ergebnis nennt "another Claude session").
    let send = ##"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s1","name":"SendMessage","input":{"to":"Beta · Lead [97e5ab]","message":"#12 ready"}}]}}"##
    let sent = ##"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s1","content":"{\"success\":true}"}]},"toolUseResult":{"success":true,"message":"“#12 ready” → Beta · Lead (another Claude session on this machine; queued there)"}}"##
    d = digestTranscript(lines(kevin, send, sent))
    assert(d.peerName == "Beta · Lead" && d.peerPid == nil, "an eine andere Session geschrieben: Name ohne [Kennung]")
    let toAgent = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s2","name":"SendMessage","input":{"to":"a1b2c304ddff2ae50","message":"go"}}]}}"#
    let agentOk = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s2","content":"ok"}]},"toolUseResult":{"success":true,"message":"delivered to a1b2c304ddff2ae50"}}"#
    assert(digestTranscript(lines(kevin, toAgent, agentOk)).peerName == nil, "Nachricht an einen eigenen Subagenten")
    // Kevin 2026-10-10 (Screenshot 23:33): an eine Socket-Adresse geschrieben, die Antwort nennt keinen Namen.
    let toSock = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s3","name":"SendMessage","input":{"to":"uds:/tmp/cc-socks/55848.sock","message":"holding"}}]}}"#
    let sockOk = ##"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s3","content":"ok"}]},"toolUseResult":{"success":true,"message":"“holding” → uds:/tmp/cc-socks/55848.sock; in that session's inbox, not yet read by its Claude"}}"##
    d = digestTranscript(lines(kevin, send, sent, toSock, sockOk))
    assert(d.peerPid == 55848 && d.peerName == nil, "an eine Socket-Adresse: pid aus dem Pfad")
    let sockFail = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s3","content":"error"}]},"toolUseResult":{"success":false,"message":"no such socket"}}"#
    assert(digestTranscript(lines(kevin, toSock, sockFail)).peerPid == nil, "Zustellung gescheitert")
    // review quick 2026-10-10 (Codex): abgelehnter Aufruf ohne toolUseResult; Antwort an dieselbe Session behaelt den Namen.
    let sockDenied = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s3","is_error":true,"content":"Tool execution was denied"}]}}"#
    assert(digestTranscript(lines(kevin, toSock, sockDenied)).peerPid == nil, "Aufruf abgelehnt")
    let toOther = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s4","name":"SendMessage","input":{"to":"uds:/tmp/service/33134.sock","message":"x"}}]}}"#
    let otherOk = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s4","content":"ok"}]},"toolUseResult":{"success":true,"message":"sent"}}"#
    assert(digestTranscript(lines(kevin, toOther, otherOk)).peerPid == nil, "fremder Socket ist keine Session")
    let fromLead = #"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"uds:/tmp/cc-socks/55848.sock","verifiedPeerPid":55848,"name":"Lead"},"message":{"content":"go"}}"#
    d = digestTranscript(lines(fromLead, toSock, sockOk))
    assert(d.peerPid == 55848 && d.peerName == "Lead", "Antwort an dieselbe Session: Name bleibt")

    // Wecker (ScheduleWakeup): gestellt, gestoppt, ausgeloest.
    let wake = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w1","name":"ScheduleWakeup","input":{"delaySeconds":1200}}]}}"#
    let woke = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w1","content":"Next wakeup scheduled for 11:57:00"}]},"toolUseResult":{"scheduledFor":1790503020000,"clampedDelaySeconds":1200,"wasClamped":false}}"#
    assert(digestTranscript(lines(wake, woke)).wakeups == ["loop": Date(timeIntervalSince1970: 1_790_503_020)], "Wecker gestellt")
    let stop = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w2","name":"ScheduleWakeup","input":{"stop":true}}]}}"#
    let stopped = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w2","content":"Loop stopped"}]},"toolUseResult":{"scheduledFor":0,"stopped":true,"cancelledWakeups":1}}"#
    assert(digestTranscript(lines(wake, woke, stop, stopped)).wakeups.isEmpty, "Wecker gestoppt")
    let fired = #"{"type":"system","subtype":"scheduled_task_fire","taskId":"c10455b9","timestamp":"2026-09-27T09:57:00.519Z"}"#
    assert(digestTranscript(lines(wake, woke, fired)).wakeups.isEmpty, "Wecker ausgelöst")
    let early = #"{"type":"system","subtype":"scheduled_task_fire","taskId":"x1","timestamp":"2026-09-27T09:30:00.000Z"}"#
    assert(digestTranscript(lines(wake, woke, early)).wakeups.count == 1, "ein anderer, früherer Lauf lässt den Wecker stehen")

    // CronCreate: einmalig (Zeitpunkt aus dem Ausdruck, Ortszeit), geloescht, ausgeloest; wiederkehrend bleibt.
    let cron = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c1","name":"CronCreate","input":{"cron":"43 2 27 9 *","recurring":false}}]}}"#
    let cronOk = #"{"type":"user","timestamp":"2026-09-26T21:03:02.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":"Scheduled one-shot task f874851d"}]},"toolUseResult":{"id":"f874851d","humanSchedule":"43 2 27 9 *","recurring":false,"durable":false}}"#
    let at = digestTranscript(lines(cron, cronOk)).wakeups["f874851d"] ?? nil
    let c = at.map { Calendar.current.dateComponents([.year, .month, .day, .hour, .minute], from: $0) }
    assert(c?.year == 2026 && c?.month == 9 && c?.day == 27 && c?.hour == 2 && c?.minute == 43,
           "einmaliger Cron: 27.09. 02:43 Ortszeit, bekam \(String(describing: at))")
    let del = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c2","name":"CronDelete","input":{"id":"f874851d"}}]}}"#
    let delOk = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c2","content":"Cancelled job f874851d."}]},"toolUseResult":{"id":"f874851d"}}"#
    assert(digestTranscript(lines(cron, cronOk, del, delOk)).wakeups.isEmpty, "Cron gelöscht")
    let cronFired = #"{"type":"system","subtype":"scheduled_task_fire","taskId":"f874851d","timestamp":"2026-09-27T00:43:00.000Z"}"#
    assert(digestTranscript(lines(cron, cronOk, cronFired)).wakeups.isEmpty, "einmaliger Cron ausgelöst")
    let every = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c3","name":"CronCreate","input":{"cron":"*/30 * * * *","recurring":true}}]}}"#
    let everyOk = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c3","content":"Scheduled recurring task 1a2b3c4d"}]},"toolUseResult":{"id":"1a2b3c4d","humanSchedule":"*/30 * * * *","recurring":true}}"#
    let everyFired = #"{"type":"system","subtype":"scheduled_task_fire","taskId":"1a2b3c4d","timestamp":"2026-09-27T00:30:00.000Z"}"#
    let rec = digestTranscript(lines(every, everyOk, everyFired)).wakeups
    assert(rec.count == 1 && rec["1a2b3c4d"] == .some(nil), "wiederkehrend: bleibt nach dem Auslösen, ohne Zeitpunkt")

    // review quick 2026-09-27 (Kimi): ein einmaliger Cron ohne lesbaren Zeitpunkt zaehlt nicht (sonst stuende er nach dem
    // Ausloesen ewig aus), ein fehlendes recurring-Flag heisst einmalig; unmoegliche Daten und ein gesetzter Wochentag
    // ergeben keinen Zeitpunkt (Calendar rechnet den 31.02. sonst still in den 03.03. um).
    let odd = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c4","name":"CronCreate","input":{"cron":"0 9 * * 1"}}]}}"#
    let oddOk = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c4","content":"Scheduled"}]},"toolUseResult":{"id":"9f9f9f9f","humanSchedule":"0 9 * * 1"}}"#
    assert(digestTranscript(lines(odd, oddOk)).wakeups.isEmpty, "einmalig ohne lesbaren Zeitpunkt: zählt nicht")
    let noFlag = #"{"type":"user","timestamp":"2026-09-26T21:03:02.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":"Scheduled"}]},"toolUseResult":{"id":"f874851d","humanSchedule":"43 2 27 9 *"}}"#
    let cronNoFlag = cron.replacingOccurrences(of: #","recurring":false"#, with: "")
    assert((digestTranscript(lines(cronNoFlag, noFlag)).wakeups["f874851d"] ?? nil) != nil, "ohne Flag: einmalig mit Zeitpunkt")
    let t = Date(timeIntervalSince1970: 1_790_000_000)
    assert(oneShotCronDate("0 9 31 2 *", after: t) == nil, "31. Februar ist kein Zeitpunkt")
    assert(oneShotCronDate("0 9 15 10 3", after: t) == nil, "gesetzter Wochentag: kein einfacher Zeitpunkt")
    assert(oneShotCronDate("0 25 1 1 *", after: t) == nil, "Stunde 25")
    assert(oneShotCronDate("30 8 15 10 *", after: t) != nil, "gültiger einmaliger Zeitpunkt")
}

/// Regel ohne Modell (eingefroren 27.09., geprueft an 189 + 46 echten Zug-Enden, keine versteckte Frage): ausdruecklich
/// "nichts noetig", keine Frage, keine Bitte an Kevin, keine Fertig-Meldung.
func testNothingNeeded() {
    assert(saysNothingNeeded("Nothing needed from you, Kevin, and nothing is running. #12 waits for the lead's go."))
    assert(saysNothingNeeded("Kevin, für dich gerade nichts. Die Lead-Session meldet sich, sobald #11 gemergt ist."))
    assert(saysNothingNeeded("NOTHING NEEDED FROM YOU. Waiting on CI."), "groß/klein egal")
    assert(!saysNothingNeeded("Nothing needed from you. Should I also rebase?"), "Frage")
    assert(!saysNothingNeeded("Nothing needed from you, Kevin. You can close this window now."), "fertig: Fenster schließen")
    assert(!saysNothingNeeded("Kevin, nothing needed from you beyond the night-card pick."), "offene Wahl")
    assert(!saysNothingNeeded("Nothing needed from you. Pull request #83 waits only for your merge on GitHub."), "Merge")
    assert(!saysNothingNeeded("Nichts von dir nötig, erledigt."), "Fertig-Meldung")
    assert(!saysNothingNeeded("#12 waits for the lead's go."), "ohne ausdrückliches 'nichts nötig': wie heute")

    // Nachgeschaerft 2026-10-01 (frische Zug-Enden 28.09.-01.10.: die Regel versteckte 15 von 26 Anliegen): weitere
    // Bitten und Fertig-Meldungen sperren. Die geplante Lockerung von "done" fiel an denselben Faellen durch.
    assert(!saysNothingNeeded("Nothing needed from you. Waiting for your next point."), "wartet auf deinen nächsten Punkt")
    assert(!saysNothingNeeded("Nothing more needed from you except the merges: #12 first, then #13."), "außer den Merges")
    assert(!saysNothingNeeded("Nothing more needed from you after you click merge on GitHub."), "nach deinem Klick")
    assert(!saysNothingNeeded("Nothing needed from you. Card #12 is closed and the lead takes over."), "Karte geschlossen")
    assert(!saysNothingNeeded("Nothing needed from you. This window stays open until you close it."), "Fenster bleibt offen")
    assert(!saysNothingNeeded("Für dich gerade nichts, bitte nur kurz den PR freigeben."), "deutsch: bitte")
    assert(!saysNothingNeeded("Für dich gerade nichts außer dem Merge."), "deutsch: außer")
    // review quick 2026-10-01 (GLM): "click" nur als Wort, nicht in Namen wie ClickHouse.
    assert(saysNothingNeeded("Nothing needed from you. The ClickHouse backfill runs; the lead pings me."), "ClickHouse")
    assert(saysNothingNeeded("Nothing needed from you. The lead session pings me when the shared ports are free."),
           "wartet nur auf die andere Session: bleibt grau")

    // R11 A' (Kevin 2026-10-05, Nachmessung an 1.204 frischen Zug-Enden): "done/finished" zaehlt nur als echte
    // Fertig-Meldung — nicht als Teilschritt, verneint, in der Zukunft oder als "Done:" neben "Still to do:".
    assert(saysNothingNeeded("Nothing needed from you, Kevin. I wait for the lead's turn line.\n\n"
                             + "- Part 1 of the check is done and committed."), "Teilschritt")
    assert(saysNothingNeeded("Nothing needed from you. I'll pick up again when #12's run is done."), "Zukunft")
    assert(saysNothingNeeded("Nothing needed from you. I resume once done with the lead's list."), "once done")
    assert(saysNothingNeeded("Nichts von dir nötig. Ich warte, bis #12 erledigt ist."), "Zukunft deutsch")
    assert(saysNothingNeeded("Nothing needed from you. The card is not finished yet; the lead pings me."), "verneint")
    assert(saysNothingNeeded("Nothing needed from you. The card isn’t done, the lead pings me."), "verneint, typografisch")
    // review quick 2026-10-05 (GLM): zwei Adverbien in der Verneinung; ein letzter Schritt bleibt eine Fertig-Meldung.
    assert(saysNothingNeeded("Nothing needed from you. The card is not yet quite done, the lead pings me."), "zwei Adverbien")
    assert(!saysNothingNeeded("Nothing needed from you. Step 2 is done — that was the last one."), "letzter Schritt")
    assert(!saysNothingNeeded("Nichts von dir nötig. Schritt 3 ist erledigt, das waren alle Schritte."), "alle Schritte")
    // review quick 2026-10-05 (Codex): Satzteilgrenzen, leere Restliste, "please".
    assert(!saysNothingNeeded("Nothing needed from you. After two rounds, the implementation is done."), "after + Komma")
    assert(!saysNothingNeeded("Nothing needed from you. Step 1 passed; the entire task is complete."), "Semikolon")
    assert(!saysNothingNeeded("Nothing needed from you.\nDone: PR merged, card closed.\nStill to do: nothing."),
           "leere Restliste")
    assert(!saysNothingNeeded("Nothing needed from you.\nDone: PR merged.\nNext: none."), "Next: none")
    assert(!saysNothingNeeded("Nothing needed from you on the tests. Please approve the deployment now."), "please")
    // review quick 2026-10-05 Runde 2 (Kimi): Plural, deutsch, "Next:" ist keine Restliste, letzter Schritt nur im selben Satz.
    assert(!saysNothingNeeded("Kevin, nothing needed from you. #12 runs. My questions above still stand."), "Plural")
    assert(!saysNothingNeeded("Für dich gerade nichts. Meine Fragen von heute Morgen sind noch offen."), "Fragen offen")
    assert(!saysNothingNeeded("Nothing needed from you.\n- **Done:** PR merged, card closed.\n- **Next:** release tomorrow."),
           "Done neben Next: Fertig-Meldung")
    assert(saysNothingNeeded("Nothing needed from you. Part 1 of the check is done. The final part waits for the lead's go."),
           "final in einem anderen Satz")
    // Runde 2 (GLM): eine Bedingung mit folgendem Komma leitet einen moeglichen Auftrag ein.
    assert(!saysNothingNeeded("Nothing needed from you. When the CI run is done, merge PR #12."), "Bedingung + Auftrag")
    assert(!saysNothingNeeded("Nichts von dir nötig. Sobald #12 erledigt ist, merge den PR."), "deutsch")
    assert(saysNothingNeeded("Nothing needed from you.\n- **Done:** the first call.\n- **Still to do:** the tests."),
           "Done neben Still to do: Zwischenstand")
    assert(!saysNothingNeeded("Nothing needed from you. Card #98 is done: merged."), "echte Fertig-Meldung")
    assert(!saysNothingNeeded("Nothing needed from you. All done."), "all done")
    assert(!saysNothingNeeded("Nothing needed from you.\n- **Done:** PR merged, card closed."), "Done ohne Offenes")
    assert(!saysNothingNeeded("Nothing needed from you. I'm done here."), "I'm done")
    assert(!saysNothingNeeded("Nothing needed from you. When you're done testing, merge PR #12."), "Bedingung an Kevin")
    assert(!saysNothingNeeded("Nichts von dir nötig. Teil 1 ist erledigt, alles erledigt."), "Teilschritt plus alles erledigt")
    assert(!saysNothingNeeded("Nothing needed from you. Its review closed after two rounds and the reviewer finished the tests."),
           "'after' als Praeposition: echte Fertig-Meldung bleibt")
    // R11 C: eine fruehere Frage an Kevin steht noch offen (live versteckte der Aufpasser 18 von 67 Bitten, meist so).
    assert(!saysNothingNeeded("Kevin, nothing needed from you. #12 runs. My question above stands."), "Frage steht")
    assert(!saysNothingNeeded("Nothing needed from you. Still open with you: the two settings lines."), "offen bei dir")
    assert(!saysNothingNeeded("Kevin, nothing needed from you beyond what my last message named."), "letzte Nachricht")
    assert(!saysNothingNeeded("Für dich gerade nichts. Die fünf Entscheidungen von oben sind weiter offen, darauf warte ich."),
           "deutsch")
    assert(!asksNothing("Kevin, nothing new needs you. #12 runs its browser part. My question on the plan stands."),
           "Aufpasser: Frage steht")

    // Kevin 2026-10-10 (Screenshots 22:50, zwei Fenster warteten auf den Team Lead und standen FERTIG): "nichts zu fragen"
    // heisst "nichts noetig"; sagt die Session selbst, dass sie wartet, sperren Fortschrittswoerter wie "is merged" nicht.
    assert(saysNothingNeeded("I have nothing to ask you, Kevin. Nothing of mine runs. I wait for the team lead to start "
                             + "the next window.\n\n- **To know:** the two faults Codex found are still fixed in this pull request."),
           "nichts zu fragen")
    assert(saysNothingNeeded("Ich habe nichts zu fragen. Die Lead-Session meldet sich."), "nichts zu fragen, deutsch")
    assert(!saysNothingNeeded("I have nothing to ask you. Should I merge #12?"), "nichts zu fragen, aber Frage")
    assert(saysNothingNeeded("Nothing needed from you, Kevin. Main, with the case, is merged into my branch, and my checks "
                             + "are green. Nothing runs now; I hold until card #258's signed run, about 01:00."), "wartet selbst")
    assert(saysNothingNeeded("Nichts von dir nötig. #12 ist gemergt und abgeschlossen. Ich warte auf die Lead-Session."),
           "wartet selbst, deutsch")
    assert(!saysNothingNeeded("Nothing needed from you. PR #12 is merged."), "ohne Warten bleibt die Fertig-Meldung")
    assert(!saysNothingNeeded("Nothing needed from you. PR #12 is merged. I hold until you merge #13."), "Warten auf Kevin")
    assert(!saysNothingNeeded("Nothing needed from you. PR #12 is merged. I wait for your go."), "Warten auf Kevins Go")
    assert(!saysNothingNeeded("Nothing needed from you. Card #98 is done. I wait for the lead to close this window."),
           "Fenster schliessen sperrt weiter")
    assert(!saysNothingNeeded("Nothing needed from you. The card is handed over; I wait for the lead."), "Übergabe sperrt weiter")
    assert(!asksNothing("PR #12 is merged. I hold until the lead's go."), "Aufpasser ohne 'nichts nötig': wie bisher")
    // review quick 2026-10-10 (GLM): Warten auf Kevin per "dir/dich" oder Vorname ist kein eigenes Warten; verneint auch nicht.
    assert(!saysNothingNeeded("Nichts von dir nötig. Der Bericht ist abgeschlossen. Ich warte auf Rückmeldung von dir."),
           "wartet auf dich (dir)")
    let saved = greetingName
    defer { greetingName = saved }
    greetingName = "Will"
    assert(!saysNothingNeeded("Nothing needed from you. The audit is complete. I hold until Will reviews the PR."), "Vorname")
    assert(saysNothingNeeded("Nothing needed from you. The audit is complete. I hold until Willow's run."), "nur ganzes Wort")
    assert(!saysNothingNeeded("Nichts von dir nötig. Ich warte nicht mehr, alles erledigt."), "verneint, deutsch")
    assert(!saysNothingNeeded("Nothing needed from you. All done; I wait no longer."), "verneint, englisch")
    // review quick 2026-10-10 (Codex): Satz ueber den Zeilenumbruch, Bitte an Kevin in einem anderen Satz.
    assert(!saysNothingNeeded("Nichts von dir nötig. Die Aufgabe ist abgeschlossen. Ich warte nicht mehr auf die Lead-Session."),
           "verneint, mit Ziel")
    assert(!saysNothingNeeded("Nothing needed from you on testing. PR #12 is merged. I hold until\nyou approve the release."),
           "Zeilenumbruch im Warte-Satz")
    assert(!saysNothingNeeded("Nothing needed from you. I wait for the lead. When you're done testing, merge PR #12."),
           "Bedingung an Kevin in einem anderen Satz")
    // review quick 2026-10-10 Runde 2 (GLM): Satzteil nach dem Fertig-Wort, Vorname im Fertig-Satz, Besitz, "for nothing".
    assert(!saysNothingNeeded("Nothing needed from you. I wait for the lead's green light. The summary is finished — reply "
                              + "with any corrections by noon."), "Auftrag nach Gedankenstrich")
    assert(!saysNothingNeeded("Nothing more to ask. I wait for the nightly build. Card #9 is finished — approval whenever suits."),
           "nichts zu fragen, Auftrag nach Gedankenstrich")
    assert(!saysNothingNeeded("Nothing needed from you. I wait for the timer. I hold until Will reviews the PR; the review "
                              + "is done after that."), "Vorname im Fertig-Satz")
    assert(saysNothingNeeded("Nothing needed from you. I wait for the lead's line.\n\n- **Done:** all five updates are "
                             + "committed.\n- The review loop is closed: four rounds."), "Doppelpunkt: Liste, kein Auftrag")
    // review quick 2026-10-10 Runde 2 (Codex).
    assert(!saysNothingNeeded("Nothing needed from you. The card is closed out. I wait for the team lead."), "closed out")
    assert(!saysNothingNeeded("Nothing needed from you on testing. I wait for the lead. When the CI run is done, merge PR #12."),
           "Bedingung + Auftrag bleibt")
    assert(!saysNothingNeeded("Nothing needed from you on testing. PR #12 is merged. I wait for the team lead. I hold until "
                              + "you approve the release."), "zweiter Warte-Satz auf Kevin sperrt")
    assert(!saysNothingNeeded("Nothing needed from you on testing. PR #12 is merged. I hold until README.md is approved by you."),
           "Punkt im Dateinamen")
    assert(!saysNothingNeeded("Nichts von dir nötig. Die Aufgabe ist abgeschlossen. Ich warte jetzt nicht mehr auf die "
                              + "Lead-Session."), "Fuellwort vor der Verneinung")
    assert(!saysNothingNeeded("I have nothing to ask the lead. The release waits."), "nichts zu fragen an Dritte")
    assert(saysNothingNeeded("Nothing needed from you. PR #12 is merged. I will wait for the lead."), "'will' ist nicht Will")
    // Kontrollrunde (GLM): Dritter nach dem Zeilenumbruch; "keine Frage" oeffnet nicht den ersten Satz beim Aufpasser.
    assert(!saysNothingNeeded("I have nothing more to ask\nthe team lead; the release waits."), "Dritter nach Umbruch")
    assert(!opensWithNothingNeeded("I have nothing to ask you. Please approve the deployment."), "keine Frage != keine Bitte")
    assert(!saysNothingNeeded("I have nothing to ask you. Please approve the deployment."), "Bitte am ganzen Text")

    // Kevin 2026-10-10 (Screenshot 23:33): wartet die Session selbst und sagt der erste Satz "nichts noetig", zaehlen
    // Bitten im Rest nicht (Bericht ueber ein anderes Fenster); "?", offene Frage, Schliessen sperren weiter.
    let rulesPage = "Nothing needed from you, Kevin. Nothing runs now. I hold until the team lead's go for the test step "
        + "tomorrow.\n\n- **To know:** my turn comes after the rule editor's. The editor waits on your answer to its spec "
        + "question in the morning."
    assert(!saysNothingNeeded(rulesPage) && waitsWithNothingNeeded(rulesPage), "anderes Fenster wartet auf Kevin")
    assert(!waitsWithNothingNeeded("Nothing needed from you. The editor waits on your answer."), "ohne eigenes Warten")
    assert(!waitsWithNothingNeeded("Nothing needed from you. I hold until the lead's go. Should I merge #12?"), "Fragezeichen")
    assert(!waitsWithNothingNeeded("Nothing needed from you. I wait for the lead. My question above stands."), "offene Frage")
    assert(!waitsWithNothingNeeded("Nothing needed from you. I wait for the lead. You can close this window."), "schliessen")
    assert(!waitsWithNothingNeeded("I have nothing to ask you. I wait for the lead. Please approve the release."),
           "keine Frage ist kein erster Satz")
    assert(!waitsWithNothingNeeded("Nothing needed from you. I wait for the lead. Please approve the deployment now."),
           "please sperrt weiter (review quick, GLM)")
    assert(!waitsWithNothingNeeded("Nichts von dir nötig. Ich warte auf die Lead-Session. Bitte gib den PR frei."), "bitte")
    assert(!waitsWithNothingNeeded("Nothing needed from you. I wait for the lead. The report for you is complete."),
           "Fertig-Meldung an Kevin (review quick, Codex)")
    assert(!waitsWithNothingNeeded("Nothing needed from you on testing. I wait for the lead. I need your approval for the "
                                   + "deployment now."), "I need your")
    assert(!waitsWithNothingNeeded("Nothing needed from you. I wait for the lead. The final report is complete. It is ready "
                                   + "for you to review."), "ready for you")
    assert(!waitsWithNothingNeeded("Nothing needed from you. I hold until you approve the release.\n- The editor waits on "
                                   + "your answer."), "Warten auf Kevin ist kein eigenes Warten")
    assert(!saysNothingNeeded("Nothing needed from you. The four tickets I hold are resolved, the queue is finished."), "Besitz")
    assert(!saysNothingNeeded("Nothing needed from you. The queue is finished. I'm waiting for nothing else tonight."),
           "waiting for nothing")
    assert(!saysNothingNeeded("Nichts von dir nötig. Die Liste ist abgeschlossen. Ich warte auf nichts mehr."), "auf nichts")
    assert(saysNothingNeeded("Nothing needed from you. PR #12 is merged. I hold my report until the lead's check arrives."),
           "hold … until")
}

func testStandby() {
    let now = Date(timeIntervalSince1970: 1_790_503_000)
    func worker(_ id: String, ready: Bool = true, waiting: Bool = false,
                text: String = "Nothing needed from you. I wait for the lead's go.", ended: Bool = true,
                peerPid: pid_t? = 33134, peerName: String? = "Beta · Lead",
                wakeups: [String: Date?] = [:], question: String? = nil, kevin: Bool = false,
                answerAt: Date? = nil) -> Session {
        var s = Session(id: "claude:\(id)", engine: .claude, pid: 88855, name: "Beta · #12 Theme check", cwd: "/x",
                        ready: ready, waiting: waiting, alive: true)
        s.digest = Digest(text: text, last: .text, answerAt: answerAt, question: question, turnEnded: ended, kevinTurn: kevin,
                          peerPid: peerPid, peerName: peerName, wakeups: wakeups)
        return s
    }
    let lead = Session(id: "claude:lead", engine: .claude, pid: 33134, name: "Beta · Lead", cwd: "/u",
                       ready: false, alive: true)
    var said: [String] = []
    let standby = Standby(report: { said.append($0) })
    var out = standby.adjust([worker("a"), lead], now: now)
    assert(out[0].watching && !out[0].ready, "wartet auf lebende andere Session und sagt 'nichts nötig': überwacht")
    assert(!out[1].watching && !out[1].ready, "die andere Session selbst bleibt, wie gemeldet")
    assert(said.count == 1, "eine Log-Zeile je Wechsel, bekam \(said)")
    out = standby.adjust([worker("a"), lead], now: now)
    assert(out[0].watching && said.count == 1, "gleicher Zustand: keine neue Zeile")

    let s = Standby(report: { _ in })
    assert(s.adjust([worker("b", peerPid: 4242, peerName: nil)], now: now)[0].ready, "andere Session beendet: fertig")
    // review quick 2026-09-27 (GLM): pid wiedervergeben — nur eine laufende Claude-Session zaehlt, nicht irgendein Prozess.
    assert(s.adjust([worker("b2", peerName: nil)], now: now)[0].ready, "pid ohne Claude-Session dazu: fertig")
    assert(s.adjust([worker("c", peerPid: nil), lead], now: now)[0].watching, "nur Name (SendMessage), Session läuft")
    // review quick 2026-10-10 (Codex): eine Session, die erst nach der Nachricht startete, traegt nur die alte pid.
    var reborn = worker("c2", peerName: nil)
    reborn.digest?.peerAt = now.addingTimeInterval(-3600)
    let newcomer = Session(id: "claude:new", engine: .claude, pid: 33134, name: "Gamma · Other", cwd: "/v", ready: false,
                           alive: true, startedAt: now.addingTimeInterval(-600))
    assert(s.adjust([reborn, newcomer], now: now)[0].ready, "wiedervergebene pid: fertig")
    var same = worker("c3", peerName: nil)
    same.digest?.peerAt = now.addingTimeInterval(-60)
    assert(s.adjust([same, newcomer], now: now)[0].watching, "pid schon vor der Nachricht gestartet: dieselbe Session")
    assert(s.adjust([worker("d", peerPid: nil)], now: now)[0].ready, "nur Name, keine solche Session: fertig")
    let soon: [String: Date?] = ["loop": now.addingTimeInterval(600)]
    assert(s.adjust([worker("e", peerPid: nil, peerName: nil, wakeups: soon)], now: now)[0].watching, "Wecker steht aus")
    let late: [String: Date?] = ["loop": now.addingTimeInterval(-400)]
    assert(s.adjust([worker("f", peerPid: nil, peerName: nil, wakeups: late)], now: now)[0].ready, "Wecker längst vorbei")
    assert(s.adjust([worker("g", peerPid: nil, peerName: nil, wakeups: ["x": nil])], now: now)[0].watching, "wiederkehrend")
    assert(s.adjust([worker("h", peerPid: nil, peerName: nil)], now: now)[0].ready, "ohne Weck-Quelle: fertig wie heute")

    assert(s.adjust([worker("i", text: "Nothing needed from you. Should I merge?"), lead], now: now)[0].ready, "Frage: fertig")
    assert(s.adjust([worker("j", text: "Done. You can close this window."), lead], now: now)[0].ready, "fertig bleibt fertig")
    let asks = s.adjust([worker("k", waiting: true), lead], now: now)[0]
    assert(asks.waiting && !asks.watching, "offene Frage/Freigabe bleibt 'wartet auf dich'")
    assert(!s.adjust([worker("l", question: "Welche Farbe?"), lead], now: now)[0].watching, "offene AskUserQuestion")
    let busy = s.adjust([worker("m", ready: false), lead], now: now)[0]
    assert(!busy.watching && !busy.ready, "arbeitet: bleibt")
    assert(s.adjust([worker("n", ended: false), lead], now: now)[0].ready, "Zug abgebrochen (kein Zug-Ende): fertig")
    // Kevin 2026-10-01: hat seine eigene Nachricht den Zug ausgeloest, ist er dran — nie grau (frische Faelle 28.09.-01.10.).
    assert(s.adjust([worker("o", kevin: true), lead], now: now)[0].ready, "Antwort auf Kevins Nachricht: fertig")
    // R11 B1 (Kevin 2026-10-05, Screenshot 13:11): hatte Kevin die Antwort auf seine Nachricht vor sich, zaehlt sie nicht
    // mehr — grau aber nur mit ausdruecklichem "nichts noetig" (B1 streng, Kevins Wahl gegen das lockere B2).
    let answered = now.addingTimeInterval(-60)
    assert(s.adjust([worker("p", kevin: true, answerAt: answered), lead], now: now,
                    seen: ["claude:p": answered.addingTimeInterval(5)])[0].watching, "Antwort gesehen: überwacht")
    assert(s.adjust([worker("q", kevin: true, answerAt: answered), lead], now: now,
                    seen: ["claude:q": answered.addingTimeInterval(-5)])[0].ready, "nur vor der Antwort vorn: fertig")
    assert(s.adjust([worker("q2", kevin: true), lead], now: now, seen: ["claude:q2": now])[0].ready,
           "ohne Zeitpunkt der Antwort: nicht gesehen")
    let keepOpen = "Please keep this window open, Kevin. The card is not finished. The window waits for its turn."
    assert(s.adjust([worker("r", text: keepOpen, kevin: true, answerAt: answered), lead], now: now,
                    seen: ["claude:r": now])[0].ready, "gesehen, aber ohne 'nichts nötig': fertig (B1 streng)")
    assert(s.adjust([worker("r2", kevin: true, answerAt: answered)], now: now, seen: ["claude:r2": now])[0].ready,
           "gesehen, andere Session läuft nicht mehr: fertig")
    var seenSaid: [String] = []
    _ = Standby(report: { seenSaid.append($0) }).adjust([worker("p", kevin: true, answerAt: answered), lead], now: now,
                                                         seen: ["claude:p": now])
    assert(seenSaid.first?.contains("Antwort gesehen") == true, "Log nennt den Grund, bekam \(seenSaid)")

    // Zurueck zu fertig: eine Zeile.
    said.removeAll()
    let fresh = Standby(report: { said.append($0) })
    _ = fresh.adjust([worker("a"), lead], now: now)
    _ = fresh.adjust([worker("a", peerPid: nil, peerName: nil), lead], now: now)
    assert(said.count == 2, "hin und zurück: je eine Zeile, bekam \(said)")
}

/// Kevin 2026-10-12: die Session sagt am Zugende selbst, ob sie Kevin braucht ("◌ / ? / ✓ Lookout: …") — das geht der
/// Wortliste vor; fehlt die Zeile, bleibt alles wie bisher.
func testLookoutLine() {
    // Lesen
    assert(lookoutLine("Erledigt.\n\n✓ Lookout: fertig (Karte #121)") == LookoutLine(status: .done, reason: "Karte #121"))
    assert(lookoutLine("x\n? Lookout: Bitte (Merge #126)\n\n") == LookoutLine(status: .ask, reason: "Merge #126"),
           "Leerzeilen danach")
    assert(lookoutLine("◌ Lookout: braucht nichts (wartet auf CI)")?.status == .nothing)
    assert(lookoutLine("**? Lookout: Bitte (Passwort eingeben)**")?.reason == "Passwort eingeben", "Markdown drumherum")
    assert(lookoutLine("`✓ Lookout: done (shipped)`")?.status == .done, "Backticks, englisch")
    assert(lookoutLine("\u{2713}\u{FE0E} Lookout: fertig")?.status == .done, "Haken mit Text-Auswahlzeichen")
    assert(lookoutLine("? Lookout: Bitte Freigabe für den Merge von PR #42 geben")?.reason
           == "Freigabe für den Merge von PR #42 geben", "ohne Klammer: Text ohne Statuswort (Probe 12.10.)")
    assert(lookoutLine("✓ Lookout: fertig")?.reason == "fertig", "nur das Statuswort: bleibt stehen")
    assert(lookoutLine("✓ Lookout: fertig ()")?.reason == "fertig", "leere Klammer")
    assert(lookoutLine("? Lookout: Bitte (a) oder (b)")?.reason == "(a) oder (b)", "zwei Klammern (review quick, GLM)")
    assert(lookoutLine("? Lookout: Bitte (Merge #126 (PR-3))")?.reason == "Merge #126 (PR-3)", "Klammer in der Klammer")
    assert(lookoutLine("? Lookout: (Passwort)")?.reason == "Passwort", "ohne Statuswort")
    assert(lookoutLine("◌ Lookout:") == nil && lookoutLine("? Lookout:   ") == nil, "leer: keine Zeile (review quick, Muse)")
    assert(lookoutLine("? Lookout: Bitte (Merge #126).")?.reason == "Merge #126", "Punkt am Ende (review quick, Grok)")
    assert(lookoutLine("? Lookout: Bitte (Merge #126)\nNoch ein Satz danach.") == nil, "nur die letzte Zeile zählt")
    assert(lookoutLine("Soll ich mergen?") == nil && lookoutLine("") == nil, "ohne Zeile: nichts")
    assert(lookoutLine("? Lookouts: x") == nil && lookoutLine("! Lookout: x") == nil, "nur die drei Zeichen")

    // Transkript: die Zeile gilt nur fuer den Zug, in dem sie steht.
    func entry(_ o: [String: Any]) -> String {
        String(data: try! JSONSerialization.data(withJSONObject: o), encoding: .utf8)!
    }
    let answer = entry(["type": "assistant", "message": ["content": [["type": "text", "text": "Fertig.\n✓ Lookout: fertig (X)"]]]])
    let ended = entry(["type": "system", "subtype": "turn_duration"])
    let prompt = entry(["type": "user", "message": ["content": "weiter"]])
    let fire = entry(["type": "system", "subtype": "scheduled_task_fire"])
    assert(digestTranscript([answer, ended].joined(separator: "\n")).line?.reason == "X")
    assert(digestTranscript([answer, ended, prompt].joined(separator: "\n")).line == nil, "neue Nachricht: alte Zeile weg")
    // Nach einer anderen Session oder einem Wecker ohne neuen Text bleibt die letzte Selbstauskunft (review quick, Grok).
    let peerMsg = entry(["type": "user", "message": ["content": "go"], "origin": ["kind": "peer"]])
    assert(digestTranscript([answer, ended, fire].joined(separator: "\n")).line != nil, "Wecker: Zeile bleibt")
    assert(digestTranscript([answer, ended, peerMsg, ended].joined(separator: "\n")).line != nil, "andere Session: bleibt")
    let talk = [entry(["type": "user", "message": ["content": "q"], "origin": ["kind": "peer", "from": "uds:/tmp/cc-socks/1.sock",
                                                                                   "name": "Beta · A"]]),
                entry(["type": "user", "message": ["content": "q"], "origin": ["kind": "peer", "from": "uds:/tmp/cc-socks/2.sock",
                                                                                   "name": "Beta · B"]])]
    assert(digestTranscript(talk.joined(separator: "\n")).partners == ["Beta · A", "Beta · B"], "alle Partnerinnen")
    let midTurn = entry(["type": "attachment", "attachment": ["commandMode": "prompt", "origin": ["kind": "human"]]])
    assert(digestTranscript([answer, midTurn].joined(separator: "\n")).line == nil,
           "Kevins Nachricht mitten im Zug: alte Zeile weg (review quick, Codex)")
    assert(digestTranscript([answer, ended, prompt, answer].joined(separator: "\n")).line != nil, "neue Antwort: neue Zeile")

    // Andere Session oder Wecker (`Standby`)
    let now = Date(timeIntervalSince1970: 1_790_503_000)
    func session(_ text: String, ready: Bool = true, background: Bool = false, ended: Bool = true, kevin: Bool = false,
                 peer: Bool = true) -> Session {
        var s = Session(id: "claude:w", engine: .claude, pid: 88855, name: "Beta · Worker", cwd: "/x",
                        ready: ready, alive: true, backgroundOnly: background)
        s.digest = Digest(text: text, last: .text, turnEnded: ended, kevinTurn: kevin, peerPid: peer ? 33134 : nil)
        return s
    }
    let lead = Session(id: "claude:lead", engine: .claude, pid: 33134, name: "Beta · Lead", cwd: "/u", ready: false,
                       alive: true)
    var said: [String] = []
    let s = Standby(report: { said.append($0) })
    let grau = s.adjust([session("Merged. Soll ich?\n◌ Lookout: braucht nichts (Lead prüft)", kevin: true), lead], now: now)
    assert(grau[0].watching && !grau[0].ready, "'braucht nichts' schlägt Fragezeichen und Kevins Nachricht")
    assert(said.first?.contains("(Lookout-Zeile)") == true, "Log nennt den Grund, bekam \(said)")
    assert(s.adjust([session("Nothing needed from you. I wait for the lead.\n✓ Lookout: fertig (Karte #12)"), lead],
                    now: now)[0].ready, "'fertig' schlägt 'nothing needed'")
    assert(s.adjust([session("◌ Lookout: braucht nichts (CI)", peer: false)], now: now)[0].ready,
           "ohne Weck-Quelle: fertig — nichts weckt sie von selbst")
    // Kevin 2026-10-12 (Screenshot 16:08, "Key account"): die letzte Gespraechspartnerin ist beendet, eine andere laeuft
    var hub = session("◌ Lookout: braucht nichts (naechste Frage eines Workers)", peer: false)
    hub.digest?.partners = ["Beta · #294 beendet", "Beta · Lead"]
    assert(s.adjust([hub, lead], now: now)[0].watching, "Sammel-Session: irgendeine laufende Partnerin weckt")
    assert(s.adjust([hub], now: now)[0].ready, "keine Partnerin laeuft mehr: fertig")
    var wortHub = session("Nothing needed from you. I wait.", peer: false)
    wortHub.digest?.partners = ["Beta · Lead"]
    assert(s.adjust([wortHub, lead], now: now)[0].ready, "ohne Lookout-Zeile bleibt es bei der letzten Partnerin (gemessene Regel)")
    assert(s.adjust([session("Nothing needed from you. I wait for the lead's go."), lead], now: now)[0].watching,
           "ohne Zeile: Wortliste wie bisher")

    // Eigener Aufpasser (`BackgroundWatch`)
    let rows = backgroundFixture.split(separator: "\n").compactMap(parsePsLine)
    let t0 = Date(timeIntervalSince1970: 60_000)
    let watch = BackgroundWatch(netUsage: { [:] }, listening: { _ in [542] }, report: { _ in })
    func bg(_ text: String, kevin: Bool = false) -> Session {
        var s = Session(id: "claude:q", engine: .claude, pid: 530, name: "lookout-q", cwd: "", ready: false, alive: true,
                        backgroundOnly: true)
        s.digest = Digest(text: text, last: .text, turnEnded: true, kevinTurn: kevin)
        return s
    }
    let leise = bg("Soll ich danach mergen? Erst CI.\n◌ Lookout: braucht nichts (CI läuft)", kevin: true)
    for dt in [0.0, 96] { _ = watch.adjust([leise], rows: rows, now: t0.addingTimeInterval(dt), window: 90) }
    assert(watch.adjust([leise], rows: rows, now: t0.addingTimeInterval(99), window: 90)[0].watching,
           "Aufpasser + 'braucht nichts': überwacht")
    assert(watch.adjust([bg("Waiting for CI. Nothing needed from you.\n✓ Lookout: fertig (PR offen)")], rows: rows,
                        now: t0.addingTimeInterval(102), window: 90)[0].ready, "'fertig' schlägt 'nothing needed'")

    // Bitte: wartet auf dich
    let bitte = applyLookoutLines([session("Bereit.\n? Lookout: Bitte (Merge #126)")])[0]
    assert(bitte.waiting && bitte.ready && !bitte.watching, "Bitte: wartet auf dich")
    assert(applyLookoutLines([session("? Lookout: Bitte (Test)", ready: false, background: true)])[0].waiting,
           "auch wenn nur noch ein Hintergrund-Befehl läuft")
    var grauBitte = session("? Lookout: Bitte (Test)", ready: false)
    grauBitte.watching = true
    let vorn = applyLookoutLines([grauBitte])[0]
    assert(vorn.waiting && !vorn.watching, "überwacht wird zu wartet auf dich")
    let arbeitet = applyLookoutLines([session("? Lookout: Bitte (x)", ready: false)])[0]
    assert(!arbeitet.waiting && !arbeitet.ready, "busy: das Modell arbeitet")
    assert(!applyLookoutLines([session("? Lookout: Bitte (x)", ended: false)])[0].waiting, "ohne Zug-Ende nicht")
    assert(!applyLookoutLines([session("✓ Lookout: fertig (x)")])[0].waiting, "fertig bleibt fertig")
    var alt = session("? Lookout: Bitte (x)")
    alt.digest?.textCurrent = false
    assert(!applyLookoutLines([alt])[0].waiting, "Zeile aus einem früheren Zug")
    let codex = Session(id: "codex:c", engine: .codex, pid: 1, name: "c", cwd: "", ready: true, alive: true)
    assert(!applyLookoutLines([codex])[0].waiting, "nur Claude")

    // Schild: die Zeile sagt, worum es geht
    var f = Session(id: "claude:l", engine: .claude, pid: 2, name: "alpha-l", cwd: "/Users/test/Projects/Alpha",
                    ready: true, alive: true)
    f.digest = Digest(title: "Rechnungen", text: "Kevin, alles gebucht. Mehr folgt.\n✓ Lookout: fertig (12 Rechnungen)",
                      last: .text)
    assert(labelContent(for: tabWindow([f], title: "✳ Rechnungen"), since: nil, now: now).detail == "12 Rechnungen")
    let wartet = applyLookoutLines([session("Bereit.\n? Lookout: Bitte (Merge #126)")])[0]
    var w = wartet
    w.digest?.step = "deploy"
    assert(labelContent(for: tabWindow([w], title: "✳ Shop"), since: nil, now: now).detail == "Merge #126",
           "Bitte statt 'Freigabe: <Schritt>'")
}

// ---------- Plan 1b: Menueleisten-Symbol ----------

/// Kevin 26.09. im Praxistest: "?", "!" und Zahlen neben dem Symbol irritieren — nur das Symbol. Wie viele und welche,
/// steht im Menue; Frage oder fertig zeigen die Rahmen.
func testStatusBadge() {
    assert(statusBadge(ready: 0, asking: 0, trusted: true) == (false, false), "nichts wartet: leere Kachel")
    assert(statusBadge(ready: 3, asking: 0, trusted: true) == (true, false), "fertig: gefuellt")
    assert(statusBadge(ready: 0, asking: 1, trusted: true) == (true, false), "Frage: gefuellt")
    assert(statusBadge(ready: 2, asking: 1, trusted: true) == (true, false), "Frage und fertig: gefuellt")
    assert(statusBadge(ready: 2, asking: 1, trusted: false) == (false, true), "Freigabe fehlt: grau, Kachel leer")
}

// ---------- Plan 1d: Status bestimmt die Reihenfolge, auf dem MacBook verschwindet nichts ----------

func testRailSplit() {
    assert(railSplit(height: 1005, flexible: 8) == RailSplit(cards: 4, rows: 4, more: 0), "4 × 192 + 28 + 4 × 46 = 980")
    assert(railSplit(height: 1005, flexible: 6) == RailSplit(cards: 4, rows: 2, more: 0))
    assert(railSplit(height: 1005, flexible: 5) == RailSplit(cards: 5, rows: 0, more: 0), "passt alles: keine Liste")
    assert(railSplit(height: 1239, flexible: 6) == RailSplit(cards: 6, rows: 0, more: 0), "LG: sechs Fenster je Streifen")
    assert(railSplit(height: 1239, flexible: 7) == RailSplit(cards: 6, rows: 1, more: 0), "LG: ab dem 7. greift die Liste")
    assert(railSplit(height: 1005, flexible: 2, listOnly: 8) == RailSplit(cards: 2, rows: 8, more: 0))
    assert(railSplit(height: 1005, flexible: 0, listOnly: 30) == RailSplit(cards: 0, rows: 20, more: 10), "+10 weitere (im Menü)")
    assert(railSplit(height: 1005, flexible: 0) == RailSplit(cards: 0, rows: 0, more: 0))
}

/// Kevin 2026-09-26: bei 11 Sessions fasst ein Rand auf dem MacBook hoechstens 5 Fenster; der Rest blieb liegen und
/// verdeckte Wartende. Aktive Mitte (0, laeuft) + fertig (1) + Frage (2) + 8 laufend (3…10), alle noch auf dem LG.
private func elevenSessions() -> [TermWindow] {
    let t0 = Date(timeIntervalSince1970: 1000)
    var w = [fakeWindow(ready: false, firstSeen: t0),
             fakeWindow(ready: true, firstSeen: t0.addingTimeInterval(1), readySince: Date(timeIntervalSince1970: 5000)),
             fakeWindow(ready: true, firstSeen: t0.addingTimeInterval(2), readySince: Date(timeIntervalSince1970: 6000),
                        waiting: true)]
    for i in 3..<11 { w.append(fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(Double(i)))) }
    for i in w.indices { w[i].frame = CGRect(x: 100 + CGFloat(i) * 10, y: 100, width: 800, height: 600) }
    return w
}

/// Jede Session hat genau eine Rolle (Mitte, Fenster oder geparkt); nichts ragt ueber den Schirm; sichtbare Fenster
/// ueberlappen weder sich noch eine Liste.
private func assertRoles(_ a: Arrangement, count: Int, screen: CGRect) {
    assert(Set(a.frames.keys) == Set(0..<count), "jede Session hat einen Platz, bekam \(a.frames.keys.sorted())")
    for r in a.frames.values { assert(screen.contains(r), "\(r) ragt ueber den Schirm") }
    let shown = a.frames.filter { !a.parked.contains($0.key) }.map(\.value) + a.lists.map(\.rect)
    for (i, r) in shown.enumerated() { for s in shown.dropFirst(i + 1) { assert(!r.intersects(s), "\(r) ueberlappt \(s)") } }
    let rows = a.lists.flatMap(\.entries).compactMap { e -> Int? in if case .row(let i) = e { return i }; return nil }
    assert(Set(rows) == a.parked && rows.count == a.parked.count, "jedes geparkte Fenster steht genau einmal als Zeile")
}

func testMacBookFocus1ThreeColumns() {
    let w = elevenSessions()
    let a = planArrangement(mode: .reading, target: .single(1), windows: w, screens: [oled, macbook], side: .right,
                            focusedIndex: 0)
    guard let cols = threeColumnLayout(macbook) else { return assert(false, "MacBook: drei Spalten") }
    assert(cols.left.width == MIN_CARD_W && cols.right.width == MIN_CARD_W, "beide Seiten 480 pt")
    assert(cols.main.width == macbook.width - 2 * MIN_CARD_W && cols.main.minX == cols.left.maxX, "Mitte = Rest")
    assert(a.frames[0] == cols.main && a.main == 0, "die aktive Session in der Mitte")
    for i in [1, 2] { assert(cols.left.contains(a.frames[i]!) && !a.parked.contains(i), "Fenster \(i) wartet -> links") }
    assert(a.frames[2]!.maxY > a.frames[1]!.maxY, "die Frage steht oben")
    let rechts = (3..<11).filter { cols.right.contains(a.frames[$0]!) }
    assert(rechts == [3, 4, 5, 6], "die ersten vier Laufenden als Fenster rechts, bekam \(rechts)")
    assert(a.parked == [7, 8, 9, 10], "der Rest ist geparkt, bekam \(a.parked.sorted())")
    for i in a.parked { assert(a.frames[i] == cols.main, "geparkt = exakt auf der Mitte") }
    assert(a.lists.map(\.entries) == [[.header("+4 weitere"), .row(7), .row(8), .row(9), .row(10)]], "bekam \(a.lists)")
    assert(cols.right.contains(a.lists[0].rect) && a.lists[0].rect.minY == macbook.minY, "Liste unten rechts")
    assertRoles(a, count: 11, screen: macbook)
    assert(planLayout(mode: .reading, target: .single(1), windows: w, screens: [oled, macbook], side: .right,
                      focusedIndex: 0) == a.frames, "planLayout liefert dieselben Rahmen")
}

func testMacBookFocus2OneRail() {
    let w = elevenSessions()
    let a = planArrangement(mode: .focus2, target: .single(1), windows: w, screens: [oled, macbook], side: .right,
                            focusedIndex: 0)
    let (main, left, rail) = readingLayout(macbook, side: .right)
    assert(left == nil && a.frames[0] == main, "Fokus 2 auf dem MacBook: ein Rand")
    for i in [1, 2] { assert(rail!.contains(a.frames[i]!) && !a.parked.contains(i), "2 Karten") }
    assert(a.parked == Set(3..<11), "alle 8 Laufenden geparkt")
    for i in 3..<11 { assert(a.frames[i] == main, "exakt auf dem Hauptrahmen") }
    assert(a.lists.map(\.entries) == [[.header("ARBEITET (8)")] + (3..<11).map { .row($0) }], "8 Zeilen, bekam \(a.lists)")
    assert(rail!.contains(a.lists[0].rect))
    assertRoles(a, count: 11, screen: macbook)
}

/// Fall 7 wartend + 3 laufend: ueberzaehlige Wartende stehen als Zeilen — in Fokus 1 links, in Fokus 2 ueber ARBEITET.
func testSevenWaitingThreeRunning() {
    let t0 = Date(timeIntervalSince1970: 1000)
    var w = [fakeWindow(ready: false, firstSeen: t0)]
    for i in 1...7 {
        w.append(fakeWindow(ready: true, firstSeen: t0.addingTimeInterval(Double(i)),
                            readySince: Date(timeIntervalSince1970: 5000 + Double(i))))
    }
    for i in 8...10 { w.append(fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(Double(i)))) }
    let f1 = planArrangement(mode: .reading, target: .single(1), windows: w, screens: [oled, macbook], side: .right,
                             focusedIndex: 0)
    assert(f1.parked == [5, 6, 7], "Fokus 1: 4 Wartende als Fenster, 3 als Zeilen, bekam \(f1.parked.sorted())")
    assert(f1.lists.map(\.entries) == [[.header("+3 weitere"), .row(5), .row(6), .row(7)]], "bekam \(f1.lists)")
    assertRoles(f1, count: 11, screen: macbook)
    let f2 = planArrangement(mode: .focus2, target: .single(1), windows: w, screens: [oled, macbook], side: .right,
                             focusedIndex: 0)
    assert(f2.parked == [4, 5, 6, 7, 8, 9, 10], "Fokus 2: 3 Wartende als Fenster, bekam \(f2.parked.sorted())")
    assert(f2.lists.map(\.entries) == [[.row(4), .row(5), .row(6), .row(7), .header("ARBEITET (3)"), .row(8), .row(9),
                                        .row(10)]], "uebrige Wartende direkt ueber ARBEITET, bekam \(f2.lists)")
    assertRoles(f2, count: 11, screen: macbook)
}

/// R2-2: wird es eng, kuerzt Lookout zuerst die ARBEITET-Liste — Wartende gehen vor.
func testFocus2WaitingFirst() {
    let t0 = Date(timeIntervalSince1970: 1000)
    var w = [fakeWindow(ready: false, firstSeen: t0)]
    for i in 1...5 {
        w.append(fakeWindow(ready: true, firstSeen: t0.addingTimeInterval(Double(i)),
                            readySince: Date(timeIntervalSince1970: 5000 + Double(i))))
    }
    for i in 6...25 { w.append(fakeWindow(ready: false, firstSeen: t0.addingTimeInterval(Double(i)))) }
    assert(railSplit(height: macbook.height, flexible: 5, listOnly: 20) == RailSplit(cards: 0, rows: 20, more: 5))
    let a = planArrangement(mode: .focus2, target: .single(1), windows: w, screens: [oled, macbook], side: .right,
                            focusedIndex: 0)
    let e = a.lists.first?.entries ?? []
    assert(Array(e.prefix(5)) == (1...5).map { .row($0) }, "alle Wartenden stehen, bekam \(e.prefix(6))")
    assert(e.contains(.header("ARBEITET (20)")) && e.last == .more(5), "die ARBEITET-Liste endet mit +5 weitere")
    assert(e.filter { if case .row = $0 { return true }; return false }.count == 20)
    assert(a.lists[0].rect.height <= macbook.height && macbook.contains(a.lists[0].rect))
    assert(a.parked.count == 25, "wer keine Zeile hat, liegt trotzdem geparkt (im Menue erreichbar)")
    assert(a.lists[0].count == 25, "die Liste zaehlt Zeilen und '+N weitere' (Vorschau 'Liste (N)')")
}

/// LG, Fokus 1: die Seitenfenster folgen dem Status — Frage vorn, dann fertig, dann laufend; die Reihenfolge fuellt
/// links bis zur Haelfte, dann rechts (wie bisher).
func testFocus1OnLGByStatus() {
    let w = spreadWindows()                       // 0 laeuft (aktiv), 1 fertig, 2 Frage, 3 laeuft
    let plan = planLayout(mode: .reading, target: .single(0), windows: w, screens: [oled, macbook], side: .right,
                          focusedIndex: 0)
    let (main, left, right) = readingLayout(oled, side: .right)
    assert(plan[0] == main)
    assert(left!.contains(plan[2]!) && left!.contains(plan[1]!), "Frage und fertig links")
    assert(plan[2]!.maxY > plan[1]!.maxY, "die Frage steht vorn")
    assert(right!.contains(plan[3]!), "laufend rechts")
}

/// GP 5 / R2-3: getauscht wird nur unter Laufenden. Klick ueber eine Gruppengrenze: die alte Mitte geht an ihren
/// Gruppenplatz — laeuft sie, an den Anfang der Laufenden.
func testClickAcrossGroups() {
    let w = spreadWindows()                       // 0 laeuft, 1 fertig, 2 Frage, 3 laeuft
    let a = statusSwap([1, 2, 3], oldMain: 0, newMain: 1, windows: w)
    assert(statusSequence(a, windows: w) == [2, 0, 3], "Frage vorn, die alte Mitte als erste Laufende, bekam \(a)")
    let b = statusSwap([0, 2, 3], oldMain: 1, newMain: 3, windows: w)
    assert(statusSequence(b, windows: w) == [2, 1, 0], "die fertige alte Mitte geht zu den Fertigen, bekam \(b)")
    assert(statusSwap([1, 2, 3], oldMain: 0, newMain: 3, windows: w) == [1, 2, 0], "zwei Laufende tauschen")
    let plan = planLayout(mode: .reading, target: .single(0), windows: w, screens: [oled, macbook], side: .right,
                          focusedIndex: 1, restHint: a)
    let (main, left, right) = readingLayout(oled, side: .right)
    assert(plan[1] == main && left!.contains(plan[2]!) && left!.contains(plan[0]!) && right!.contains(plan[3]!),
           "die alte Mitte steht jetzt links unter der Frage")
}

/// Schmaler Schirm (1280 pt): keine drei Spalten (die Mitte waere schmaler als Warps Minimum), ein Rand wie bisher.
func testNarrowScreenKeepsOneRail() {
    let narrow = CGRect(x: 0, y: 0, width: 1280, height: 800)
    assert(threeColumnLayout(narrow) == nil, "unter 3 × 480 pt keine drei Spalten")
    let w = spreadWindows()
    let a = planArrangement(mode: .reading, target: .single(0), windows: w, screens: [narrow], side: .right,
                            focusedIndex: 0)
    let (main, left, right) = readingLayout(narrow, side: .right)
    assert(left == nil && a.frames[0] == main, "ein Rand wie heute")
    for i in [1, 2, 3] { assert(right!.contains(a.frames[i]!), "Fenster \(i) im Rand") }
    assert(a.frames[2]!.maxY > a.frames[1]!.maxY && a.frames[1]!.maxY > a.frames[3]!.maxY, "nach Status")
}

/// GP 2: geparkte Fenster bekommen nie Rahmen oder Schild — auch nicht ein Ghostty-Fenster hinter einer Warp-Mitte
/// (die Deckungsgleichheit gilt nur innerhalb derselben App).
func testParkedWindowsHaveNoOverlay() {
    let t0 = Date(timeIntervalSince1970: 1000)
    var mitte = fakeWindow(ready: false, firstSeen: t0)                                   // Warp, pid 0
    var geparkt = TermWindow(app: "Ghostty", bundleId: GHOSTTY_BID, ownerPid: 7, ax: AXUIElementCreateApplication(7),
                             frame: .zero, title: "",
                             sessions: [Session(id: "claude:g", engine: .claude, pid: 70, name: "g", cwd: "", ready: true,
                                                alive: true)],
                             firstSeen: t0, readySince: t0, movable: true, minimized: false, fullscreen: false)
    let r = CGRect(x: 687, y: -1037, width: 1248, height: 1005)
    mitte.frame = r
    geparkt.frame = r
    let ax = cocoaToAX(r, .topLeft, screenH: 1239)
    let list = [OnScreenWindow(number: 1, pid: 0, rect: ax), OnScreenWindow(number: 2, pid: 7, rect: ax)]
    assert(glowTargets(windows: [mitte, geparkt], onScreen: list, screenH: 1239, screens: [macbook]).count == 1,
           "ohne Parken bekaeme das Ghostty-Fenster einen Rahmen")
    assert(glowTargets(windows: [mitte, geparkt], onScreen: list, screenH: 1239, screens: [macbook], parked: [1]).isEmpty,
           "geparkt: kein Rahmen")
    let labels = labelTargets(windows: [mitte, geparkt], onScreen: list, screenH: 1239, screens: [macbook],
                              focusedIndex: nil, since: [:], now: t0, parked: [1])
    assert(labels.map(\.id) == [mitte.sessions[0].id], "geparkt: kein Schild, bekam \(labels.map(\.id))")
}

/// R2-1: geparkte Fenster liegen deckungsgleich hinter der Mitte — `locateOnScreen` ist mehrdeutig. Die Liste nimmt die
/// vorderste Fensternummer dieser App auf dem Mitte-Rahmen (die Fensterliste kommt von vorn nach hinten).
func testListFindsMainWindowNumber() {
    var mitte = fakeWindow(ready: false, firstSeen: Date(timeIntervalSince1970: 1000))   // pid 0
    mitte.frame = CGRect(x: 687, y: -1037, width: 1248, height: 1005)
    let ax = cocoaToAX(mitte.frame, .topLeft, screenH: 1239)
    let stapel = [OnScreenWindow(number: 71, pid: 0, rect: ax), OnScreenWindow(number: 72, pid: 0, rect: ax),
                  OnScreenWindow(number: 73, pid: 0, rect: ax)]
    assert(locateOnScreen(mitte, onScreen: stapel, screenH: 1239) == .ambiguous, "deckungsgleich = mehrdeutig")
    assert(frontmostNumber(mitte, onScreen: stapel, screenH: 1239) == 71, "die vorderste ist nach dem Anheben die Mitte")
    assert(frontmostNumber(mitte, onScreen: [OnScreenWindow(number: 80, pid: 9, rect: ax)] + stapel, screenH: 1239) == 71,
           "nur Fenster derselben App")
    assert(frontmostNumber(mitte, onScreen: [], screenH: 1239) == nil)
}

/// GP 6: die Liste nimmt Klicks an, ohne Lookout zu aktivieren, liegt nicht auf allen Schreibtischen und wird von AppKit
/// nicht zurechtgerueckt (wie die Schilder).
func testListPanelTakesClicks() {
    _ = NSApplication.shared
    let p = makeListPanel()
    defer { p.orderOut(nil) }
    assert(!p.ignoresMouseEvents && p.styleMask.contains(.nonactivatingPanel), "Klicks ja, aktivieren nein")
    assert(!p.collectionBehavior.contains(.canJoinAllSpaces), "nur auf dem Schreibtisch der Mitte")
    assert(p.animationBehavior == .none)
    guard let first = NSScreen.screens.first else { return }
    let oben = CGRect(x: first.visibleFrame.minX + 40, y: first.visibleFrame.maxY - 100, width: 480, height: 120)
    p.setFrame(oben, display: false)
    p.orderFrontRegardless()
    assert(p.frame == oben, "nicht zurechtgerueckt: \(p.frame) statt \(oben)")
}

// ---------- Phase 2: teilbar (Sprache, Einstellungen, Anrede) ----------

/// Anrede (GP 9): der Vorname des macOS-Benutzers steht am Anfang, und das naechste Nicht-Leerzeichen ist kein Buchstabe
/// und keine Ziffer. "Name, dann ein Satzzeichen" liesse "Kevin 👋" stehen.
func testGreetingName() {
    assert(firstName("Kevin Theobald") == "Kevin" && firstName("  Ada  Lovelace ") == "Ada" && firstName("") == "")
    assert(firstSentence("Kevin, alles gut. Mehr folgt.") == "Alles gut.", "Komma")
    assert(firstSentence("Kevin 👋 Alles gut. Mehr folgt.") == "Alles gut.", "Emoji")
    assert(firstSentence("kevin, alles gut.") == "Alles gut.", "groß/klein egal")
    assert(firstSentence("Kevins Plan steht.") == "Kevins Plan steht.", "Buchstabe dahinter: keine Anrede")
    let saved = greetingName
    defer { greetingName = saved }
    greetingName = "Will"
    assert(firstSentence("Will this work? Maybe.") == "Will this work?", "Name Will, aber ein Wort dahinter")
    assert(firstSentence("Will, it works. More.") == "It works.")
    assert(firstSentence("Kevin, alles gut.") == "Kevin, alles gut.", "fremder Name bleibt")
    greetingName = ""
    assert(firstSentence("Kevin, alles gut.") == "Kevin, alles gut.", "ohne Namen nimmt Lookout nichts weg")
}

/// Einstellungen aus `~/.config/lookout.json`: alle bisherigen Schluessel, dazu `language` und `claudeConfigDirs`.
func testParseConfig() {
    let home = NSHomeDirectory()
    let d = parseConfig([:])
    assert(d.language == nil && d.claudeConfigDirs == [home + "/.claude"], "Standard: Mac-Sprache, ~/.claude")
    assert(d.layout == "reading" && d.stackSide == "right" && d.readyThresholdSec == 90 && d.pollSec == 3
           && d.sessionPollSec == 6 && d.hotkeys && d.glow && d.labels && d.glowColor == "#5ECFA0"
           && d.glowColorWaiting == "#E8B25C", "bisherige Standardwerte")
    let c = parseConfig(["language": "en", "claudeConfigDirs": ["~/.claude", "~/.claude-work/", "/opt/claude", 7],
                         "layout": "focus2", "stackSide": "left", "readyThresholdSec": 120, "pollSec": 5,
                         "sessionPollSec": 10, "hotkeys": false, "glow": false, "glowColor": "#FFFFFF",
                         "glowColorWaiting": "#000000", "labels": false])
    assert(c.language == "en")
    assert(c.claudeConfigDirs == [home + "/.claude", home + "/.claude-work", "/opt/claude"],
           "~ aufgeloest, Schraegstrich am Ende weg, Unlesbares faellt raus — bekam \(c.claudeConfigDirs)")
    assert(c.layout == "focus2" && c.stackSide == "left" && c.readyThresholdSec == 120 && c.pollSec == 5
           && c.sessionPollSec == 10 && !c.hotkeys && !c.glow && c.glowColor == "#FFFFFF"
           && c.glowColorWaiting == "#000000" && !c.labels, "jeder Schluessel kommt an")
    assert(parseConfig(["language": "fr"]).language == nil, "unbekannte Sprache: Mac-Sprache")
    assert(parseConfig(["claudeConfigDirs": [String]()]).claudeConfigDirs == [home + "/.claude"], "leer: Standard")
    assert(parseConfig(["claudeConfigDirs": "~/.claude-work"]).claudeConfigDirs == [home + "/.claude"], "kein Array")
    // review quick R2 (2026-10-01, Kimi): leere und relative Eintraege sind kein Ordner — sonst verdraengte [""] den Standard.
    assert(parseConfig(["claudeConfigDirs": ["", "  ", "relativ/claude"]]).claudeConfigDirs == [home + "/.claude"],
           "nur Unbrauchbares: Standard")
    assert(parseConfig(["claudeConfigDirs": ["", " ~/.claude-work "]]).claudeConfigDirs == [home + "/.claude-work"],
           "Leerzeichen am Rand weg, Leeres faellt raus")
    assert(parseConfig(["pollSec": 0, "stackSide": "oben"]).pollSec == 3, "ungueltig: Standard")
    assert(d.leadNames.isEmpty && parseConfig(["leadNames": [" Beta · Lead ", "", 7]]).leadNames == ["Beta · Lead"],
           "Lead-Namen: Rand weg, Leeres und Unlesbares fallen raus")
    // GP 14: der Schluessel hat Vorrang vor der Mac-Sprache.
    assert(prefersGerman(language: "de", preferred: ["en-US"]) && !prefersGerman(language: "en", preferred: ["de-CH"]))
    assert(prefersGerman(language: nil, preferred: ["de-CH", "en"]) && !prefersGerman(language: nil, preferred: ["en-US", "de"]))
    assert(!prefersGerman(language: nil, preferred: []), "ohne Angabe englisch")
}

/// Code-Punkt 3: `claude agents` fuer den Standardordner ohne CLAUDE_CONFIG_DIR — sonst legt claude dort eine zweite
/// Konfigurationsdatei an. Aktiv entfernt: Lookout erbt die Umgebung seines Starters.
func testAgentsEnvironment() {
    let home = NSHomeDirectory()
    let standard = agentsEnvironment(home + "/.claude")
    assert(standard.set.isEmpty && standard.unset == ["CLAUDE_CONFIG_DIR"], "Standardordner: Variable entfernen")
    let other = agentsEnvironment(home + "/.claude-work")
    assert(other.set == ["CLAUDE_CONFIG_DIR": home + "/.claude-work"] && other.unset.isEmpty, "zweiter Ordner: setzen")
    // review quick R2 (2026-10-01): ein Verweis (Symlink) auf den Standardordner ist der Standardordner.
    let fakeHome = NSTemporaryDirectory() + "lookout-home-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: fakeHome) }
    try! FileManager.default.createDirectory(atPath: fakeHome + "/.claude", withIntermediateDirectories: true)
    try! FileManager.default.createSymbolicLink(atPath: fakeHome + "/alias", withDestinationPath: fakeHome + "/.claude")
    assert(agentsEnvironment(fakeHome + "/alias", home: fakeHome).unset == ["CLAUDE_CONFIG_DIR"], "Verweis auf ~/.claude")
    assert(agentsEnvironment(fakeHome + "/.claude", home: fakeHome).unset == ["CLAUDE_CONFIG_DIR"])
    assert(agentsEnvironment(fakeHome + "/anders", home: fakeHome).set == ["CLAUDE_CONFIG_DIR": fakeHome + "/anders"])
    setenv("LOOKOUT_TEST_VAR", "geerbt", 1)
    defer { unsetenv("LOOKOUT_TEST_VAR") }
    let env = { (set: [String: String], unset: [String]) in
        runCommand("/usr/bin/env", [], env: set, unset: unset).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }
    assert(env([:], []).contains("LOOKOUT_TEST_VAR=geerbt"), "ohne Angabe geerbt")
    assert(!env([:], ["LOOKOUT_TEST_VAR"]).contains("LOOKOUT_TEST_VAR"), "aktiv entfernt, nicht nur weggelassen")
    assert(env(["LOOKOUT_TEST_VAR": "gesetzt"], []).contains("LOOKOUT_TEST_VAR=gesetzt"))
}

/// Englische Oberflaeche: dieselben Stellen wie die deutschen Tests, nur `uiGerman` aus.
func testEnglishTexts() {
    uiGerman = false
    defer { uiGerman = true }
    let seconds: [TimeInterval] = [30, 380, 3900, 7220]
    assert(seconds.map(durationText) == ["just now", "6 min", "1 h 5 min", "2 h"])
    assert([LabelTone.waiting, .ready, .running, .watching].map(\.word) == ["WAITING FOR YOU", "DONE", "WORKING", "WATCHING"])
    assert(sessionsHeader(asking: 1, ready: 2) == "SESSIONS (1 waiting for you · 2 done)")
    assert(sessionsHeader(asking: 2, ready: 0) == "SESSIONS (2 waiting for you)")
    assert(LayoutMode.allCases.map(\.label) == ["Focus", "Focus 2", "Grid"])
    assert(WindowWhereabouts.minimized.label == "minimized (in the Dock)" && WindowWhereabouts.otherSpace.label == "other desktop")
    assert(stepLabel(tool: "Edit", input: ["file_path": "/a/b/Layout.swift"]) == "edits Layout.swift")
    assert(stepLabel(tool: "WebFetch", input: ["url": "https://docs.warp.dev/x"]) == "reads web page docs.warp.dev")
    assert(stepLabel(tool: "AskUserQuestion", input: ["questions": [["question": "Which one?"]]]) == "Question: Which one?")

    let now = Date(timeIntervalSince1970: 20_000)
    var s = Session(id: "claude:e1", engine: .claude, pid: 1, name: "alpha-e1", cwd: "/Users/test/Projects/Alpha",
                    ready: false, alive: true)
    s.digest = Digest(name: "Build queue cards", step: "Run the card tests", last: .tool)
    let w = tabWindow([s], title: "◐ Build queue cards")
    let seit = SinceEntry(tone: .running, since: now.addingTimeInterval(-22 * 60))
    assert(labelContent(for: w, since: seit, now: now).lines == ["● WORKING  for 22 min", "Build queue cards"])
    assert(sessionLine(w, since: [s.id: seit], now: now).title == "● Build queue cards — for 22 min")
    var q = Session(id: "claude:q1", engine: .claude, pid: 2, name: "q1", cwd: "", ready: true, waiting: true, alive: true)
    q.digest = Digest(name: "Release notes", step: "Question: Ship it?", question: "Ship it?")
    let frage = labelContent(for: tabWindow([q], title: "✳ Release notes"), since: nil, now: now)
    assert(frage.lines == ["? WAITING FOR YOU", "Release notes", "“Ship it?”"], "bekam \(frage.lines)")
    q.digest = Digest(name: "Release notes", step: "Upload the build")
    assert(labelContent(for: tabWindow([q], title: "✳ Release notes"), since: nil, now: now).detail
           == "Approval: Upload the build")
    var alt = q
    alt.digest = Digest(title: "Old task")
    assert(labelContent(for: tabWindow([alt, s], title: "◐ Build queue cards"), since: nil, now: now).hint
           == "A hidden tab has a question")

    let f1 = planArrangement(mode: .reading, target: .single(1), windows: elevenSessions(), screens: [oled, macbook],
                             side: .right, focusedIndex: 0)
    assert(f1.lists.first?.entries.first == .header("+4 more"), "bekam \(f1.lists)")
    let f2 = planArrangement(mode: .focus2, target: .single(1), windows: elevenSessions(), screens: [oled, macbook],
                             side: .right, focusedIndex: 0)
    assert(f2.lists.first?.entries.first == .header("WORKING (8)"), "bekam \(f2.lists)")
}

// ---------- Jeder Bildschirm (Kevin 2026-10-01 abends) ----------
//
// "Wer es runterlaedt, hat nicht unbedingt einen LG und ein MacBook": jeder Schirm wird erkannt und ist einzeln waehlbar,
// mit seinem echten Namen; ab drei Schirmen waehlt man die beiden Leitstand-Schirme im Menue.

private let dellLinks = ScreenInfo(visible: CGRect(x: 0, y: 0, width: 2560, height: 1415), builtin: false, id: "A",
                                   name: "DELL U2723QE")
private let dellRechts = ScreenInfo(visible: CGRect(x: 2560, y: 0, width: 2560, height: 1440), builtin: false, id: "B",
                                    name: "DELL U2723QE")

/// Mac mini mit zwei gleichen Monitoren: beide einzeln waehlbar, gleiche Namen bekommen (1)/(2) von links.
func testEveryScreenSelectable() {
    let mini = [dellLinks, dellRechts]
    assert(screenNames(mini) == ["DELL U2723QE (1)", "DELL U2723QE (2)"], "bekam \(screenNames(mini))")
    let opts = targetOptions(mini)
    assert(opts == [.screen("A"), .screen("B"), .both, .both2, .both3], "jeder Schirm, von links — bekam \(opts)")
    assert(targetOptions([dellRechts, dellLinks]) == opts, "Reihenfolge wie auf dem Tisch, nicht wie macOS sie liefert")
    assert(opts.compactMap { targetTitle($0, screens: mini) }
           == ["Alle auf DELL U2723QE (1)", "Alle auf DELL U2723QE (2)", "Leitstand (fertige links)",
               "Leitstand 2 (fertige rechts)", "Leitstand 3 (aktiv auf DELL U2723QE (1))"],
           "bekam \(opts.compactMap { targetTitle($0, screens: mini) })")
    assert(resolveTarget(.screen("A"), screens: mini) == .single(0) && resolveTarget(.screen("B"), screens: mini) == .single(1))
    assert(resolveTarget(.screen("weg"), screens: mini) == .single(1), "abgesteckt: der groesste vorhandene")
    assert(targetTitle(.screen("weg"), screens: mini) == nil, "nicht angeschlossen: kein Eintrag")
    // Gespeichert wird der Rohwert; die bisherigen Werte bleiben lesbar.
    assert(TargetChoice(rawValue: "screen:A") == .screen("A") && TargetChoice.screen("A").rawValue == "screen:A")
    assert(["external", "builtin", "both", "both2", "both3"].compactMap(TargetChoice.init(rawValue:))
           == [.external, .builtin, .both, .both2, .both3])
    assert(TargetChoice(rawValue: "quatsch") == nil && TargetChoice(rawValue: "screen:") == nil)
}

/// Zwei gleiche Monitore ohne Seriennummer koennen dieselbe Display-Kennung melden: dann entscheidet die Lage.
func testDuplicateScreenIDs() {
    var a = dellLinks, b = dellRechts
    a.id = "X"
    b.id = "X"
    let u = uniqueScreenIDs([b, a])                        // macOS liefert rechts zuerst
    assert(u.map(\.id) == ["X#2", "X"], "links behaelt die Kennung, rechts bekommt #2 — bekam \(u.map(\.id))")
    assert(targetOptions(u) == [.screen("X"), .screen("X#2"), .both, .both2, .both3], "zwei waehlbare Eintraege")
    assert(resolveTarget(.screen("X#2"), screens: u) == .single(0), "der rechte")
    // review quick (Codex): wird der linke abgesteckt, heisst der rechte wieder "X" — die Wahl "X#2" findet ihn trotzdem.
    let mb = ScreenInfo(visible: CGRect(x: 0, y: -982, width: 3000, height: 2000), builtin: true, id: "M", name: "MacBook")
    let nurRechts = uniqueScreenIDs([b, mb])
    assert(resolveTarget(.screen("X#2"), screens: nurRechts) == .single(0), "Zwilling statt groesster Schirm")
    assert(uniqueScreenIDs([dellLinks, dellRechts]).map(\.id) == ["A", "B"], "verschiedene Kennungen bleiben")
    var leer = dellRechts
    leer.id = ""
    assert(uniqueScreenIDs([dellLinks, leer, leer]).map(\.id) == ["A", "", ""], "unbekannt bleibt unbekannt")
}

/// "MacBook" stand fest im Menue — auf einem iMac falsch, am Mac mini hiess Leitstand 3 "aktiv aufs MacBook" ohne MacBook.
func testScreenNamesNotFixed() {
    let lg = ScreenInfo(visible: oled, builtin: false, id: "LG", name: "LG ULTRAGEAR+")
    let mb = ScreenInfo(visible: macbook, builtin: true, id: "MB", name: "Built-in Retina Display")
    let kevin = [lg, mb]
    assert(targetOptions(kevin).compactMap { targetTitle($0, screens: kevin) }
           == ["Alle auf LG ULTRAGEAR+", "Alle aufs MacBook", "Leitstand (fertige links)", "Leitstand 2 (fertige rechts)",
               "Leitstand 3 (aktiv aufs MacBook)"], "Kevins Menue bleibt wortgleich")
    var imac = ScreenInfo(visible: CGRect(x: 0, y: 0, width: 2240, height: 1235), builtin: true, id: "I", name: "iMac")
    imac.laptop = false
    assert(targetOptions([imac]).compactMap { targetTitle($0, screens: [imac]) } == ["Alle auf den iMac"],
           "iMac allein: kein MacBook, kein Leitstand")
    assert(targetTitle(.both3, screens: [dellRechts, imac]) == "Leitstand 3 (aktiv auf den iMac)")
    assert(targetTitle(.external, screens: kevin) == nil && targetTitle(.builtin, screens: kevin) == nil,
           "Altwerte stehen nicht mehr als eigene Eintraege im Menue")
    uiGerman = false
    defer { uiGerman = true }
    assert(targetOptions(kevin).compactMap { targetTitle($0, screens: kevin) }
           == ["All on LG ULTRAGEAR+", "All on the MacBook", "Control room (done on the left)",
               "Control room 2 (done on the right)", "Control room 3 (active on the MacBook)"])
}

/// Drei Schirme (MacBook + 2 Monitore): automatisch wie bisher der groesste und der kleinste; im Menue waehlbar.
func testLeitstandScreensChoice() {
    let hp = ScreenInfo(visible: CGRect(x: 2560, y: 0, width: 1920, height: 1080), builtin: false, id: "H", name: "HP 24")
    let mb = ScreenInfo(visible: CGRect(x: 600, y: -982, width: 1512, height: 944), builtin: true, id: "M", name: "MacBook")
    let three = [dellLinks, hp, mb]
    assert(resolveTarget(.both, screens: three) == .leitstand(work: 0, control: 2), "automatisch: groesster + kleinster")
    let pair = (work: "H", control: "A")
    assert(resolveTarget(.both, screens: three, pair: pair) == .leitstand(work: 1, control: 0), "gewaehlt")
    assert(resolveTarget(.both2, screens: three, pair: pair) == .leitstand(work: 1, control: 0, readySide: .right))
    assert(resolveTarget(.both3, screens: three, pair: pair) == .leitstand(work: 1, control: 0, flipped: true))
    assert(targetTitle(.both3, screens: three, pair: pair) == "Leitstand 3 (aktiv auf HP 24)")
    assert(resolveTarget(.both, screens: [dellLinks, mb, dellRechts], pair: pair) == .leitstand(work: 2, control: 1),
           "gewaehlter Schirm fehlt: automatisch")
    assert(resolveTarget(.both, screens: [dellLinks, hp], pair: pair) == .leitstand(work: 0, control: 1),
           "zwei Schirme: automatisch, die Wahl gilt erst ab drei")
    assert(resolveTarget(.screen("M"), screens: three, pair: pair) == .single(2), "Einzelziel unberuehrt")
    // Derselbe Schirm fuer beide Rollen tauscht sie.
    assert(choosePair(current: (work: "A", control: "M"), work: "H", screens: three) == (work: "H", control: "M"))
    assert(choosePair(current: (work: "A", control: "M"), work: "M", screens: three) == (work: "M", control: "A"), "tauschen")
    assert(choosePair(current: (work: "A", control: "M"), control: "A", screens: three) == (work: "M", control: "A"),
           "tauschen")
}

/// Menue-Haken und Anordnung loesen das gewaehlte Paar gleich auf: fehlt einer von zwei gleichen Monitoren, nutzte die
/// Anordnung ueber den Zwilling das Paar, das Menue zeigte aber "Automatisch" (review quick 2026-10-01, Rest aus Runde 1).
func testPairSharedResolution() {
    var links = dellLinks, rechts = dellRechts
    links.id = "X"
    rechts.id = "X"
    let hp = ScreenInfo(visible: CGRect(x: 5120, y: 0, width: 1920, height: 1080), builtin: false, id: "H", name: "HP 24")
    let mb = ScreenInfo(visible: CGRect(x: 600, y: -982, width: 1512, height: 944), builtin: true, id: "M", name: "MacBook")
    let pair = (work: "X#2", control: "H")
    let vier = uniqueScreenIDs([links, rechts, hp, mb])            // X, X#2, H, M
    assert(activePair(pair, screens: vier).map { [$0.work, $0.control] } == [1, 2], "alle da: das gewaehlte Paar")
    // Der linke ist abgesteckt, der rechte heisst wieder "X": Anordnung und Haken nutzen beide das Paar.
    let drei = uniqueScreenIDs([rechts, hp, mb])                   // X, H, M
    let p = activePair(pair, screens: drei)
    assert(p.map { [$0.work, $0.control] } == [0, 1], "Zwilling gilt auch im Menue — bekam \(String(describing: p))")
    assert(resolveTarget(.both, screens: drei, pair: pair) == .leitstand(work: 0, control: 1), "Anordnung nutzt das Paar")
    assert(resolveTarget(.both, screens: drei) == .leitstand(work: 0, control: 2), "automatisch waere ein anderes")
    // Sonst automatisch: zwei Schirme, leere Kennung, beide Rollen auf demselben Schirm, nichts gewaehlt.
    assert(activePair(pair, screens: [rechts, mb]) == nil, "zwei Schirme")
    assert(activePair((work: "", control: "H"), screens: vier) == nil, "leere Kennung zaehlt nicht")
    assert(activePair((work: "X#2", control: "X"), screens: drei) == nil, "beide Rollen auf dem einen X")
    assert(activePair(nil, screens: vier) == nil)
}

/// Gleiche Monitore nach dem Abstecken eines Nachbarn: die Wahl bleibt beim richtigen Schirm, auch wenn sie im Menue
/// neu gesetzt wird (review quick 2026-10-01 Runde 2, GLM).
func testTwinChoiceSurvivesReplug() {
    func dell(_ x: CGFloat) -> ScreenInfo {
        ScreenInfo(visible: CGRect(x: x, y: 0, width: 2560, height: 1440), builtin: false, id: "X", name: "DELL U2723QE")
    }
    let links = dell(0), mitte = dell(2560), rechts = dell(5120)
    // Drei gleiche, der rechte (X#3) gewaehlt, der linke abgesteckt: der rechte heisst jetzt X#2, nicht der mittlere zaehlt.
    assert(resolveTarget(.screen("X#3"), screens: uniqueScreenIDs([mitte, rechts])) == .single(1), "der rechte")
    assert(resolveTarget(.screen("X#3"), screens: uniqueScreenIDs([links, rechts])) == .single(1), "mittlerer weg (Kimi)")
    assert(resolveTarget(.screen("X#3"), screens: uniqueScreenIDs([links, mitte])) == .single(1), "rechter weg: Nachbar")
    // Paar Arbeit = rechter von zwei gleichen (X#2), Leitstand HP. Linker ab, nur den Leitstand aufs MacBook gelegt:
    // X#2 bleibt gespeichert — sonst stuende "X" da und meinte nach dem Wiederanstecken den linken.
    let hp = ScreenInfo(visible: CGRect(x: 5120, y: 0, width: 1920, height: 1080), builtin: false, id: "H", name: "HP 24")
    let mb = ScreenInfo(visible: CGRect(x: 600, y: -982, width: 1512, height: 944), builtin: true, id: "M", name: "MacBook")
    let ohneLinks = uniqueScreenIDs([mitte, hp, mb])             // der rechte der beiden heisst jetzt "X"
    let neu = choosePair(current: (work: "X#2", control: "H"), control: "M", screens: ohneLinks)
    assert(neu == (work: "X#2", control: "M"), "bekam \(neu)")
    assert(resolveTarget(.both, screens: uniqueScreenIDs([links, mitte, hp, mb]), pair: neu)
           == .leitstand(work: 1, control: 3), "wieder angesteckt: Arbeit bleibt der rechte")
    // Derselbe Schirm unter dem kurzen Namen: tauschen bzw. nichts aendern, die genauere Kennung bleibt.
    assert(choosePair(current: (work: "X#2", control: "H"), control: "X", screens: ohneLinks) == (work: "H", control: "X#2"))
    assert(choosePair(current: (work: "X#2", control: "H"), work: "X", screens: ohneLinks) == (work: "X#2", control: "H"))
    assert(sameScreen("X", "X#2", screens: ohneLinks) && !sameScreen("X", "H", screens: ohneLinks))
    assert(!sameScreen("weg", "auch-weg", screens: ohneLinks) && sameScreen("weg", "weg", screens: ohneLinks))
    // Kaputte gespeicherte Kennung mit riesiger Nummer (von Hand in den Einstellungen): sofort beantwortet, nicht
    // Millionen Runden je Takt.
    let t0 = Date()
    assert(screenIndex(id: "X#10000000", screens: ohneLinks) == 0, "der verbliebene Zwilling")
    assert(screenIndex(id: "X#9223372036854775807", screens: ohneLinks) == 0, "groesste Zahl (review quick R3, Codex)")
    assert(screenIndex(id: "X#99999999999999999999", screens: ohneLinks) == nil, "zu gross fuer eine Zahl: nichts")
    assert(Date().timeIntervalSince(t0) < 0.1, "dauerte \(Date().timeIntervalSince(t0)) s")
}

// ---------- Lauf ----------

@main
enum LayoutTestMain {
    static func main() {
        print("Lookout — Tests")
        // Die Tests sprechen Deutsch und reden Kevin an — unabhaengig von Sprache und Namen des Macs, auf dem sie laufen.
        uiGerman = true
        greetingName = "Kevin"
        test("testGreetingName", testGreetingName)
        test("testParseConfig", testParseConfig)
        test("testAgentsEnvironment", testAgentsEnvironment)
        test("testEnglishTexts", testEnglishTexts)
        test("testEveryScreenSelectable", testEveryScreenSelectable)
        test("testDuplicateScreenIDs", testDuplicateScreenIDs)
        test("testScreenNamesNotFixed", testScreenNamesNotFixed)
        test("testLeitstandScreensChoice", testLeitstandScreensChoice)
        test("testPairSharedResolution", testPairSharedResolution)
        test("testTwinChoiceSurvivesReplug", testTwinChoiceSurvivesReplug)
        test("testParseAgentsJson", testParseAgentsJson)
        test("testParseAgentsJsonRobust", testParseAgentsJsonRobust)
        test("testTurnStateFromTail", testTurnStateFromTail)
        test("testLogReadyClassification", testLogReadyClassification)
        test("testAttachProcesses", testAttachProcesses)
        test("testParsePsLine", testParsePsLine)
        test("testWindowlessSessions", testWindowlessSessions)
        test("testCoordinateRoundTrip", testCoordinateRoundTrip)
        test("testFramesDiffer", testFramesDiffer)
        test("testGrid22", testGrid22)
        test("testGrid22OverflowReservesRail", testGrid22OverflowReservesRail)
        test("testFocusFills", testFocusFills)
        test("testRailOverflowCap", testRailOverflowCap)
        test("testFocusLeftSide", testFocusLeftSide)
        test("testAttentionOrderReadyFirst", testAttentionOrderReadyFirst)
        test("testAttentionOrderReadyQueue", testAttentionOrderReadyQueue)
        test("testWaitingBeatsEverything", testWaitingBeatsEverything)
        test("testWaitingFromPermissionEvents", testWaitingFromPermissionEvents)
        test("testGridShrinksToWindowCount", testGridShrinksToWindowCount)
        test("testScreenIndexByOverlap", testScreenIndexByOverlap)
        test("testAutoGridPerScreen", testAutoGridPerScreen)
        test("testReadingWidthOnUltrawide", testReadingWidthOnUltrawide)
        test("testNoGlowOnMaximizedWindow", testNoGlowOnMaximizedWindow)
        test("testNoGlowOnActiveWindow", testNoGlowOnActiveWindow)
        test("testGlowKnowsItsWindow", testGlowKnowsItsWindow)
        test("testWindowWhereabouts", testWindowWhereabouts)
        test("testResolveTarget", testResolveTarget)
        test("testTargetMovesAllToOneScreen", testTargetMovesAllToOneScreen)
        test("testFocusMainIsActiveSession", testFocusMainIsActiveSession)
        test("testFocus2SidesByStatus", testFocus2SidesByStatus)
        test("testFocus2FollowsStatus", testFocus2FollowsStatus)
        test("testFocus2Narrow", testFocus2Narrow)
        test("testGridChoicesFitScreen", testGridChoicesFitScreen)
        test("testFixedGridKeepsCellSize", testFixedGridKeepsCellSize)
        test("testGridIgnoresClicks", testGridIgnoresClicks)
        test("testExcludedWindowsStayPut", testExcludedWindowsStayPut)
        test("testNoEndlessRetry", testNoEndlessRetry)
        test("testFrameWritesStayInside", testFrameWritesStayInside)
        test("testGlidePath", testGlidePath)
        test("testPingSchedule", testPingSchedule)
        test("testOverlayPanelsDoNotAnimate", testOverlayPanelsDoNotAnimate)
        test("testLeitstand", testLeitstand)
        test("testLeitstand2", testLeitstand2)
        test("testLeitstand3", testLeitstand3)
        test("testClickSwapsOnlyTwoWindows", testClickSwapsOnlyTwoWindows)
        test("testLegacyLayoutNamesStillWork", testLegacyLayoutNamesStillWork)
        test("testTitleGlyph", testTitleGlyph)
        test("testDigestTranscript", testDigestTranscript)
        test("testStepLabel", testStepLabel)
        test("testVisibleTabDecides", testVisibleTabDecides)
        test("testVisibleTabFallbacks", testVisibleTabFallbacks)
        test("testVisibleTabRobust", testVisibleTabRobust)
        test("testHiddenQuestion", testHiddenQuestion)
        test("testTranscriptReader", testTranscriptReader)
        test("testAdvanceSince", testAdvanceSince)
        test("testDurationText", testDurationText)
        test("testFirstSentence", testFirstSentence)
        test("testLabelContent", testLabelContent)
        test("testLabelTargets", testLabelTargets)
        test("testSessionsHeader", testSessionsHeader)
        test("testDigestCustomTitle", testDigestCustomTitle)
        test("testNameOnLabelAndTab", testNameOnLabelAndTab)
        test("testTitleMatchesWindows", testTitleMatchesWindows)
        test("testTranscriptTitlesFarBack", testTranscriptTitlesFarBack)
        test("testMoveGate", testMoveGate)
        test("testFocusSourceAtStart", testFocusSourceAtStart)
        test("testProcessListFailureIsUnknown", testProcessListFailureIsUnknown)
        test("testOverlayPanelsStayWhereTold", testOverlayPanelsStayWhereTold)
        test("testGlowFollowsGlidingWindow", testGlowFollowsGlidingWindow)
        test("testSnapshotDuringOwnMove", testSnapshotDuringOwnMove)
        test("testLogSessionsFollowLiveProcesses", testLogSessionsFollowLiveProcesses)
        test("testLogWithDateAndRotation", testLogWithDateAndRotation)
        test("testProcessTreeInTerminal", testProcessTreeInTerminal)
        test("testForeignTerminalLeftAlone", testForeignTerminalLeftAlone)
        test("testTurnEnded", testTurnEnded)
        test("testBackgroundActivity", testBackgroundActivity)
        test("testListeningPids", testListeningPids)
        test("testWatcherNeedsKevin", testWatcherNeedsKevin)
        test("testLeadReportsOnOtherWindows", testLeadReportsOnOtherWindows)
        test("testWatcherAnswerSeen", testWatcherAnswerSeen)
        test("testSessionInFront", testSessionInFront)
        test("testOpensWithNothingNeeded", testOpensWithNothingNeeded)
        test("testLaunchPromptIsNotKevins", testLaunchPromptIsNotKevins)
        test("testWatchingLabel", testWatchingLabel)
        test("testDigestPeerAndWakeups", testDigestPeerAndWakeups)
        test("testNothingNeeded", testNothingNeeded)
        test("testStandby", testStandby)
        test("testLookoutLine", testLookoutLine)
        test("testStatusBadge", testStatusBadge)
        test("testRailSplit", testRailSplit)
        test("testMacBookFocus1ThreeColumns", testMacBookFocus1ThreeColumns)
        test("testMacBookFocus2OneRail", testMacBookFocus2OneRail)
        test("testSevenWaitingThreeRunning", testSevenWaitingThreeRunning)
        test("testFocus2WaitingFirst", testFocus2WaitingFirst)
        test("testFocus1OnLGByStatus", testFocus1OnLGByStatus)
        test("testClickAcrossGroups", testClickAcrossGroups)
        test("testNarrowScreenKeepsOneRail", testNarrowScreenKeepsOneRail)
        test("testParkedWindowsHaveNoOverlay", testParkedWindowsHaveNoOverlay)
        test("testListFindsMainWindowNumber", testListFindsMainWindowNumber)
        test("testListPanelTakesClicks", testListPanelTakesClicks)
        test("testReadSessionFiles", testReadSessionFiles)
        test("testClaudeStatusAsksRarely", testClaudeStatusAsksRarely)
        test("testClaudeStatusDistrustsWrongStates", testClaudeStatusDistrustsWrongStates)
        test("testBackgroundAgentWithPid", testBackgroundAgentWithPid)
        test("testProcessStartDate", testProcessStartDate)
        test("testClaudeStatusChecksBothWays", testClaudeStatusChecksBothWays)
        test("testAgentsReply", testAgentsReply)
        test("testSitsAbove", testSitsAbove)
        // Ein Filter ohne Treffer (Tippfehler) waere sonst "ALL TESTS PASSED (0)" (review quick 2026-09-26, Kimi).
        if let only, ran == 0 {
            print("KEIN TEST passt zu '\(only)'")
            exit(1)
        }
        print("ALL TESTS PASSED (\(ran))")
    }
}
