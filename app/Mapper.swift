// Lookout — Fenster-Mapper: welches Terminalfenster gehoert zu welcher Session?
//
// Der Spike (2026-09-18) hat gezeigt, dass Titel-Matching NICHT traegt: Claude schreibt
// seinen Aufgabentext in den Titel, Codex den Ordnernamen, Kimi den Prompt, unbenutzte
// Shells "zsh". Was traegt, ist der Titel-Ping: einen eindeutigen Marker als OSC-0-Titel
// auf das tty der Session schreiben, EINEN AX-Schnappschuss ziehen, Titel zurueckschreiben.
// Der Terminal-Emulator verarbeitet die Sequenz selbst — das laufende Programm sieht sie
// nicht. Live 8 von 9 Sessions in einem Durchgang zugeordnet; die neunte lag in einem
// Hintergrund-Tab und wurde korrekt als "nicht zuzuordnen" gemeldet.

import ApplicationServices
import Cocoa

struct TermWindow {
    let app: String
    let bundleId: String
    let ownerPid: pid_t      // Prozess der Terminal-App (Warp/Ghostty), nicht der Session
    let ax: AXUIElement
    var frame: CGRect          // Cocoa
    var title: String
    var sessions: [Session] = []
    let firstSeen: Date
    /// Seit wann ist dieses Fenster fertig? Bestimmt die Reihenfolge links in Fokus 2:
    /// wer zuerst fertig war, steht oben; wer spaeter fertig wird, stellt sich dahinter.
    var readySince: Date?
    var movable: Bool
    var minimized: Bool
    var fullscreen: Bool

    /// Session im SICHTBAREN Tab (Design Schilder 2026-09-21). Eine Session: diese. Mehrere
    /// (Tabs, typisch nach /handoff): die, deren Thema (`ai-title`) dem Fenstertitel ohne Zeichen
    /// gleicht; ohne eindeutigen Treffer keine.
    /// review quick 2026-09-21: ohne Claude-Zeichen im Titel ist die EINE Nicht-Claude-Session
    /// sichtbar (Codex/Grok/Kimi schreiben keine Zeichen, /handoff startet auch sie im Tab); und ein
    /// Themen-Treffer beweist nur etwas, wenn jede Claude-Session im Fenster ihr Thema kennt — sonst
    /// koennte die ohne Thema die sichtbare sein (GLM).
    var visibleSession: Session? {
        let live = sessions.filter(\.alive)
        guard live.count > 1 else { return live.first }
        let (glyph, text) = titleGlyph(title)
        guard glyph != .none else {
            let others = live.filter { $0.engine != .claude }
            return others.count == 1 ? others[0] : nil
        }
        let claude = live.filter { $0.engine == .claude }
        let known = claude.map(titleTexts)
        guard !known.contains(where: \.isEmpty) else { return nil }
        let hits = claude.indices.filter { known[$0].contains(text) }
        return hits.count == 1 ? claude[hits[0]] : nil
    }

    /// Zustand, den das Fenster ZEIGT. Bis 2026-09-21 galt "fertig, sobald irgendein Tab fertig
    /// ist" — nach jedem /handoff leuchtete das Fenster dann, obwohl der sichtbare Tab arbeitete
    /// (Kevin). Ohne erkennbaren Tab entscheidet das Zeichen im Titel, danach die alte Regel.
    private var shownState: (ready: Bool, waiting: Bool, watching: Bool) {
        if let v = visibleSession { return (v.ready, v.waiting, v.watching) }
        let live = sessions.filter(\.alive)
        guard live.count > 1 else { return (false, false, false) }
        switch titleGlyph(title).glyph {
        case .busy: return (false, false, false)
        case .idle:
            // ✳ = der sichtbare Tab arbeitet nicht. Ist keine Session fertig, aber eine ueberwacht, ist er diese
            // (review quick 2026-09-27, GLM) — sonst leuchtete ein Aufpasser-Tab doch wieder als fertig.
            let watchingOnly = !live.contains(where: \.ready) && live.contains(where: \.watching)
            return (!watchingOnly, live.contains(where: \.waiting), watchingOnly)
        case .none: return (live.contains(where: \.ready), live.contains(where: \.waiting), false)
        }
    }

    /// Fertig im sichtbaren Tab — Rahmen, Fokus-2-Seite, Menueleisten-Zahl haengen daran.
    var isReady: Bool { shownState.ready }
    /// Frage im sichtbaren Tab — hoechste Prioritaet (`attentionOrder`).
    var isWaiting: Bool { shownState.waiting }
    /// Wartet nur auf den eigenen Hintergrund-Aufpasser (Kevin 2026-09-27): kein Rahmen, bleibt bei den Laufenden.
    /// Frage und fertig gehen vor.
    var isWatching: Bool { !isReady && !isWaiting && shownState.watching }

    /// Eine Session wartet auf Kevin, aber das Fenster zeigt keine Frage — sie steckt in einem
    /// VERSTECKTEN Tab. Nur ein Hinweis auf dem Schild (Kevin 2026-09-21: fertige versteckte Tabs
    /// werden ignoriert).
    var hiddenQuestion: Bool { !isWaiting && sessions.contains { $0.alive && $0.waiting } }
    /// Layout-Slots bekommen nur Fenster, die sich auch bewegen lassen.
    var layoutable: Bool { movable && !minimized && !fullscreen }
}

/// Claude-Session, die Kevin gerade vor sich hat (Kevin 2026-10-04, `answerSeen`): der sichtbare Tab des fokussierten
/// Fensters, nur wenn dessen Terminal das vorderste Programm ist und das Fenster auf dem sichtbaren Schreibtisch liegt
/// (`onScreen`: minimiert oder anderer Space zaehlt nicht, review quick 2026-10-04, GLM). `title` = frisch gelesener
/// Fenstertitel; fehlt er, gilt nichts (der gemerkte koennte den Tab von vor 3 s zeigen). Auch bei EINER Session braucht
/// es den Nachweis im Titel — Claude-Zeichen und ihr Thema —, sonst stuende ein normaler Shell-Tab (Codex) oder der Tab
/// einer beendeten Session mit altem "✳ …" (GLM) daneben fuer sie; ohne bekanntes Thema gilt nichts. Nil = nichts gilt
/// als gesehen.
func sessionInFront(_ windows: [TermWindow], focused: AXUIElement?, title: String?, frontPid: pid_t?,
                    onScreen: [OnScreenWindow]?, screenH: CGFloat) -> String? {
    guard let focused, let title, var w = windows.first(where: { CFEqual($0.ax, focused) }), w.ownerPid == frontPid
    else { return nil }
    switch locateOnScreen(w, onScreen: onScreen, screenH: screenH) {
    case .unique, .ambiguous: break
    case .none, .unknown: return nil
    }
    w.title = title
    guard let s = w.visibleSession, s.engine == .claude else { return nil }
    let (glyph, text) = titleGlyph(w.title)
    let texts = titleTexts(s)
    return glyph != .none && texts.contains(text) ? s.id : nil
}

// ---------- Titel-Zeichen: was zeigt der sichtbare Tab? (Design Schilder 2026-09-21) ----------

/// Claude Code schreibt den Fenstertitel selbst: Kreisel (◐ und ◑ im Wechsel) = arbeitet,
/// ✳ = fertig oder Frage, dahinter das Thema (`ai-title`). Gemessen 2026-09-21: 12 Stichproben
/// × 8 Warp-Fenster, keine anderen Zeichen; 7 von 7 deckungsgleich mit `claude agents --json`.
enum TitleGlyph { case busy, idle, none }

func titleGlyph(_ title: String) -> (glyph: TitleGlyph, text: String) {
    let glyph: TitleGlyph
    switch title.unicodeScalars.first {
    case "\u{25D0}"?, "\u{25D1}"?: glyph = .busy
    case "\u{2733}"?: glyph = .idle
    default: return (.none, title.trimmingCharacters(in: .whitespaces))
    }
    // dropFirst auf Zeichen-Ebene nimmt ein angehaengtes Variantenzeichen (✳️) mit.
    return (glyph, String(title.dropFirst()).trimmingCharacters(in: .whitespaces))
}

/// Texte, die eine Claude-Session in den Fenstertitel schreibt: eigener Name (v4, Namens-Hook oder
/// /rename) und Auto-Titel — beide, weil der Titel nach einer Umbenennung kurz hinterherhinkt.
func titleTexts(_ s: Session) -> Set<String> {
    Set([s.digest?.name, s.digest?.title].compactMap { $0?.trimmingCharacters(in: .whitespaces) }
        .filter { !$0.isEmpty })
}

/// Fenster am Namen erkennen (v4, Kevin 2026-09-21: ein neues Fenster kam erst spaet in die Mitte).
/// Arbeitende Sessions uebermalen den Ping-Marker mit ihrem Kreisel-Titel ("ohne Marker", viermal in
/// Folge im Log) — ihr Titel verraet sie aber selbst. Nur eindeutige Paare: eine Claude-Session mit
/// bekanntem Namen/Auto-Titel und genau ein Fenster mit Claude-Zeichen, dessen Titeltext dazu passt
/// (ohne Treffer: Notweg fester Tab-Titel, im Code unten).
/// Rueckgabe: Session-pid -> Fensterindex. Mehrdeutiges bleibt dem Ping.
func titleMatches(sessions: [Session], windows: [TermWindow], procs all: [EngineProc]) -> [pid_t: Int] {
    // Nur Prozesse unter Warp/Ghostty und ihre Sessions (Plan R2-8): eine Session in einem fremden Terminal steht nie in
    // diesen Fenstern und blockierte sonst ohne Titel alle Namens-Treffer. Hier gefiltert, nicht beim Aufrufer — so
    // kommt der Fehler mit einer ungefilterten Liste nicht zurueck (review quick 2026-09-26, Kimi).
    let procs = all.filter(\.inTerminal)
    let pids = Set(procs.map(\.pid))
    let candidates = sessions.filter { $0.engine == .claude && $0.alive && $0.pid.map(pids.contains) == true }
    // Eindeutig ist ein Titel nur, wenn JEDE Claude-Session im Terminal bekannt ist: eine frische (noch nicht
    // abgefragt oder noch ohne Transkript) traegt womoeglich schon denselben Startnamen — gleicher Ordner,
    // gleicher Projektname. Eine falsche Zuordnung hielte, denn zugeordnete pids pingt niemand mehr (review quick v4).
    let known = Set(candidates.compactMap(\.pid))
    guard procs.allSatisfy({ $0.engine != .claude || known.contains($0.pid) }),
          candidates.allSatisfy({ !titleTexts($0).isEmpty }) else { return [:] }
    var out: [pid_t: Int] = [:]
    for s in candidates {
        let texts = titleTexts(s)
        // Traegt eine zweite Session denselben Namen, beweist der Titel nichts.
        guard !candidates.contains(where: { $0.id != s.id && !titleTexts($0).isDisjoint(with: texts) }) else { continue }
        var hits = windows.indices.filter { i in
            let (glyph, text) = titleGlyph(windows[i].title)
            return glyph != .none && texts.contains(text)
        }
        // Notweg (Kevin 2026-10-24): ein fest gesetzter Tab-Titel (Warp-Startvorlage mit `title:`) zeigt nie ein
        // Claude-Zeichen und schluckt den Ping-Marker. Dann zaehlt der eigene Name als GANZER Titel — der Auto-Titel
        // nicht, der gleicht zu leicht einem Ordnernamen (Codex).
        if hits.isEmpty, let name = s.digest?.name?.trimmingCharacters(in: .whitespaces), !name.isEmpty {
            hits = windows.indices.filter { i in
                let (glyph, text) = titleGlyph(windows[i].title)
                return glyph == .none && text == name
            }
        }
        if hits.count == 1, let pid = s.pid { out[pid] = hits[0] }
    }
    return out
}

struct EngineProc {
    let pid: pid_t
    let engine: Engine
    let tty: String       // z. B. "ttys004"
    let cwd: String
    /// Laeuft unter Warp oder Ghostty (Plan 1a)? Nur dann kann die Session in einem ihrer Fenster stehen — nur solche
    /// Prozesse werden angepingt und am Namen zugeordnet. Sessions in VS Code, iTerm oder Terminal.app bleiben gelistet
    /// ("Ohne Fenster"), ihr Tab-Titel bleibt aber in Ruhe (Review-Fund 8 vom 25.09.: er blinkte alle 2 min).
    var inTerminal = true
}

// ---------- Prozesse ----------

/// Arbeitsverzeichnis eines Prozesses ohne Subprozess (libproc).
func processCwd(_ pid: pid_t) -> String? {
    var info = proc_vnodepathinfo()
    let size = Int32(MemoryLayout<proc_vnodepathinfo>.size)
    let got = withUnsafeMutablePointer(to: &info) {
        proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, 0, UnsafeMutableRawPointer($0), size)
    }
    guard got == size else { return nil }
    return withUnsafeBytes(of: &info.pvi_cdir.vip_path) { raw in
        raw.baseAddress.map { String(cString: $0.assumingMemoryBound(to: CChar.self)) }
    }
}

/// Startzeit eines Prozesses per sysctl `KERN_PROC_PID`, wie `ps` — auch fuer root-Prozesse: libproc `proc_pidinfo`
/// liefert fuer die nichts (gemessen 27.09. an launchd, logd, WindowServer), eine liegengebliebene Status-Datei, deren pid
/// an einen root-Prozess ging, bestand so die Pruefung in `readSessionFiles`. Codex legt seine Session nach Starttag ab.
/// Nil = kein solcher Prozess.
func processStartDate(_ pid: pid_t) -> Date? {
    var info = kinfo_proc()
    var size = MemoryLayout<kinfo_proc>.stride
    var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
    // Fuer eine unbekannte pid meldet sysctl Erfolg mit Laenge 0.
    guard sysctl(&mib, u_int(mib.count), &info, &size, nil, 0) == 0, size > 0 else { return nil }
    let start = info.kp_proc.p_un.__p_starttime
    return Date(timeIntervalSince1970: TimeInterval(start.tv_sec) + TimeInterval(start.tv_usec) / 1_000_000)
}

/// Eine Zeile aus `ps -Ao pid=,ppid=,tty=,time=,comm=` — jede, nicht nur KI-CLIs: die Elternkarte braucht auch Shell
/// und Warp-Helfer (Plan 1a), die Rechenzeit misst die Hintergrund-Befehle (Plan 1e).
struct PsRow: Equatable {
    let pid: pid_t
    let ppid: pid_t
    let tty: String
    /// Rechenzeit (user + system) in Sekunden.
    let cpu: Double
    /// Programmname, wie ps ihn zeigt: Pfad, nackter Name oder ganze Befehlszeile (`npm exec …`).
    let comm: String
}

/// `ps` rueckt die Spalten mit mehreren Leerzeichen ein; der Name steht zuletzt und darf Leerzeichen enthalten. Der
/// Rest nach dem letzten Schnitt beginnt dabei mit Leerzeichen — bis 2026-09-18 war " claude" keine Engine, und die
/// Session stand "Ohne Fenster" (eine als nacktes `claude` gestartete Session, Abnahme 2026-09-18).
func parsePsLine(_ line: Substring) -> PsRow? {
    let parts = line.split(separator: " ", maxSplits: 4, omittingEmptySubsequences: true)
    guard parts.count == 5, let pid = pid_t(parts[0]), let ppid = pid_t(parts[1]) else { return nil }
    // Unlesbare Rechenzeit heisst 0 statt "Zeile verworfen": die Elternkarte braucht die Zeile trotzdem.
    return PsRow(pid: pid, ppid: ppid, tty: String(parts[2]), cpu: cpuSeconds(parts[3]) ?? 0,
                 comm: parts[4].trimmingCharacters(in: .whitespaces))
}

/// ps-Rechenzeit "337:28.94" (Minuten laufen ueber 60 hinaus), "1:02:03.5" oder "1-02:03:04" -> Sekunden.
func cpuSeconds(_ s: Substring) -> Double? {
    var days = 0.0
    var rest = s
    if let dash = s.firstIndex(of: "-") {
        guard let d = Double(s[..<dash]) else { return nil }
        days = d
        rest = s[s.index(after: dash)...]
    }
    var total = 0.0
    for part in rest.split(separator: ":") {
        guard let v = Double(part) else { return nil }
        total = total * 60 + v
    }
    return days * 86_400 + total
}

/// KI-CLI mit echtem Terminal? Nur die kann in einem Terminal-Fenster stehen.
func engine(of row: PsRow) -> Engine? {
    row.tty.hasPrefix("ttys") ? Engine(rawValue: (row.comm as NSString).lastPathComponent) : nil
}

/// Liegt einer der Prozesse `ancestors` ueber `pid` (der Prozess selbst zaehlt nicht)? Hoechstens 32 Stufen — eine
/// Schleife in der Elternkarte laeuft so nicht endlos.
func hasAncestor(_ pid: pid_t, in ancestors: Set<pid_t>, parents: [pid_t: pid_t]) -> Bool {
    var current = pid
    for _ in 0..<32 {
        guard let parent = parents[current], parent != current else { return false }
        if ancestors.contains(parent) { return true }
        current = parent
    }
    return false
}

/// Prozessliste aus der ps-Ausgabe (Plan 1a): erst die Elternkarte aus ALLEN Zeilen, dann die KI-CLIs mit Terminal, je
/// mit `inTerminal` — liegt ein laufender Warp- oder Ghostty-Prozess (`terminalPids`) in ihrer Ahnenkette? Live
/// 2026-09-26: `claude ← -zsh ← Warp-Helfer ← Warp ← launchd`. Nil = nicht leer, aber keine einzige Zeile lesbar (R2-8):
/// "unbekannt", nicht "keine Prozesse" — eine leere Liste loeschte jede Fenster-Zuordnung.
func processList(_ text: String, terminalPids: Set<pid_t>, cwd: (pid_t) -> String = { processCwd($0) ?? "" })
    -> (procs: [EngineProc], rows: [PsRow])? {
    let rows = text.split(separator: "\n").compactMap(parsePsLine)
    guard !rows.isEmpty || text.allSatisfy(\.isWhitespace) else { return nil }
    let parents = Dictionary(rows.map { ($0.pid, $0.ppid) }, uniquingKeysWith: { first, _ in first })
    let procs = rows.compactMap { r in
        engine(of: r).map {
            EngineProc(pid: r.pid, engine: $0, tty: r.tty, cwd: cwd(r.pid),
                       inTerminal: hasAncestor(r.pid, in: terminalPids, parents: parents))
        }
    }
    return (procs, rows)
}

/// Alle Prozesse, einmal `ps` (~10 ms — vertretbar je Tick): die KI-CLIs mit Terminal fuer Sessions und Log-Finder,
/// dazu jede Zeile (Elternkarte, Rechenzeit). Nil = nicht lesbar (Zeitlimit, Fehler): "unbekannt", nicht "keine
/// Prozesse" — eine leere Liste loeschte jede Fenster-Zuordnung, danach wurde alles neu angepingt (6x nachts im Log,
/// Review 2026-09-25 Fund 10).
func engineProcesses(psPath: String = "/bin/ps",
                     terminalPids: Set<pid_t> = Set(runningTerminals().map { $0.app.processIdentifier }))
    -> (procs: [EngineProc], rows: [PsRow])? {
    guard let data = runCommand(psPath, ["-Ao", "pid=,ppid=,tty=,time=,comm="], timeout: 3),
          let text = String(data: data, encoding: .utf8) else { return nil }
    return processList(text, terminalPids: terminalPids)
}

/// Startargumente eines Prozesses (argv, wie gestartet), nil = nicht lesbar. `sysctl(KERN_PROCARGS2)`: erst argc, dann der
/// Programmpfad, Fuellbytes, danach die Argumente, je mit NUL beendet.
func processArguments(_ pid: pid_t) -> [String]? {
    var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
    var size = 0
    guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > MemoryLayout<Int32>.size else { return nil }
    var buf = [UInt8](repeating: 0, count: size)
    guard sysctl(&mib, 3, &buf, &size, nil, 0) == 0, size > MemoryLayout<Int32>.size else { return nil }
    let argc = Int(buf.withUnsafeBytes { $0.loadUnaligned(as: Int32.self) })
    var i = MemoryLayout<Int32>.size
    while i < size, buf[i] != 0 { i += 1 }          // Programmpfad
    while i < size, buf[i] == 0 { i += 1 }          // Fuellbytes
    var args: [String] = []
    while args.count < argc, i < size {
        let start = i
        while i < size, buf[i] != 0 { i += 1 }
        args.append(String(decoding: buf[start..<i], as: UTF8.self))
        i += 1
    }
    return args.count == argc ? args : nil
}

// ---------- Fenster ----------

/// Alle Fenster der bekannten Terminals. NUR auf `axQueue` aufrufen.
/// `firstSeenLookup` liefert den Erstsichtungs-Zeitpunkt eines bereits bekannten Fensters —
/// die Slot-Reihenfolge haengt daran. Nachschlagen per CFEqual, NICHT ueber ein Dictionary:
/// `AXUIElement as AnyObject` erzeugt je Aufruf eine neue Bruecke, deren Identitaet wechselt.
func collectWindows(firstSeenLookup: (AXUIElement) -> Date? = { _ in nil }) -> [TermWindow] {
    var out: [TermWindow] = []
    for (app, bid) in runningTerminals() {
        let origin = axOrigin(for: bid)
        for win in axWindows(pid: app.processIdentifier) {
            let frame = readBackFrame(win, origin: origin) ?? .zero
            out.append(TermWindow(
                app: app.localizedName ?? bid,
                bundleId: bid,
                ownerPid: app.processIdentifier,
                ax: win,
                frame: frame,
                title: axString(win, kAXTitleAttribute as String) ?? "",
                firstSeen: firstSeenLookup(win) ?? Date(),
                movable: axSettable(win, kAXPositionAttribute as String)
                    && axSettable(win, kAXSizeAttribute as String),
                minimized: axBool(win, kAXMinimizedAttribute as String) ?? false,
                fullscreen: axBool(win, "AXFullScreen") ?? false))
        }
    }
    return out
}

func indexOfWindow(_ needle: AXUIElement, in windows: [TermWindow]) -> Int? {
    windows.firstIndex { CFEqual($0.ax, needle) }
}

/// Schnappschuss, waehrend dessen Lookout selbst bewegt hat: seine Rahmen sind Zwischenstaende. Die zuletzt gemessenen
/// (`known`) gelten weiter — sonst sprang ein gerade angekommenes Fenster zurueck und glitt neu los (2026-09-25).
/// Ebenso "minimiert": `bringToFront` holt ein Fenster aus dem Dock, ein schon laufender Tick kennt es noch minimiert
/// (Plan R2-4). Stimmt das Bekannte nicht, heilt es der naechste Tick.
func keepMeasuredFrames(_ fresh: [TermWindow], known: [TermWindow]) -> [TermWindow] {
    fresh.map { w in
        var w = w
        if let old = known.first(where: { CFEqual($0.ax, w.ax) }) {
            w.frame = old.frame
            w.minimized = old.minimized
        }
        return w
    }
}

/// Ein Fenster frisch lesen (Rahmen, minimiert, Vollbild, Titel) — nach `bringToFront` (Plan 1c, R2-4). NUR auf `axQueue`.
func rereadWindow(_ w: TermWindow) -> TermWindow {
    var w = w
    w.frame = readBackFrame(w.ax, origin: axOrigin(for: w.bundleId)) ?? w.frame
    w.minimized = axBool(w.ax, kAXMinimizedAttribute as String) ?? false
    w.fullscreen = axBool(w.ax, "AXFullScreen") ?? false
    w.title = axString(w.ax, kAXTitleAttribute as String) ?? w.title
    return w
}

// ---------- Titel-Ping ----------

private let PING_MARKER = "wg-map-"

@discardableResult
func writeOSCTitle(tty: String, title: String) -> Bool {
    let fd = open("/dev/" + tty, O_WRONLY | O_NONBLOCK)
    guard fd >= 0 else { return false }
    defer { close(fd) }
    let seq = "\u{1B}]0;\(title)\u{07}"
    return seq.withCString { write(fd, $0, strlen($0)) > 0 }
}

// ---------- Wann wird gepingt? (Design v3, Fehler 1) ----------

/// Ping-Buchfuehrung je Prozess.
struct PingState: Equatable {
    var attempts = 0
    var lastTry = Date.distantPast
}

/// Abstand bis zum naechsten Versuch: die ersten vier im 15-s-Takt, danach alle 2 Minuten —
/// nie endgueltig aufgeben. v2 gab nach 4 Fehlversuchen auf und verlor so jede Session, deren
/// Spinner den Marker gerade ueberschrieb. Der lange Takt schont Hintergrund-Tabs: deren
/// Tab-Titel blinkt bei jedem Versuch kurz auf.
func pingRetryInterval(attempts: Int) -> TimeInterval { attempts < 4 ? 15 : 120 }

/// Wer wird jetzt gepingt? Prozesse ohne Fenster, bis es klappt. Taucht ein NEUES Fenster auf,
/// jeder, und zwar sofort: ein herausgezogener Tab laesst sein altes Fenster stehen, die alte
/// Zuordnung galt sonst ewig weiter (Kevin 2026-09-18, Fenster #10192 blieb ungeordnet). Die
/// Zahl der Fehlversuche bleibt dabei stehen — ein Hintergrund-Tab faellt danach in seinen
/// langen Takt zurueck, statt wieder im 15-s-Takt zu blinken. Verschwundene Prozesse fallen raus.
/// Ohne Bedienungshilfen (`trusted` false) ist kein Titel lesbar — dann wird nur vorgemerkt:
/// jeder Versuch schriebe sonst Marker und Ordnernamen in Kevins Tab-Titel. Dasselbe, wenn AX kein einziges
/// Terminal-Fenster liefert (`windowsReadable` false, gesperrter Bildschirm): bis 2026-09-25 pingte Lookout dann alle
/// 2 min jede Session und setzte 15.696-mal einen Tab-Titel auf den Ordnernamen.
func pingSchedule(pids: [pid_t], mapped: Set<pid_t>, pending: [pid_t: PingState], newWindow: Bool, now: Date,
                  trusted: Bool = true, windowsReadable: Bool = true) -> (due: Set<pid_t>, pending: [pid_t: PingState]) {
    let live = Set(pids)
    var next = pending.filter { live.contains($0.key) }
    for pid in pids where newWindow || (!mapped.contains(pid) && next[pid] == nil) {
        next[pid] = PingState(attempts: next[pid]?.attempts ?? 0, lastTry: .distantPast)
    }
    guard trusted, windowsReadable else { return ([], next) }
    let due = next.filter { now.timeIntervalSince($0.value.lastTry) >= pingRetryInterval(attempts: $0.value.attempts) }
    return (Set(due.keys), next)
}

/// Nach einer Ping-Runde: gefunden -> Ruhe, sonst ein Fehlversuch mehr.
func pingResult(due: Set<pid_t>, found: Set<pid_t>, pending: [pid_t: PingState],
                now: Date) -> [pid_t: PingState] {
    var next = pending
    for pid in due {
        next[pid] = found.contains(pid) ? nil : PingState(attempts: (pending[pid]?.attempts ?? 0) + 1, lastTry: now)
    }
    return next
}

/// Ordnet Prozesse ihren Fenstern zu und PFLEGT die bestehende Zuordnung fort.
///
/// Lehren aus den Live-Laeufen (2026-09-18, Kevin):
/// 1. Eine Ping-Runde, die nur einen Teil trifft, darf NICHT die ganze Karte ersetzen —
///    sonst springt die Zuordnung zwischen 12/12 und 1/12, und die Fenster bleiben liegen.
///    Bekannte Paare bleiben gueltig, solange Prozess UND Fenster noch existieren; wird ein
///    bekanntes Paar neu geprueft und sein Marker nicht gesehen, bleibt es als Notnagel stehen.
/// 2. Eine arbeitende Claude-Session malt ihren Spinner-Titel laufend neu und ueberschreibt
///    den Marker. Darum wird er in JEDER Runde neu geschrieben — und danach werden NUR die
///    Titel gelesen, alle 10 ms bis zu 150 ms lang. Ein voller Schnappschuss (7 AX-Abfragen je
///    Fenster) dauerte laenger, als der Marker lebte.
///
/// Gepingt werden nur die Prozesse in `ping` (siehe `pingSchedule`) — eine eingeschwungene
/// Zuordnung laesst die Titel voellig in Ruhe.
func pingMap(_ procs: [EngineProc], known: [pid_t: AXUIElement],
             ping: Set<pid_t>) -> (map: [pid_t: AXUIElement], found: Set<pid_t>) {
    let before = axQueue.sync { collectWindows() }
    let livePids = Set(procs.map(\.pid))
    var result = known.filter { pid, win in
        livePids.contains(pid) && before.contains { CFEqual($0.ax, win) }
    }
    let pinged = procs.filter { ping.contains($0.pid) }
    guard !pinged.isEmpty else { return (result, []) }

    // Titel zurueck. Ohne bekanntes Original (Hintergrund-Tab) faellt der Titel auf den
    // Ordnernamen zurueck — das ist die uebliche Terminal-Benennung und nie ein leerer Tab.
    func restore(_ p: EngineProc, _ original: String?) {
        let fallback = p.cwd.isEmpty ? p.engine.rawValue : (p.cwd as NSString).lastPathComponent
        writeOSCTitle(tty: p.tty, title: original ?? fallback)
    }

    var open = pinged
    var found = Set<pid_t>()
    for _ in 0..<4 {
        for p in open { writeOSCTitle(tty: p.tty, title: PING_MARKER + String(p.pid)) }
        let deadline = Date().addingTimeInterval(0.15)
        repeat {
            usleep(10_000)
            let titles = axQueue.sync { before.map { ($0, axString($0.ax, kAXTitleAttribute as String) ?? "") } }
            for (w, title) in titles where title.hasPrefix(PING_MARKER) {
                guard let pid = pid_t(title.dropFirst(PING_MARKER.count)),
                      let proc = open.first(where: { $0.pid == pid }), !found.contains(pid) else { continue }
                result[pid] = w.ax
                found.insert(pid)
                // Sofort zuruecksetzen, nicht erst nach allen Runden: sonst zeigt ein sichtbares
                // Fenster den Marker, solange irgendein Hintergrund-Tab gesucht wird (review quick).
                restore(proc, w.title.hasPrefix(PING_MARKER) ? nil : w.title)
            }
            open = open.filter { !found.contains($0.pid) }
        } while !open.isEmpty && Date() < deadline
        if open.isEmpty { break }
    }
    for p in open { restore(p, nil) }
    // Zuordnen laesst sich nur, was unter Warp/Ghostty laeuft — fremde Terminals zaehlen nicht mit (Plan 1a).
    log("Mapper: \(found.count)/\(pinged.count) gepingt, \(result.count)/\(procs.filter(\.inTerminal).count) Sessions zugeordnet"
        + (open.isEmpty ? "" : " (\(open.count) ohne Marker — Hintergrund-Tab oder Spinner, neuer Versuch folgt)"))
    return (result, found)
}

// ---------- Zusammenfuehren ----------

/// Haengt jeder Session ihren Prozess an (Claude ueber die pid, die anderen ueber den
/// Arbeitsordner) und verwirft Sessions ohne laufenden Prozess — deren Log-Datei liegt
/// nur noch herum (Spike: 11 Log-Sessions, davon die meisten laengst beendet).
func attachProcesses(_ sessions: [Session], _ procs: [EngineProc]) -> [Session] {
    var used = Set<pid_t>()
    return sessions.compactMap { s -> Session? in
        if let pid = s.pid {
            guard procs.contains(where: { $0.pid == pid }) else { return nil }
            used.insert(pid)
            return s
        }
        // Claude-Hintergrundagenten haben kein Terminal und kein Fenster. Sie ueber den
        // Arbeitsordner an irgendeinen Prozess zu haengen, waere geraten — sie bleiben
        // ohne pid in der Liste und damit ausserhalb jeder Fenster-Zuordnung.
        if s.engine == .claude { return s }
        guard !s.cwd.isEmpty,
              let match = procs.first(where: { $0.engine == s.engine && $0.cwd == s.cwd && !used.contains($0.pid) })
        else { return nil }
        used.insert(match.pid)
        return Session(id: s.id, engine: s.engine, pid: match.pid, name: s.name,
                       cwd: s.cwd, ready: s.ready, waiting: s.waiting, alive: true)
    }
}

/// Traegt die Sessions in die Fenster ein. Fenster ohne Session bleiben unberuehrt
/// (Design: nur KI-Fenster werden bewegt).
func matchSessions(_ windows: inout [TermWindow], _ sessions: [Session],
                   pidWindows: [pid_t: AXUIElement]) {
    for i in windows.indices { windows[i].sessions = [] }
    for s in sessions {
        guard let pid = s.pid, let ax = pidWindows[pid],
              let idx = indexOfWindow(ax, in: windows) else { continue }
        windows[idx].sessions.append(s)
    }
}
