// Lookout — Gemeinsame Grundlagen: Konfiguration, Logging, AX-Zugriff, Koordinaten.
//
// Koordinaten-Konvention (Plan, Konvention 1): intern wird IMMER in Cocoa gerechnet
// (Ursprung unten-links, y nach oben). Umgerechnet wird ausschliesslich an den
// AX-Raendern — beim Schreiben (cocoaToAX) und beim Lesen (axToCocoa).

import ApplicationServices
import Cocoa

// ---------- Terminals ----------

let WARP_BID = "dev.warp.Warp-Stable"
let GHOSTTY_BID = "com.mitchellh.ghostty"
let TERMINAL_BIDS = [WARP_BID, GHOSTTY_BID]

/// AX-Ursprung je App. Spike 2026-09-18: beide Terminals liefern oben-links
/// (gegen CGWindowList gegengeprueft).
let AX_ORIGINS: [String: AXOrigin] = [WARP_BID: .topLeft, GHOSTTY_BID: .topLeft]

func axOrigin(for bundleId: String) -> AXOrigin { AX_ORIGINS[bundleId] ?? .topLeft }

/// Kleinste Fensterbreite, die Warp zulaesst (Spike: 480×192; Ghostty 90×48).
let MIN_CARD_W: CGFloat = 480
let MIN_CARD_H: CGFloat = 192

// ---------- Konfiguration (~/.config/lookout.json) ----------

struct AppConfig {
    // cols/rows/command gehoeren zur alten Einmal-Starter-App (warp_grid.swift) und werden
    // hier nicht mehr gelesen — "Grid starten" ist raus (Kevin 2026-09-18). `waitingRoom`
    // ebenso: das Wartezimmer ist durch die Ziel-Knoepfe ersetzt (Kevin 2026-09-18).
    /// Start-Anordnung, solange im Menue noch nichts gewaehlt wurde (danach UserDefaults).
    var layout = "reading"
    var stackSide = "right"
    var readyThresholdSec = 90
    var pollSec = 3
    /// `claude agents --json` kostet ~0,25 s CPU je Aufruf (gemessen 2026-09-18, zwei
    /// Config-Dirs) — deshalb seltener als der billige AX-Tick.
    var sessionPollSec = 6
    var hotkeys = true
    /// Ready-Anzeige: Farbe als Hex. Default Mint — "fertig", nicht "Alarm" (Kevin 2026-09-18).
    var glow = true
    var glowColor = "#5ECFA0"
    /// Eigener Ton fuer "wartet auf deine Antwort" — warmes Bernstein, kein Alarmrot.
    var glowColorWaiting = "#E8B25C"
    /// Schilder in den Fenstern (Design 2026-09-21). Nur Startwert — danach gilt der Menuepunkt.
    var labels = true
    /// Oberflaeche "de" oder "en"; nil = Sprache des Macs (`prefersGerman`).
    var language: String?
    /// Claude-Konfigurationen, deren Sessions Lookout zeigt (je eine fuer `CLAUDE_CONFIG_DIR`); fehlende werden
    /// uebersprungen.
    var claudeConfigDirs = [NSHomeDirectory() + "/.claude"]
}

/// Hex -> NSColor. Unlesbares faellt auf Mint zurueck, nie auf Rot.
func colorFromHex(_ hex: String) -> NSColor {
    var s = hex.trimmingCharacters(in: .whitespaces)
    if s.hasPrefix("#") { s.removeFirst() }
    guard s.count == 6, let v = UInt32(s, radix: 16) else {
        return NSColor(srgbRed: 0.369, green: 0.812, blue: 0.627, alpha: 1)
    }
    return NSColor(srgbRed: CGFloat((v >> 16) & 0xFF) / 255,
                   green: CGFloat((v >> 8) & 0xFF) / 255,
                   blue: CGFloat(v & 0xFF) / 255, alpha: 1)
}

func loadConfig() -> AppConfig {
    let path = (NSHomeDirectory() as NSString).appendingPathComponent(".config/lookout.json")
    guard let data = FileManager.default.contents(atPath: path),
          let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return AppConfig() }
    return parseConfig(obj)
}

/// Einstellungen aus dem JSON-Objekt der Konfigurationsdatei. Fehlendes und Ungueltiges laesst den Standardwert stehen.
func parseConfig(_ obj: [String: Any]) -> AppConfig {
    var c = AppConfig()
    if let v = obj["layout"] as? String { c.layout = v }
    if let v = obj["stackSide"] as? String, v == "left" || v == "right" { c.stackSide = v }
    if let v = obj["readyThresholdSec"] as? Int, v > 0 { c.readyThresholdSec = v }
    if let v = obj["pollSec"] as? Int, v > 0 { c.pollSec = v }
    if let v = obj["sessionPollSec"] as? Int, v > 0 { c.sessionPollSec = v }
    if let v = obj["hotkeys"] as? Bool { c.hotkeys = v }
    if let v = obj["glow"] as? Bool { c.glow = v }
    if let v = obj["glowColor"] as? String { c.glowColor = v }
    if let v = obj["glowColorWaiting"] as? String { c.glowColorWaiting = v }
    if let v = obj["labels"] as? Bool { c.labels = v }
    if let v = obj["language"] as? String, v == "de" || v == "en" { c.language = v }
    // `~` aufgeloest, "~/.claude/" wie "~/.claude": am Standardordner haengt, ob `fetchAgents` ohne
    // CLAUDE_CONFIG_DIR fragt. Leere und relative Eintraege sind kein Ordner und fallen raus (review quick, Kimi).
    let dirs = (obj["claudeConfigDirs"] as? [Any] ?? []).compactMap { $0 as? String }
        .map { ($0.trimmingCharacters(in: .whitespaces) as NSString).standardizingPath }
        .filter { $0.hasPrefix("/") }
    if !dirs.isEmpty { c.claudeConfigDirs = dirs }
    return c
}

// ---------- Sprache (Kevin 26.09.: Deutsch und Englisch automatisch) ----------

/// Oberflaeche deutsch? Beim Start gesetzt (`prefersGerman`), die Tests nageln es fest. Das Log bleibt deutsch.
var uiGerman = false

/// Text der Oberflaeche in ihrer Sprache.
func tr(_ de: String, _ en: String) -> String { uiGerman ? de : en }

/// `language` aus der Konfiguration hat Vorrang vor der Mac-Sprache (GP 14); ohne beides englisch.
func prefersGerman(language: String?, preferred: [String]) -> Bool {
    language.map { $0 == "de" } ?? (preferred.first?.hasPrefix("de") ?? false)
}

// ---------- Logging ----------

/// `LOOKOUT_LOG` setzt nur `build.sh --tests`: sonst standen die Probezeilen der Tests ("Abgleich …") zwischen den
/// echten im Log (2026-09-27).
let logPath = ProcessInfo.processInfo.environment["LOOKOUT_LOG"]
    ?? (NSHomeDirectory() as NSString).appendingPathComponent("Library/Logs/Lookout.log")
/// Ab dieser Groesse wandert das Log nach `Lookout.log.1` (eine Generation zum Nachsehen reicht; ~4 Wochen).
let LOG_ROTATE_BYTES: UInt64 = 5 << 20
/// Mit Datum (review 2026-09-25): ohne sah eine Endlosschleife aus der Zeit vor v3 aktuell aus.
private let logFormatter: DateFormatter = {
    let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd HH:mm:ss"; return f
}()
private let logLock = NSLock()

func log(_ s: String) {
    logLock.lock(); defer { logLock.unlock() }
    appendLog(s, to: logPath, rotateAt: LOG_ROTATE_BYTES)
}

/// Eine Zeile anhaengen; ist die Datei schon `rotateAt` Bytes gross, beginnt vorher eine neue (die alte wird `.1`).
/// Scheitert das Umbenennen, wird an die alte weiter angehaengt — nie ueberschrieben (review quick 2026-09-25).
func appendLog(_ message: String, to path: String, rotateAt: UInt64) {
    let line = logFormatter.string(from: Date()) + "  " + message + "\n"
    guard let data = line.data(using: .utf8) else { return }
    let fm = FileManager.default
    if let size = (try? fm.attributesOfItem(atPath: path))?[.size] as? UInt64, size >= rotateAt {
        try? fm.removeItem(atPath: path + ".1")
        try? fm.moveItem(atPath: path, toPath: path + ".1")
    }
    if let fh = FileHandle(forWritingAtPath: path) {
        fh.seekToEndOfFile()
        fh.write(data)
        try? fh.close()
    } else {
        try? line.write(toFile: path, atomically: true, encoding: .utf8)
    }
}

// ---------- Queues (Plan, Konvention 3) ----------

/// NUR AX-Aufrufe, jeder einzelne kurz. AXObserver-Callbacks reihen hier ein, rufen selbst kein AX.
let axQueue = DispatchQueue(label: "lookout.ax")
/// Prozess-Starts (claude agents, ps) und Log-I/O.
let workQueue = DispatchQueue(label: "lookout.work")

// ---------- Koordinaten ----------

enum AXOrigin { case cocoaBottomLeft, topLeft }

// Bildschirm-Masse werden GECACHT: NSScreen ist AppKit und gehoert dem Main-Thread,
// gelesen wird aber aus axQueue/workQueue (Koordinaten-Umrechnung, Kachelung).
// `refreshScreenMetrics()` laeuft auf main, alle anderen lesen nur den Cache.
private let screenMetricsLock = NSLock()
private var cachedPrimaryHeight: CGFloat = 0
private var cachedPrimaryVisible = CGRect.zero

/// NUR auf dem Main-Thread aufrufen (Start, je Tick, bei Bildschirm-Aenderung).
func refreshScreenMetrics() {
    let primary = NSScreen.screens.first { $0.frame.origin == .zero } ?? NSScreen.screens.first
    let h = primary?.frame.height ?? 0
    let vis = primary?.visibleFrame ?? .zero
    screenMetricsLock.lock()
    cachedPrimaryHeight = h
    cachedPrimaryVisible = vis
    screenMetricsLock.unlock()
}

/// Hoehe des Primaerbildschirms — Bezugspunkt der AX/CG-Koordinaten (obere linke Ecke).
/// Primaer = der Schirm, dessen Cocoa-Frame bei (0,0) beginnt.
func primaryScreenHeight() -> CGFloat {
    screenMetricsLock.lock(); defer { screenMetricsLock.unlock() }
    return cachedPrimaryHeight
}

/// Nutzbare Flaeche des Primaerbildschirms (ohne Menueleiste/Dock), threadsicher gelesen.
func primaryVisibleFrame() -> CGRect {
    screenMetricsLock.lock(); defer { screenMetricsLock.unlock() }
    return cachedPrimaryVisible
}

/// Cocoa (unten-links) -> AX. Mit `screenH` testbar ohne Fensterserver.
func cocoaToAX(_ r: CGRect, _ o: AXOrigin, screenH: CGFloat) -> CGRect {
    guard o == .topLeft else { return r }
    return CGRect(x: r.minX, y: screenH - r.maxY, width: r.width, height: r.height)
}

/// AX -> Cocoa (unten-links). Gleiche Spiegelung, daher selbstinvers.
func axToCocoa(_ r: CGRect, _ o: AXOrigin, screenH: CGFloat) -> CGRect {
    guard o == .topLeft else { return r }
    return CGRect(x: r.minX, y: screenH - r.maxY, width: r.width, height: r.height)
}

func cocoaToAX(_ r: CGRect, _ o: AXOrigin) -> CGRect { cocoaToAX(r, o, screenH: primaryScreenHeight()) }
func axToCocoa(_ r: CGRect, _ o: AXOrigin) -> CGRect { axToCocoa(r, o, screenH: primaryScreenHeight()) }

// ---------- AX-Zugriff ----------

func axRaw(_ el: AXUIElement, _ attr: String) -> CFTypeRef? {
    var v: CFTypeRef?
    guard AXUIElementCopyAttributeValue(el, attr as CFString, &v) == .success else { return nil }
    return v
}

func axPoint(_ el: AXUIElement, _ attr: String) -> CGPoint? {
    guard let raw = axRaw(el, attr), CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
    var p = CGPoint.zero
    guard AXValueGetValue(raw as! AXValue, .cgPoint, &p) else { return nil }
    return p
}

func axSize(_ el: AXUIElement, _ attr: String) -> CGSize? {
    guard let raw = axRaw(el, attr), CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
    var s = CGSize.zero
    guard AXValueGetValue(raw as! AXValue, .cgSize, &s) else { return nil }
    return s
}

func axString(_ el: AXUIElement, _ attr: String) -> String? { axRaw(el, attr) as? String }
func axBool(_ el: AXUIElement, _ attr: String) -> Bool? { axRaw(el, attr) as? Bool }

func axSettable(_ el: AXUIElement, _ attr: String) -> Bool {
    var b = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(el, attr as CFString, &b) == .success && b.boolValue
}

/// AX-Fenster einer App. Nur auf `axQueue` aufrufen.
func axWindows(pid: pid_t) -> [AXUIElement] {
    (axRaw(AXUIElementCreateApplication(pid), kAXWindowsAttribute as String) as? [AXUIElement]) ?? []
}

/// Ist-Rahmen in AX-Koordinaten (oben links).
func axFrame(_ win: AXUIElement) -> CGRect? {
    guard let p = axPoint(win, kAXPositionAttribute as String),
          let s = axSize(win, kAXSizeAttribute as String) else { return nil }
    return CGRect(origin: p, size: s)
}

/// Ist-Frame eines Fensters in Cocoa-Koordinaten.
func readBackFrame(_ win: AXUIElement, origin: AXOrigin) -> CGRect? {
    axFrame(win).map { axToCocoa($0, origin) }
}

// ---------- Schreibfolge beim Umsetzen (Design v3, Fehler 2) ----------
//
// Gemessen 2026-09-18 an eigenen Testfenstern (Kevins Fenster blieben unberuehrt): schiebt ein
// Positions-Schreibzugriff ein Warp-Fenster, das noch seine alte Groesse hat, ueber den Rand
// seines Schirms auf den anderen, klemmt jede folgende Verkleinerung die Unterkante — 804×677
// @(0,-57) statt 804×619 @(0,1), exakt wie im Log, bei jedem Neuversuch wieder. Ghostty
// klemmt nicht. Die alte Folge "Position, Groesse, Position" loeste genau das aus, sobald ein
// hohes Fenster auf eine untere Kachel musste.

/// Ein Schreibzugriff auf ein Fenster, in AX-Koordinaten (oben links).
enum AXWrite: Equatable {
    case position(CGPoint)
    case size(CGSize)
}

/// Am alten Platz nur verkleinern (oben links bleibt stehen), dann verschieben — in dieser
/// Groesse passt das Fenster ins Ziel —, erst am Ziel vergroessern. Kein Zwischenstand
/// verlaesst den alten oder den neuen Rahmen. Wachsen erst am Ziel verhindert zugleich, dass
/// macOS eine Groesse beschneidet, die am alten Platz nicht mehr auf den Schirm passte.
func frameWrites(from current: CGRect, to target: CGRect) -> [AXWrite] {
    let shrunk = CGSize(width: min(current.width, target.width), height: min(current.height, target.height))
    var out: [AXWrite] = []
    if shrunk != current.size { out.append(.size(shrunk)) }
    out.append(.position(target.origin))
    if shrunk != target.size { out.append(.size(target.size)) }
    return out
}

/// Rettung fuer ein Fenster, das schon ueber zwei Schirmen klemmt (etwa aus v2): erst in
/// Ist-Groesse so schieben, dass seine Unterkante auf der des Ziels liegt, dann die Groesse,
/// dann die Position. Gemessen: 804×677 @(0,-57) -> exakt 804×619 @(0,1).
func rescueWrites(from current: CGRect, to target: CGRect) -> [AXWrite] {
    [.position(CGPoint(x: target.minX, y: target.maxY - current.height)),
     .size(target.size), .position(target.origin)]
}

/// Gleitbahn einer Bewegung (Cocoa): unterwegs aendert sich die Groesse nicht. Am alten Platz
/// auf `min(alt, neu)` verkleinern (oben links verankert wie AX), in dieser Groesse gleiten,
/// am Ziel wachsen (`setFrame`). Groessenwechsel in jedem Bild klemmten Warp-Fenster, sobald
/// ein Zwischenbild ueber den Schirmrand ragte — und jeder baut das Zeichenraster neu auf.
func glidePath(from: CGRect, to: CGRect) -> (start: CGRect, end: CGRect) {
    let w = min(from.width, to.width)
    let h = min(from.height, to.height)
    return (CGRect(x: from.minX, y: from.maxY - h, width: w, height: h),
            CGRect(x: to.minX, y: to.maxY - h, width: w, height: h))
}

private func writePosition(_ win: AXUIElement, _ p: CGPoint) {
    var v = p
    if let val = AXValueCreate(.cgPoint, &v) {
        AXUIElementSetAttributeValue(win, kAXPositionAttribute as CFString, val)
    }
}

private func writeSize(_ win: AXUIElement, _ s: CGSize) {
    var v = s
    if let val = AXValueCreate(.cgSize, &v) {
        AXUIElementSetAttributeValue(win, kAXSizeAttribute as CFString, val)
    }
}

private func apply(_ win: AXUIElement, _ writes: [AXWrite]) {
    for w in writes {
        switch w {
        case .position(let p): writePosition(win, p)
        case .size(let s): writeSize(win, s)
        }
    }
}

/// Setzt Groesse + Position (Cocoa-Rechteck) und liefert den GEMESSENEN Ist-Frame zurueck.
/// Read-back ist Pflicht: der Dirty-Check vergleicht gegen das Ist, nie gegen das Soll
/// (Plan, Konvention 4). Schreibfolge: `frameWrites`; landet das Fenster trotzdem daneben
/// (es klemmte schon vorher), einmal `rescueWrites`.
@discardableResult
func setFrame(_ win: AXUIElement, _ cocoaRect: CGRect, origin: AXOrigin) -> CGRect? {
    let target = cocoaToAX(cocoaRect, origin)
    // Ist unbekannt (AX-Abfrage gescheitert): schlicht beides schreiben.
    apply(win, axFrame(win).map { frameWrites(from: $0, to: target) }
        ?? [.position(target.origin), .size(target.size)])
    guard let got = axFrame(win) else { return nil }
    guard framesDiffer(axToCocoa(got, origin), cocoaRect, tolerance: 2) else { return axToCocoa(got, origin) }
    apply(win, rescueWrites(from: got, to: target))
    return readBackFrame(win, origin: origin)
}

/// Nur verschieben, ohne Read-back — fuer die Zwischenbilder einer Bewegung. Eine reine
/// Verschiebung kostet fast nichts; die Groesse aendert sich nur am Anfang und am Ende
/// (`glidePath`), weil ein Terminal bei jeder Groessenaenderung sein Zeichenraster neu aufbaut.
func movePosition(_ win: AXUIElement, _ cocoaRect: CGRect, origin: AXOrigin, screenH: CGFloat) {
    writePosition(win, cocoaToAX(cocoaRect, origin, screenH: screenH).origin)
}

/// Nur die Groesse setzen, oben links bleibt stehen — das Verkleinern am alten Platz.
func resizeInPlace(_ win: AXUIElement, _ size: CGSize) {
    writeSize(win, size)
}

/// Abweichung Soll/Ist, ab der wirklich bewegt wird (Plan, Konvention 4).
let FRAME_TOLERANCE: CGFloat = 24

func framesDiffer(_ a: CGRect, _ b: CGRect, tolerance: CGFloat = FRAME_TOLERANCE) -> Bool {
    abs(a.minX - b.minX) > tolerance || abs(a.minY - b.minY) > tolerance
        || abs(a.width - b.width) > tolerance || abs(a.height - b.height) > tolerance
}

/// Wohin wurde ein Fenster zuletzt geschickt, und wo blieb es stehen?
struct MoveMemory: Equatable {
    let target: CGRect
    let reached: CGRect
}

/// Muss das Fenster bewegt werden? Nein, wenn es schon am Ziel steht — und auch nicht, wenn es
/// genau dieses Ziel schon einmal verfehlt hat und seitdem unveraendert dasteht. Sonst
/// wiederholt jeder Tick (alle 3 s) denselben vergeblichen Versuch: im Log vom 18.09. 883-mal.
/// Neues Ziel oder von Kevin verschoben -> wieder anordnen.
func needsMove(current: CGRect, dest: CGRect, memory: MoveMemory?) -> Bool {
    guard framesDiffer(current, dest) else { return false }
    guard let m = memory else { return true }
    return framesDiffer(m.target, dest) || framesDiffer(m.reached, current)
}

// ---------- Apps ----------

func runningApp(_ bundleId: String) -> NSRunningApplication? {
    NSWorkspace.shared.runningApplications.first { $0.bundleIdentifier == bundleId }
}

func warpApp() -> NSRunningApplication? { runningApp(WARP_BID) }

func runningTerminals() -> [(app: NSRunningApplication, bundleId: String)] {
    NSWorkspace.shared.runningApplications.compactMap { app in
        guard let bid = app.bundleIdentifier, TERMINAL_BIDS.contains(bid) else { return nil }
        return (app, bid)
    }
}

/// Bildschirm gesperrt? Dann liefert AX die Terminal-Fenster teils noch, aber ohne lesbare Titel — ein Ping faende nie
/// seinen Marker und setzte nur Tab-Titel auf den Ordnernamen (live 2026-09-25 19:35: gesperrt, je Runde 10 Sessions
/// angepingt, obwohl "kein Fenster lesbar" schon abgefangen war).
func screenLocked() -> Bool {
    (CGSessionCopyCurrentDictionary() as? [String: Any])?["CGSSessionScreenIsLocked"] as? Bool ?? false
}

// ---------- Menueleisten-Symbol (Plan 1b) ----------

/// Das Mini-App-Icon in der Menueleiste, nur das Symbol (Kevin 26.09. im Praxistest: "?", "!" und Zahlen daneben
/// irritieren): die obere rechte Kachel ist gefuellt, sobald eine Session fertig ist oder fragt. Fehlt die Freigabe,
/// ist das Symbol grau. Wie viele und welche, steht im Menue; Frage oder fertig zeigen die Rahmen.
func statusBadge(ready: Int, asking: Int, trusted: Bool) -> (filled: Bool, dimmed: Bool) {
    (trusted && ready + asking > 0, !trusted)
}

// ---------- Bedienungshilfen ----------

func accessibilityTrusted(prompt: Bool = false) -> Bool {
    guard prompt else { return AXIsProcessTrusted() }
    let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
    return AXIsProcessTrustedWithOptions([key: true] as CFDictionary)
}
