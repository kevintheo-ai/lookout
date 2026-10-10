// Lookout — Session-Watcher: welche KI-Session laeuft, und ist sie fertig?
//
// Claude liefert den Status offiziell (`claude agents --json`). Spike-Befund 2026-09-18:
// Sessions einer ZWEITEN Claude-Konfiguration tauchen dort nur auf, wenn
// CLAUDE_CONFIG_DIR gesetzt ist — darum wird je Config-Dir einmal gefragt (`claudeConfigDirs`).
// Seit 2026-09-27 liest Lookout die Status-Dateien, aus denen `claude agents` seine interaktiven Sessions bezieht,
// direkt (`ClaudeStatus`) — die Abfrage selbst laeuft nur noch alle 5 Min. (Absturz-Diagnose: 13,4 Starts je Minute).

import Foundation

enum Engine: String {
    case claude, codex, grok, kimi
}

struct Session {
    let id: String          // engine + ":" + sessionId — stabil ueber Ticks
    let engine: Engine
    let pid: pid_t?         // nur interaktive Sessions; Hintergrund-Agenten haben keine
    let name: String
    let cwd: String
    var ready: Bool         // arbeitet gerade NICHT — Zug beendet oder wartet auf Kevin
    /// Die Session hat eine FRAGE gestellt und kommt ohne Antwort nicht weiter.
    /// Kevin 2026-09-18: "das hat prio 1" — steht in Fokus 2 links ganz oben.
    let waiting: Bool
    let alive: Bool
    /// Start (nur aus `claude agents --json`) — trennt frische von wochenalten Hintergrund-Agenten.
    let startedAt: Date?
    /// Was Claudes Transkript ueber die Session verraet (Thema, Schritt, Antwort) — nur Claude.
    var digest: Digest? = nil
    /// Antwort fertig, wartet aber auf den eigenen Hintergrund-Aufpasser (CI, Worker) oder nur auf eine andere Session bzw.
    /// einen eigenen Wecker (`Standby`) — braucht Kevin nicht (Kevin 2026-09-27: "er überwacht jetzt nur die Sessions").
    /// `ready` bleibt dabei aus.
    var watching = false
    /// Claude meldet `shell`: Zug vorbei, nur noch Hintergrund-Befehle laufen. `busy` heisst dagegen immer, dass das
    /// Modell arbeitet (CLI 2.1.294/2.1.295, gemessen 2026-10-09) — auch wenn das Transkript noch das Ende des letzten
    /// Zuges zeigt, weil eine Nachricht einer anderen Session oder ein Wecker den neuen Zug ohne Prompt-Zeile begann.
    let backgroundOnly: Bool

    init(id: String, engine: Engine, pid: pid_t?, name: String, cwd: String,
         ready: Bool, waiting: Bool = false, alive: Bool, startedAt: Date? = nil, backgroundOnly: Bool = false) {
        self.id = id; self.engine = engine; self.pid = pid; self.name = name
        self.cwd = cwd; self.ready = ready; self.waiting = waiting; self.alive = alive
        self.startedAt = startedAt; self.backgroundOnly = backgroundOnly
    }
}

/// "Ohne Fenster" im Menue (Kevin 2026-09-18: "Nur Echtes"): laufende Sessions ohne sichtbares
/// Fenster (z. B. Hintergrund-Tabs) und Hintergrund-Agenten der letzten 24 h. Beendete
/// Codex-/Kimi-/Grok-Laeufe sind in `live` gar nicht mehr enthalten (`attachProcesses`);
/// aeltere Agenten (im Bestand: fuenf, seit Juli blockiert) zaehlen nur noch mit.
func windowlessSessions(_ live: [Session], inWindows: Set<String>, now: Date,
                        maxAgentAge: TimeInterval = 86_400) -> (shown: [Session], staleAgents: Int) {
    var shown: [Session] = []
    var stale = 0
    for s in live where !inWindows.contains(s.id) {
        // Ohne pid = Hintergrund-Agent: kein Terminal, also nie ein Fenster.
        if s.pid == nil, !(s.startedAt.map { now.timeIntervalSince($0) <= maxAgentAge } ?? false) {
            stale += 1
        } else {
            shown.append(s)
        }
    }
    return (shown, stale)
}

private let CLAUDE_BIN_CANDIDATES = [
    NSHomeDirectory() + "/.local/bin/claude",
    "/opt/homebrew/bin/claude",
    "/usr/local/bin/claude",
]

func claudeBinary() -> String? {
    CLAUDE_BIN_CANDIDATES.first { FileManager.default.isExecutableFile(atPath: $0) }
}

/// Wandelt die Ausgabe von `claude agents --json` in Sessions.
func parseAgentsJson(_ data: Data) -> [Session] {
    guard let arr = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] else { return [] }
    return arr.compactMap(parseAgent)
}

/// Ein Eintrag aus `claude agents --json` oder eine Status-Datei (`readSessionFiles`) — dieselben Felder.
/// Eintraege ohne `sessionId` werden verworfen — lieber eine Session weniger als eine geratene.
func parseAgent(_ obj: [String: Any]) -> Session? {
    guard let sid = obj["sessionId"] as? String, !sid.isEmpty else { return nil }
    let kind = obj["kind"] as? String ?? ""
    let ready: Bool
    let waiting: Bool
    let status = obj["status"] as? String ?? ""
    if kind == "background" {
        // "blocked" = der Agent haengt an einer Freigabe/Rueckfrage -> Kevin ist dran.
        waiting = (obj["state"] as? String) == "blocked"
        ready = waiting
    } else {
        // Live gemessen 2026-09-18 an einer Session mit offener Rueckfrage: interaktive
        // Sessions melden dafuer einen EIGENEN Status `waiting` — vorher nie beobachtet,
        // weil in den Stichproben zufaellig nie eine Frage offen war. `idle` = Zug
        // beendet, `waiting` = wartet auf Kevins Antwort. Beides heisst: arbeitet nicht.
        waiting = status == "waiting"
        ready = status == "idle" || waiting
    }
    // Hintergrund-Agenten haben kein Terminal und kein Fenster: sie laufen ohne pid, auch falls `claude agents` eine
    // meldet (review quick 2026-09-27, Kimi) — sonst hielte `ClaudeStatus` ihn fuer eine in den Dateien fehlende
    // Session (Dauer-Rueckfall) und `attachProcesses` wuerfe ihn als "Prozess ohne Terminal" weg.
    let pid = kind == "background" ? nil : (obj["pid"] as? Int).map { pid_t($0) }
    return Session(id: "\(Engine.claude.rawValue):\(sid)",
                   engine: .claude,
                   pid: pid,
                   name: obj["name"] as? String ?? sid,
                   cwd: obj["cwd"] as? String ?? "",
                   ready: ready,
                   waiting: waiting,
                   alive: true,
                   startedAt: (obj["startedAt"] as? Double).map { Date(timeIntervalSince1970: $0 / 1000) },
                   backgroundOnly: kind != "background" && status == "shell")
}

/// Status-Dateien eines Config-Dirs: `<dir>/sessions/<pid>.json`, eine je laufender Session, von Claude selbst geschrieben
/// (Felder wie bei `claude agents --json`: `status` busy/shell/idle/waiting). `claude agents` listet interaktive Sessions
/// genau aus diesen Dateien — 27.09.2026 geprueft: 5/5 deckungsgleich, `waiting` bei offener Rueckfrage sofort, auch
/// `claude -p`-Laeufe stehen in beiden. Hintergrund-Agenten stehen NICHT darin.
/// Nil = Ordner fehlt, oder Dateien da, aber keine lesbar (Format geaendert?) — dann gilt die alte Abfrage.
func readSessionFiles(_ dir: String) -> [Session]? {
    let folder = dir + "/sessions"
    guard let names = try? FileManager.default.contentsOfDirectory(atPath: folder) else { return nil }
    let files = names.filter { $0.hasSuffix(".json") }
    let parse = { (name: String) -> Session? in
        guard let data = FileManager.default.contents(atPath: folder + "/" + name),
              let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              obj["pid"] is Int else { return nil }
        return parseAgent(obj)
    }
    var parsed = 0
    let sessions = files.compactMap { name -> Session? in
        // Claude schreibt die Datei an Ort und Stelle neu (gleiche Inode, gemessen 27.09.): wer genau zwischen Leeren und
        // Schreiben liest, sieht nichts Ganzes — ein zweiter Versuch Millisekunden spaeter sieht die fertige Datei.
        guard let s = parse(name) ?? { Thread.sleep(forTimeInterval: 0.01); return parse(name) }() else { return nil }
        parsed += 1
        // Liegengebliebene Datei (Session abgestuerzt, pid inzwischen an einen neueren Prozess vergeben — auch einen
        // root-Prozess, `processStartDate`): der Prozess ist juenger als der Eintrag. 2 s Luft fuer die Uhren.
        if let pid = s.pid, let started = s.startedAt, let proc = processStartDate(pid),
           proc.timeIntervalSince(started) > 2 { return nil }
        return s
    }
    return files.isEmpty || parsed > 0 ? sessions : nil
}

// ---------- Claude-Transkript: was tut die Session gerade? (Design Schilder 2026-09-21) ----------
//
// Claude schreibt je Session ein JSONL-Transkript: `ai-title` (das Thema, laufend neu),
// `assistant` (Antworttext, Werkzeugschritte, Denkbloecke), `user` (Kevins Prompt oder
// Werkzeug-Ergebnisse). Gelesen wird nur das Dateiende; der Zugbeginn liegt bei langen Zuegen
// davor — dann bleibt `promptAt` leer.

enum LastBlock: Equatable { case none, text, tool, thinking }

struct Digest: Equatable {
    var title: String?
    /// Eigener Name ("custom-title": Namens-Hook oder /rename, v4 2026-09-21) — geht dem Auto-Titel vor.
    var name: String?
    /// Letzter Werkzeugschritt in lesbarer Form (`stepLabel`).
    var step: String?
    /// Letzter Antworttext, roh.
    var text: String?
    /// Was zuletzt kam — bestimmt, was "jetzt" heisst.
    var last: LastBlock = .none
    var answerAt: Date?
    var promptAt: Date?
    /// Offene AskUserQuestion-Frage; faellt weg mit IHRER Antwort (tool_result zur Frage) oder
    /// einem neuen Prompt — nicht mit dem Ergebnis eines parallel gestarteten Werkzeugs.
    var question: String?
    var questionId: String?
    /// Antwort fertig (Plan 1e): `system`/`turn_duration` kam — er folgt auf die letzte Antwort, erst nach den
    /// Stop-Hooks. Jeder neue Prompt (auch eine Hintergrund-Meldung) und jede spaetere Antwort heben es auf.
    var turnEnded = false
    /// Kevins eigener Prompt hat den laufenden bzw. letzten Zug ausgeloest (Kevin 2026-10-01) — dann ist er dran, die
    /// Session gilt nie als ueberwacht. Nachrichten anderer Sessions, Hintergrund-Meldungen und Wecker setzen es zurueck.
    /// Nil = im gelesenen Ausschnitt kein Ausloeser; `TranscriptReader` behaelt dann den zuletzt gesehenen (review quick
    /// 2026-10-01, Codex: nach einem langen Zug faellt Kevins Prompt aus dem Dateiende).
    var kevinTurn: Bool?
    /// Text und Zeitpunkt des ersten Prompts der Session, wenn er Kevins Zug ausloeste (sonst nil, auch bei seiner Nachricht
    /// mitten im Zug): steht er in den Startargumenten und kam kurz nach dem Prozessstart, war es ein Start-Prompt, keine
    /// Nachricht von Kevin (`launchCorrected`, Kevin 2026-10-01 abends).
    var kevinPrompt: String?
    var kevinPromptAt: Date?
    /// Andere laufende Session im Gespraech mit dieser (Kevin 2026-09-27): im gelesenen Ausschnitt kam eine Nachricht von
    /// ihr (`peerPid` aus `verifiedPeerPid`) oder diese Session schrieb ihr (`SendMessage`, nur der Name). Kevins Prompt
    /// beendet das Gespraech seit R11 nicht mehr (Kevin 2026-10-05).
    var peerPid: pid_t?
    var peerName: String?
    /// Wann die andere Session zuletzt per pid belegt wurde: eine Session, die erst danach startete, traegt nur ihre
    /// wiedervergebene pid (`Standby`, review quick 2026-10-10, Codex).
    var peerAt: Date?
    /// Selbst gestellte Wecker, die noch ausstehen: "loop" (`ScheduleWakeup`) bzw. Cron-ID (`CronCreate`) -> Zeitpunkt,
    /// nil = wiederkehrend.
    var wakeups: [String: Date?] = [:]
}

private let isoFractional: ISO8601DateFormatter = {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return f
}()
private let isoPlain = ISO8601DateFormatter()

private func transcriptDate(_ obj: [String: Any]) -> Date? {
    guard let s = obj["timestamp"] as? String else { return nil }
    return isoFractional.date(from: s) ?? isoPlain.date(from: s)
}

private func firstQuestion(_ input: [String: Any]) -> String? {
    (input["questions"] as? [[String: Any]])?.first?["question"] as? String
}

/// Werkzeugschritt in lesbarer Form. Haeufigste Werkzeuge (Stichprobe 2026-09-21, 25 Transkripte):
/// Bash, Read, Edit, Write, AskUserQuestion, WebFetch, Agent, Skill.
func stepLabel(tool: String, input: [String: Any]) -> String {
    func str(_ k: String) -> String { input[k] as? String ?? "" }
    let file = (str("file_path") as NSString).lastPathComponent
    switch tool {
    case "Bash":
        let d = str("description")
        return d.isEmpty ? String(str("command").split(separator: "\n").first ?? "Bash") : d
    case "Read": return tr("liest ", "reads ") + file
    case "Edit", "NotebookEdit": return tr("ändert ", "edits ") + file
    case "Write": return tr("schreibt ", "writes ") + file
    case "Agent", "Task": return "Agent: \(str("description"))"
    case "Skill": return "Skill \(str("skill"))"
    case "WebSearch": return tr("sucht im Web: ", "searches the web: ") + str("query")
    case "WebFetch": return tr("liest Webseite ", "reads web page ") + (URLComponents(string: str("url"))?.host ?? "")
    case "AskUserQuestion": return tr("Frage: ", "Question: ") + (firstQuestion(input) ?? "")
    default:
        // mcp__<server>__<werkzeug> -> "<server>: <werkzeug>"
        let parts = tool.components(separatedBy: "__")
        return parts.count == 3 && parts[0] == "mcp" ? "\(parts[1]): \(parts[2])" : tool
    }
}

/// Kevins Prompt — nicht ein Werkzeug-Ergebnis, keine System-Einblendung (`isMeta`) und nicht die Zusammenfassung nach
/// einer Verdichtung (`isCompactSummary`, Kevin 2026-10-10: zwei wartende Fenster standen nach /compact FERTIG).
private func isPrompt(_ obj: [String: Any]) -> Bool {
    guard obj["isMeta"] as? Bool != true, obj["isCompactSummary"] as? Bool != true,
          let msg = obj["message"] as? [String: Any] else { return false }
    if msg["content"] is String { return true }
    guard let blocks = msg["content"] as? [[String: Any]] else { return false }
    return blocks.contains { $0["type"] as? String == "text" }
        && !blocks.contains { $0["type"] as? String == "tool_result" }
}

/// Text eines Prompts: die Zeichenkette bzw. die Text-Bloecke.
private func promptText(_ obj: [String: Any]) -> String? {
    let content = (obj["message"] as? [String: Any])?["content"]
    if let s = content as? String { return s }
    let texts = (content as? [[String: Any]] ?? []).compactMap { $0["type"] as? String == "text" ? $0["text"] as? String : nil }
    return texts.isEmpty ? nil : texts.joined(separator: "\n")
}

/// Wertet ein Transkript-Stueck aus. Zeilen NUR an "\n" trennen — Pythons splitlines() trennte im
/// Spike auch an U+2028 mitten im JSON. Die erste Zeile eines Dateiendes ist meist angeschnitten
/// und faellt beim Parsen einfach heraus.
func digestTranscript(_ text: String) -> Digest {
    var d = Digest()
    /// Werkzeugaufrufe, deren Ergebnis ueber Weck-Quellen entscheidet (id -> Name, Eingabe).
    var pending: [String: (name: String, input: [String: Any])] = [:]
    for line in text.split(separator: "\n", omittingEmptySubsequences: true) {
        guard let data = line.data(using: .utf8),
              let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { continue }
        switch obj["type"] as? String {
        case "ai-title":
            if let t = obj["aiTitle"] as? String, !t.isEmpty { d.title = t }
        case "custom-title":
            if let t = obj["customTitle"] as? String, !t.isEmpty { d.name = t }
        case "user":
            // review quick 2026-09-21 (Kimi): bei 1 von 211 Fragen kam zuerst das Ergebnis eines
            // parallel gestarteten Werkzeugs — das darf die Frage nicht schliessen.
            let results = (obj["message"] as? [String: Any])?["content"] as? [[String: Any]] ?? []
            let answered = d.questionId.map { id in results.contains { $0["tool_use_id"] as? String == id } } ?? false
            if answered || isPrompt(obj) {
                d.question = nil
                d.questionId = nil
            }
            if isPrompt(obj) {
                d.promptAt = transcriptDate(obj) ?? d.promptAt
                d.turnEnded = false
            }
            noteWakeSources(obj, results: results, pending: &pending, into: &d)
        case "system":
            if obj["subtype"] as? String == "turn_duration" { d.turnEnded = true }
            // Der Weck-Prompt danach ist eine Systemeinblendung (isMeta, ohne origin) — der Zug ist nicht Kevins (review
            // quick 2026-10-01, GLM).
            if obj["subtype"] as? String == "scheduled_task_fire" {
                noteFire(obj, into: &d)
                d.kevinTurn = false
            }
        case "attachment":
            // Was Kevin mitten in einen laufenden Zug schreibt, kommt als Anhang (CLI 2.1.283+, 106-mal seit 28.09.):
            // die Antwort gilt dann auch ihm.
            let a = obj["attachment"] as? [String: Any]
            if a?["commandMode"] as? String == "prompt", (a?["origin"] as? [String: Any])?["kind"] as? String == "human" {
                d.kevinTurn = true
                d.kevinPrompt = nil                // mitten im Zug ist immer getippt, nie ein Start-Prompt
                d.kevinPromptAt = nil
            }
        case "assistant":
            d.answerAt = transcriptDate(obj) ?? d.answerAt
            d.turnEnded = false
            let blocks = (obj["message"] as? [String: Any])?["content"] as? [[String: Any]] ?? []
            for b in blocks {
                switch b["type"] as? String {
                case "text":
                    guard let t = b["text"] as? String,
                          !t.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
                    d.text = t
                    d.last = .text
                case "tool_use":
                    let name = b["name"] as? String ?? ""
                    let input = b["input"] as? [String: Any] ?? [:]
                    d.step = stepLabel(tool: name, input: input)
                    d.last = .tool
                    if name == "AskUserQuestion" {
                        d.question = firstQuestion(input)
                        d.questionId = b["id"] as? String
                    }
                    if WAKE_TOOLS.contains(name), let id = b["id"] as? String { pending[id] = (name, input) }
                case "thinking":
                    d.last = .thinking
                default:
                    break
                }
            }
        default:
            break
        }
    }
    return d
}

/// Findet und merkt Transkripte je Session. NUR auf `workQueue` benutzen (Zustand ohne Sperre).
/// Der Projektordner ist nicht immer der aktuelle cwd (Worktree-Sessions): einmal ueber alle
/// Projektordner suchen, Pfad merken. Gelesen wird nur, wenn Groesse oder Aenderungszeit wechseln.
final class TranscriptReader {
    private let configDirs: [String]
    private let tailBytes: Int
    private var paths: [String: String] = [:]
    private var cache: [String: (size: UInt64, modified: Date, digest: Digest)] = [:]

    init(configDirs: [String] = AppConfig().claudeConfigDirs, tailBytes: Int = 262_144) {
        self.configDirs = configDirs
        self.tailBytes = tailBytes
    }

    /// Auswertung fuer eine Claude-Session-ID (ohne "claude:"). Nil = (noch) kein Transkript.
    func digest(sessionId: String) -> Digest? {
        guard let path = paths[sessionId] ?? locate(sessionId) else { return nil }
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: path) else {
            paths[sessionId] = nil                 // Datei weg: beim naechsten Mal neu suchen
            cache[sessionId] = nil
            return nil
        }
        let size = (attrs[.size] as? NSNumber)?.uint64Value ?? 0
        let modified = attrs[.modificationDate] as? Date ?? .distantPast
        if let c = cache[sessionId], c.size == size, c.modified == modified { return c.digest }
        guard let text = tailString(path: path, bytes: tailBytes) else { return nil }
        var d = digestTranscript(text)
        if d.kevinTurn == nil, let seen = cache[sessionId] {
            d.kevinTurn = seen.digest.kevinTurn
            // Wuchs die Datei seit der letzten Lesung um mehr als den Ausschnitt, kann dazwischen Kevins Nachricht mitten
            // im Zug stehen: dann gilt der gemerkte Start-Prompt nicht mehr (review quick 2026-10-01, Codex).
            let gap = size > seen.size + UInt64(tailBytes)
            d.kevinPrompt = gap ? nil : seen.digest.kevinPrompt
            d.kevinPromptAt = gap ? nil : seen.digest.kevinPromptAt
        }
        if d.peerPid == nil, d.peerName == nil, let seen = cache[sessionId]?.digest {
            // Nach einem langen Zug faellt die Nachricht der anderen Session aus dem Ausschnitt: die zuletzt gesehene
            // behalten — ob sie noch laeuft, prueft `Standby` (review quick 2026-10-10, GLM; so fiel "#8 Rules page" um).
            d.peerPid = seen.peerPid
            d.peerName = seen.peerName
            d.peerAt = seen.peerAt
        }
        if d.name == nil || d.title == nil {
            // Nach einem langen Zug fehlen Name und Auto-Titel im Ausschnitt (`lastTitles`): die zuletzt
            // gesehenen behalten, beim ersten Lesen einmal rueckwaerts suchen.
            let seen = cache[sessionId]?.digest
            let found = seen == nil ? lastTitles(path: path) : (name: nil, title: nil)
            d.name = d.name ?? seen?.name ?? found.name
            d.title = d.title ?? seen?.title ?? found.title
        }
        cache[sessionId] = (size, modified, d)
        return d
    }

    /// Nur lebende Sessions behalten — sonst waechst der Speicher mit jeder beendeten Session.
    func forget(except alive: Set<String>) {
        paths = paths.filter { alive.contains($0.key) }
        cache = cache.filter { alive.contains($0.key) }
    }

    private func locate(_ sessionId: String) -> String? {
        let fm = FileManager.default
        for dir in configDirs {
            let projects = dir + "/projects"
            for p in (try? fm.contentsOfDirectory(atPath: projects)) ?? [] {
                let candidate = projects + "/" + p + "/" + sessionId + ".jsonl"
                if fm.fileExists(atPath: candidate) {
                    paths[sessionId] = candidate
                    return candidate
                }
            }
        }
        return nil
    }
}

// ---------- Wartet auf andere Session oder Wecker (Kevin 2026-09-27) ----------
//
// "Untaetig" meldet Claude auch, wenn die Session nur auf etwas wartet, das sie selbst wieder weckt: die Nachricht einer
// anderen Session (Lead-Session -> Worker, `origin.kind` peer) oder einen selbst gestellten Wecker (`ScheduleWakeup`,
// `CronCreate`). Lookout zeigte dann fertig — ein Worker stand so, obwohl er auf die Lead-Session wartete. Ob die Session
// dabei wirklich nichts von Kevin braucht, steht nur im Antworttext: nach einer Nachricht der Lead-Session war es in
// 4 Tagen je etwa ein Drittel "wartet weiter", "fertig" und "braucht Kevin" (Replay 27.09.). Ohne KI, damit Lookout
// bei jedem laeuft (Kevin: "nicht jeder kann ein lokales LLM laufen lassen").

private let WAKE_TOOLS: Set<String> = ["ScheduleWakeup", "CronCreate", "CronDelete", "SendMessage"]
/// Nur Claudes eigene Session-Sockets ("uds:/tmp/cc-socks/<pid>.sock"), keine beliebigen (review quick, Codex).
private let SOCKET_PID = try! NSRegularExpression(pattern: #"/cc-socks/(\d+)\.sock$"#)

/// Weck-Quellen aus einem `user`-Eintrag: die Nachricht einer anderen Session beginnt ein Gespraech mit ihr; Ergebnisse von
/// Wecker- und SendMessage-Aufrufen stellen oder loeschen Wecker bzw. nennen die andere Session. Kevins Prompt beendet das
/// Gespraech nicht (R11, Kevin 2026-10-05, Screenshot 13:11: auf seine Frage "kann ich schliessen?" wartete die Session
/// weiter auf die Lead-Session) — ob die andere Session noch laeuft, prueft `Standby`.
private func noteWakeSources(_ obj: [String: Any], results: [[String: Any]],
                             pending: inout [String: (name: String, input: [String: Any])], into d: inout Digest) {
    let origin = obj["origin"] as? [String: Any]
    let kind = origin?["kind"] as? String
    if isPrompt(obj) {
        d.kevinTurn = kind == nil || kind == "human"
        // Start-Prompt kann nur der erste Prompt der Session sein (`turnPosition.promptIndex` 1); derselbe Text spaeter
        // erneut abgeschickt ist getippt, ohne Positionsangabe im Zweifel auch (review quick 2026-10-01, Codex).
        let first = (obj["turnPosition"] as? [String: Any])?["promptIndex"] as? Int == 1
        d.kevinPrompt = d.kevinTurn == true && first ? promptText(obj) : nil
        d.kevinPromptAt = d.kevinPrompt == nil ? nil : transcriptDate(obj)
    }
    // Andere Session (Socket "uds:…", von Claude geprueft), nicht der Bericht eines eigenen Subagenten (`handback`).
    if kind == "peer", (origin?["from"] as? String)?.hasPrefix("uds:") == true, origin?["handback"] as? Bool != true {
        d.peerPid = (origin?["verifiedPeerPid"] as? Int).map { pid_t($0) }
        d.peerName = origin?["name"] as? String
        d.peerAt = transcriptDate(obj)
        d.kevinTurn = false
    }
    let outcome = obj["toolUseResult"] as? [String: Any] ?? [:]
    for r in results where r["type"] as? String == "tool_result" {
        guard let id = r["tool_use_id"] as? String, let call = pending.removeValue(forKey: id) else { continue }
        switch call.name {
        case "ScheduleWakeup":
            if outcome["stopped"] as? Bool == true {
                d.wakeups.removeValue(forKey: "loop")
            } else if let ms = outcome["scheduledFor"] as? Double, ms > 0 {
                d.wakeups["loop"] = Date(timeIntervalSince1970: ms / 1000)
            }
        case "CronCreate":
            guard let cid = outcome["id"] as? String else { continue }
            // Im Zweifel einmalig, und einmalig ohne lesbaren Zeitpunkt zaehlt nicht — sonst stuende er nach dem Ausloesen
            // ewig aus (review quick 2026-09-27, Kimi). Wiederkehrend steht aus, solange die Session lebt.
            if outcome["recurring"] as? Bool ?? call.input["recurring"] as? Bool ?? false {
                d.wakeups.updateValue(nil, forKey: cid)
            } else if let at = oneShotCronDate(outcome["humanSchedule"] as? String ?? call.input["cron"] as? String ?? "",
                                               after: transcriptDate(obj) ?? Date()) {
                d.wakeups[cid] = at
            }
        case "CronDelete":
            if let cid = outcome["id"] as? String ?? call.input["id"] as? String { d.wakeups.removeValue(forKey: cid) }
        case "SendMessage":
            // An eine Socket-Adresse ("uds:/tmp/cc-socks/<pid>.sock") ist es immer eine andere Session; deren Antwort nennt
            // dann keinen Namen und nicht "another Claude session" (Kevin 2026-10-10, Screenshot 23:33: "#8 Rules page
            // build" schrieb dem Team Lead so und stand FERTIG). Nur nachweislich zugestellt (`success` true, kein
            // `is_error`); die pid prueft `Standby` gegen die laufenden Sessions und ihren Start (review quick, Codex).
            if let to = call.input["to"] as? String, to.hasPrefix("uds:"), outcome["success"] as? Bool == true,
               r["is_error"] as? Bool != true,
               let m = SOCKET_PID.firstMatch(in: to, range: NSRange(to.startIndex..., in: to)),
               let range = Range(m.range(at: 1), in: to), let pid = pid_t(to[range]) {
                if pid != d.peerPid { d.peerName = nil }   // Antwort an dieselbe Session behaelt ihren Namen
                d.peerPid = pid
                d.peerAt = transcriptDate(obj)
                continue
            }
            // Nur an eine andere laufende Session ("another Claude session on this machine"), nicht an einen Subagenten.
            let said = outcome["message"] as? String ?? r["content"] as? String ?? ""
            guard said.contains("another Claude session"), let to = call.input["to"] as? String else { continue }
            let name = to.replacingOccurrences(of: #"\s*\[[^\]]*\]$"#, with: "", options: .regularExpression)
            if name != d.peerName { d.peerPid = nil }
            d.peerName = name
        default:
            break
        }
    }
}

/// Ein Wecker hat ausgeloest: ein einmaliger Cron ist weg, ein wiederkehrender bleibt; der Loop-Wecker ist weg, wenn die
/// Ausloesung zu seinem Zeitpunkt passt (ein frueherer Lauf eines anderen Auftrags laesst ihn stehen).
private func noteFire(_ obj: [String: Any], into d: inout Digest) {
    if let tid = obj["taskId"] as? String, let at = d.wakeups[tid], at != nil { d.wakeups.removeValue(forKey: tid) }
    if let loop = d.wakeups["loop"] ?? nil, let fired = transcriptDate(obj), fired >= loop.addingTimeInterval(-60) {
        d.wakeups.removeValue(forKey: "loop")
    }
}

/// Zeitpunkt eines einmaligen Crons "M H T Mo *" in Ortszeit, der naechste nach `after`. Nil = anderes Muster oder kein
/// echtes Datum — Calendar rechnet den 31.02. sonst still in den 03.03. um (review quick 2026-09-27, Kimi).
func oneShotCronDate(_ expr: String, after: Date) -> Date? {
    let f = expr.split(separator: " ").map(String.init)
    guard f.count == 5, f[4] == "*", let minute = Int(f[0]), let hour = Int(f[1]), let day = Int(f[2]),
          let month = Int(f[3]), (0...59).contains(minute), (0...23).contains(hour) else { return nil }
    let cal = Calendar.current
    let year = cal.component(.year, from: after)
    for y in [year, year + 1] {
        guard let at = cal.date(from: DateComponents(year: y, month: month, day: day, hour: hour, minute: minute)),
              cal.component(.month, from: at) == month, cal.component(.day, from: at) == day else { continue }
        if at > after.addingTimeInterval(-60) { return at }
    }
    return nil
}

private func caseless(_ pattern: String) -> NSRegularExpression {
    try! NSRegularExpression(pattern: pattern, options: [.caseInsensitive])
}

// Regel ohne KI, eingefroren 27.09. vor der Pruefung an frischen Faellen: 189 Zug-Enden (Peer: 86 von 107 Wartenden grau,
// 0 von 14, die Kevin brauchten) und 46 frische (29 von 40 grau, 0 von 6 falsch grau). Vorsichtig gebaut: im Zweifel
// bleibt es beim heutigen "fertig". Andere Sprachen als Deutsch/Englisch treffen nie — dann ebenfalls wie heute.
// Nachgeschaerft 2026-10-01: an 388 frischen Zug-Enden (28.09.-01.10.) versteckte sie 15 von 26 Anliegen ("waiting for
// your next point", "except the merges", "after you click merge") -> weitere Sperren; Entwicklungszahlen, Nachmessung
// an neuen Faellen steht aus.
private let NOTHING_NEEDED = caseless(#"nothing (is |more )?needed|no action (is )?needed|nothing for you|"#
    + #"nichts (zu tun )?für dich|für dich (gerade )?nichts|nichts von dir|noch nichts zu entscheiden"#)
/// "Keine Frage an dich" seit Kevin 2026-10-10 (Screenshot 22:50: "I have nothing to ask you, Kevin … I wait for the team
/// lead" stand FERTIG; 2 von 3.016 Zug-Enden seit 05.10.) — nicht an Dritte ("nothing to ask the lead", auch ueber den
/// Zeilenumbruch; review quick, Codex + GLM). Schwaecher als "nichts noetig" (keine Frage heisst nicht keine Bitte): gilt
/// nur am ganzen Text (`saysNothingNeeded`), nicht als erster Satz beim Aufpasser (`opensWithNothingNeeded`).
private let NOTHING_TO_ASK = caseless(#"nothing (more |else )?to ask( you)?(?!\s+\w)|nichts (mehr )?zu fragen"#)
/// Fertig-Meldung, Teil 1: Kevin kann das Fenster schliessen oder es ist uebergeben — sperrt immer.
private let CLOSE_WORDS = caseless(#"close this window|can close|kannst (das|dieses) fenster|schlie(ß|ss)en|stood down|"#
    + #"nothing (is )?left|handed over|übergeben|"#
    + #"until you (close|end)|for you to (end|close)|stays open|leave (this|the) (session|window) open|closed out"#)
/// Fertig-Meldung, Teil 2: Fortschrittswoerter — sperren nicht, wenn die Session selbst sagt, dass sie wartet (`holdsItself`).
private let PROGRESS_DONE_PATTERN = #"erledigt|\bfinished\b|\bdone\b|abgeschlossen|"#
    + #"\b(is|are) (now )?(done|closed|merged|finished|complete|completed)\b"#
private let PROGRESS_DONE = caseless(PROGRESS_DONE_PATTERN)
/// Nur diese blendet das Warten aus: ohne folgenden Satzteil — "The summary is finished — reply with any corrections"
/// kann einen Auftrag einleiten (review quick 2026-10-10 Runde 2, GLM). Anders als `DONE_LATER` nicht nach Doppelpunkt:
/// in 48 echten "nichts noetig"-Antworten seit 05.10. stand danach nie ein Auftrag, sondern "**Done:**"-Listen und
/// Erklaerungen ("The review loop is closed: four rounds …").
private let PROGRESS_DONE_PLAIN = caseless("(" + PROGRESS_DONE_PATTERN + #")(?!\s*[,;—–]|\s+-\s)"#)
/// Die Session wartet selbst ("I hold until #258's run", "I'm waiting for the lead", "ich warte auf …"). "hold" nur mit
/// "until" im Satz ("The tickets I hold are resolved" ist Besitz), nicht verneint, auch nicht mit Fuellwort dazwischen ("I
/// wait no longer", "waiting for nothing", "ich warte jetzt nicht mehr") — review quick 2026-10-10, Codex + GLM.
private let HOLDS = caseless(#"\bI(['’]m| am|['’]ll| will)? (now |still )?(wait|waiting|(hold|holding)(?=[^.!?]*\b(until|till)\b))\b"#
    + #"(?!\s+(\w+\s+)?((for|on)\s+)?(not|no longer|nothing)\b)|\b(ich warte|warte ich)\b(?!\s+(\w+\s+)?(auf\s+)?(nicht|nichts)\b)"#)
/// Kevin im Satz, auch "dir/dich" (review quick 2026-10-10, GLM: "Ich warte auf Rückmeldung von dir") und sein Vorname
/// (`greetingName`: "I hold until Kevin reviews the PR"), dieser nur gross geschrieben ("Will" ist nicht "I will wait").
private let MENTIONS_KEVIN_PATTERN = #"you|your|du|dein\w*|dich|dir"#
private func mentionsKevin() -> NSRegularExpression {
    let name = greetingName.isEmpty ? "" : "|(?-i:" + NSRegularExpression.escapedPattern(for: greetingName) + ")"
    return caseless(#"\b("# + MENTIONS_KEVIN_PATTERN + name + #")\b"#)
}
private let ASKS_KEVIN = caseless(#"your (call|go|answer|decision|pick|merge|word)|for you to (merge|review|test|look|decide)|"#
    + #"ready for you|waits? (only )?for your|need(s)? (your|from you)|deine (entscheidung|antwort|freigabe|wahl)|"#
    + #"brauche ich|soll ich|willst du|möchtest du|beyond the|"#
    + #"your next|(waiting|wait|waits) (only )?(on|for) (you|your)\b|\bexcept\b|\bafter you\b|\bonce you\b|\bclick(s|ed|ing)?\b|"#
    + #"\bbitte\b|\bau(ß|ss)er\b|sobald du|\bdu bist dran|\bdein (klick|wort)|"#
    // "auf deine" nur als ganzes Wort: "auf deinen/deinem Mac" ist ein Ort, keine Bitte (Kevin 2026-10-06, Screenshot 16:55 —
    // ein Aufpasser stand deshalb fertig). Warten auf Kevin faengt das Verb ("warte auf deinen Stick"). An 1.204 Faellen
    // 0 Wechsel.
    + #"\bwarte\w*( nur| noch)* auf (dich|dein)|auf deine\b|"#
    // review quick 2026-10-05 (Codex): "please" ist eine Bitte — seit R11 kann eine gelesene Antwort auf Kevins Nachricht
    // grau werden ("Nothing needed from you on the tests. Please approve the deployment"). An 1.204 Faellen 0 graue Treffer.
    + #"\bplease\b"#)
/// R11 C (Kevin 2026-10-05): eine fruehere Frage an Kevin steht noch offen — sperrt auch hinter "nichts noetig" im ersten
/// Satz. Nachmessung an 1.204 frischen Zug-Enden (01.10. 22:58 bis 05.10.): live versteckte der Aufpasser 18 von 67
/// Bitten, meist Lageberichte "nothing needs you … my question above stands"; mit dieser Sperre 3 (alle unsicher), 1
/// richtige Grau-Anzeige weniger. Nach der Messung entworfen (Entwicklungszahl).
private let OPEN_QUESTION = caseless(#"questions? (above |on [^.\n]{0,40})?(still )?stands?\b|(still )?open with you|"#
    + #"beyond what my last|weiter offen|darauf warte ich|\bfragen?\b[^.\n]{0,40}\b(noch|weiter) offen"#)
/// R11 A' (Kevin 2026-10-05): Stellen, an denen "done/finished/erledigt" KEINE Fertig-Meldung dieser Session ist —
/// verneint ("not finished"), Bedingung/Zukunft ohne Kevin ("when #12's run is done", "sobald #12 erledigt ist"),
/// Teilschritt ("Part 1 … is done") und "Done:" in einer Liste, die auch "Still to do:/Next:/Offen:" fuehrt. Nachmessung:
/// bei "wartet auf andere Session" 326 -> 333 richtig grau, keine Bitte zusaetzlich versteckt. Die Bedingung braucht ein
/// Verb vor dem Fertig-Wort ("after two rounds and the reviewer finished" ist eine Fertig-Meldung; die eingefrorene Fassung ohne
/// Verb versteckte so eine Bitte).
private let DONE_WORD = #"(done|finished|complete|completed|erledigt|abgeschlossen)"#
/// Bedingung und Teilschritt enden an Satz- und Teilsatzgrenzen: "After two rounds, the implementation is done" und
/// "Step 1 passed; the entire task is complete" sind Fertig-Meldungen (review quick 2026-10-05, Codex). Folgt auf die
/// Bedingung ein Komma, leitet sie einen Hauptsatz ein, der ein Auftrag sein kann ("When the CI run is done, merge PR
/// #12") — dann bleibt sie stehen (Runde 2, GLM; kostet 2 richtige Grau-Anzeigen an 1.204 Faellen).
private let IN_CLAUSE = #"[^.!?\n,;:—–]"#
private let NOT_DONE = caseless(#"(\bnot|n['’]t)\s+((yet|quite)\s+){0,2}"# + DONE_WORD + #"\b|\bnoch nicht\s+"# + DONE_WORD + #"\b"#)
private let DONE_LATER = caseless(#"\b(when|once|after|until|till|before|as soon as)\b"# + IN_CLAUSE + #"{0,60}?\b"#
    + #"(is|are|['’]s|has been|have been|gets?)\s+(now\s+)?"# + DONE_WORD + #"\b(?!\s*[,;:—–])|\bonce done\b(?!\s*[,;:—–])|"#
    + #"\b(sobald|wenn|bis|nachdem)\b"# + IN_CLAUSE + #"{0,60}?\b"# + DONE_WORD + #"\s+(ist|sind|wird|werden)\b(?!\s*[,;:—–])"#)
private let PART_DONE = caseless(#"\b(part|step|phase|stage|round|call|teil|schritt|runde)\s+"#
    + #"(\d+|one|two|three|[a-c]|i{1,3}|first|second|third)\b"# + IN_CLAUSE + #"{0,40}?\b(is|are|ist|sind)\s+(now\s+|jetzt\s+)?"#
    + DONE_WORD + #"\b"#)
private let DONE_HEADING = try! NSRegularExpression(pattern: #"^[ \t]*[-*•]?[ \t]*\**[ \t]*(done|erledigt)[ \t]*(:\**|\**[ \t]*:)"#,
                                                    options: [.caseInsensitive, .anchorsMatchLines])
private let OPEN_HEADING = try! NSRegularExpression(
    // Nur eindeutige Restlisten, nicht "Next:"/"Open:" (review quick 2026-10-05, Kimi: "Done: PR merged. Next: release
    // tomorrow" ist eine Fertig-Meldung); die Ueberschrift braucht Inhalt in derselben Zeile, "Still to do: nothing" ist
    // kein Zwischenstand (Codex).
    pattern: #"^[ \t]*[-*•]?[ \t]*\**[ \t]*(still to do|to do|todo|remaining|offen|noch offen|noch zu tun)"#
        + #"[ \t]*(:\**|\**[ \t]*:)[ \t*_]*(?!(nothing|none|nichts|keine[ns]?|n/?a)\b)[^\s—–-]"#,
    options: [.caseInsensitive, .anchorsMatchLines])
/// Nennt derselbe Satz einen letzten Schritt ("Step 2 is done — that was the last one", "alle Schritte"), bleibt "Part/Step N
/// is done" eine Fertig-Meldung (review quick 2026-10-05, GLM; satzweise, Kimi: "The final part waits …" in einem anderen
/// Satz zaehlt nicht).
private let FINAL_PART = caseless(#"\b(last|final|letzte[mnrs]?|finale[mnrs]?)\b|\ball (steps|parts|rounds|phases)\b|"#
    + #"\balle (schritte|teile|runden|phasen)\b"#)
/// Eine Bedingung an Kevin ("when you're done testing") bleibt eine Fertig-Stelle.
private let ADDRESSES_KEVIN = caseless(#"\b(you|your|du|dein\w*)\b"#)

private let SENTENCE_SPAN = try! NSRegularExpression(pattern: #"[^.!?\n]+"#)

/// Satz ueber Zeilenumbrueche hinweg ("I hold until\nyou approve …"); ein Punkt ohne folgenden Leerraum ("README.md",
/// "v1.2") trennt nicht (review quick 2026-10-10, Codex).
private let SENTENCE_ACROSS_LINES = try! NSRegularExpression(pattern: #"(?:[^.!?]|[.!?](?=\S))+"#)

/// Wie `blank`, aber Satz fuer Satz: Saetze mit einem `unless`-Treffer bleiben, wie sie sind.
private func blankBySentence(_ r: NSRegularExpression, in text: String, unless: NSRegularExpression,
                             span: NSRegularExpression = SENTENCE_SPAN) -> String {
    let s = NSMutableString(string: text)
    for m in span.matches(in: text, range: NSRange(location: 0, length: s.length)).reversed() {
        let sentence = s.substring(with: m.range)
        if !has(unless, sentence) { s.replaceCharacters(in: m.range, with: blank(r, in: sentence)) }
    }
    return s as String
}

/// Ersetzt die Treffer durch ein Leerzeichen; `keep` behaelt einzelne Treffer.
private func blank(_ r: NSRegularExpression, in text: String, keep: (String) -> Bool = { _ in false }) -> String {
    let s = NSMutableString(string: text)
    for m in r.matches(in: text, range: NSRange(location: 0, length: s.length)).reversed()
    where !keep(s.substring(with: m.range)) {
        s.replaceCharacters(in: m.range, with: " ")
    }
    return s as String
}

/// Text ohne Stellen, an denen "done/finished/erledigt" keine Fertig-Meldung dieser Session ist (nur fuer die
/// Fertig-Pruefung; Bitten werden am ganzen Text geprueft).
func withoutProgressNotes(_ text: String) -> String {
    var t = blank(NOT_DONE, in: text)
    t = blank(DONE_LATER, in: t) { has(ADDRESSES_KEVIN, $0) }
    t = blankBySentence(PART_DONE, in: t, unless: FINAL_PART)
    return has(OPEN_HEADING, t) ? blank(DONE_HEADING, in: t) : t
}

/// Zusage, sich von selbst zu melden — nur in der ersten Person ("Sag Bescheid", "Melde mich … an" sind Bitten an Kevin).
private let PROMISES_REPORT = caseless(#"\b(ich melde|melde ich) mich\b|\b(ich (gebe|sage|sag)|(gebe|sage|sag) ich) (dir )?bescheid\b|"#
    + #"\bI['’]ll (report|check|get|come) back\b|\bI['’]ll (let you know|ping you|update you)\b|"#
    + #"\bI will (report back|let you know)\b"#)
/// Bitte oder Bedingung an Kevin irgendwo in der Antwort: dann zaehlt keine Zusage ("Please approve …", "wenn du …").
private let PROMISE_BLOCKS = caseless(#"\bplease\b|\b(wenn|falls|nachdem) du\b|\b(if|when) you\b"#)
/// Satzende fuer die Zusage, enger als `SENTENCE_END`: "."/"!" plus Leerraum, wenn in derselben Zeile noch ein Buchstabe
/// folgt, auch klein oder hinter Emoji/Nummer ("… is green. merge PR #12", "… ist. ✅ Merge …"), oder ein Zeilenumbruch;
/// ein Rest ohne Buchstaben ("… durch ist. ✅") gehoert zum Satz davor.
private let PROMISE_SENTENCE_END = try! NSRegularExpression(pattern: #"[.!][*_"'`“”’‘»«›‹)\]]*\s+(?=[^\p{L}\n]*\p{L})|\n"#)
/// Zweiter Satzteil nach der Zusage: Gedankenstrich, Doppelpunkt, Semikolon ("… durch ist — merge danach PR #12").
private let CLAUSE_BREAK = try! NSRegularExpression(pattern: #"\s[—–-]\s|[:;]\s"#)

private func has(_ r: NSRegularExpression, _ text: String) -> Bool {
    r.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
}

/// Will die Antwort nichts von Kevin — keine Frage, keine Bitte an ihn, keine noch offene fruehere Frage, keine
/// Fertig-Meldung? `holding`: die Session wartet selbst, dann zaehlen Fortschrittswoerter nur noch in Saetzen an Kevin
/// ("When you're done testing, merge PR #12", review quick 2026-10-10, Codex) oder vor einem weiteren Satzteil.
func asksNothing(_ text: String, holding: Bool = false) -> Bool {
    !text.contains("?") && !reportsDone(text, holding: holding) && !has(ASKS_KEVIN, text) && !has(OPEN_QUESTION, text)
}

/// Fertig-Meldung im Text: Schliessen/Uebergabe immer, Fortschrittswoerter mit `holding` wie bei `asksNothing`.
private func reportsDone(_ text: String, holding: Bool) -> Bool {
    var rest = withoutProgressNotes(text)
    // Schliessen/Uebergabe vor dem Ausblenden pruefen: "is closed out" verloere sonst sein "is closed" (review quick, Codex).
    if has(CLOSE_WORDS, rest) { return true }
    if holding { rest = blankBySentence(PROGRESS_DONE_PLAIN, in: rest, unless: mentionsKevin(), span: SENTENCE_ACROSS_LINES) }
    return has(PROGRESS_DONE, rest)
}

/// Sagt die Session in einem Satz ohne Bezug auf Kevin, dass sie selbst wartet — und in keinem, dass sie auf ihn wartet?
/// "I hold until you merge" und "I hold until Kevin reviews" zaehlen nicht und sperren auch neben "I wait for the lead"
/// (review quick 2026-10-10, Codex).
func holdsItself(_ text: String) -> Bool {
    let ns = text as NSString
    let kevin = mentionsKevin()
    let holds = SENTENCE_ACROSS_LINES.matches(in: text, range: NSRange(location: 0, length: ns.length))
        .map { ns.substring(with: $0.range) }.filter { has(HOLDS, $0) }
    return !holds.isEmpty && !holds.contains { has(kevin, $0) }
}

/// Sagt die Antwort ausdruecklich, dass Kevin nichts tun muss — ohne Frage, ohne Bitte an ihn, ohne Fertig-Meldung?
/// Kevin 2026-10-10 (Screenshot 22:50: "Nothing needed … Main is merged into my branch … I hold until card #258's run"
/// stand FERTIG): sagt sie selbst, dass sie wartet, sperren Fortschrittswoerter wie "is merged" nicht mehr — Bitten, Fragen
/// und "Fenster schliessen/uebergeben" weiter. Seit 05.10. 12 von 3.016 Zug-Enden, dem Augenschein nach alle Warte-Zustaende
/// (Entwicklungszahl, Nachmessung ab 12.10.).
func saysNothingNeeded(_ text: String) -> Bool {
    (has(NOTHING_NEEDED, text) || has(NOTHING_TO_ASK, text)) && asksNothing(text, holding: holdsItself(text))
}

/// Endet die Antwort mit der Zusage, sich von selbst zu melden ("Ich melde mich, sobald v3 durch ist", "I'll report
/// back")? Beim Aufpasser zaehlt dann Kevins eigene Nachricht nicht mehr: die Antwort hat er, die Session wartet nur noch
/// (Kevin 2026-10-03, Screenshot 08:58; Messung an den 410 Aufpasser-Faellen vom 01.10.: grau 282 -> 284, nichts
/// zusaetzlich versteckt). Im Zweifel keine Zusage (review quick 2026-10-03, GLM, Codex, Kimi): nur der letzte Satz zaehlt
/// und nur als ein Satzteil ("Ich melde mich, sobald … Merge/— merge in der Zwischenzeit PR #12" ist ein Auftrag ohne
/// Signalwort), und eine Bitte oder Bedingung an Kevin irgendwo in der Antwort sperrt. Grenze: ein Auftrag nur nach Komma
/// ("…, merge danach PR #12") bleibt unerkannt.
func promisesReport(_ text: String) -> Bool {
    guard !has(PROMISE_BLOCKS, text) else { return false }
    let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
    let end = PROMISE_SENTENCE_END.matches(in: t, range: NSRange(t.startIndex..., in: t)).last
        .flatMap { Range($0.range, in: t) }
    let last = String(t[(end?.upperBound ?? t.startIndex)...])
    return has(PROMISES_REPORT, last) && !has(CLAUSE_BREAK, last)
}

/// Beginnt die Antwort ausdruecklich mit "nichts noetig" — erster Satz ohne Frage, Bitte oder Fertig-Meldung, und nirgends
/// ein Fragezeichen? Fuer den Aufpasser zaehlt dann der Rest nicht: ein Lagebericht, der eine andere Session nennt, die auf
/// Kevin wartet, oder etwas "complete" meldet, braucht ihn in DIESEM Fenster nicht (Kevin 2026-10-01 abends; Messung an
/// den 798 Faellen vom 01.10.: grau 239 -> 282 von 380, versteckt 1 Fertig-Meldung und 1 unsichere Bitte). Fuer die
/// andere Session gilt das nicht: dort versteckte es 17 Bitten und 16 Fertig-Meldungen. Eine noch offene fruehere Frage
/// ("My question above stands", "Still open with you: …") sperrt im ganzen Text (R11 C, Kevin 2026-10-05).
func opensWithNothingNeeded(_ text: String) -> Bool {
    guard !text.contains("?"), !has(OPEN_QUESTION, text) else { return false }
    let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
    let end = SENTENCE_END.firstMatch(in: t, range: NSRange(t.startIndex..., in: t)).flatMap { Range($0.range, in: t) }
    let first = String(t[..<(end?.lowerBound ?? t.endIndex)])
    return has(NOTHING_NEEDED, first) && saysNothingNeeded(first)
}

/// Braucht eine Session, die nur auf eine andere Session oder einen Wecker wartet, nichts von Kevin (`Standby`)? Der ganze
/// Text sagt "nichts noetig" (`saysNothingNeeded`) — oder der erste Satz sagt es und die Session sagt selbst, dass sie
/// wartet (`holdsItself`): dann zaehlen Bitten im Rest nicht, wie beim Aufpasser. Weiter sperren: ein "?", eine offene
/// fruehere Frage, eine Fertig-Meldung wie in R13 (`reportsDone` mit Warten: "The report for you is complete") und eine
/// direkte Bitte (`DIRECT_ASK`: "please/bitte", "I need your …", "ready for you", "for you to review" — review quick, GLM +
/// Codex). Kevin 2026-10-10 (Screenshot 23:33): "#8 Rules page build" ("Nothing needed … I hold until the team lead's go
/// … The editor waits on your answer") stand FERTIG — Bericht ueber ein ANDERES Fenster. Entwicklungszahl: 14 von 3.022
/// Zug-Enden seit 05.10. werden zusaetzlich grau (Wortlisten-Fehlgriffe "click/except", Hinweise "closing box waits for
/// you in the morning", Berichte ueber andere Fenster). Der Preis der Regel vom 05.10. (17 Bitten versteckt) galt ohne
/// eigenes Warten.
func waitsWithNothingNeeded(_ text: String) -> Bool {
    saysNothingNeeded(text)
        || (opensWithNothingNeeded(text) && holdsItself(text) && !reportsDone(text, holding: true) && !has(DIRECT_ASK, text))
}
private let DIRECT_ASK = caseless(#"\bplease\b|\bbitte\b|\bI need (you|your)\b|\bich brauche (dich|dein\w*)\b|"#
    + #"ready for you|for you to (merge|review|test|look|decide|approve|check)"#)

/// Satzende: "." oder "!" (auch mit schliessendem Markdown, Anfuehrungszeichen, Guillemets), dann Leerraum, dann — nach
/// beliebigen Zeichen, die weder Buchstabe noch Ziffer sind (Markdown, Backticks, Klammern, "#") — ein Grossbuchstabe oder
/// eine Ziffer ("e.g. the", "e.g. **the", "e.g. `the" enden nicht), oder ein Zeilenumbruch (review quick 2026-10-01,
/// Codex Runde 1, 3, 4).
private let SENTENCE_END = try! NSRegularExpression(
    pattern: #"[.!][*_"'`“”’‘»«›‹)\]]*(?=\s+[^\p{L}\p{N}\s]*[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{N}])|\n"#)

/// Kam der Prompt beim Start mit (`claude "…"`, etwa eine Uebergabe aus einer anderen Session)? Dann ist er genau eines
/// der Startargumente. Ein Teil davon zaehlt nicht (eine getippte Nachricht, die zufaellig im Start-Prompt vorkommt), ein
/// ohne Anfuehrungszeichen getippter `claude mach das` auch nicht (das war Kevin selbst); unter 12 Zeichen zaehlt er nie.
func launchPrompt(_ prompt: String, arguments: [String]) -> Bool {
    let p = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    let args = Array(arguments.dropFirst())
    // Ein Start-Prompt ist ein Satz: Pfade, IDs und Modellnamen (Werte wie `--add-dir`) haben keinen Leerraum (review quick
    // 2026-10-01 Runde 4, Codex).
    guard p.contains(where: \.isWhitespace) else { return false }
    return p.count >= 12 && args.indices.contains { i in
        args[i].trimmingCharacters(in: .whitespacesAndNewlines) == p && (i == 0 || !TEXT_OPTIONS.contains(args[i - 1]))
    }
}

/// Freitext-Optionen der Claude-CLI (`claude --help`, 2.1.287): ihr Wert ist kein Start-Prompt (review quick 2026-10-01,
/// Codex: ein Sitzungsname wie die erste getippte Nachricht).
private let TEXT_OPTIONS: Set<String> = ["-n", "--name", "--system-prompt", "--append-system-prompt"]

/// Kevins Zug, dessen Prompt beim Start mitkam, ist nicht seiner (Kevin 2026-10-01 abends: ein Fenster, das eine andere
/// Session per Uebergabe gestartet hatte, stand nach "nichts noetig" gruen). Der Prompt muss in den ersten 30 s nach dem
/// Prozessstart gekommen sein — nach `/clear` beginnt im selben Prozess eine neue Unterhaltung, derselbe Text dort ist
/// getippt (review quick 2026-10-01 Runde 3 + 4, Codex; gemessen: Start-Prompt 3 s nach dem Start). Startargumente und
/// -zeit werden nur gelesen, wenn es darauf ankommt; unlesbar = wie bisher Kevins Zug.
func launchCorrected(_ d: Digest, arguments: @autoclosure () -> [String]?, started: @autoclosure () -> Date?) -> Digest {
    guard d.kevinTurn == true, let p = d.kevinPrompt, let at = d.kevinPromptAt, let s = started(),
          (-5..<30).contains(at.timeIntervalSince(s)), let a = arguments(), launchPrompt(p, arguments: a) else { return d }
    var out = d
    out.kevinTurn = false
    return out
}

/// Fertige Claude-Sessions, die nur auf eine andere laufende Session oder einen eigenen Wecker warten und das
/// ausdruecklich sagen: "ueberwacht" statt fertig (wie der Aufpasser in Plan 1e+, gleiche Anzeige, Kevin: "einheitlich
/// ÜBERWACHT"). Alles andere bleibt, wie es ist. NUR auf `workQueue` benutzen (Zustand ohne Sperre).
final class Standby {
    /// Ein Wecker gilt noch 5 Min. nach seinem Zeitpunkt als ausstehend: Ausloesung und neuer Zug brauchen einen Moment.
    static let wakeGrace: TimeInterval = 300
    private let report: (String) -> Void
    /// Sessions, die gerade als wartend gelten (ID -> Name) — fuer eine Log-Zeile je Wechsel.
    private var quiet: [String: String] = [:]

    init(report: @escaping (String) -> Void = { log($0) }) {
        self.report = report
    }

    /// `seen`: wann Kevin welches Fenster zuletzt vor sich hatte (R10). Hat Kevins eigene Nachricht den Zug ausgeloest,
    /// zaehlt sie nicht mehr, sobald er die Antwort gesehen hat (R11 B1, Kevin 2026-10-05, Screenshot 13:11) — grau aber
    /// weiter nur mit ausdruecklichem "nichts noetig" (B1 streng; Kevins Wahl gegen das lockere B2, das an den Faellen
    /// 4 Bitten und 5 Fertig-Meldungen versteckte). Nachmessung (1.204 Zug-Enden, Obergrenze "alles gesehen"): 0 -> 15
    /// richtig grau, nichts versteckt.
    func adjust(_ sessions: [Session], now: Date, seen: [String: Date] = [:]) -> [Session] {
        // Die andere Session lebt, wenn sie unter den laufenden Claude-Sessions steht (Status-Dateien, liegengebliebene
        // mit wiedervergebener pid filtert `readSessionFiles`) — nicht schon, wenn irgendein Prozess die pid traegt
        // (review quick 2026-09-27, GLM). Ohne pid (eigener SendMessage) zaehlt der Name. Startete die Session mit der pid
        // erst nach der Nachricht, ist es nicht die andere Session (wiedervergebene pid, review quick 2026-10-10, Codex).
        let live = sessions.filter { $0.engine == .claude && $0.pid != nil }
        let started = Dictionary(live.map { ($0.pid!, $0.startedAt) }, uniquingKeysWith: { a, _ in a })
        let peerAlive = { (pid: pid_t, at: Date?) -> Bool in
            guard let start = started[pid] else { return false }
            guard let start, let at else { return true }
            return start <= at.addingTimeInterval(2)
        }
        let names = Set(live.map(\.name))
        var out = sessions
        var nowQuiet: [String: String] = [:]
        for i in out.indices {
            let s = out[i]
            guard s.engine == .claude, s.ready, !s.waiting, !s.watching, let d = s.digest, d.turnEnded, d.question == nil,
                  d.kevinTurn != true || answerSeen(d, seenAt: seen[s.id]), let text = d.text, waitsWithNothingNeeded(text)
            else { continue }
            // Nachfolger mit gleichem Namen (neue Lead-Session) zaehlt auch, wenn die gepruefte pid schon beendet ist.
            let peer = (d.peerPid.map { peerAlive($0, d.peerAt) } ?? false) || (d.peerName.map(names.contains) ?? false)
            let wake = d.wakeups.values.contains { $0.map { now < $0.addingTimeInterval(Self.wakeGrace) } ?? true }
            guard peer || wake else { continue }
            out[i].ready = false
            out[i].watching = true
            nowQuiet[s.id] = s.name
            if quiet[s.id] == nil {
                let why = d.kevinTurn == true ? " (Antwort gesehen)" : ""
                report("Wartet nur noch auf \(peer ? "eine andere Session" : "einen Wecker"), nichts nötig\(why): \(s.name) gilt als überwacht")
            }
        }
        for (id, name) in quiet where nowQuiet[id] == nil && out.contains(where: { $0.id == id && $0.ready }) {
            report("Wartet nicht mehr: \(name) gilt als fertig")
        }
        quiet = nowQuiet
        return out
    }
}

// ---------- Fertig trotz Hintergrund-Befehl (Plan 1e, Kevins Regel 2026-09-26 abends) ----------
//
// Claude meldet `shell`, solange nach der fertigen Antwort noch ein Hintergrund-Befehl der Session laeuft (bis CLI 2.1.283
// hiess das `busy`; seit 2026-10-09 zaehlt nur `shell`, Kevin — `busy` + altes Zug-Ende im Transkript zeigte den Team
// Lead FERTIG, waehrend er eine Nachricht einer anderen Session bearbeitete). Kevin: arbeitet der Befehl, laeuft die
// Session; ist es nur ein Beobachter oder etwas Ruhendes, ist sie fertig. Live 26.09.: eine Session rechnete im
// Hintergrund einen Bild-Neusatz (laeuft,
// richtig), eine alte hatte nur einen ruhenden Node-Prozess (0 % Rechenzeit, richtig waere fertig).
// Kevin 2026-09-27 (Lead-Session): wartet die Session auf ihren eigenen Aufpasser (CI-Lauf, Worker), braucht sie Kevin
// nicht — "ueberwacht" statt fertig. Unterschied: ein Server lauscht auf einem Port, ein Aufpasser nicht.

/// Hintergrund-Auftraege einer Claude-Session: je Shell-Kind (zsh/bash/sh) die Shell samt allem darunter. MCP-Server und
/// `caffeinate` sind keine Shells und zaehlen nicht.
func backgroundTasks(of claude: pid_t, rows: [PsRow]) -> [Set<pid_t>] {
    let children = Dictionary(grouping: rows, by: \.ppid)
    let isShell = { (r: PsRow) in
        ["zsh", "bash", "sh"].contains((r.comm as NSString).lastPathComponent.drop { $0 == "-" })
    }
    return (children[claude] ?? []).filter(isShell).map { shell in
        var tree = Set<pid_t>()
        var todo = [shell.pid]
        while let pid = todo.popLast() {
            guard tree.insert(pid).inserted else { continue }
            todo += (children[pid] ?? []).map(\.pid)
        }
        return tree
    }
}

/// Alle Hintergrund-Prozesse einer Claude-Session (Vereinigung der Auftraege).
func backgroundPids(of claude: pid_t, rows: [PsRow]) -> Set<pid_t> {
    backgroundTasks(of: claude, rows: rows).reduce(into: Set()) { $0.formUnion($1) }
}

/// pids aus `lsof -Fp`: je Prozess eine Zeile "p<pid>", darunter "f<fd>".
func parseLsofPids(_ text: String) -> Set<pid_t> {
    Set(text.split(separator: "\n").compactMap { $0.first == "p" ? pid_t($0.dropFirst()) : nil })
}

/// Welche dieser Prozesse lauschen auf einem TCP-Port, sind also Server? Eigene Prozesse, ohne Freigabe lesbar (gemessen
/// 2026-09-27: 0,02 s). lsof endet mit 1, wenn nichts lauscht oder ein Prozess inzwischen weg ist — kein Fehler.
/// Nil = Abfrage gescheitert.
func listeningPids(_ pids: [pid_t]) -> Set<pid_t>? {
    guard !pids.isEmpty else { return [] }
    let list = pids.map { String($0) }.joined(separator: ",")
    guard let data = runCommand("/usr/sbin/lsof", ["-nP", "-a", "-p", list, "-iTCP", "-sTCP:LISTEN", "-Fp"],
                                timeout: 3, accept: [0, 1]),
          let text = String(data: data, encoding: .utf8) else { return nil }
    return parseLsofPids(text)
}

/// Eine Messung der Hintergrund-Befehle: Rechenzeit (s) und Netz-Bytes offener Verbindungen, je Prozess.
struct UsageSample: Equatable {
    let at: Date
    let cpu: [pid_t: Double]
    let net: [pid_t: UInt64]
}

/// Zuwachs von `old` nach `new`, je Prozess: ein neuer Prozess zaehlt ganz, ein verschwundener nicht, nichts wird
/// negativ — nettop zaehlt nur offene Verbindungen, schliesst eine, faellt ihr Stand weg (gemessen 2026-09-26).
private func growth<T: Numeric & Comparable>(_ old: [pid_t: T], _ new: [pid_t: T]) -> T {
    new.reduce(0) { sum, kv in
        let before = old[kv.key] ?? 0
        return sum + (kv.value > before ? kv.value - before : 0)
    }
}

/// Arbeitet der Hintergrund? Ja, wenn seit der juengsten Messung, die mindestens `window` zurueckliegt, mehr als
/// `cpuLimit` s Rechenzeit oder `netLimit` Bytes Netz dazukamen — und solange noch nicht `window` lang beobachtet ist:
/// erst so lange Ruhe heisst "fertig". Netz zaehlt mit, damit ein Befehl, der auf ein Sprachmodell wartet, als
/// arbeitend gilt. `samples` aufsteigend nach Zeit. Startwerte (Plan): 1 s bzw. 50 KB in 90 s.
func backgroundBusy(_ samples: [UsageSample], now: Date, window: TimeInterval = 90, cpuLimit: Double = 1,
                    netLimit: UInt64 = 50_000) -> Bool {
    guard let last = samples.last,
          let base = samples.last(where: { now.timeIntervalSince($0.at) >= window }) else { return true }
    return growth(base.cpu, last.cpu) > cpuLimit || growth(base.net, last.net) > netLimit
}

/// nettop-Zeile "name.pid,bytes_in,bytes_out," — der Name darf Punkte enthalten (Claude heisst dort "2.1.283").
func parseNettop(_ text: String) -> [pid_t: UInt64] {
    var out: [pid_t: UInt64] = [:]
    for line in text.split(separator: "\n") {
        let f = line.split(separator: ",", omittingEmptySubsequences: false)
        guard f.count >= 3, let dot = f[0].lastIndex(of: "."), let pid = pid_t(f[0][f[0].index(after: dot)...]),
              let bytesIn = UInt64(f[1]), let bytesOut = UInt64(f[2]) else { continue }
        out[pid, default: 0] += bytesIn + bytesOut
    }
    return out
}

/// Netz-Bytes je Prozess (ohne Freigabe lesbar, geprueft 26.09.). `-n` ohne Namensaufloesung: mit ihr dauerte ein
/// Aufruf 5 s statt 0,04 s (gemessen 2026-09-26).
func nettopUsage() -> [pid_t: UInt64]? {
    guard let data = runCommand("/usr/bin/nettop", ["-P", "-x", "-n", "-L", "1", "-J", "bytes_in,bytes_out"], timeout: 3),
          let text = String(data: data, encoding: .utf8) else { return nil }
    return parseNettop(text)
}

/// Messreihen je Claude-Prozess, nur solange seine Antwort fertig ist und Claude `shell` meldet (Hintergrund laeuft).
/// NUR auf `workQueue` benutzen (Zustand ohne Sperre).
final class BackgroundWatch {
    private var samples: [pid_t: [UsageSample]] = [:]
    private let netUsage: () -> [pid_t: UInt64]?
    /// Welche Prozesse lauschen auf einem Port (Server)? Nil = unbekannt.
    private let listening: ([pid_t]) -> Set<pid_t>?
    /// Je Wechsel eine Log-Zeile — damit im Praxistest nachvollziehbar ist, warum eine Session als fertig gilt.
    private let report: (String) -> Void
    /// Sessions, deren Hintergrund gerade ruht (ID -> Name, ueberwacht oder fertig).
    private var quiet: [String: (name: String, watching: Bool)] = [:]
    /// Ruhende Sessions, deren Antwort Kevin braucht (fertig statt ueberwacht) — nur fuer den Grund in der Log-Zeile.
    private var forKevin: Set<String> = []
    /// Namen von Sessions, die andere Sessions fuehren (`leadNames` in der Konfiguration, genau so geschrieben).
    private let leads: Set<String>

    init(netUsage: @escaping () -> [pid_t: UInt64]? = nettopUsage,
         listening: @escaping ([pid_t]) -> Set<pid_t>? = listeningPids,
         leads: Set<String> = [],
         report: @escaping (String) -> Void = { log($0) }) {
        self.netUsage = netUsage
        self.listening = listening
        self.leads = leads
        self.report = report
    }

    /// `shell` + fertige Antwort + Hintergrund ruht seit `window` -> ueberwacht, solange ein Aufpasser laeuft; nur ruhende
    /// Server -> fertig. Alles andere bleibt, wie es gemeldet ist — auch eine Session ohne Hintergrund-Shell: dann haelt
    /// etwas anderes sie busy (etwa ein Hintergrund-Agent), und Lookout raet nicht. Nur Claude; Codex, Grok und Kimi
    /// melden ihr Zug-Ende selbst. `seen`: Session-ID -> wann Kevin ihr Fenster zuletzt vor sich hatte (`answerSeen`).
    func adjust(_ sessions: [Session], rows: [PsRow], now: Date, window: TimeInterval,
                seen: [String: Date] = [:]) -> [Session] {
        let out = decide(sessions, rows: rows, now: now, window: window, seen: seen)
        var nowQuiet: [String: (name: String, watching: Bool)] = [:]
        for (before, after) in zip(sessions, out) where after.watching || (after.ready && !before.ready) {
            nowQuiet[after.id] = (after.name, after.watching)
        }
        let seenAnswers = Set(out.filter { s in
            s.digest.map { $0.kevinTurn == true && answerSeen($0, seenAt: seen[s.id]) } ?? false
        }.map(\.id))
        for (id, q) in nowQuiet where quiet[id]?.watching != q.watching {
            let why = q.watching
                ? seenAnswers.contains(id) ? "überwacht (Antwort gesehen, Aufpasser läuft)" : "überwacht (Aufpasser läuft)"
                : forKevin.contains(id) ? "fertig (Antwort an dich)" : "fertig (nur ein Server läuft)"
            report("Hintergrund ruht seit \(Int(window)) s: \(q.name) gilt als \(why)")
        }
        // Nur wenn sie wirklich wieder laeuft — endet der Hintergrund-Befehl (idle) oder die Session, arbeitet nichts
        // (review quick 2026-09-26, GLM + Kimi).
        let running = Set(out.filter { !$0.ready && !$0.watching }.map(\.id))
        for (id, q) in quiet where nowQuiet[id] == nil && running.contains(id) {
            report("Hintergrund arbeitet wieder: \(q.name)")
        }
        quiet = nowQuiet
        return out
    }

    private func decide(_ sessions: [Session], rows: [PsRow], now: Date, window: TimeInterval,
                        seen: [String: Date]) -> [Session] {
        forKevin = []
        var watched: [pid_t: [Set<pid_t>]] = [:]
        // Zug-Ende im Transkript dazu: geurteilt wird ueber die fertige Antwort, nicht ueber einen Zwischenstand.
        for s in sessions where s.engine == .claude && s.backgroundOnly && s.digest?.turnEnded == true {
            guard let pid = s.pid else { continue }
            let tasks = backgroundTasks(of: pid, rows: rows)
            if !tasks.isEmpty { watched[pid] = tasks }
        }
        samples = samples.filter { watched[$0.key] != nil }
        // nettop nur in diesem Fall; scheitert es, bleibt alles wie gemeldet (lieber "laeuft" als falsch "fertig") — nur
        // wer schon ruhte, behaelt seinen Stand.
        guard !watched.isEmpty else { return sessions }
        guard let net = netUsage() else {
            return keepQuiet(sessions, at: sessions.indices.filter { sessions[$0].pid.map { watched[$0] != nil } ?? false },
                             seen: seen)
        }
        let cpu = Dictionary(rows.map { ($0.pid, $0.cpu) }, uniquingKeysWith: { first, _ in first })
        var out = sessions
        var calm: [(index: Int, tasks: [Set<pid_t>])] = []
        for i in out.indices {
            guard let pid = out[i].pid, let tasks = watched[pid] else { continue }
            let background = tasks.reduce(into: Set<pid_t>()) { $0.formUnion($1) }
            var list = samples[pid] ?? []
            list.append(UsageSample(at: now, cpu: cpu.filter { background.contains($0.key) },
                                    net: net.filter { background.contains($0.key) }))
            // Behalten, was die naechste Entscheidung braucht: die juengste Messung vor dem Fenster und alles danach.
            if let base = list.lastIndex(where: { now.timeIntervalSince($0.at) >= window }) { list.removeFirst(base) }
            samples[pid] = list
            if !backgroundBusy(list, now: now, window: window) { calm.append((i, tasks)) }
        }
        // Server oder Aufpasser? Eine Abfrage fuer alle ruhenden Sessions; scheitert sie, gilt dasselbe wie bei nettop.
        guard !calm.isEmpty else { return out }
        guard let servers = listening(calm.flatMap { $0.tasks.flatMap { $0 } }) else {
            return keepQuiet(out, at: calm.map(\.index), seen: seen)
        }
        for c in calm {
            // Ein Auftrag ohne lauschenden Prozess ist ein Aufpasser, auf den die Session wartet (CI-Lauf, Worker);
            // nur ruhende Server heissen fertig (Kevins Regel 1e: Vorschau fuer ihn oder vergessen).
            guard c.tasks.contains(where: { $0.isDisjoint(with: servers) }) else {
                out[c.index].ready = true
                continue
            }
            // Kevin 2026-10-01: braucht die Antwort Kevin (Frage, Bitte, Fertig-Meldung) oder hat seine eigene Nachricht
            // den Zug ausgeloest, versteckt grau sie — fertig wie vor 1e+ (frische Zug-Enden 28.09.-01.10.: 23 von 23).
            // Ausser der erste Satz sagt ausdruecklich "nichts noetig" (`opensWithNothingNeeded`, Kevin 01.10. abends).
            // Auf Kevins Nachricht: ausser die Antwort sagt nur zu, sich zu melden (`promisesReport`, Kevin 03.10.), oder
            // Kevin hatte das Fenster nach ihr schon vor sich (`answerSeen`, Kevin 04.10.).
            if answerForKevin(out[c.index], seen: seen) {
                out[c.index].ready = true
                forKevin.insert(out[c.index].id)
            } else {
                out[c.index].watching = true
            }
        }
        return out
    }

    /// Messung gescheitert: wer im letzten Takt schon ruhte, behaelt ueberwacht bzw. fertig — sonst stuende "arbeitet
    /// wieder" im Log und das Schild flackerte einen Takt (review quick 2026-09-27, Kimi). Alle anderen: wie gemeldet.
    /// Braucht die Antwort inzwischen Kevin, wird ueberwacht zu fertig (review quick 2026-10-01, Codex).
    private func keepQuiet(_ sessions: [Session], at indices: [Int], seen: [String: Date]) -> [Session] {
        var out = sessions
        for i in indices {
            guard let prev = quiet[out[i].id] else { continue }
            if prev.watching && !answerForKevin(out[i], seen: seen) {
                out[i].watching = true
            } else {
                out[i].ready = true
                if prev.watching { forKevin.insert(out[i].id) }
            }
        }
        return out
    }

    /// Braucht die Antwort Kevin — Frage, Bitte, Fertig-Meldung — oder hat seine eigene Nachricht den Zug ausgeloest?
    /// Ohne lesbaren Antworttext im Zweifel ja (review quick 2026-10-01, GLM) — wie bei `Standby`. Auf Kevins Nachricht
    /// genuegt eine Zusage, sich zu melden, ohne Frage, Bitte, Fertig-Meldung (Kevin 2026-10-03). Hatte Kevin das Fenster
    /// nach der Antwort schon vor sich, zaehlt seine Nachricht nicht mehr (Kevin 2026-10-04).
    private func answerForKevin(_ s: Session, seen: [String: Date]) -> Bool {
        guard let d = s.digest, let text = d.text else { return true }
        if d.kevinTurn == true && !answerSeen(d, seenAt: seen[s.id]) {
            return !(promisesReport(text) && asksNothing(text))
        }
        // Kevin 2026-10-09: eine Lead-Session berichtet ueber ANDERE Fenster ("two windows wait for you", "the handover is
        // complete") — die Wortliste hielt das fuer Anliegen (6 Tage eines Team Leads: 211 von 523 Zug-Enden ohne Kevins
        // Nachricht FERTIG, 2 davon mit "?"). Bei ihr zaehlt nur eine Frage. Name aus der Status-Datei oder der im Menue
        // (eigener Name im Transkript, `/rename`) — review quick 2026-10-09, Codex + GLM.
        if leads.contains(s.name) || d.name.map(leads.contains) == true { return text.contains("?") }
        return !(asksNothing(text) || opensWithNothingNeeded(text))
    }
}

/// Hatte Kevin das Fenster nach der letzten Antwort vor sich (Kevin 2026-10-04, Screenshot 22:38: "die Session
/// überwacht")? Dann hat er gelesen, was auf seine Nachricht kam — die Antwort braucht ihn nur noch, wenn sie selbst etwas
/// von ihm will. Messung an den 410 Aufpasser-Faellen vom 01.10. (Kevins Nachricht ganz weggelassen, Obergrenze): grau
/// 284 -> 303 von 380; 4 Anleitungen fuer Kevin (Anmeldung, Datei-Upload, Bestellung, 1 unsicher) werden grau — erst
/// nachdem er sie gesehen hat. Ohne Zeitpunkt der Antwort im Zweifel nicht gesehen.
func answerSeen(_ d: Digest, seenAt: Date?) -> Bool {
    guard let seenAt, let answered = d.answerAt else { return false }
    return seenAt >= answered
}

// ---------- Codex / Grok / Kimi: Zug-Ende aus den Session-Logs ----------
//
// Spike 2026-09-18: alle drei protokollieren Beginn UND Ende eines Zuges explizit.
// Darum wird das letzte Zug-Ereignis gelesen statt geraten, ob "lange nichts geschrieben"
// schon fertig heisst — genau die Schwaeche, die das Design sonst in Kauf genommen haette.

enum TurnState { case working, done, waiting, unknown }

/// Ereignisse, mit denen eine Engine eine RUECKFRAGE meldet. Gemessen 2026-09-18: Grok
/// schreibt `permission_requested`/`permission_resolved`; Codex und Kimi benennen ihre
/// Freigabe-Ereignisse aehnlich, waren im Bestand aber nicht zu beobachten (die Laeufe
/// liefen auto-freigegeben). Darum Muster statt fester Namen — taucht so ein Ereignis auf,
/// wird es erkannt, ohne dass hier etwas nachgezogen werden muss.
private func waitingMarker(_ type: String) -> Bool? {
    let t = type.lowercased()
    guard t.contains("permission") || t.contains("approval") || t.contains("confirmation") else { return nil }
    if t.contains("request") { return true }
    if t.contains("resolved") || t.contains("response") || t.contains("granted")
        || t.contains("denied") || t.contains("answer") { return false }
    return nil
}

private let TURN_MARKERS: [Engine: (start: Set<String>, end: Set<String>)] = [
    .codex: (["task_started"], ["task_complete"]),
    .grok: (["turn_started"], ["turn_ended"]),
    .kimi: (["agent.turn.started"], ["agent.turn.ended"]),
]

/// Letztes Zug-Ereignis in einem JSONL-Ausschnitt. Von hinten gelesen, kaputte Zeilen
/// werden uebersprungen (der Ausschnitt beginnt in der Regel mitten in einer Zeile).
func lastTurnState(in text: String, engine: Engine) -> TurnState {
    guard let markers = TURN_MARKERS[engine] else { return .unknown }
    for line in text.split(separator: "\n", omittingEmptySubsequences: true).reversed() {
        guard let data = line.data(using: .utf8),
              let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { continue }
        var types: [String] = []
        if let t = obj["type"] as? String { types.append(t) }
        if let p = obj["payload"] as? [String: Any], let t = p["type"] as? String { types.append(t) }
        // Rueckfragen zuerst: sie stechen jeden anderen Zustand.
        for ty in types {
            if let isRequest = waitingMarker(ty) { return isRequest ? .waiting : .working }
        }
        if types.contains(where: { markers.end.contains($0) }) { return .done }
        if types.contains(where: { markers.start.contains($0) }) { return .working }
    }
    return .unknown
}

/// ponytail: liest nur das Dateiende — Rollout-Dateien werden mehrere MB gross, und das
/// letzte Zug-Ereignis steht immer hinten. Findet sich dort keins, greift die Zeit-Regel.
func lastTurnState(path: String, engine: Engine, tailBytes: Int = 262_144) -> TurnState {
    guard let text = tailString(path: path, bytes: tailBytes) else { return .unknown }
    return lastTurnState(in: text, engine: engine)
}

/// Notnagel, wenn kein Zug-Ereignis auffindbar ist (Plan Task 4): Ruhe ueber der Schwelle
/// UND kein Prozess-Signal. Bekannte Grenze: stilles Nachdenken kann fertig vortaeuschen.
func classifyLogReady(lastWrite: Date, now: Date, thresholdSec: Int, procActive: Bool) -> Bool {
    !procActive && now.timeIntervalSince(lastWrite) >= TimeInterval(thresholdSec)
}

// ---------- Datei-Helfer ----------

func tailString(path: String, bytes: Int) -> String? {
    guard let fh = FileHandle(forReadingAtPath: path) else { return nil }
    defer { try? fh.close() }
    guard let end = try? fh.seekToEnd() else { return nil }
    let start = end > UInt64(bytes) ? end - UInt64(bytes) : 0
    try? fh.seek(toOffset: start)
    guard let data = try? fh.readToEnd() else { return nil }
    return String(decoding: data, as: UTF8.self)
}

private let NAME_MARK = Data(#""custom-title""#.utf8)
private let TITLE_MARK = Data(#""ai-title""#.utf8)

/// Letzter eigener Name und Auto-Titel eines Transkripts, rueckwaerts in Bloecken gesucht. Claude schreibt beide
/// nur bei jeder Nachricht neu — nach einem langen Zug liegen sie bis ~5,5 MB vor dem Dateiende (gemessen
/// 2026-09-21 an 57 Transkripten), weit vor dem gelesenen Ausschnitt. `maxBytes` begrenzt die Suche.
func lastTitles(path: String, chunk: Int = 1 << 20, maxBytes: Int = 32 << 20) -> (name: String?, title: String?) {
    guard let fh = FileHandle(forReadingAtPath: path) else { return (nil, nil) }
    defer { try? fh.close() }
    guard let size = try? fh.seekToEnd() else { return (nil, nil) }
    let floor = size > UInt64(maxBytes) ? size - UInt64(maxBytes) : 0
    var pos = size
    var carry = Data()
    var name: String?, title: String?
    while pos > floor, name == nil || title == nil {
        let start = max(floor, pos > UInt64(chunk) ? pos - UInt64(chunk) : 0)
        try? fh.seek(toOffset: start)
        guard let block = try? fh.read(upToCount: Int(pos - start)) else { break }
        var data = block + carry
        pos = start
        // Die erste Zeile beginnt womoeglich im frueheren Block: sie wandert als Rest in die naechste Runde
        // (ohne Zeilenende ist der ganze Block Rest einer langen Zeile).
        if start > floor {
            guard let nl = data.firstIndex(of: 0x0A) else { carry = data; continue }
            carry = Data(data[..<nl])
            data = Data(data[data.index(after: nl)...])
        }
        // Nur Bloecke mit einer noch gesuchten Zeile zerlegen — alte Sessions ohne eigenen Namen kosteten
        // sonst beim ersten Lesen ~0,8 s (163-MB-Transkript, gemessen).
        guard (name == nil && data.range(of: NAME_MARK) != nil) || (title == nil && data.range(of: TITLE_MARK) != nil)
        else { continue }
        for line in String(decoding: data, as: UTF8.self).split(separator: "\n").reversed() where line.contains("-title\"") {
            guard let obj = (try? JSONSerialization.jsonObject(with: Data(line.utf8))) as? [String: Any] else { continue }
            if name == nil, obj["type"] as? String == "custom-title", let t = obj["customTitle"] as? String, !t.isEmpty { name = t }
            if title == nil, obj["type"] as? String == "ai-title", let t = obj["aiTitle"] as? String, !t.isEmpty { title = t }
        }
    }
    return (name, title)
}

func firstJSONLine(path: String, bytes: Int = 65_536) -> [String: Any]? {
    guard let fh = FileHandle(forReadingAtPath: path) else { return nil }
    defer { try? fh.close() }   // auch bei Lesefehler schliessen, sonst leckt der Deskriptor
    guard let data = try? fh.read(upToCount: bytes) else { return nil }
    guard let line = String(decoding: data, as: UTF8.self).split(separator: "\n").first,
          let d = line.data(using: .utf8) else { return nil }
    return (try? JSONSerialization.jsonObject(with: d)) as? [String: Any]
}

private func modified(_ path: String) -> Date {
    (try? FileManager.default.attributesOfItem(atPath: path))?[.modificationDate] as? Date ?? .distantPast
}

private func subdirectories(_ path: String) -> [String] {
    ((try? FileManager.default.contentsOfDirectory(atPath: path)) ?? [])
        .map { path + "/" + $0 }
        .filter { var d: ObjCBool = false
                  return FileManager.default.fileExists(atPath: $0, isDirectory: &d) && d.boolValue }
}

// ---------- Session-Quellen je Engine ----------

private func session(from engine: Engine, id: String, pid: pid_t? = nil, name: String, cwd: String,
                     path: String, thresholdSec: Int) -> Session {
    let state = lastTurnState(path: path, engine: engine)
    let ready: Bool
    var waiting = false
    switch state {
    case .done: ready = true
    case .waiting: ready = true; waiting = true
    case .working: ready = false
    case .unknown: ready = classifyLogReady(lastWrite: modified(path), now: Date(),
                                            thresholdSec: thresholdSec, procActive: false)
    }
    return Session(id: "\(engine.rawValue):\(id)", engine: engine, pid: pid,
                   name: name, cwd: cwd, ready: ready, waiting: waiting, alive: true)
}

/// Codex-/Grok-/Kimi-Sessions der LAUFENDEN Prozesse: je Engine und Arbeitsordner die neuesten Log-Dateien — so viele,
/// wie dort Prozesse laufen — ohne Altersgrenze. Bis 2026-09-25 galt "Datei juenger als 6 h" (Codex nur heute und
/// gestern): ein Zug, der abends fertig wurde, fiel ueber Nacht samt Fenster aus Rahmen, Schild und Anordnung (GLM+Kimi).
/// Ohne passenden Prozess wird nichts gelesen — das spart zugleich den Scan von ~700 Session-Ordnern alle 6 s.
/// Nur auf `workQueue` benutzen (Zustand ohne Sperre).
final class LogSessionFinder {
    private struct LogFile {
        let path: String
        let id: String
        let cwd: String
        let modified: Date
        /// Anlage der Session — sie entsteht beim Start ihres Prozesses (Zuordnung bei mehreren Prozessen je Ordner).
        var created: Date { (try? FileManager.default.attributesOfItem(atPath: path))?[.creationDate] as? Date ?? modified }
    }

    private let home: String
    private let startOf: (pid_t) -> Date?
    /// Arbeitsordner + Session-ID je Kopfdatei (Codex: erste Zeile, Kimi: state.json) — aendern sich nie.
    /// ponytail: waechst mit jeder je gesehenen Datei (heute ~800 kurze Eintraege); ausduennen, falls es je zaehlt.
    private var meta: [String: (cwd: String, id: String)] = [:]

    init(home: String = NSHomeDirectory(), startOf: @escaping (pid_t) -> Date? = processStartDate) {
        self.home = home
        self.startOf = startOf
    }

    func sessions(for procs: [EngineProc], thresholdSec: Int) -> [Session] {
        var out: [Session] = []
        for engine in [Engine.codex, .grok, .kimi] {
            let live = procs.filter { $0.engine == engine && !$0.cwd.isEmpty }
            guard !live.isEmpty else { continue }
            let started = Dictionary(live.map { ($0.pid, startOf($0.pid) ?? .distantPast) }, uniquingKeysWith: { a, _ in a })
            let perCwd = Dictionary(grouping: live, by: \.cwd)
            let files = logFiles(engine, cwds: Set(perCwd.keys), since: started.values.filter { $0 != .distantPast }.min())
            for (cwd, here) in perCwd {
                // Je Prozess eine Session, gepaart nach Alter: die zuerst angelegte gehoert zum zuerst gestarteten
                // Prozess (review quick 2026-09-25, GLM: sonst tauschten zwei Terminals im selben Ordner ihren Status).
                let newest = files.filter { $0.cwd == cwd }.sorted { $0.modified > $1.modified }.prefix(here.count)
                let byStart = here.sorted { started[$0.pid]! < started[$1.pid]! }
                for (f, p) in zip(newest.sorted { $0.created < $1.created }, byStart) {
                    out.append(session(from: engine, id: f.id, pid: p.pid, name: (cwd as NSString).lastPathComponent,
                                       cwd: cwd, path: f.path, thresholdSec: thresholdSec))
                }
            }
        }
        return out
    }

    private func logFiles(_ engine: Engine, cwds: Set<String>, since: Date?) -> [LogFile] {
        switch engine {
        case .codex: return codexFiles(cwds: cwds, since: since)
        case .grok: return grokFiles(cwds: cwds)
        case .kimi: return kimiFiles(cwds: cwds)
        case .claude: return []
        }
    }

    /// Kopfdaten einmal lesen und merken — nur mit Arbeitsordner: eine gerade angelegte, noch leere Datei wird beim
    /// naechsten Mal neu gelesen, statt fuer immer als "ohne Ordner" zu gelten.
    private func cachedMeta(_ path: String, read: () -> (cwd: String, id: String)) -> (cwd: String, id: String) {
        if let m = meta[path] { return m }
        let m = read()
        if !m.cwd.isEmpty { meta[path] = m }
        return m
    }

    /// Rollouts liegen nach Starttag in `sessions/JJJJ/MM/TT/`: gesucht wird vom Tag vor dem aeltesten Prozessstart
    /// (Zeitzonen-Rand) bis heute, hoechstens 60 Tage zurueck; ohne bekannten Start heute und gestern.
    private func codexFiles(cwds: Set<String>, since: Date?) -> [LogFile] {
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy/MM/dd"
        let now = Date()
        let back = min(60, max(1, Int(now.timeIntervalSince(since ?? now) / 86_400) + 1))
        var out: [LogFile] = []
        for day in 0...back {
            let dir = home + "/.codex/sessions/" + fmt.string(from: now.addingTimeInterval(-Double(day) * 86_400))
            for name in (try? FileManager.default.contentsOfDirectory(atPath: dir)) ?? [] where name.hasSuffix(".jsonl") {
                let path = dir + "/" + name
                let m = cachedMeta(path) {
                    let p = firstJSONLine(path: path)?["payload"] as? [String: Any] ?? [:]
                    return (p["cwd"] as? String ?? "", p["session_id"] as? String ?? name)
                }
                if cwds.contains(m.cwd) { out.append(LogFile(path: path, id: m.id, cwd: m.cwd, modified: modified(path))) }
            }
        }
        return out
    }

    /// Ordnername = prozent-kodierter Arbeitsordner, darin eine Session je Unterordner.
    private func grokFiles(cwds: Set<String>) -> [LogFile] {
        subdirectories(home + "/.grok/sessions").flatMap { dir -> [LogFile] in
            let cwd = (dir as NSString).lastPathComponent.removingPercentEncoding ?? ""
            guard cwds.contains(cwd) else { return [] }
            return subdirectories(dir).compactMap { s in
                let path = s + "/events.jsonl"
                guard FileManager.default.fileExists(atPath: path) else { return nil }
                return LogFile(path: path, id: (s as NSString).lastPathComponent, cwd: cwd, modified: modified(path))
            }
        }
    }

    /// Je Arbeitsordner ein `wd_…`-Ordner, darin eine Session je Unterordner; Ordner und ID stehen in `state.json`,
    /// die Zug-Ereignisse in `agents/main/wire.jsonl`.
    private func kimiFiles(cwds: Set<String>) -> [LogFile] {
        subdirectories(home + "/.kimi-code/sessions").flatMap(subdirectories).compactMap { s in
            let statePath = s + "/state.json"
            let m = cachedMeta(statePath) {
                let obj = FileManager.default.contents(atPath: statePath)
                    .flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any] ?? [:]
                return (obj["cwd"] as? String ?? "", obj["id"] as? String ?? (s as NSString).lastPathComponent)
            }
            guard cwds.contains(m.cwd) else { return nil }
            let wire = s + "/agents/main/wire.jsonl"
            let path = FileManager.default.fileExists(atPath: wire) ? wire : statePath
            return LogFile(path: path, id: m.id, cwd: m.cwd, modified: max(modified(statePath), modified(path)))
        }
    }
}

/// Fuehrt ein Kommando mit hartem Zeitlimit aus. Nil = Fehler oder Zeitueberschreitung. `accept`: Exit-Codes, die
/// kein Fehler sind (lsof endet mit 1, wenn nichts passt). Die Umgebung wird geerbt; `env` setzt, `unset` entfernt.
func runCommand(_ path: String, _ args: [String], env: [String: String] = [:], unset: [String] = [],
                timeout: TimeInterval = 5, accept: Set<Int32> = [0]) -> Data? {
    let task = Process()
    task.executableURL = URL(fileURLWithPath: path)
    task.arguments = args
    var environment = ProcessInfo.processInfo.environment
    unset.forEach { environment[$0] = nil }
    env.forEach { environment[$0] = $1 }
    task.environment = environment
    let out = Pipe()
    task.standardOutput = out
    // stderr nach /dev/null: eine Pipe, die niemand liest, laeuft nach 64 KB voll und
    // blockiert das Kind — wir werten stderr hier ohnehin nicht aus.
    task.standardError = FileHandle.nullDevice
    do {
        try task.run()
    } catch {
        log("Watcher: \(path) nicht startbar — \(error.localizedDescription)")
        return nil
    }
    // Das Zeitlimit MUSS am Prozess haengen, nicht am Lesen: `readDataToEndOfFile`
    // blockiert bis zum EOF, und EOF kommt erst, wenn der Prozess seine Pipe schliesst.
    // Ein haengendes Kind wuerde eine Pruefschleife nach dem Lesen nie erreichen.
    let watchdog = DispatchWorkItem {
        if task.isRunning {
            task.terminate()
            log("Watcher: \(path) \(args.joined(separator: " ")) ueberschritt \(Int(timeout))s — abgebrochen")
        }
    }
    DispatchQueue.global().asyncAfter(deadline: .now() + timeout, execute: watchdog)
    let data = out.fileHandleForReading.readDataToEndOfFile()
    task.waitUntilExit()
    watchdog.cancel()
    // Vom Zeitlimit beendet: nie ein Ergebnis, auch wenn die Signal-Nummer in `accept` stuende.
    return task.terminationReason == .exit && accept.contains(task.terminationStatus) ? data : nil
}

/// Die offizielle Abfrage fuer EIN Config-Dir (startet `claude`, ~0,25 s CPU, ~154 Log-Zeilen). Nil = nicht moeglich.
func fetchAgents(_ dir: String) -> [Session]? {
    guard let bin = claudeBinary() else {
        log("Watcher: kein claude-Binary gefunden")
        return nil
    }
    let e = agentsEnvironment(dir)
    return runCommand(bin, ["agents", "--json"], env: e.set, unset: e.unset).flatMap(agentsReply)
}

/// Umgebung fuer `claude agents` je Config-Dir: der Standardordner OHNE CLAUDE_CONFIG_DIR — mit der Variable legte claude
/// dort eine zweite Konfigurationsdatei an (`~/.claude/.claude.json`). Aktiv entfernt, denn Lookout erbt die Umgebung
/// seines Starters. Geprueft 2026-10-01: beide Wege liefern dieselben Session-IDs (2 Runden, je 13). Verglichen wird
/// nach aufgeloesten Verweisen — ein Symlink auf `~/.claude` ist der Standardordner (review quick 2026-10-01, GLM).
func agentsEnvironment(_ dir: String, home: String = NSHomeDirectory()) -> (set: [String: String], unset: [String]) {
    let real = { (path: String) in URL(fileURLWithPath: path).resolvingSymlinksInPath().path }
    return real(dir) == real(home + "/.claude") ? ([:], ["CLAUDE_CONFIG_DIR"]) : (["CLAUDE_CONFIG_DIR": dir], [])
}

/// Antwort von `claude agents --json` — nil statt leerer Liste, wenn sie nichts taugt: kein JSON-Array, oder Eintraege da,
/// aber keiner lesbar (Format geaendert?). Sonst hielte der Abgleich jede Datei-Session fuer unbekannt (Gegenrichtung),
/// und Lookout zeigte die leere Liste statt der Dateien.
func agentsReply(_ data: Data) -> [Session]? {
    guard let entries = (try? JSONSerialization.jsonObject(with: data)) as? [Any] else { return nil }
    let sessions = parseAgentsJson(data)
    return entries.isEmpty || !sessions.isEmpty ? sessions : nil
}

/// Lebt der Prozess? `kill(pid, 0)` antwortet fuer Prozesse anderer Nutzer (root) mit EPERM — die leben auch.
/// pid 0 und negative meinen bei `kill` ganze Prozessgruppen, nie eine Session.
func processAlive(_ pid: pid_t) -> Bool {
    pid > 0 && (kill(pid, 0) == 0 || errno == EPERM)
}

/// Claude-Sessions aller Konfigurationen ohne Dauer-Programmstarts (Kevin 2026-09-27, Absturz-Diagnose: Lookout startete
/// `claude agents --json` 13,4-mal je Minute, rund um die Uhr). Interaktive Sessions kommen aus den Status-Dateien
/// (`readSessionFiles`), die offizielle Abfrage laeuft nur alle 5 Min. (Kevin: "Alle 5 Min.") — fuer die Hintergrund-
/// Agenten ("Ohne Fenster") und als Abgleich mit den Dateien. Fehlen oder taugen die Dateien eines Config-Dirs nicht,
/// fragt Lookout dort wie frueher in jedem Session-Takt, bis der naechste Abgleich wieder stimmt. Nur auf `workQueue`.
final class ClaudeStatus {
    static let officialEvery: TimeInterval = 300
    /// Datei-Sessions, die kurz vor der Abfrage starteten, zaehlen im Gegenabgleich nicht — sie koennen danach begonnen
    /// haben.
    static let youngFile: TimeInterval = 60
    private let dirs: [String]
    private let agents: (String) -> [Session]?
    private var lastOfficial = Date.distantPast
    /// Hintergrund-Agenten je Config-Dir aus der letzten Abfrage (gescheitert = leer, sonst stuenden sie ewig da).
    private var background: [String: [Session]] = [:]
    /// Config-Dirs, deren Dateien beim Abgleich nicht taugten (`compare`) — dort gilt die alte Abfrage.
    private var distrusted: Set<String> = []
    /// Config-Dirs, in denen beim letzten Abgleich ein Zustand abwich.
    private var differed: Set<String> = []
    private var lastNote: [String: String] = [:]

    init(dirs: [String] = AppConfig().claudeConfigDirs, agents: @escaping (String) -> [Session]? = fetchAgents) {
        self.dirs = dirs
        self.agents = agents
    }

    func sessions(now: Date = Date()) -> [Session] {
        let official = now.timeIntervalSince(lastOfficial) >= Self.officialEvery
        if official { lastOfficial = now }
        var seen = Set<String>()
        var result: [Session] = []
        for dir in dirs where FileManager.default.fileExists(atPath: dir) {
            // Erst die Abfrage, dann die Dateien: endet eine Session dazwischen, fehlt sie nur in den Dateien — ihre pid
            // ist dann tot und zaehlt beim Abgleich nicht.
            var asked = official ? agents(dir) : nil
            var files = readSessionFiles(dir)
            if let asked, let got = files { compare(dir, asked: asked, files: got, askedAt: now) }
            let problem = files == nil ? "Status-Dateien fehlen oder unlesbar"
                : distrusted.contains(dir) ? "Status-Dateien weichen von claude agents ab" : nil
            note(dir, problem.map { "\($0) — alte Abfrage in jedem Takt" } ?? "Status aus den Dateien")
            if problem != nil {
                files = nil
                if asked == nil { asked = agents(dir) }
            }
            if official { background[dir] = asked?.filter { $0.pid == nil } ?? [] }
            let live = files ?? asked?.filter { $0.pid != nil } ?? []
            for s in live + (background[dir] ?? []) where seen.insert(s.id).inserted { result.append(s) }
        }
        return result
    }

    /// Abgleich Abfrage gegen Dateien, in beide Richtungen: fehlt in den Dateien eine Session, deren Prozess noch lebt,
    /// steht dort eine lebende, die `claude agents` nicht kennt (Kevin 2026-09-28, wie das Register nach review quick
    /// 27.09., Kimi — sonst fiele eine Abweichung dieser Art nie auf), oder weicht ein Zustand in zwei Abgleichen
    /// hintereinander ab (etwa ein umbenanntes Status-Feld — alle stuenden dauerhaft auf "arbeitet"; review quick
    /// 2026-09-27, GLM), gilt fuer dieses Config-Dir die alte Abfrage bis zum naechsten stimmigen Abgleich.
    /// Eine einzelne Zustands-Abweichung nur ins Log: die zwei Lesungen liegen Sekundenbruchteile auseinander, ein Wechsel
    /// dazwischen ist normal.
    private func compare(_ dir: String, asked: [Session], files: [Session], askedAt: Date) {
        let byId = Dictionary(files.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let listed = Set(asked.map(\.id))
        let running = asked.filter { $0.pid.map(processAlive) ?? false }
        let missing = running.filter { byId[$0.id] == nil }
        let extra = files.filter { f in
            !listed.contains(f.id) && (f.pid.map(processAlive) ?? false)
                && (f.startedAt.map { $0 < askedAt.addingTimeInterval(-Self.youngFile) } ?? true)
        }
        let differs = running.filter { a in byId[a.id].map { $0.ready != a.ready || $0.waiting != a.waiting } ?? false }
        let persistent = !differs.isEmpty && differed.contains(dir)
        if differs.isEmpty { differed.remove(dir) } else { differed.insert(dir) }
        if missing.isEmpty && extra.isEmpty && !persistent { distrusted.remove(dir) } else { distrusted.insert(dir) }
        if !missing.isEmpty || !extra.isEmpty || !differs.isEmpty {
            log("Abgleich \(short(dir)): fehlt in den Dateien \(missing.map(\.name)), nur in den Dateien "
                + "\(extra.map(\.name)), Zustand weicht ab \(differs.map(\.name))")
        }
    }

    /// Eine Zeile je Wechsel (und beim Start), nicht je Takt.
    private func note(_ dir: String, _ text: String) {
        guard lastNote[dir] != text else { return }
        lastNote[dir] = text
        log("Claude-Status \(short(dir)): \(text)")
    }

    private func short(_ dir: String) -> String { (dir as NSString).lastPathComponent }
}
