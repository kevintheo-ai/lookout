// Lookout — Ready-Anzeige und Fokus-Beobachtung.
//
// Gestaltungsleitlinie (Kevin 2026-09-18): "smoothe Prozesse, die den Geist calm halten,
// aber informieren". Darum:
//   * kein Rot (Alarmfarbe), sondern ein ruhiges Mint — Bedeutung "fertig", nicht "Gefahr"
//   * kein harter Balken, sondern eine duenne Linie mit weichem Schein nach aussen
//   * kein Pulsieren, sondern sehr langsames Atmen (4,5 s) knapp unter der Reizschwelle
//   * jedes Fenster atmet leicht versetzt — synchrones Blinken wirkt wie ein Alarm
//   * Erscheinen und Verschwinden blenden weich, nichts springt ins Bild
//
// Technik: ein klick-durchlaessiges, nicht aktivierendes Panel je fertigem Fenster, exakt
// auf dessen Cocoa-Frame (keine Umrechnung noetig — Konvention 1).
//
// Ebene (Kevin 2026-09-18: "wenn Chrome davor ist, soll der Rand auch in den Hintergrund"):
// das Panel liegt auf der NORMALEN Ebene direkt ueber SEINEM Fenster (`order(.above,
// relativeTo:)` mit der CGWindowID). Fremde Fenster davor verdecken damit auch den Rahmen.
// Gemessen 2026-09-18: das Panel einer accessory-App landet direkt ueber dem Warp-Fenster
// und unter fremden Fenstern davor. Vorher lag es auf `.floating` ueber allem.
//
// Sichtbarkeit: Layout-Zustand PLUS ein Space-Test ueber CGWindowList. Der Plan wollte
// CGWindowList vermeiden, weil er Bildschirmaufnahme-Rechte und rohe Titel unterstellte
// (R3 K4/C1/G8) — der Spike 2026-09-18 hat gemessen, dass Geometrie ohne jede Freigabe
// kommt; Titel werden hier nie gelesen. Ohne den Test leuchtet die Anzeige auf jedem Space.
//
// Fokus: AXObserver je Terminal-Prozess. Die Callbacks laufen auf dem Runloop-Thread und
// fassen KEIN AX an — sie reichen nur die pid weiter (Konvention 3).

import ApplicationServices
import Cocoa

/// AppKit rueckt jedes Fenster beim Setzen zurecht (`constrainFrameRect`): unter die Menueleiste und beim Wechsel
/// auf einen anderen Schirm zurueck auf den alten. Ein Overlay muss aber exakt ueber seinem Fenster liegen — sonst
/// sass der Rahmen oben 7 pt zu tief, und beim Wechsel vom MacBook an die LG-Unterkante blieb er oben links auf dem
/// MacBook haengen, quer ueber einer laufenden Session (Kevin 2026-09-25, Screenshot; Probe −7,−460 statt −7,−7).
private class OverlayPanel: NSPanel {
    override func constrainFrameRect(_ frameRect: NSRect, to _: NSScreen?) -> NSRect { frameRect }
}

/// Klick-durchlaessiges, nicht aktivierendes Panel fuer Rahmen und Vorschau-Umrisse.
/// `animationBehavior = .none`: sonst blendet macOS jedes Zeigen und Verbergen selbst
/// animiert, und alte und neue Umrisse ueberlappen kurz (Befund 2026-09-18). Das weiche
/// Ein- und Ausblenden der Rahmen macht Lookout ohnehin selbst (`GlowPanel.alpha`).
func makeOverlayPanel(level: NSWindow.Level) -> NSPanel {
    let panel = OverlayPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel],
                             backing: .buffered, defer: false)
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = false
    panel.ignoresMouseEvents = true
    panel.level = level
    panel.hidesOnDeactivate = false
    panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenAuxiliary]
    panel.animationBehavior = .none
    return panel
}

// ---------- Ready-Anzeige ----------

/// Zeiten in Sekunden. Bewusst lang: schnelle Wechsel ziehen den Blick, langsame informieren.
private let GLOW_FADE_IN: TimeInterval = 0.55
private let GLOW_FADE_OUT: TimeInterval = 0.4
private let GLOW_BREATH_PERIOD: TimeInterval = 4.5
private let GLOW_ALPHA_BASE: CGFloat = 0.62
private let GLOW_ALPHA_SWING: CGFloat = 0.13

private final class GlowView: NSView {
    var tint: NSColor = .systemGreen

    override func draw(_: NSRect) {
        // Der Schein liegt AUSSEN: die Linie sitzt auf dem Fensterrand, der Schatten
        // franst nach aussen aus. Ein harter Rahmen wirkt wie eine Warnung, ein Halo nicht.
        let path = NSBezierPath(roundedRect: bounds.insetBy(dx: 7, dy: 7), xRadius: 11, yRadius: 11)
        path.lineWidth = 2.5

        NSGraphicsContext.saveGraphicsState()
        let halo = NSShadow()
        halo.shadowColor = tint.withAlphaComponent(0.75)
        halo.shadowBlurRadius = 13
        halo.shadowOffset = .zero
        halo.set()
        tint.withAlphaComponent(0.9).setStroke()
        path.stroke()
        NSGraphicsContext.restoreGraphicsState()
        // Zweiter Zug ohne Schatten haelt die Linie klar, waehrend der Schein weich bleibt.
        tint.withAlphaComponent(0.85).setStroke()
        path.stroke()
    }
}

private final class GlowPanel {
    let panel: NSPanel
    let view: GlowView
    let bornAt = Date()
    var dyingAt: Date?
    var waiting = false
    /// Versatz im Atem-Rhythmus: synchron atmende Rahmen wirken wie ein Blinklicht.
    let phaseOffset: Double

    init(tint: NSColor, offset: Double) {
        view = GlowView()
        view.tint = tint
        panel = makeOverlayPanel(level: .normal)   // Ebene der Fenster — eingeordnet wird in `update`
        panel.contentView = view
        panel.alphaValue = 0
        phaseOffset = offset
    }

    /// Deckkraft zum Zeitpunkt `now`. Nil = fertig ausgeblendet, das Panel kann weg.
    func alpha(now: Date) -> CGFloat? {
        if let dyingAt {
            let t = now.timeIntervalSince(dyingAt) / GLOW_FADE_OUT
            if t >= 1 { return nil }
            return breath(now) * CGFloat(1 - t)
        }
        let t = min(1, now.timeIntervalSince(bornAt) / GLOW_FADE_IN)
        // Weiches Einblenden (smoothstep), kein linearer Ruck.
        return breath(now) * CGFloat(t * t * (3 - 2 * t))
    }

    private func breath(_ now: Date) -> CGFloat {
        let t = now.timeIntervalSinceReferenceDate / GLOW_BREATH_PERIOD + phaseOffset
        return GLOW_ALPHA_BASE + GLOW_ALPHA_SWING * CGFloat(sin(t * 2 * .pi))
    }
}

/// Ein Ziel fuer die Ready-Anzeige: stabile Kennung + Fenster-Rechteck (Cocoa) + Anlass.
/// Der Anlass entscheidet die Farbe: "fertig" und "wartet auf deine Antwort" duerfen nicht
/// gleich aussehen, sonst ist die Anzeige nur Dekoration.
struct GlowTarget {
    let id: String
    let rect: CGRect
    let waiting: Bool
    /// CGWindowID des Terminal-Fensters — das Panel wird direkt darueber eingeordnet.
    /// Nil = unbekannt (Fensterliste leer): dann vorn auf der normalen Ebene, als Notnagel.
    let windowNumber: Int?
}

final class GlowOverlay {
    private var panels: [String: GlowPanel] = [:]
    private var ticker: Timer?
    private var tintReady: NSColor = .systemGreen
    private var tintWaiting: NSColor = .systemOrange
    private var nextOffset: Double = 0

    func setTints(ready: NSColor, waiting: NSColor) {
        tintReady = ready
        tintWaiting = waiting
        for p in panels.values {
            p.view.tint = p.waiting ? waiting : ready
            p.view.needsDisplay = true
        }
    }

    /// Ziele setzen. Neue Kennungen blenden ein, verschwundene blenden aus. `frozen`: Rahmen von Fenstern, die gerade
    /// gleiten — sie folgen ihrem Fenster ueber `follow` und bleiben hier unangetastet. Ohne das zoege ein Tick sie an
    /// den veralteten Platz zurueck oder liesse sie ausblenden, weil das Fenster gerade nicht dort steht.
    func update(targets: [GlowTarget], frozen: Set<String> = [], stack: [OnScreenWindow]? = nil) {
        let wanted = Set(targets.map(\.id)).union(frozen)
        for t in targets where !frozen.contains(t.id) {
            let tint = t.waiting ? tintWaiting : tintReady
            let p: GlowPanel
            if let existing = panels[t.id] {
                p = existing
                if p.waiting != t.waiting {     // Anlass gewechselt -> Farbe nachziehen
                    p.waiting = t.waiting
                    p.view.tint = tint
                    p.view.needsDisplay = true
                }
            } else {
                nextOffset += 0.37          // ungerader Schritt -> kein Gleichtakt
                p = GlowPanel(tint: tint, offset: nextOffset)
                p.waiting = t.waiting
                panels[t.id] = p
            }
            p.dyingAt = nil
            // Aktiviert Kevin das Terminal, hebt macOS dessen Fenster ueber das Panel — der naechste Aufruf legt den
            // Rahmen wieder darauf (`place`: nur dann, nicht mehr bei jedem Aufruf).
            place(p.panel, t.rect.insetBy(dx: -7, dy: -7), above: t.windowNumber, stack: stack)
        }
        for (id, p) in panels where !wanted.contains(id) {
            if p.dyingAt == nil { p.dyingAt = Date() }
        }
        panels.isEmpty ? stop() : start()
    }

    func hideAll() { update(targets: []) }

    /// Waehrend einer Gleitfahrt: der Rahmen bewegt sich mit seinem Fenster (Cocoa-Rechteck je Kennung), statt an der
    /// alten Stelle zu stehen und am Ende zu springen (GLM+Kimi, Kevin 2026-09-25).
    func follow(_ rects: [String: CGRect]) {
        for (id, r) in rects { panels[id]?.panel.setFrame(r.insetBy(dx: -7, dy: -7), display: false) }
    }

    private func start() {
        guard ticker == nil else { return }
        ticker = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            self?.animate()
        }
    }

    private func stop() {
        ticker?.invalidate()
        ticker = nil
    }

    private func animate() {
        let now = Date()
        for (id, p) in panels {
            guard let a = p.alpha(now: now) else {
                p.panel.orderOut(nil)
                panels[id] = nil
                continue
            }
            p.panel.alphaValue = max(0, a)
        }
        if panels.isEmpty { stop() }
    }
}

/// Ein Fenster, das GERADE auf dem sichtbaren Space liegt: CGWindowID, Besitzer-App und
/// Rechteck in AX-Koordinaten (oben links).
struct OnScreenWindow {
    let number: Int
    let pid: pid_t
    let rect: CGRect
}

/// `CGWindowListCopyWindowInfo` liefert Geometrie und Fensternummer ohne Bildschirmaufnahme-
/// Recht (im Spike 2026-09-18 geprueft); Fenstertitel werden bewusst NICHT gelesen, die
/// waeren TCC-pflichtig. Nil = Abfrage gescheitert; leer = gelesen, aber keins dieser Fenster
/// liegt auf dem sichtbaren Space. Beides war frueher dasselbe leere Ergebnis — dann bekam auf
/// einem Schreibtisch ohne Terminal jedes fertige Fenster einen Rahmen an leerer Stelle.
/// `pids` nil = Fenster aller Apps (fuer `sitsAbove`). Reihenfolge wie die Liste: von vorn nach hinten.
func onScreenWindows(pids: Set<pid_t>?) -> [OnScreenWindow]? {
    let opts: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    guard let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] else { return nil }
    return list.compactMap { w in
        guard let owner = w[kCGWindowOwnerPID as String] as? Int, pids?.contains(pid_t(owner)) ?? true,
              (w[kCGWindowLayer as String] as? Int) == 0,
              let number = w[kCGWindowNumber as String] as? Int,
              let b = w[kCGWindowBounds as String] as? [String: Any],
              let r = CGRect(dictionaryRepresentation: b as CFDictionary) else { return nil }
        return OnScreenWindow(number: number, pid: pid_t(owner), rect: r)
    }
}

/// Liegt das Panel schon direkt ueber seinem Fenster — dazwischen hoechstens eigene Panels? `stack`: Fenster aller Apps
/// von vorn nach hinten (`onScreenWindows(pids: nil)`); nil = unbekannt → nein, also neu einordnen wie frueher.
func sitsAbove(_ panel: Int, _ target: Int, in stack: [OnScreenWindow]?, own: pid_t = getpid()) -> Bool {
    guard let stack, let p = stack.firstIndex(where: { $0.number == panel }),
          let t = stack.firstIndex(where: { $0.number == target }), p < t else { return false }
    return stack[(p + 1)..<t].allSatisfy { $0.pid == own }
}

/// Legt ein Overlay-Panel auf `frame` direkt ueber das Fenster `number` — aber nur, was sich geaendert hat. Bis 27.09.
/// geschah beides bei jedem Aufruf (Tick alle 3 s): jedes `order(.above, relativeTo:)` auf das Fenster einer ANDEREN App
/// kostet WindowServer eine Rechte-Pruefung samt Fehlermeldung ("_CGXGetWindowOrderingGroup … requiring rights", gemessen
/// 284/min, danach 5,8/min) — Log-Last, die am 27.09. logd mit ueberlud.
/// Neu eingeordnet wird nur, wenn ein fremdes Fenster dazwischen liegt (Kevin klickt ins Terminal: macOS hebt es ueber
/// das Panel). Ohne Fensternummer wie frueher nach vorn.
func place(_ panel: NSWindow, _ frame: CGRect, above number: Int?, stack: [OnScreenWindow]?) {
    if panel.frame != frame { panel.setFrame(frame, display: false) }
    guard let number else { panel.orderFrontRegardless(); return }
    if !sitsAbove(panel.windowNumber, number, in: stack) { panel.order(.above, relativeTo: number) }
}

/// Fuellt das Fenster praktisch den ganzen Bildschirm aus? Dann schaut Kevin es gerade an
/// und der Hinweis ist nur noch Stoerung (Kevin 2026-09-18: "sobald ein Fenster maximiert
/// wurde sollte der Rahmen weg sein"). Schwelle 95 % — ein maximiertes Fenster laesst in
/// der Regel ein paar Punkte Rand, ein gekacheltes deutlich mehr.
func isMaximized(_ w: TermWindow, screens: [CGRect]) -> Bool {
    guard !screens.isEmpty else { return false }
    let vis = screens[min(screenIndex(of: w.frame, screens: screens), screens.count - 1)]
    guard vis.width > 0, vis.height > 0 else { return false }
    return w.frame.width >= vis.width * 0.95 && w.frame.height >= vis.height * 0.95
}

/// Liegt ein Fenster gerade auf dem sichtbaren Space? Abgleich ueber App + Rechteck —
/// CGWindowList kennt keine AX-Identitaet.
enum OnScreenMatch: Equatable {
    case unknown          // keine Fensterliste (Abfrage fehlgeschlagen)
    case none             // nicht auf diesem Space (anderer Schreibtisch, minimiert)
    case unique(Int)      // genau ein Treffer: seine CGWindowID
    case ambiguous        // zwei Fenster derselben App auf (fast) demselben Rechteck
}

func locateOnScreen(_ w: TermWindow, onScreen: [OnScreenWindow]?, screenH: CGFloat,
                    tolerance: CGFloat = 6) -> OnScreenMatch {
    guard let onScreen else { return .unknown }
    let hits = onScreenHits(w, onScreen, screenH: screenH, tolerance: tolerance)
    switch hits.count {
    case 0: return .none
    case 1: return .unique(hits[0].number)
    default: return .ambiguous
    }
}

/// Fenster derselben App auf (fast) demselben Rechteck, in der Reihenfolge der Fensterliste.
private func onScreenHits(_ w: TermWindow, _ onScreen: [OnScreenWindow], screenH: CGFloat,
                          tolerance: CGFloat) -> [OnScreenWindow] {
    let ax = cocoaToAX(w.frame, axOrigin(for: w.bundleId), screenH: screenH)
    return onScreen.filter { o in
        o.pid == w.ownerPid
            && abs(o.rect.minX - ax.minX) <= tolerance && abs(o.rect.minY - ax.minY) <= tolerance
            && abs(o.rect.width - ax.width) <= tolerance && abs(o.rect.height - ax.height) <= tolerance
    }
}

/// Fensternummer der Mitte fuer die Liste (Plan R2-1): geparkte Fenster liegen deckungsgleich dahinter, `locateOnScreen`
/// ist dann mehrdeutig. Die Fensterliste kommt von vorn nach hinten [Modellwissen, live geprueft] — die erste passende
/// Nummer dieser App ist nach dem Anheben die Mitte.
func frontmostNumber(_ w: TermWindow, onScreen: [OnScreenWindow], screenH: CGFloat, tolerance: CGFloat = 6) -> Int? {
    onScreenHits(w, onScreen, screenH: screenH, tolerance: tolerance).first?.number
}

/// Ueber welchen Fenstern darf eine Anzeige liegen? Sichtbar, auf dem AKTUELLEN Space — und nie
/// die aktive Session (`focusedIndex`): in der arbeitet Kevin, zwischen zwei Antworten ist sie
/// technisch "fertig" (Kevin 2026-09-18). Der Rahmen (`glowTargets`) nimmt davon die fertigen,
/// das Schild (`labelTargets`) alle mit Session.
///
/// Der Space-Test ist noetig, weil die Panels `canJoinAllSpaces` tragen: ohne ihn schwebte
/// die Anzeige auf JEDEM Space an derselben Bildschirmstelle. Jedes Panel liegt direkt ueber
/// seinem Fenster; Fenster davor verdecken es. Ausgeschlossene Fenster behalten ihre Anzeige —
/// Ausschliessen betrifft nur das Anordnen (Kevin 2026-09-18).
/// Liefert Index + CGWindowID (nil = Fensterliste nicht lesbar: nicht faelschlich ausblenden,
/// nur ohne Ebene). `skipMaximized`: die Maximiert-Regel gilt dem Rahmen (Kevin 2026-09-18) — ein Schild braucht
/// auch ein Fenster, das Lookout selbst voll gekachelt hat (einzige laufende Session auf dem MacBook, Kimi 2026-09-25).
func overlayCandidates(windows: [TermWindow], onScreen: [OnScreenWindow]?, screenH: CGFloat,
                       screens: [CGRect], focusedIndex: Int?, skipMaximized: Bool = true,
                       tolerance: CGFloat = 6, parked: Set<Int> = []) -> [(index: Int, number: Int?)] {
    windows.indices.compactMap { i -> (index: Int, number: Int?)? in
        let w = windows[i]
        // Geparkt (Plan 1d, GP 2): nie Rahmen oder Schild, egal in welchem Terminal — die Deckungsgleichheit allein
        // reicht nicht, sie gilt nur innerhalb derselben App (ein Ghostty-Fenster hinter einer Warp-Mitte bekaeme beides).
        guard i != focusedIndex, !parked.contains(i), !w.minimized, !w.fullscreen,
              !(skipMaximized && isMaximized(w, screens: screens)) else { return nil }
        switch locateOnScreen(w, onScreen: onScreen, screenH: screenH, tolerance: tolerance) {
        case .unknown: return (i, nil)
        case .unique(let n): return (i, n)
        // Mehrdeutig: lieber keine Anzeige als eine ueber dem falschen Fenster (review quick
        // 2026-09-18). Nicht auf diesem Space: keine Anzeige.
        case .none, .ambiguous: return nil
        }
    }
}

func glowTargets(windows: [TermWindow], onScreen: [OnScreenWindow]?, screenH: CGFloat,
                 screens: [CGRect] = [], focusedIndex: Int? = nil, tolerance: CGFloat = 6,
                 parked: Set<Int> = []) -> [GlowTarget] {
    overlayCandidates(windows: windows, onScreen: onScreen, screenH: screenH, screens: screens,
                      focusedIndex: focusedIndex, tolerance: tolerance, parked: parked)
        .filter { windows[$0.index].isReady }
        .map { c in
            let w = windows[c.index]
            // Kennung ueber die Session, nicht ueber die Position: so bleibt beim Umsortieren
            // dasselbe Panel am selben Fenster und muss nicht neu einblenden.
            return GlowTarget(id: w.sessions.first?.id ?? "win\(c.index)", rect: w.frame,
                              waiting: w.isWaiting, windowNumber: c.number)
        }
}

/// Wo steht ein KI-Fenster fuer Kevin? Fuer die Fensterliste im Menue (Kevin 2026-09-18:
/// "kevin-05 sehe ich nicht, aber er zeigt an, dass da ein Fenster sei" — es lag minimiert im
/// Dock, und beim Drueberfahren leuchtete ein Umriss an einer leeren Stelle auf).
enum WindowWhereabouts: Equatable {
    case visible, minimized, fullscreen, otherSpace, immovable

    var label: String {
        switch self {
        case .visible: return ""
        case .minimized: return tr("minimiert (im Dock)", "minimized (in the Dock)")
        case .fullscreen: return tr("Vollbild", "full screen")
        case .otherSpace: return tr("anderer Schreibtisch", "other desktop")
        case .immovable: return tr("nicht beweglich", "not movable")
        }
    }
}

/// `onScreen`: liegt das Fenster auf dem sichtbaren Space (`locateOnScreen` != `.none`)?
func whereabouts(_ w: TermWindow, onScreen: Bool) -> WindowWhereabouts {
    if w.minimized { return .minimized }
    if w.fullscreen { return .fullscreen }
    if !onScreen { return .otherSpace }
    return w.movable ? .visible : .immovable
}

// ---------- Schilder (Design 2026-09-21) ----------
//
// Kevin 2026-09-21: "rechts erkenne ich gar nicht, was da genau passiert". Jedes KI-Fenster ausser
// der aktiven Session traegt oben ein ruhiges Schild: Status + Dauer, Thema, aktueller Schritt bzw.
// Ergebnis. Gleiche Gestaltungsleitlinie wie die Rahmen: informieren, nicht alarmieren.

/// Was ein Schild meldet — bestimmt Zeichen, Wort und Farbe.
enum LabelTone: Equatable {
    case waiting, ready, running, watching

    var symbol: String {
        switch self {
        case .waiting: return "?"
        case .ready: return "✓"
        case .running: return "●"
        case .watching: return "◌"
        }
    }

    /// Klartext (Kevin 2026-09-26, Schild B): "wartet auf dich" deckt Frage UND Freigabe ab, "arbeitet" statt "läuft";
    /// "überwacht" (2026-09-27): Antwort fertig, wartet auf den eigenen Hintergrund-Aufpasser, braucht Kevin nicht.
    var word: String {
        switch self {
        case .waiting: return tr("WARTET AUF DICH", "WAITING FOR YOU")
        case .ready: return tr("FERTIG", "DONE")
        case .running: return tr("ARBEITET", "WORKING")
        case .watching: return tr("ÜBERWACHT", "WATCHING")
        }
    }
}

func sessionTone(_ s: Session) -> LabelTone {
    s.waiting ? .waiting : (s.ready ? .ready : (s.watching ? .watching : .running))
}

/// Zustand eines Fensters, wie sein Schild ihn zeigt (sichtbarer Tab).
func windowTone(_ w: TermWindow) -> LabelTone {
    w.isWaiting ? .waiting : (w.isReady ? .ready : (w.isWatching ? .watching : .running))
}

/// Seit wann ist eine Session in ihrem Zustand? `since` nil = unbekannt (Beginn nicht gesehen).
struct SinceEntry: Equatable {
    let tone: LabelTone
    let since: Date?
}

/// Je Session-Tick fortschreiben: gleicher Zustand -> bleibt; Wechsel -> jetzt; neu gesehen -> aus
/// dem Transkript angefangen (fertig/Frage: letzte Antwort; laeuft: Kevins letzter Prompt, falls er
/// im gelesenen Dateiende steht). Verschwundene Sessions fallen raus.
func advanceSince(_ old: [String: SinceEntry], sessions: [Session], now: Date) -> [String: SinceEntry] {
    var next: [String: SinceEntry] = [:]
    for s in sessions {
        let tone = sessionTone(s)
        if let prev = old[s.id] {
            next[s.id] = prev.tone == tone ? prev : SinceEntry(tone: tone, since: now)
        } else {
            next[s.id] = SinceEntry(tone: tone, since: tone == .running ? s.digest?.promptAt : s.digest?.answerAt)
        }
    }
    return next
}

/// "gerade eben" · "6 Min." · "1 Std. 5 Min." · "2 Std." — ausgeschrieben, das Schild liest sich wie ein Satz
/// ("FERTIG  seit 6 Min.", Kevin 2026-09-26).
func durationText(_ seconds: TimeInterval) -> String {
    let s = max(0, Int(seconds))
    if s < 60 { return tr("gerade eben", "just now") }
    let (h, m) = (s / 3600, s % 3600 / 60)
    if h == 0 { return tr("\(m) Min.", "\(m) min") }
    return m == 0 ? tr("\(h) Std.", "\(h) h") : tr("\(h) Std. \(m) Min.", "\(h) h \(m) min")
}

/// Vorname fuer die Anrede, die `firstSentence` wegnimmt (GP 9) — beim Start `firstName(NSFullUserName())`, die Tests
/// nageln ihn fest. Leer = nichts wegnehmen.
var greetingName = ""

/// Erstes Wort des vollen macOS-Benutzernamens: "Kevin Theobald" -> "Kevin" (`NSUserName()` waere "kevin").
func firstName(_ fullName: String) -> String {
    String(fullName.split(whereSeparator: \.isWhitespace).first ?? "")
}

/// Erster Satz einer Antwort, schildtauglich: Markdown weg, Zeilen zusammengezogen, die Anrede
/// "Kevin, …" weg (fast jede Antwort beginnt so — Kevins Regel), hoechstens `limit` Zeichen.
func firstSentence(_ text: String, limit: Int = 160) -> String {
    // Markdown am Zeilenanfang ("## ", "- ", "> ") nur mit folgendem Leerzeichen — "#12" und "-5" bleiben (2026-09-27).
    let lines = text.split(whereSeparator: \.isNewline).map { raw -> String in
        let line = raw.trimmingCharacters(in: .whitespaces)
        let marker = line.prefix { "#>-*".contains($0) }
        let rest = line.dropFirst(marker.count)
        return !marker.isEmpty && (rest.isEmpty || rest.first == " ") ? rest.trimmingCharacters(in: .whitespaces) : line
    }
    var s = lines.filter { !$0.isEmpty }.joined(separator: " ")
        .replacingOccurrences(of: "**", with: "").replacingOccurrences(of: "`", with: "")
    // Anrede: der Name am Anfang, das naechste Nicht-Leerzeichen kein Buchstabe und keine Ziffer — "Kevin, …" und
    // "Kevin 👋 …" gehen, "Kevins Plan" und "Will this work?" (Name Will) bleiben.
    if !greetingName.isEmpty, let r = s.range(of: greetingName, options: [.anchored, .caseInsensitive]),
       let next = s[r.upperBound...].first(where: { !$0.isWhitespace }), !next.isLetter, !next.isNumber {
        s = String(s[r.upperBound...].drop { !$0.isLetter && !$0.isNumber && $0 != "#" })
        s = s.prefix(1).uppercased() + s.dropFirst()
    }
    let chars = Array(s)
    for i in chars.indices where ".!?".contains(chars[i]) && (i + 1 == chars.count || chars[i + 1] == " ") {
        s = String(chars[...i])
        break
    }
    return s.count > limit ? String(s.prefix(limit - 1)) + "…" : s
}

/// Inhalt eines Schilds. Eine reine Funktion, damit Schild und ▦-Liste dasselbe sagen.
struct LabelContent: Equatable {
    let tone: LabelTone
    let duration: String?
    let topic: String
    let detail: String?
    let hint: String?

    /// Zeilen fuer das Schild: Status (+ Dauer), Thema, Schritt/Ergebnis, Hinweis.
    var lines: [String] {
        [tone.symbol + " " + tone.word + (duration.map { "  " + $0 } ?? ""), topic]
            + [detail, hint].compactMap { $0 }
    }
}

/// `entry`: Dauer-Eintrag der sichtbaren Session — gilt nur, wenn er denselben Zustand meint.
/// Ist der sichtbare Tab nicht erkennbar, kommt das Thema aus dem Fenstertitel und es gibt keinen
/// Schritt: der gespeicherte gehoerte womoeglich zum versteckten Tab.
func labelContent(for w: TermWindow, since entry: SinceEntry?, now: Date) -> LabelContent {
    let tone = windowTone(w)
    let s = w.visibleSession
    let d = s?.digest
    let duration = (entry?.tone == tone ? entry?.since : nil).map { start -> String in
        let secs = now.timeIntervalSince(start)
        return secs < 60 ? durationText(secs) : tr("seit ", "for ") + durationText(secs)
    }
    // v4: der eigene Name ("Projekt · Aufgabe", Namens-Hook) traegt das Projekt schon — ohne Ordner dahinter.
    var parts: [String] = []
    let folder = d?.name == nil ? ((s?.cwd ?? "") as NSString).lastPathComponent : ""
    for p in [d?.name ?? d?.title ?? titleGlyph(w.title).text, folder] where !p.isEmpty && !parts.contains(p) {
        parts.append(p)
    }
    let detail: String?
    switch tone {
    case .waiting:
        detail = d?.question.map { tr("„\($0)“", "“\($0)”") } ?? d?.step.map { tr("Freigabe: ", "Approval: ") + $0 }
            ?? d?.text.map { firstSentence($0) }
    case .ready, .watching:
        detail = d?.text.map { firstSentence($0) }
    case .running:
        // Kein "denkt nach" im Wechsel mit dem Schritt — das flackerte im 6-s-Takt.
        detail = d?.last == .text ? d?.text.map { firstSentence($0) } : d?.step.map { tr("jetzt: ", "now: ") + $0 }
    }
    let fallback = w.sessions.first?.engine.rawValue ?? w.app
    return LabelContent(tone: tone, duration: duration,
                        topic: parts.isEmpty ? fallback : parts.joined(separator: " · "),
                        detail: detail,
                        hint: w.hiddenQuestion ? tr("Tab dahinter hat eine Frage", "A hidden tab has a question") : nil)
}

/// Hoehe der Tab- bzw. Titelleiste; das Schild sitzt direkt darunter, damit die Tabs sichtbar
/// bleiben (Kevin 2026-09-21). Warp gemessen 2026-09-21: Ampelknoepfe 12 pt unter der
/// Fensteroberkante, 16 pt hoch -> Leiste ~40 pt. Ghostty [ANNAHME] native Titelleiste 28 pt.
let LABEL_TOP_INSET: [String: CGFloat] = [WARP_BID: 40, GHOSTTY_BID: 28]
let LABEL_PADDING: CGFloat = 9
/// Fast deckend (Kevin 2026-09-26): bei 58 % fiel der Kontrast der Statuszeile ueber hellem Terminal-Text auf 2,8:1,
/// ab 92 % bleibt er ueberall bei mindestens 9,5:1 (WCAG-Formel, nachgerechnet).
let LABEL_BG_ALPHA: CGFloat = 0.92

struct LabelLineStyle {
    let size: CGFloat
    let weight: NSFont.Weight
    let height: CGFloat
}

/// Schrift je Schildzeile (Kevin 2026-09-26, Vorschlag B "größer und klarer"; vorher alle Zeilen 14 pt): Status fett
/// und Name je 16 pt, darunter Schritt/Ergebnis/Frage und Hinweis kleiner.
func labelLineStyle(_ i: Int) -> LabelLineStyle {
    switch i {
    case 0: return LabelLineStyle(size: 16, weight: .bold, height: 22)
    case 1: return LabelLineStyle(size: 16, weight: .semibold, height: 22)
    default: return LabelLineStyle(size: 13.5, weight: .regular, height: 18)
    }
}

/// Schild-Rechteck (Cocoa): volle Fensterbreite minus Rand, direkt unter der Leiste.
func labelRect(window frame: CGRect, bundleId: String, lines: Int) -> CGRect {
    let inset = LABEL_TOP_INSET[bundleId] ?? 28
    let h = LABEL_PADDING * 2 + (0..<lines).reduce(0) { $0 + labelLineStyle($1).height }
    return CGRect(x: frame.minX + 6, y: frame.maxY - inset - h - 2, width: max(0, frame.width - 12), height: h)
}

struct LabelTarget {
    let id: String
    let rect: CGRect           // Cocoa
    let windowNumber: Int?
    let content: LabelContent
}

/// Schilder: jedes KI-Fenster aus `overlayCandidates` (also nie die aktive Session), egal ob fertig — auch voll
/// gekachelte. Zu kleine Fenster (das Schild faende unter der Leiste keinen Platz) bleiben ohne.
func labelTargets(windows: [TermWindow], onScreen: [OnScreenWindow]?, screenH: CGFloat, screens: [CGRect],
                  focusedIndex: Int?, since: [String: SinceEntry], now: Date,
                  parked: Set<Int> = []) -> [LabelTarget] {
    overlayCandidates(windows: windows, onScreen: onScreen, screenH: screenH, screens: screens,
                      focusedIndex: focusedIndex, skipMaximized: false, parked: parked)
        .compactMap { c -> LabelTarget? in
            let w = windows[c.index]
            guard let first = w.sessions.first else { return nil }
            let content = labelContent(for: w, since: w.visibleSession.flatMap { since[$0.id] }, now: now)
            let rect = labelRect(window: w.frame, bundleId: w.bundleId, lines: content.lines.count)
            guard rect.width >= 220, rect.minY >= w.frame.minY + 20 else { return nil }
            return LabelTarget(id: first.id, rect: rect, windowNumber: c.number, content: content)
        }
}

private final class LabelView: NSView {
    var content: LabelContent?
    var tint: NSColor = .white

    override func draw(_: NSRect) {
        guard let content else { return }
        NSColor.black.withAlphaComponent(LABEL_BG_ALPHA).setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 10, yRadius: 10).fill()
        let para = NSMutableParagraphStyle()
        para.lineBreakMode = .byTruncatingTail
        var y = bounds.maxY - LABEL_PADDING
        for (i, line) in content.lines.enumerated() {
            let style = labelLineStyle(i)
            y -= style.height
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: style.size, weight: style.weight),
                .foregroundColor: i == 0 ? tint : NSColor.white.withAlphaComponent(i == 1 ? 1 : 0.8),
                .paragraphStyle: para]
            (line as NSString).draw(in: CGRect(x: 11, y: y, width: bounds.width - 22, height: style.height),
                                    withAttributes: attrs)
        }
    }
}

private final class LabelPanel {
    let panel = makeOverlayPanel(level: .normal)   // Ebene der Fenster, wie der Rahmen
    let view = LabelView()

    init() {
        panel.contentView = view
        panel.alphaValue = 0
    }
}

/// Schilder ueber den Fenstern. Klick-durchlaessig, blendet weich ein und aus, kein Atmen.
final class LabelOverlay {
    private var panels: [String: LabelPanel] = [:]
    private var tintReady: NSColor = .systemGreen
    private var tintWaiting: NSColor = .systemOrange
    private let tintRunning = NSColor(white: 0.82, alpha: 1)

    func setTints(ready: NSColor, waiting: NSColor) {
        tintReady = ready
        tintWaiting = waiting
    }

    func update(_ targets: [LabelTarget], stack: [OnScreenWindow]? = nil) {
        let wanted = Set(targets.map(\.id))
        for t in targets {
            let isNew = panels[t.id] == nil
            let p = panels[t.id] ?? LabelPanel()
            panels[t.id] = p
            let tint = t.content.tone == .waiting ? tintWaiting : (t.content.tone == .ready ? tintReady : tintRunning)
            if p.view.content != t.content || p.view.tint != tint {
                p.view.content = t.content
                p.view.tint = tint
                p.view.needsDisplay = true
            }
            place(p.panel, t.rect, above: t.windowNumber, stack: stack)   // wie beim Rahmen
            if isNew { fade(p.panel, to: 1) }
        }
        for (id, p) in panels where !wanted.contains(id) {
            panels[id] = nil
            fade(p.panel, to: 0) { p.panel.orderOut(nil) }
        }
    }

    func hideAll() { update([]) }

    private func fade(_ panel: NSPanel, to alpha: CGFloat, then done: (() -> Void)? = nil) {
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = alpha > 0 ? 0.45 : 0.35
            panel.animator().alphaValue = alpha
        }, completionHandler: done)
    }
}

// ---------- Liste am Fuss einer Seite (Plan 1d, GP 6) ----------
//
// Was auf einer Seite nicht als Fenster passt, steht dort als Zeile; ein Klick holt die Session in die Mitte. Eigener
// Panel-Typ statt der Schilder-Fabrik: die ist klick-durchlaessig und liegt auf allen Schreibtischen.

/// Menue- und Listenzeile eines Fensters: dasselbe wie sein Schild (Kevin 2026-09-21), darunter der Schritt.
func sessionLine(_ w: TermWindow, since: [String: SinceEntry], now: Date)
    -> (title: String, subtitle: String?, tone: LabelTone) {
    let c = labelContent(for: w, since: w.visibleSession.flatMap { since[$0.id] }, now: now)
    let more = w.sessions.count - 1
    let tabs = more > 0 ? "  (+\(more) " + tr("Tab", more == 1 ? "tab" : "tabs") + ")" : ""
    let sub = [c.detail, c.hint].compactMap { $0 }.joined(separator: " · ")
    return (c.tone.symbol + " " + c.topic + (c.duration.map { " — " + $0 } ?? "") + tabs, sub.isEmpty ? nil : sub, c.tone)
}

/// Kopf der Sessions im Menue: Zahlen in denselben Woertern wie das Schild (Kevin 2026-09-26); "fertig" zaehlt nur
/// Fertige ohne Frage (GP 12).
func sessionsHeader(asking: Int, ready: Int) -> String {
    let counts = [asking > 0 ? "\(asking) " + tr(asking == 1 ? "wartet auf dich" : "warten auf dich", "waiting for you") : nil,
                  ready > 0 ? "\(ready) " + tr("fertig", "done") : nil].compactMap { $0 }
    return "SESSIONS" + (counts.isEmpty ? "" : " (\(counts.joined(separator: " · ")))")
}

/// Zeile der Liste, fertig zum Zeichnen. `ids`: die Sessions des Fensters (fuer den Klick).
enum ListLine: Equatable {
    case header(String)
    case row(ids: [String], title: String, detail: String?, tone: LabelTone)
    case more(Int)

    var height: CGFloat {
        if case .header = self { return LIST_HEADER_H }
        return LIST_ROW_H
    }
}

struct ListTarget: Equatable {
    let rect: CGRect        // Cocoa
    let lines: [ListLine]
    /// CGWindowID der Mitte: die Liste liegt direkt darueber — fremde Fenster davor verdecken auch sie.
    let windowNumber: Int
}

/// Nimmt Klicks an, ohne Lookout zu aktivieren. Nicht auf allen Schreibtischen: beim Zeigen wandert es auf den aktuellen
/// (Lookout zeigt es nur, wenn die Mitte dort liegt). `OverlayPanel`, damit AppKit es nicht zurechtrueckt.
func makeListPanel() -> NSPanel {
    let panel = OverlayPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel],
                             backing: .buffered, defer: false)
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = false
    panel.level = .normal
    panel.hidesOnDeactivate = false
    panel.becomesKeyOnlyIfNeeded = true
    panel.collectionBehavior = [.moveToActiveSpace, .ignoresCycle]
    panel.animationBehavior = .none
    return panel
}

/// Schrift der Liste: bleibt 14 pt — ihre Zeilen haben feste Hoehen (`LIST_ROW_H`), mit denen `railSplit` rechnet.
let LIST_FONT_SIZE: CGFloat = 14
let LIST_LINE_HEIGHT: CGFloat = 19

private final class ListView: NSView {
    var lines: [ListLine] = []
    var tints: [LabelTone: NSColor] = [:]
    var onClick: (([String]) -> Void)?

    override var isFlipped: Bool { true }          // von oben nach unten, wie die Zeilen stehen
    override func acceptsFirstMouse(for _: NSEvent?) -> Bool { true }

    override func draw(_: NSRect) {
        NSColor.black.withAlphaComponent(LABEL_BG_ALPHA).setFill()   // lesbar wie das Schild, egal was dahinter liegt
        NSBezierPath(roundedRect: bounds, xRadius: 8, yRadius: 8).fill()
        let para = NSMutableParagraphStyle()
        para.lineBreakMode = .byTruncatingTail
        func text(_ s: String, y: CGFloat, size: CGFloat = LIST_FONT_SIZE, weight: NSFont.Weight = .regular,
                  color: NSColor) {
            (s as NSString).draw(in: CGRect(x: 10, y: y, width: bounds.width - 20, height: LIST_LINE_HEIGHT),
                                 withAttributes: [.font: NSFont.systemFont(ofSize: size, weight: weight),
                                                  .foregroundColor: color, .paragraphStyle: para])
        }
        var y: CGFloat = 0
        for line in lines {
            switch line {
            case .header(let t):
                text(t, y: y + (LIST_HEADER_H - LIST_LINE_HEIGHT) / 2 + 2, size: 12, weight: .semibold,
                     color: .white.withAlphaComponent(0.6))
            case .row(_, let title, let detail, let tone):
                text(title, y: y + 4, weight: .semibold, color: tints[tone] ?? .white)
                if let detail { text(detail, y: y + 4 + LIST_LINE_HEIGHT, color: .white.withAlphaComponent(0.78)) }
            case .more(let n):
                text("+\(n) " + tr("weitere (im Menü)", "more (in the menu)"), y: y + (LIST_ROW_H - LIST_LINE_HEIGHT) / 2,
                     color: .white.withAlphaComponent(0.6))
            }
            y += line.height
        }
    }

    override func mouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        var y: CGFloat = 0
        for line in lines {
            if p.y >= y, p.y < y + line.height, case .row(let ids, _, _, _) = line {
                onClick?(ids)
                return
            }
            y += line.height
        }
    }
}

/// Die Listen, hoechstens eine je Seite. Klick auf eine Zeile -> `onClick` mit den Session-IDs des Fensters.
final class ListOverlay {
    private var panels: [(panel: NSPanel, view: ListView)] = []
    private var tints: [LabelTone: NSColor] = [.running: NSColor(white: 0.78, alpha: 1),
                                               .watching: NSColor(white: 0.78, alpha: 1)]
    var onClick: (([String]) -> Void)?

    func setTints(ready: NSColor, waiting: NSColor) {
        tints[.ready] = ready
        tints[.waiting] = waiting
    }

    func update(_ targets: [ListTarget], stack: [OnScreenWindow]? = nil) {
        while panels.count < targets.count {
            let view = ListView()
            view.onClick = { [weak self] ids in self?.onClick?(ids) }
            let panel = makeListPanel()
            panel.contentView = view
            panels.append((panel, view))
        }
        for (t, p) in zip(targets, panels) {
            if p.view.lines != t.lines || p.view.tints != tints {
                p.view.lines = t.lines
                p.view.tints = tints
                p.view.needsDisplay = true
            }
            place(p.panel, t.rect, above: t.windowNumber, stack: stack)
        }
        for p in panels.dropFirst(targets.count) where p.panel.isVisible { p.panel.orderOut(nil) }
    }

    func hideAll() { update([]) }
}

// ---------- Vorschau: Umrisse beim Ueberfahren im Menue (Kevin 2026-09-18) ----------
//
// Zeigt, wo die echten Fenster landen WUERDEN, bevor Kevin umschaltet. Waehrend das Menue
// offen ist, laeuft der Runloop im Tracking-Modus — Default-Timer feuern dann nicht. Darum
// wird hier nichts animiert: die Umrisse werden einmal gezeichnet und stehen, bis das
// Menue einen anderen Eintrag hervorhebt oder sich schliesst.

struct PreviewBox {
    let rect: CGRect     // Cocoa
    let title: String
    let detail: String
}

private final class PreviewView: NSView {
    var title = ""
    var detail = ""

    override func draw(_: NSRect) {
        let accent = NSColor.controlAccentColor
        let box = NSBezierPath(roundedRect: bounds.insetBy(dx: 4, dy: 4), xRadius: 12, yRadius: 12)
        accent.withAlphaComponent(0.16).setFill()
        box.fill()
        box.lineWidth = 3
        box.setLineDash([12, 7], count: 2, phase: 0)
        accent.withAlphaComponent(0.9).setStroke()
        box.stroke()

        let para = NSMutableParagraphStyle()
        para.alignment = .center
        para.lineBreakMode = .byTruncatingTail
        let titleAttrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: min(26, max(14, bounds.width / 16)), weight: .semibold),
            .foregroundColor: NSColor.white, .paragraphStyle: para]
        let detailAttrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13, weight: .medium),
            .foregroundColor: NSColor.white.withAlphaComponent(0.85), .paragraphStyle: para]
        let th = (title as NSString).size(withAttributes: titleAttrs).height
        let dh = (detail as NSString).size(withAttributes: detailAttrs).height
        let textW = min(bounds.width - 32, 520)
        let block = CGRect(x: bounds.midX - textW / 2 - 12, y: bounds.midY - (th + dh + 4) / 2 - 8,
                           width: textW + 24, height: th + dh + 4 + 16)
        // Dunkler Grund hinter der Schrift: lesbar ueber jedem Fensterinhalt.
        NSColor.black.withAlphaComponent(0.5).setFill()
        NSBezierPath(roundedRect: block, xRadius: 10, yRadius: 10).fill()
        (detail as NSString).draw(in: CGRect(x: block.minX + 12, y: block.minY + 8, width: textW, height: dh),
                                  withAttributes: detailAttrs)
        (title as NSString).draw(in: CGRect(x: block.minX + 12, y: block.minY + 8 + dh + 4, width: textW, height: th),
                                 withAttributes: titleAttrs)
    }
}

final class PreviewOverlay {
    private var panels: [NSPanel] = []

    /// Ersetzt alle Umrisse. Klick-durchlaessig, stiehlt keinen Fokus.
    func show(_ boxes: [PreviewBox]) {
        hide()
        for b in boxes {
            let view = PreviewView()
            view.title = b.title
            view.detail = b.detail
            // Vorschau liegt bewusst ueber allem, solange das Menue offen ist.
            let panel = makeOverlayPanel(level: .floating)
            panel.contentView = view
            panel.setFrame(b.rect, display: true)
            panel.orderFrontRegardless()
            panels.append(panel)
        }
    }

    func hide() {
        for p in panels { p.orderOut(nil) }
        panels.removeAll()
    }
}

// ---------- Fokus-Beobachtung ----------

/// Ereignisse, die waehrend einer eigenen Bewegung eintreffen: vormerken, nach der Bewegung einmal nachholen.
/// Klicks gingen sonst verloren — die Fokus-Meldung kam in der Gleitfahrt und wurde als eigene Bewegung verworfen
/// (Kevin 2026-09-21: "hat eine laengere Zeit gedauert, bis ich es in die Mitte bekommen konnte"). Seit 2026-09-25
/// ebenso das Anordnen: waehrend der Fahrt stimmen die gemerkten Rahmen nicht, ein neuer Plan liess Fenster an ihren
/// Start zurueckspringen (GLM+Kimi).
struct MoveGate {
    private(set) var pending = false

    /// Fokus-Meldung. true = jetzt auswerten; false = vorgemerkt (Lookout bewegt gerade selbst).
    mutating func event(selfMove: Bool) -> Bool {
        pending = selfMove
        return !selfMove
    }

    /// Bewegung zu Ende oder abgebrochen. true = den vorgemerkten Klick jetzt auswerten.
    mutating func moveEnded() -> Bool {
        defer { pending = false }
        return pending
    }
}

/// Wessen Fokus zaehlt? Das vorderste Terminal. Solange noch keine aktive Session bekannt ist (Start), auch ein
/// Terminal mit KI-Fenstern, das gerade nicht vorn ist: sein zuletzt fokussiertes Fenster ist die Session, an der Kevin
/// zuletzt sass (AX liefert es auch im Hintergrund). Live 2026-09-25 nach dem Neubau: Chrome war vorn, ein beliebiges
/// Fenster stand mit Mint-Rahmen in der Mitte, bis Kevin ein Terminal anklickte — mit Autostart nach jeder Anmeldung.
func focusSource(frontPid: pid_t?, terminalPids: [pid_t], aiTerminalPids: [pid_t], activeKnown: Bool) -> pid_t? {
    if let f = frontPid, terminalPids.contains(f) { return f }
    return activeKnown ? nil : aiTerminalPids.first
}

private func focusCallback(_: AXObserver, _: AXUIElement, _: CFString, _ refcon: UnsafeMutableRawPointer?) {
    guard let refcon else { return }
    let watcher = Unmanaged<FocusWatcher>.fromOpaque(refcon).takeUnretainedValue()
    // KEIN AX hier — nur weiterreichen (Runloop-Thread, Konvention 3).
    watcher.report()
}

final class FocusWatcher {
    private var observers: [pid_t: AXObserver] = [:]
    private var watchedPids: Set<pid_t> = []
    /// Wird nach jedem Fokuswechsel auf main gerufen.
    var onFocusChange: (() -> Void)?

    func report() {
        DispatchQueue.main.async { [weak self] in self?.onFocusChange?() }
    }

    /// Beobachter an die aktuellen Terminal-Prozesse anpassen. Startet neu, wenn ein
    /// Terminal neu gestartet wurde (neue pid) — sonst laeuft die Beobachtung ins Leere.
    func rebuild(pids: Set<pid_t>) {
        guard pids != watchedPids else { return }
        for (pid, obs) in observers where !pids.contains(pid) {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(obs), .defaultMode)
            observers[pid] = nil
        }
        for pid in pids where observers[pid] == nil {
            var obs: AXObserver?
            guard AXObserverCreate(pid, focusCallback, &obs) == .success, let obs else { continue }
            let app = AXUIElementCreateApplication(pid)
            let refcon = Unmanaged.passUnretained(self).toOpaque()
            AXObserverAddNotification(obs, app, kAXFocusedWindowChangedNotification as CFString, refcon)
            AXObserverAddNotification(obs, app, kAXApplicationActivatedNotification as CFString, refcon)
            CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(obs), .defaultMode)
            observers[pid] = obs
        }
        watchedPids = pids
        log("FocusWatcher: beobachte \(observers.count) Terminal-Prozess(e)")
    }

    /// Fokussiertes Fenster einer App. NUR auf `axQueue`.
    static func focusedWindow(pid: pid_t) -> AXUIElement? {
        axRaw(AXUIElementCreateApplication(pid), kAXFocusedWindowAttribute as String)
            .map { $0 as! AXUIElement }
    }

    /// Prozess der App mit dem Tastaturfokus, jetzt gelesen — `axQueue` kann vorher noch eine Fensterbewegung
    /// abarbeiten, ein auf main gemerkter Wert waere dann alt (R10, review quick 2026-10-04, Codex). NUR auf `axQueue`.
    static func focusedAppPid() -> pid_t? {
        guard let app = axRaw(AXUIElementCreateSystemWide(), kAXFocusedApplicationAttribute as String) else { return nil }
        var pid: pid_t = 0
        return AXUIElementGetPid(app as! AXUIElement, &pid) == .success ? pid : nil
    }
}
