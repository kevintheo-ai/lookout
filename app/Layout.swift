// Lookout — Layout-Mathematik. Reine Funktionen, alles in COCOA-Koordinaten
// (Ursprung unten-links). Umgerechnet wird erst beim Schreiben ans Fenster.
//
// Slot 0 ist immer oben-links, dann zeilenweise nach rechts und unten.

import CoreGraphics
import Foundation

enum StackSide: String {
    case left, right

    init(_ s: String) { self = (s == "left") ? .left : .right }
}

/// Die drei Anordnungen (Design v3, Kevin 2026-09-18). Reihenfolge = Menue und Tastenkuerzel
/// Ctrl+Opt+1…3. Der Stapel ist raus ("hat den Sinn nicht erfuellt"); an seiner Stelle
/// sortiert Fokus 2 nach Status, und das Raster ist eine feste Auswahl (`GridSpec`).
enum LayoutMode: String, CaseIterable {
    case reading, focus2, grid

    var label: String {
        switch self {
        case .reading: return tr("Fokus", "Focus")
        case .focus2: return tr("Fokus 2", "Focus 2")
        case .grid: return tr("Raster", "Grid")
        }
    }

    /// Alte Konfigurationswerte weiterverstehen, statt auf den Default zu fallen.
    /// `focus` und `reading` waren zwei Menuepunkte, die auf schmalen Schirmen dasselbe
    /// Bild ergaben (Kevin 2026-09-18) — jetzt eine Ansicht, die sich dem Schirm anpasst.
    /// Ein gespeicherter Stapel wird zu Fokus, Kevins Lieblingsansicht.
    static func parse(_ raw: String) -> LayoutMode {
        if let m = LayoutMode(rawValue: raw) { return m }
        return ["grid22": .grid, "grid32": .grid, "focus": .reading, "stack": .reading][raw] ?? .grid
    }
}

/// Zielbreite einer Terminal-Kachel. 1000 pt sind bei 14-pt-Monospace rund 119 Zeichen —
/// genug fuer Code, ohne dass Zeilen unlesbar lang werden.
let IDEAL_CARD_W: CGFloat = 1000
/// Angenehme Lesebreite fuer EIN grosses Fenster. Ueber die volle Breite eines 21:9-Schirms
/// waeren es ~358 Zeichen pro Zeile — das liest niemand.
let READING_WIDTH: CGFloat = 1400

/// Spalten und Zeilen fuer `count` Fenster auf diesem Bildschirm.
/// Erst die Breite fuellen (volle Hoehe je Fenster), Zeilen erst, wenn die Spalten
/// ausgehen. Mindestgroessen werden nie unterschritten.
func autoGrid(_ vis: CGRect, count: Int) -> (cols: Int, rows: Int) {
    guard count > 0 else { return (1, 1) }
    let byIdeal = max(1, Int((vis.width / IDEAL_CARD_W).rounded()))
    let byMinimum = max(1, Int(floor(vis.width / MIN_CARD_W)))
    let cols = max(1, min(count, min(byIdeal, byMinimum)))
    let maxRows = max(1, Int(floor(vis.height / MIN_CARD_H)))
    let rows = max(1, min(maxRows, Int(ceil(Double(count) / Double(cols)))))
    return (cols, rows)
}

struct GridResult {
    var frames: [String: CGRect] = [:]
    var overflow: [String] = []
}

/// Breite des Mini-Stapels am Rand: ein Sechstel der Flaeche, aber nie unter der
/// Mindestbreite, die Warp zulaesst (Spike: 480 pt).
func miniRailWidth(_ vis: CGRect) -> CGFloat {
    max(MIN_CARD_W, floor(vis.width / 6))
}

private func railRect(_ vis: CGRect, _ side: StackSide, width: CGFloat) -> CGRect {
    CGRect(x: side == .right ? vis.maxX - width : vis.minX, y: vis.minY,
           width: width, height: vis.height)
}

/// Flaeche ohne den Rand-Streifen.
func mainArea(_ vis: CGRect, side: StackSide, railWidth: CGFloat) -> CGRect {
    CGRect(x: side == .right ? vis.minX : vis.minX + railWidth, y: vis.minY,
           width: vis.width - railWidth, height: vis.height)
}

/// cols×rows Kacheln. Passen nicht alle ids hinein, wandern die ueberzaehligen in den
/// Mini-Stapel — und die Kacheln bekommen nur noch die Restflaeche (Rail zuerst abziehen).
func gridLayout(_ vis: CGRect, cols: Int, rows: Int, side: StackSide, ids: [String]) -> GridResult {
    var out = GridResult()
    guard cols > 0, rows > 0, !ids.isEmpty else { return out }

    let capacity = cols * rows
    let overflow = ids.count > capacity
    out.overflow = overflow ? Array(ids.dropFirst(capacity)) : []

    let area = overflow ? mainArea(vis, side: side, railWidth: miniRailWidth(vis)) : vis
    // Raster auf die tatsaechliche Zahl schrumpfen: ein einzelnes Fenster gehoert auf den
    // ganzen Schirm, nicht in die obere linke Ecke eines leeren 2×2 (Kevin, Live-Lauf
    // 2026-09-18). Die Kapazitaet bleibt cols×rows — sie entscheidet nur den Ueberlauf.
    let shown = min(ids.count, capacity)
    let useCols = max(1, min(cols, shown))
    let useRows = max(1, Int(ceil(Double(shown) / Double(useCols))))
    let cellW = floor(area.width / CGFloat(useCols))
    let cellH = floor(area.height / CGFloat(useRows))

    for (i, id) in ids.prefix(capacity).enumerated() {
        let col = i % useCols
        let row = i / useCols
        out.frames[id] = CGRect(x: area.minX + CGFloat(col) * cellW,
                                y: area.maxY - CGFloat(row + 1) * cellH,
                                width: cellW, height: cellH)
    }
    return out
}

/// Mini-Stapel am Rand: gleich breite Kaertchen untereinander. Was nicht mehr in die
/// Hoehe passt, bleibt unberuehrt — lieber weniger anordnen als Fenster uebereinander
/// stapeln, die dann keiner mehr findet.
func miniStack(_ vis: CGRect, side: StackSide, ids: [String]) -> [String: CGRect] {
    guard !ids.isEmpty else { return [:] }
    let width = miniRailWidth(vis)
    let rail = railRect(vis, side, width: width)
    let maxCount = max(1, Int(floor(rail.height / MIN_CARD_H)))
    let count = min(ids.count, maxCount)
    let cardH = floor(rail.height / CGFloat(count))

    var out: [String: CGRect] = [:]
    for (i, id) in ids.prefix(count).enumerated() {
        out[id] = CGRect(x: rail.minX, y: rail.maxY - CGFloat(i + 1) * cardH,
                         width: width, height: cardH)
    }
    return out
}

/// Fokus-Ansicht: ein Fenster fuellt die Restflaeche, der Rest steht als Mini-Stapel daneben.
func focusLayout(_ vis: CGRect, side: StackSide) -> (main: CGRect, rail: CGRect) {
    let width = miniRailWidth(vis)
    return (mainArea(vis, side: side, railWidth: width), railRect(vis, side, width: width))
}

/// Lesebreite-Fokus: das Hauptfenster steht MITTIG in angenehmer Lesebreite, die uebrigen
/// liegen als Streifen an beiden Raendern. Passen keine zwei Streifen daneben (Laptop),
/// faellt die Ansicht auf einen Streifen zurueck — dann ist sie identisch mit `focusLayout`.
func readingLayout(_ vis: CGRect, side: StackSide) -> (main: CGRect, left: CGRect?, right: CGRect?) {
    let rail = miniRailWidth(vis)
    guard vis.width >= READING_WIDTH + 2 * rail else {
        let (main, r) = focusLayout(vis, side: side)
        return (main, side == .left ? r : nil, side == .right ? r : nil)
    }
    let mainW = min(READING_WIDTH, vis.width - 2 * rail)
    let leftW = floor((vis.width - mainW) / 2)
    let rightW = vis.width - mainW - leftW
    let left = CGRect(x: vis.minX, y: vis.minY, width: leftW, height: vis.height)
    let main = CGRect(x: vis.minX + leftW, y: vis.minY, width: mainW, height: vis.height)
    let right = CGRect(x: main.maxX, y: vis.minY, width: rightW, height: vis.height)
    return (main, left, right)
}

/// Kaertchen untereinander in einem beliebigen Streifen (nicht nur am Bildschirmrand).
func stripStack(_ rail: CGRect, ids: [String]) -> [String: CGRect] {
    guard !ids.isEmpty, rail.width > 0 else { return [:] }
    let maxCount = max(1, Int(floor(rail.height / MIN_CARD_H)))
    let count = min(ids.count, maxCount)
    let cardH = floor(rail.height / CGFloat(count))
    var out: [String: CGRect] = [:]
    for (i, id) in ids.prefix(count).enumerated() {
        out[id] = CGRect(x: rail.minX, y: rail.maxY - CGFloat(i + 1) * cardH,
                         width: rail.width, height: cardH)
    }
    return out
}

/// Wer braucht Kevin zuerst? (Kevin 2026-09-18) Erst Sessions mit offener FRAGE, dann fertige
/// Sessions, und unter den fertigen entscheidet, WER ZUERST fertig war — wer spaeter fertig
/// wird, stellt sich hinten an. Nicht-fertige danach nach Erstsichtung. Bestimmt die linke
/// Spalte in Fokus 2 (oben = zuerst dran).
func attentionOrder(_ windows: [TermWindow]) -> [Int] {
    windows.indices.sorted { a, b in
        let wa = windows[a], wb = windows[b]
        // Prioritaet 1 (Kevin 2026-09-18): wer eine FRAGE gestellt hat, kommt ganz nach
        // oben — vor alles, was nur fertig ist. Ohne Antwort geht dort nichts weiter.
        if wa.isWaiting != wb.isWaiting { return wa.isWaiting }
        if wa.isReady != wb.isReady { return wa.isReady }
        if wa.isReady, wb.isReady {
            let ra = wa.readySince ?? wa.firstSeen
            let rb = wb.readySince ?? wb.firstSeen
            if ra != rb { return ra < rb }
        }
        return wa.firstSeen < wb.firstSeen
    }
}

// ---------- Planung ueber alle Bildschirme ----------

/// Bildschirm, auf dem ein Fenster liegt: die GROESSTE Ueberlappung gewinnt.
/// Ein Mittelpunkt-Test greift zu kurz — liegt der Mittelpunkt im Dock-Streifen (nicht Teil
/// von `visibleFrame`), trifft er keinen Schirm, und ein Fallback auf 0 wuerde das Fenster
/// vom Zweitmonitor auf den Hauptschirm reissen. Ohne jede Ueberlappung entscheidet die
/// kuerzeste Distanz der Mittelpunkte.
func screenIndex(of frame: CGRect, screens: [CGRect]) -> Int {
    var best = 0
    var bestArea: CGFloat = 0
    for (i, s) in screens.enumerated() {
        let cut = s.intersection(frame)
        let area = cut.isNull ? 0 : cut.width * cut.height
        if area > bestArea { bestArea = area; best = i }
    }
    if bestArea > 0 { return best }

    var bestDist = CGFloat.greatestFiniteMagnitude
    for (i, s) in screens.enumerated() {
        let dx = s.midX - frame.midX
        let dy = s.midY - frame.midY
        let d = dx * dx + dy * dy
        if d < bestDist { bestDist = d; best = i }
    }
    return best
}

// ---------- Ziel: wohin kommen die Fenster? (Kevin 2026-09-18, ersetzt das Wartezimmer) ----------
//
// Das Wartezimmer schickte jede Session, die zwischen zwei Antworten "fertig" war, aufs
// MacBook — auch die, in der Kevin gerade arbeitete —, und jeder Statuswechsel wechselte den
// Schirm. Jetzt entscheidet Kevin per Knopf: alle auf einen Schirm, oder Leitstand.

/// `single` = alle KI-Fenster auf einen Schirm. `leitstand` = die aktive Session mittig auf dem
/// Arbeitsplatz-Schirm, wer auf Kevin wartet daneben auf `readySide` (Kevin 2026-09-24: Leitstand
/// links, Leitstand 2 rechts), alles Laufende als Raster auf dem zweiten. `flipped` (Leitstand 3, Kevin 2026-09-28):
/// Arbeitsplatz ist das MacBook, die Laufenden stehen am linken und rechten Rand des LG, die Mitte bleibt frei.
enum Target: Equatable {
    case single(Int)
    case leitstand(work: Int, control: Int, readySide: StackSide = .left, flipped: Bool = false)
}

/// Kevins Wahl im Menue. Sie wird je Tick gegen die angeschlossenen Schirme aufgeloest und
/// bleibt erhalten, auch wenn ihr Schirm gerade fehlt (MacBook allein, zugeklappt).
/// `screen` = ein bestimmter Schirm (Kennung, Kevin 2026-10-01: jeder Schirm waehlbar). `external` (groesster externer)
/// und `builtin` sind die bis dahin gespeicherten Werte; sie werden weiter verstanden, stehen aber nicht mehr im Menue.
enum TargetChoice: Equatable {
    case external, builtin, both, both2, both3
    case screen(String)

    /// Wie in UserDefaults und den Menue-Kennungen gespeichert: "both3", "screen:<Kennung>" usw.
    init?(rawValue: String) {
        if rawValue.hasPrefix("screen:") {
            let id = String(rawValue.dropFirst("screen:".count))
            guard !id.isEmpty else { return nil }
            self = .screen(id)
            return
        }
        let plain: [String: TargetChoice] = ["external": .external, "builtin": .builtin, "both": .both, "both2": .both2,
                                             "both3": .both3]
        guard let c = plain[rawValue] else { return nil }
        self = c
    }

    var rawValue: String {
        switch self {
        case .external: return "external"
        case .builtin: return "builtin"
        case .both: return "both"
        case .both2: return "both2"
        case .both3: return "both3"
        case .screen(let id): return "screen:" + id
        }
    }
}

struct ScreenInfo {
    let visible: CGRect
    let builtin: Bool
    /// Kennung, die An- und Abstecken ueberdauert (Display-UUID); leer = unbekannt, dann nicht einzeln waehlbar.
    var id = ""
    /// Name, wie macOS den Schirm nennt (externe); der eingebaute heisst im Menue MacBook bzw. iMac (`laptop`).
    var name = ""
    /// Eingebauter Schirm eines Laptops (MacBook) — sonst eines iMac.
    var laptop = true
}

/// Index des Schirms mit dieser Kennung; leer: nil. Gleiche Monitore heissen von links X, X#2, X#3 (`uniqueScreenIDs`);
/// ist einer abgesteckt, ruecken die rechts davon eine Nummer nach. Fehlt "X#3", gilt darum der hoechste verbliebene
/// Zwilling, nicht der linke (review quick 2026-10-01: Codex R1, GLM + Kimi R2).
/// ponytail: nur die Reihenfolge unterscheidet gleiche Monitore, macOS selbst unterscheidet sie nicht. Ab- und
/// Anstecken kann die Wahl darum zum Nachbarn schieben: X#2 gewaehlt, X weg, X#3 da -> der rechte Nachbar; den
/// rechten von zweien gewaehlt, waehrend der linke fehlte (der rechte hiess da "X") -> nach dem Wiederanstecken der
/// linke. Abhilfe, falls je gebraucht: die Lage des gewaehlten Schirms mitspeichern.
func screenIndex(id: String, screens: [ScreenInfo]) -> Int? {
    guard !id.isEmpty else { return nil }
    if let i = screens.firstIndex(where: { $0.id == id }) { return i }
    guard let r = id.range(of: #"#\d+$"#, options: .regularExpression), let n = Int(id[r].dropFirst()) else { return nil }
    let base = String(id[..<r.lowerBound])
    // Hoeher als die Zahl der Schirme kann keine Nummer sein — begrenzt auch eine kaputte gespeicherte "X#99999999".
    for k in stride(from: min(n - 1, screens.count), through: 1, by: -1) {
        if let i = screens.firstIndex(where: { $0.id == (k == 1 ? base : base + "#\(k)") }) { return i }
    }
    return nil
}

/// Zeigen zwei Kennungen gerade auf denselben Schirm — auch "X#2" und "X", wenn der Zwilling abgesteckt ist?
func sameScreen(_ a: String, _ b: String, screens: [ScreenInfo]) -> Bool {
    a == b || screenIndex(id: a, screens: screens).map { $0 == screenIndex(id: b, screens: screens) } ?? false
}

/// Das im Menue gewaehlte Leitstand-Paar als Schirm-Indizes, wenn es gerade gilt: ab drei Schirmen, beide gefunden (auch
/// ueber den Zwilling), verschieden — sonst nil = automatisch. Anordnung und Menue-Haken lesen beide hier; vorher zeigte
/// das Menue "Automatisch", waehrend die Anordnung ueber den Zwilling das Paar nutzte (review quick 2026-10-01).
func activePair(_ pair: (work: String, control: String)?, screens: [ScreenInfo]) -> (work: Int, control: Int)? {
    guard screens.count >= 3, let pair, let w = screenIndex(id: pair.work, screens: screens),
          let c = screenIndex(id: pair.control, screens: screens), w != c else { return nil }
    return (w, c)
}

/// Wahl + Schirme -> Ziel. Fehlt der gewaehlte Schirm, faellt das Ziel auf den groessten
/// vorhandenen. Der Leitstand braucht zwei Schirme: Arbeitsplatz = groesster, Leitstand =
/// kleinster — ab drei Schirmen gilt das im Menue gewaehlte Paar (`pair`, Kennungen), solange beide angeschlossen sind
/// (Kevin 2026-10-01, `activePair`). Nil nur ohne jeden Schirm.
func resolveTarget(_ choice: TargetChoice, screens: [ScreenInfo],
                   pair: (work: String, control: String)? = nil) -> Target? {
    guard !screens.isEmpty else { return nil }
    let area = { (i: Int) in screens[i].visible.width * screens[i].visible.height }
    let all = Array(screens.indices)
    let largest = all.max { area($0) < area($1) }!
    switch choice {
    case .external:
        return .single(all.filter { !screens[$0].builtin }.max { area($0) < area($1) } ?? largest)
    case .builtin:
        return .single(all.first { screens[$0].builtin } ?? largest)
    case .screen(let id):
        return .single(screenIndex(id: id, screens: screens) ?? largest)
    case .both, .both2, .both3:
        if let p = activePair(pair, screens: screens) {
            return choice == .both3 ? .leitstand(work: p.work, control: p.control, flipped: true)
                : .leitstand(work: p.work, control: p.control, readySide: choice == .both2 ? .right : .left)
        }
        guard let smallest = all.filter({ $0 != largest }).min(by: { area($0) < area($1) }) else {
            return .single(largest)
        }
        if choice == .both3 {
            // Arbeitsplatz ist das MacBook, wie der Knopf sagt — nur ohne eingebauten Schirm der kleinste (review quick).
            let work = all.first { screens[$0].builtin && $0 != largest } ?? smallest
            return .leitstand(work: work, control: largest, flipped: true)
        }
        return .leitstand(work: largest, control: smallest, readySide: choice == .both2 ? .right : .left)
    }
}

/// Eintraege unter BILDSCHIRM: jeder Schirm einzeln (externe von links nach rechts wie auf dem Tisch, der eingebaute
/// zuletzt), ab zwei Schirmen die drei Leitstaende.
func targetOptions(_ screens: [ScreenInfo]) -> [TargetChoice] {
    let order = { (s: ScreenInfo) in (s.builtin ? 1 : 0, s.visible.minX, -s.visible.maxY) }
    let single = screens.filter { !$0.id.isEmpty }.sorted { order($0) < order($1) }.map { TargetChoice.screen($0.id) }
    return single + (screens.count > 1 ? [.both, .both2, .both3] : [])
}

/// Zwei gleiche Monitore ohne Seriennummer koennen dieselbe Kennung melden: von links nach rechts behaelt der erste sie,
/// die weiteren bekommen "#2", "#3" — sonst zeigten beide Menue-Eintraege auf denselben Schirm. Leere Kennung bleibt leer.
func uniqueScreenIDs(_ screens: [ScreenInfo]) -> [ScreenInfo] {
    let place = { (i: Int) in (screens[i].visible.minX, -screens[i].visible.maxY) }
    var out = screens
    for id in Set(screens.map(\.id)) where !id.isEmpty {
        let same = screens.indices.filter { screens[$0].id == id }.sorted { place($0) < place($1) }
        for (n, i) in same.enumerated() where n > 0 { out[i].id = id + "#\(n + 1)" }
    }
    return out
}

/// Namen im Menue: externe wie macOS sie nennt, der eingebaute MacBook bzw. iMac. Gleiche Monitore bekommen (1), (2) — von
/// links nach rechts.
func screenNames(_ screens: [ScreenInfo]) -> [String] {
    let base = screens.map { $0.builtin ? ($0.laptop ? "MacBook" : "iMac") : $0.name }
    let place = { (i: Int) in (screens[i].visible.minX, -screens[i].visible.maxY) }
    return base.indices.map { i in
        let same = base.indices.filter { base[$0] == base[i] }
        guard same.count > 1, let n = same.sorted(by: { place($0) < place($1) }).firstIndex(of: i) else { return base[i] }
        return base[i] + " (\(n + 1))"
    }
}

/// Menue-Titel eines Ziels mit den echten Namen der Schirme (bis 2026-10-01 stand "MacBook" fest darin) — nil, wenn es mit
/// den angeschlossenen Schirmen nicht geht.
func targetTitle(_ option: TargetChoice, screens: [ScreenInfo], pair: (work: String, control: String)? = nil) -> String? {
    let names = screenNames(screens)
    func on(_ i: Int) -> String {
        guard screens[i].builtin else { return tr("auf ", "on ") + names[i] }
        return screens[i].laptop ? tr("aufs MacBook", "on the MacBook") : tr("auf den iMac", "on the iMac")
    }
    switch option {
    case .screen(let id):
        return screens.firstIndex { $0.id == id }.map { tr("Alle ", "All ") + on($0) }
    case .external, .builtin:
        return nil
    case .both:
        return screens.count > 1 ? tr("Leitstand (fertige links)", "Control room (done on the left)") : nil
    case .both2:
        return screens.count > 1 ? tr("Leitstand 2 (fertige rechts)", "Control room 2 (done on the right)") : nil
    case .both3:
        guard screens.count > 1, case .leitstand(let work, _, _, _)? = resolveTarget(.both3, screens: screens, pair: pair)
        else { return nil }
        return tr("Leitstand 3 (aktiv ", "Control room 3 (active ") + on(work) + ")"
    }
}

/// Leitstand-Schirme im Menue waehlen (ab drei Schirmen, Kevin 2026-10-01): derselbe Schirm fuer beide Rollen tauscht sie.
/// Verglichen wird der Schirm, nicht der Name, und die bisherigen Kennungen bleiben stehen: aus "X#2" (rechter von zwei
/// gleichen, der linke gerade abgesteckt) wuerde sonst "X" und nach dem Wiederanstecken der linke (review quick 2026-10-01
/// Runde 2, GLM).
func choosePair(current: (work: String, control: String), work: String? = nil, control: String? = nil,
                screens: [ScreenInfo]) -> (work: String, control: String) {
    let same = { (a: String, b: String) in sameScreen(a, b, screens: screens) }
    var p = current
    if let work, !same(work, p.work) {
        if same(work, p.control) { p = (p.control, p.work) } else { p.work = work }
    }
    if let control, !same(control, p.control) {
        if same(control, p.work) { p = (p.control, p.work) } else { p.control = control }
    }
    return p
}

/// Hat Kevin dieses Fenster aus der Anordnung genommen? Gemerkt wird je Session-ID
/// (Kevin 2026-09-18); ein Fenster mit mehreren Sessions ist raus, sobald eine davon raus ist.
func isExcluded(_ w: TermWindow, _ excluded: Set<String>) -> Bool {
    w.sessions.contains { excluded.contains($0.id) }
}

/// Fenster, die Lookout anordnet: mit KI-Session, beweglich und nicht ausgeschlossen — stabil
/// nach Erstsichtung. Normale Terminals, minimierte, Vollbild- und ausgeschlossene Fenster
/// bleiben unberuehrt.
func layoutCandidates(_ windows: [TermWindow], excluded: Set<String> = []) -> [Int] {
    windows.indices
        .filter { !windows[$0].sessions.isEmpty && windows[$0].layoutable && !isExcluded(windows[$0], excluded) }
        .sorted { windows[$0].firstSeen < windows[$1].firstSeen }
}

/// Reihenfolge der Nicht-Hauptfenster in Fokus und Leitstand: erst die gemerkte Reihenfolge
/// (`hint`, nur Fenster, die es noch gibt), dann neu hinzugekommene nach Erstsichtung.
func restSequence(_ ordered: [Int], main: Int, hint: [Int]) -> [Int] {
    let known = hint.filter { $0 != main && ordered.contains($0) }
    return known + ordered.filter { $0 != main && !known.contains($0) }
}

/// Klick auf ein Seitenfenster oder eine Kachel: das neue Hauptfenster TAUSCHT den Platz mit
/// dem alten — nur diese zwei Fenster bewegen sich (Vorschau-Seite 2026-09-18). Vorher
/// sortierte ein Klick alle Seitenfenster neu nach Erstsichtung.
func swappedRest(_ rest: [Int], oldMain: Int, newMain: Int) -> [Int] {
    guard let i = rest.firstIndex(of: newMain) else { return rest }
    var out = rest
    out[i] = oldMain
    return out
}

// ---------- Status bestimmt die Reihenfolge (Plan 1d, Kevin 2026-09-26) ----------
//
// Eine Regel fuer Fokus 1 (GP 5, R2-3), drei getrennte Gruppen: Frage · fertig ohne Frage · laufend. In Frage und fertig
// bestimmt allein der Fertig-Zeitpunkt den Platz (`attentionOrder`), dort wird nicht getauscht; unter Laufenden gilt die
// Klick-Reihenfolge. Fokus 2 und Leitstand behalten ihre Reihenfolge.

/// Fokus 1: Frage, dann fertig (je nach Fertig-Zeitpunkt), dann die Laufenden in Klick-Reihenfolge (`rest`).
func statusSequence(_ rest: [Int], windows: [TermWindow]) -> [Int] {
    let waiting = rest.filter { windows[$0].isReady }
    return attentionOrder(waiting.map { windows[$0] }).map { waiting[$0] } + rest.filter { !windows[$0].isReady }
}

/// Klick in Fokus 1: laufen alte Mitte und angeklicktes Fenster, tauschen beide die Plaetze (`swappedRest`), sonst bewegt
/// sich nichts. Ueber eine Gruppengrenze geht die alte Mitte an ihren Gruppenplatz — laeuft sie, an den Anfang der
/// Laufenden (Vorschlag der Gegenpruefung, im Praxistest pruefen).
func statusSwap(_ rest: [Int], oldMain: Int, newMain: Int, windows: [TermWindow]) -> [Int] {
    let oldRuns = !windows[oldMain].isReady
    if oldRuns, !windows[newMain].isReady { return swappedRest(rest, oldMain: oldMain, newMain: newMain) }
    let others = rest.filter { $0 != newMain }
    return oldRuns ? [oldMain] + others : others + [oldMain]
}

/// Drei Spalten fuer Fokus 1, wo neben die Lesebreite keine zwei Streifen passen (MacBook): beide Seiten `MIN_CARD_W`
/// (Warps Mindestbreite), die Mitte bekommt den Rest (MacBook: 768 von 1728 pt). Nil unter 3 × 480 pt — die Mitte waere
/// schmaler als Warp erlaubt; dort bleibt es beim einen Rand.
func threeColumnLayout(_ vis: CGRect) -> (left: CGRect, main: CGRect, right: CGRect)? {
    guard vis.width >= 3 * MIN_CARD_W else { return nil }
    let left = CGRect(x: vis.minX, y: vis.minY, width: MIN_CARD_W, height: vis.height)
    let right = CGRect(x: vis.maxX - MIN_CARD_W, y: vis.minY, width: MIN_CARD_W, height: vis.height)
    return (left, CGRect(x: left.maxX, y: vis.minY, width: right.minX - left.maxX, height: vis.height), right)
}

// ---------- Ueberlauf: Fenster oder Zeile (Plan 1d) ----------
//
// Passen auf einer Seite nicht alle als Fenster, stehen die uebrigen als Zeilen unten auf derselben Seite; ihre Fenster
// liegen auf dem Rahmen des Hauptfensters, dahinter ("geparkt"). Vorher blieben sie liegen und verdeckten Wartende.

/// Listenzeile: zwei Textzeilen wie die Schilder (2 × 19 pt) plus je 4 pt Rand. Die Ueberschrift ist kleiner.
let LIST_ROW_H: CGFloat = 46
let LIST_HEADER_H: CGFloat = 28

/// Aufteilung einer Seite: die ersten `cards` als Fenster, `rows` als Zeile, `more` ohne eigene Zeile.
struct RailSplit: Equatable {
    let cards: Int
    let rows: Int
    let more: Int
}

/// Wie viele Fenster und Zeilen passen auf eine Seite der Hoehe `height`? `flexible` (in Reihenfolge) duerfen Fenster
/// sein, `listOnly` stehen immer als Zeile (Fokus 2: ARBEITET). Fenster = die groesste Zahl, bei der sie (je mindestens
/// `MIN_CARD_H`) und alle Zeilen passen — MacBook, 8 Laufende: 4 × 192 + 28 + 4 × 46 = 980 ≤ 1005. Passen nicht einmal
/// alle Zeilen, wird von hinten gekuerzt — zuerst `listOnly`, Wartende gehen vor (R2-2) —, und die letzte Zeile sagt
/// "+N weitere (im Menü)". Planung und Liste nutzen dieselbe Rechnung.
func railSplit(height: CGFloat, flexible: Int, listOnly: Int = 0) -> RailSplit {
    for cards in stride(from: flexible, through: 0, by: -1) {
        let rows = flexible - cards + listOnly
        let list = rows > 0 ? LIST_HEADER_H + CGFloat(rows) * LIST_ROW_H : 0
        if CGFloat(cards) * MIN_CARD_H + list <= height { return RailSplit(cards: cards, rows: rows, more: 0) }
    }
    let fit = max(1, Int(floor((height - LIST_HEADER_H) / LIST_ROW_H)))   // Zeilen samt "+N weitere"
    return RailSplit(cards: 0, rows: fit - 1, more: flexible + listOnly - (fit - 1))
}

/// Eintrag einer Liste: Ueberschrift, Fenster (Index in `windows`) oder "+N weitere (im Menü)".
enum ListEntry: Equatable {
    case header(String)
    case row(Int)
    case more(Int)
}

/// Liste am Fuss einer Seite (Cocoa-Rechteck) mit ihren Eintraegen von oben nach unten.
struct SideList: Equatable {
    let rect: CGRect
    let entries: [ListEntry]

    /// Geparkte Fenster dieser Liste: Zeilen plus "+N weitere".
    var count: Int {
        entries.reduce(0) { n, e in
            switch e {
            case .row: return n + 1
            case .more(let m): return n + m
            case .header: return n
            }
        }
    }
}

/// Planergebnis: Rahmen je Fenster, dazu welche geparkt sind, die Listen und die Mitte (GP 2: "geparkt" kommt aus der
/// Planung, nicht aus dem Schirm-Abgleich — der zaehlt Deckungsgleiches als sichtbar).
struct Arrangement: Equatable {
    var frames: [Int: CGRect] = [:]
    var parked: Set<Int> = []
    var lists: [SideList] = []
    var main: Int?
}

/// Eine Seite belegen: oben Fenster untereinander (`stripStack`), darunter die Liste. Ueberschrift "+N weitere" (Fokus 1)
/// bzw. "ARBEITET (N)" (Fokus 2, ueberzaehlige Wartende direkt darueber). Die Fenster der Zeilen liegen auf `mainRect`.
private func fillSide(_ rail: CGRect, flexible: [Int], listOnly: [Int] = [], mainRect: CGRect,
                      into out: inout Arrangement) {
    let split = railSplit(height: rail.height, flexible: flexible.count, listOnly: listOnly.count)
    let listed = Array(flexible.dropFirst(split.cards)) + listOnly
    let listH = listed.isEmpty ? 0 : LIST_HEADER_H + CGFloat(split.rows + (split.more > 0 ? 1 : 0)) * LIST_ROW_H
    let cards = CGRect(x: rail.minX, y: rail.minY + listH, width: rail.width, height: rail.height - listH)
    for (k, rect) in stripStack(cards, ids: flexible.prefix(split.cards).map(String.init)) { out.frames[Int(k)!] = rect }
    guard !listed.isEmpty else { return }
    for i in listed {
        out.frames[i] = mainRect
        out.parked.insert(i)
    }
    let shown = listed.prefix(split.rows)
    var entries: [ListEntry]
    if listOnly.isEmpty {
        entries = [.header("+\(listed.count) " + tr("weitere", "more"))] + shown.map { .row($0) }
    } else {
        entries = shown.filter { !listOnly.contains($0) }.map { .row($0) }
            + [.header(LabelTone.running.word + " (\(listOnly.count))")]
            + shown.filter { listOnly.contains($0) }.map { .row($0) }
    }
    if split.more > 0 { entries.append(.more(split.more)) }
    out.lists.append(SideList(rect: CGRect(x: rail.minX, y: rail.minY, width: rail.width, height: listH),
                              entries: entries))
}

// ---------- Raster: feste Auswahl (Kevin 2026-09-18) ----------

/// Ein festes Raster, Spalten × Zeilen. "keine Auswahl verschiedener Raster" war Kevins
/// Befund zu v2 — die Automatik rechnete die Spalten selbst aus.
struct GridSpec: Equatable {
    let cols: Int
    let rows: Int

    init(cols: Int, rows: Int) {
        self.cols = cols
        self.rows = rows
    }

    /// "3x2" = 3 Spalten, 2 Zeilen — so steht die Wahl auch im Gedaechtnis (UserDefaults).
    init?(_ key: String) {
        let parts = key.split(separator: "x").compactMap { Int($0) }
        guard parts.count == 2, parts[0] > 0, parts[1] > 0 else { return nil }
        self.init(cols: parts[0], rows: parts[1])
    }

    var key: String { "\(cols)x\(rows)" }
    var label: String { "\(cols) × \(rows)" }
}

/// Die Raster im Menue, in Kevins Reihenfolge.
let GRID_CHOICES = [(2, 2), (3, 2), (4, 2), (3, 3), (4, 3), (5, 2)].map { GridSpec(cols: $0.0, rows: $0.1) }
/// Solange Kevin noch keins gewaehlt hat: passt auf beide Schirme.
let DEFAULT_GRID = GridSpec(cols: 3, rows: 2)

/// Wie viele Spalten/Zeilen passen, ohne dass eine Kachel unter Warps Mindestgroesse faellt?
/// Darunter stuende das Fenster groesser da als geplant und ueberlappte die Nachbarn.
private func gridLimits(_ vis: CGRect) -> (cols: Int, rows: Int) {
    (max(1, Int(floor(vis.width / MIN_CARD_W))), max(1, Int(floor(vis.height / MIN_CARD_H))))
}

/// Angeboten wird nur, was auf den Schirm passt (MacBook: hoechstens 3 Spalten).
func gridChoices(for vis: CGRect) -> [GridSpec] {
    let limit = gridLimits(vis)
    return GRID_CHOICES.filter { $0.cols <= limit.cols && $0.rows <= limit.rows }
}

/// Die gespeicherte Wahl auf den Ziel-Schirm stutzen: 5×2 vom LG wird auf dem MacBook 3×2.
func fittedGrid(_ g: GridSpec, on vis: CGRect) -> GridSpec {
    let limit = gridLimits(vis)
    return GridSpec(cols: min(g.cols, limit.cols), rows: min(g.rows, limit.rows))
}

/// Festes Raster (Kevin 2026-09-18: "Fest"): jede Kachel ist einen Platz gross, egal wie viele
/// Fenster da sind — freie Plaetze bleiben frei. Slot 0 oben links, dann zeilenweise. Mehr
/// Fenster als Plaetze: die uebrigen bekommen keinen Platz und bleiben unberuehrt liegen.
func fixedGrid(_ vis: CGRect, _ g: GridSpec, ids: [Int]) -> [Int: CGRect] {
    let cellW = floor(vis.width / CGFloat(g.cols))
    let cellH = floor(vis.height / CGFloat(g.rows))
    var out: [Int: CGRect] = [:]
    for (i, id) in ids.prefix(g.cols * g.rows).enumerated() {
        out[id] = CGRect(x: vis.minX + CGFloat(i % g.cols) * cellW,
                         y: vis.maxY - CGFloat(i / g.cols + 1) * cellH,
                         width: cellW, height: cellH)
    }
    return out
}

/// Leitstand-Kacheln: Spalten aus der Schirmbreite, Ueberzaehlige in den Mini-Stapel.
private func gridArrange(_ vis: CGRect, side: StackSide, ids: [Int]) -> [Int: CGRect] {
    let keys = ids.map(String.init)
    let g = autoGrid(vis, count: keys.count)
    let grid = gridLayout(vis, cols: g.cols, rows: g.rows, side: side, ids: keys)
    var out: [Int: CGRect] = [:]
    for (k, rect) in grid.frames { out[Int(k)!] = rect }
    for (k, rect) in miniStack(vis, side: side, ids: grid.overflow) { out[Int(k)!] = rect }
    return out
}

/// Fokus 1 auf einem Schirm: `main` mittig, der Rest nach Status (`statusSequence`). Passen neben die Lesebreite zwei
/// Streifen (LG), fuellt die Reihenfolge links bis zur Haelfte, dann rechts (wie bisher). Sonst ab 1440 pt drei Spalten
/// (Plan 1d): links wer wartet, rechts was laeuft — beide als echte Fenster. Darunter ein Rand wie bisher.
private func focusArrange(_ vis: CGRect, side: StackSide, main: Int, rest: [Int],
                          windows: [TermWindow]) -> Arrangement {
    let (mainRect, left, right) = readingLayout(vis, side: side)
    if left == nil || right == nil, let three = threeColumnLayout(vis) {
        var out = Arrangement(frames: [main: three.main], main: main)
        fillSide(three.left, flexible: rest.filter { windows[$0].isReady }, mainRect: three.main, into: &out)
        fillSide(three.right, flexible: rest.filter { !windows[$0].isReady }, mainRect: three.main, into: &out)
        return out
    }
    var out = Arrangement(frames: [main: mainRect], main: main)
    let rails = [left, right].compactMap { $0 }
    let half = rails.count == 2 ? (rest.count + 1) / 2 : rest.count
    for (rail, part) in zip(rails, [Array(rest.prefix(half)), Array(rest.dropFirst(half))]) {
        fillSide(rail, flexible: part, mainRect: mainRect, into: &out)
    }
    return out
}

/// Fokus 2 (Kevin 2026-09-18): `main` mittig in Lesebreite, LINKS wer auf Kevin wartet
/// (`attentionOrder`: Frage zuerst, dann fertig nach Fertig-Zeitpunkt), RECHTS was laeuft.
/// Wird eine Session fertig, landet ihr Fenster beim naechsten Tick links — und rechts, sobald
/// sie wieder arbeitet. Passt nur ein Streifen neben das Hauptfenster (MacBook), steht dort seit Plan 1d
/// (2026-09-26) nur, wer wartet, als Fenster; die Laufenden liegen hinter der Mitte und stehen in der ARBEITET-Liste.
private func focus2Arrange(_ vis: CGRect, side: StackSide, main: Int, others: [Int],
                           windows: [TermWindow]) -> Arrangement {
    let (mainRect, left, right) = readingLayout(vis, side: side)
    let ready = others.filter { windows[$0].isReady }
    let waiting = attentionOrder(ready.map { windows[$0] }).map { ready[$0] }
    let running = others.filter { !windows[$0].isReady }
    var out = Arrangement(frames: [main: mainRect], main: main)
    if let left, let right {
        fillSide(left, flexible: waiting, mainRect: mainRect, into: &out)
        fillSide(right, flexible: running, mainRect: mainRect, into: &out)
    } else if let rail = left ?? right {
        fillSide(rail, flexible: waiting, listOnly: running, mainRect: mainRect, into: &out)
    }
    return out
}

/// Leitstand (Kevin 2026-09-24: "fertig sollte immer neben aktiv sein"): grosser Schirm, Mitte
/// die aktive Session, direkt daneben auf `side` wer auf Kevin wartet (`attentionOrder`: Frage
/// zuerst, dann wer zuerst fertig war); die Gegenseite bleibt frei fuer Chrome. Das MacBook zeigt,
/// was laeuft; passen nicht alle Wartenden in die Spalte, reihen sie sich auf derselben Seite des
/// MacBooks ein. Folgt dem Status je Tick wie Fokus 2. `stackSide` gilt hier nicht.
private func readyBesideArrange(work: CGRect, control: CGRect, side: StackSide, main: Int, rest: [Int],
                                windows: [TermWindow]) -> [Int: CGRect] {
    let (mainRect, left, right) = readingLayout(work, side: side)
    let ready = rest.filter { windows[$0].isReady }
    let waiting = attentionOrder(ready.map { windows[$0] }).map { ready[$0] }
    let running = rest.filter { !windows[$0].isReady }
    let column = (side == .left ? left : right)!  // readingLayout liefert die Spalte auf `side` immer
    let fits = max(1, Int(floor(column.height / MIN_CARD_H)))
    let railFits = max(1, Int(floor(control.height / MIN_CARD_H)))
    let spill = Array(waiting.dropFirst(fits))
    let rail = Array(spill.prefix(railFits))

    var out: [Int: CGRect] = [main: mainRect]
    for (k, rect) in stripStack(column, ids: waiting.prefix(fits).map(String.init)) { out[Int(k)!] = rect }
    for (k, rect) in miniStack(control, side: side, ids: rail.map(String.init)) { out[Int(k)!] = rect }
    // Ist auch die MacBook-Spalte voll, stehen die uebrigen Wartenden vorn im Raster — liegen
    // gelassen ueberlappten sie dort, wo inzwischen ein anderes Fenster hinkommt (review quick).
    let area = rail.isEmpty ? control : mainArea(control, side: side, railWidth: miniRailWidth(control))
    out.merge(gridArrange(area, side: side, ids: Array(spill.dropFirst(railFits)) + running)) { a, _ in a }
    return out
}

/// Leitstand 3 (Kevin 2026-09-28: "in die andere Richtung"): MacBook = Arbeitsplatz, aktive Session rechts, wer auf
/// Kevin wartet LINKS daneben (`attentionOrder`, Spalte `miniRailWidth`). Der LG zeigt, was laeuft, in je einer Spalte am
/// linken und rechten Rand (ein Viertel der Breite), die Mitte (halber Schirm) bleibt frei fuer Chrome. Wartende, die
/// nicht in die MacBook-Spalte passen, stehen vorn in den LG-Spalten. Passt auch dort nicht alles hin, bekommt der Rest
/// die Mitte als Raster — lieber Chrome zudecken als Fenster uebereinander liegen lassen (review quick 2026-09-24).
private func flippedArrange(work: CGRect, control: CGRect, main: Int, rest: [Int],
                            windows: [TermWindow]) -> [Int: CGRect] {
    let (mainRect, column) = focusLayout(work, side: .left)
    let ready = rest.filter { windows[$0].isReady }
    let waiting = attentionOrder(ready.map { windows[$0] }).map { ready[$0] }
    let fits = max(1, Int(floor(column.height / MIN_CARD_H)))
    let others = Array(waiting.dropFirst(fits)) + rest.filter { !windows[$0].isReady }

    var out: [Int: CGRect] = [main: mainRect]
    for (k, rect) in stripStack(column, ids: waiting.prefix(fits).map(String.init)) { out[Int(k)!] = rect }

    let sideW = max(MIN_CARD_W, floor(control.width / 4))  // schmaler laesst Warp nicht zu (review quick)
    let left = CGRect(x: control.minX, y: control.minY, width: sideW, height: control.height)
    let right = CGRect(x: control.maxX - sideW, y: control.minY, width: sideW, height: control.height)
    let perSide = max(1, Int(floor(control.height / MIN_CARD_H)))
    let leftCount = min(perSide, (others.count + 1) / 2)
    let rightCount = min(perSide, others.count - leftCount)
    for (rail, part) in [(left, others.prefix(leftCount)), (right, others.dropFirst(leftCount).prefix(rightCount))] {
        for (k, rect) in stripStack(rail, ids: part.map(String.init)) { out[Int(k)!] = rect }
    }
    let middle = CGRect(x: left.maxX, y: control.minY, width: right.minX - left.maxX, height: control.height)
    out.merge(gridArrange(middle, side: .left, ids: Array(others.dropFirst(leftCount + rightCount)))) { a, _ in a }
    return out
}

/// Ziel-Frames fuer Anordnung + Ziel, dazu geparkte Fenster und Listen (Plan 1d). Nur Fenster mit KI-Session, die sich
/// bewegen lassen und nicht ausgeschlossen sind, bekommen einen Platz; alle anderen bleiben, wo sie sind. Reihenfolge
/// stabil nach Erstsichtung; die aktive Session (`focusedIndex`) steht in Fokus und Fokus 2 in der Mitte und im
/// Leitstand allein auf dem Arbeitsplatz. Die Anordnung gilt nur fuer `single` — der Leitstand ist fest.
/// `restHint`: gemerkte Klick-Reihenfolge (Fokus 1 `statusSwap`, Leitstand `swappedRest`).
/// `grid`: Kevins Raster-Wahl, auf den Ziel-Schirm gestutzt.
func planArrangement(mode: LayoutMode, target: Target, windows: [TermWindow], screens: [CGRect],
                     side: StackSide, focusedIndex: Int? = nil, restHint: [Int] = [],
                     grid: GridSpec = DEFAULT_GRID, excluded: Set<String> = []) -> Arrangement {
    let ordered = layoutCandidates(windows, excluded: excluded)
    guard let first = ordered.first, !screens.isEmpty else { return Arrangement() }
    let active = focusedIndex.flatMap { ordered.contains($0) ? $0 : nil }
    let main = active ?? first
    let rest = restSequence(ordered, main: main, hint: restHint)
    let screen = { (i: Int) in screens[min(max(i, 0), screens.count - 1)] }

    switch target {
    case .leitstand(let work, let control, _, true):
        return Arrangement(frames: flippedArrange(work: screen(work), control: screen(control), main: main, rest: rest,
                                                  windows: windows), main: main)
    case .leitstand(let work, let control, let readySide, false):
        return Arrangement(frames: readyBesideArrange(work: screen(work), control: screen(control), side: readySide,
                                                      main: main, rest: rest, windows: windows), main: main)

    case .single(let s):
        let vis = screen(s)
        switch mode {
        case .reading:
            return focusArrange(vis, side: side, main: main, rest: statusSequence(rest, windows: windows),
                                windows: windows)
        case .focus2:
            // Die Seiten ergeben sich aus dem Status, nicht aus Klicks: laufende nach Erstsichtung.
            return focus2Arrange(vis, side: side, main: main, others: ordered.filter { $0 != main },
                                 windows: windows)
        case .grid:
            // Fest und nach Erstsichtung: ein Klick ordnet im Raster nichts um (Kevin 2026-09-18).
            return Arrangement(frames: fixedGrid(vis, fittedGrid(grid, on: vis), ids: ordered))
        }
    }
}

/// Nur die Rahmen (`planArrangement`).
func planLayout(mode: LayoutMode, target: Target, windows: [TermWindow], screens: [CGRect],
                side: StackSide, focusedIndex: Int? = nil, restHint: [Int] = [],
                grid: GridSpec = DEFAULT_GRID, excluded: Set<String> = []) -> [Int: CGRect] {
    planArrangement(mode: mode, target: target, windows: windows, screens: screens, side: side,
                    focusedIndex: focusedIndex, restHint: restHint, grid: grid, excluded: excluded).frames
}
