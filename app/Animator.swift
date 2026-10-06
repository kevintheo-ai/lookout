// Lookout — Bewegung: Fenster gleiten in ihre Zielposition statt zu springen.
//
// Gestaltungsleitlinie (Kevin 2026-09-18): ruhig, aber informierend.
//   * ZEITBASIERT statt in festen Schritten. AX-Schreibzugriffe kosten je nach Last
//     Millisekunden bis Zehntel — feste Schritte machen die Bewegung dann zaeh und
//     unvorhersehbar lang. Der Fortschritt kommt jetzt aus der echten Uhr: bei Last
//     werden die Schritte groeber, die DAUER bleibt gleich.
//   * Ease-in-out statt ease-out: sanft anfahren UND sanft ankommen. Ein harter Start
//     reisst den Blick, genau das soll er nicht.
//   * Versetzter Start je Fenster (STAGGER): sechs gleichzeitig springende Fenster sind
//     Chaos, eine durchlaufende Welle liest sich als eine einzige, ruhige Bewegung.
//   * Kein Ueberschwingen. Federn wirken verspielt, aber unruhig.
//   * Unterwegs nur verschieben (v3, `glidePath`): beim eigenen Einsatz am alten Platz
//     verkleinern, bei der eigenen Ankunft am Ziel vergroessern. Groessenwechsel in jedem Bild
//     klemmten Warp-Fenster, sobald ein Zwischenbild ueber den Schirmrand ragte (2026-09-18).
//
// Laeuft auf `axQueue`, pro Bild EIN kurzes Enqueue (Konvention 3/G14) — die Queue bleibt
// zwischen den Bildern frei. Ein Generations-Token loest laufende Bewegungen ab: ein neuer
// Layout-Wechsel gewinnt. Bei seiner Ankunft wird jedes Fenster exakt gesetzt und
// zurueckgelesen (Konvention 4); bleibt es stehen (Mindestgroesse, Vollbild-Nachbar), wird das
// protokolliert — und der Aufrufer merkt es sich, damit es nicht jeden Tick erneut versucht wird.

import ApplicationServices
import Cocoa

struct WindowMove {
    let win: AXUIElement
    let origin: AXOrigin
    let from: CGRect     // Cocoa
    let to: CGRect       // Cocoa
}

/// Gesamtdauer einer Bewegung. 0,45 s ist langsam genug, um ihr zu folgen, und kurz genug,
/// dass sie sich nicht zaeh anfuehlt.
let MOVE_DURATION: TimeInterval = 0.45
/// Startversatz je Fenster in Reihenfolge — gedeckelt, damit die Welle nicht ausufert.
let MOVE_STAGGER: TimeInterval = 0.045
let MOVE_STAGGER_MAX: TimeInterval = 0.27
/// Bildtakt. Schafft die AX-Schicht das nicht, werden die Schritte groeber statt langsamer.
private let FRAME_INTERVAL: TimeInterval = 1.0 / 60.0

final class Animator {
    private var generation = 0        // nur auf axQueue anfassen
    private(set) var running = false  // nur auf axQueue anfassen

    /// Ease-in-out kubisch: sanft los, sanft an, kein Ueberschwingen.
    private func eased(_ t: Double) -> CGFloat {
        let c = min(1, max(0, t))
        return CGFloat(c < 0.5 ? 4 * c * c * c : 1 - pow(-2 * c + 2, 3) / 2)
    }

    private func lerp(_ a: CGRect, _ b: CGRect, _ p: CGFloat) -> CGRect {
        CGRect(x: a.minX + (b.minX - a.minX) * p,
               y: a.minY + (b.minY - a.minY) * p,
               width: a.width + (b.width - a.width) * p,
               height: a.height + (b.height - a.height) * p)
    }

    /// `onFinish` bekommt die GEMESSENEN Schluss-Frames (nicht die Soll-Werte) — bei
    /// geclampten Fenstern weichen die ab, und der Aufrufer muss mit dem Ist weiterrechnen.
    /// `onStep` (auf `axQueue`): je Bild die Cocoa-Rechtecke der Fenster, die sich gerade bewegt haben — damit ihr
    /// Rahmen mitfaehrt (Kevin 2026-09-25). Das letzte Bild kommt vor `onFinish`.
    func run(_ moves: [WindowMove], screenH: CGFloat,
             duration: TimeInterval = MOVE_DURATION,
             onStep: (([(AXUIElement, CGRect)]) -> Void)? = nil,
             onFinish: (([(AXUIElement, CGRect)]) -> Void)? = nil) {
        guard !moves.isEmpty else { onFinish?([]); return }
        axQueue.async {
            self.generation += 1
            self.running = true
            let delays = moves.indices.map { min(Double($0) * MOVE_STAGGER, MOVE_STAGGER_MAX) }
            let paths = moves.map { glidePath(from: $0.from, to: $0.to) }
            self.frame(gen: self.generation, moves: moves, paths: paths, delays: delays, screenH: screenH,
                       start: Date(), duration: max(0.05, duration), started: [], finished: [], measured: [],
                       onStep: onStep, onFinish: onFinish)
        }
    }

    /// Bricht die laufende Bewegung ab (Pause, App-Ende).
    func cancel() {
        axQueue.async {
            self.generation += 1
            self.running = false
        }
    }

    /// Ein Bild der Bewegung. Jedes Fenster verkleinert sich bei SEINEM Einsatz (noch am alten
    /// Platz), gleitet und wird bei SEINER Ankunft exakt gesetzt (am Ziel wachsen, Read-back).
    /// Alle auf einmal am Anfang oder am Ende waere ein Sammel-Ruck, der die Welle bricht
    /// (review quick). `started`/`finished`: Fenster, deren Einsatz begonnen hat bzw. die
    /// angekommen sind; `measured`: deren gemessene Schluss-Frames.
    private func frame(gen: Int, moves: [WindowMove], paths: [(start: CGRect, end: CGRect)],
                       delays: [TimeInterval], screenH: CGFloat, start: Date, duration: TimeInterval,
                       started: Set<Int>, finished: Set<Int>, measured: [(AXUIElement, CGRect)],
                       onStep: (([(AXUIElement, CGRect)]) -> Void)?,
                       onFinish: (([(AXUIElement, CGRect)]) -> Void)?) {
        guard gen == generation else { return }   // abgeloest — nichts mehr anfassen

        let elapsed = Date().timeIntervalSince(start)
        var started = started, finished = finished, measured = measured
        var steps: [(AXUIElement, CGRect)] = []
        for (i, m) in moves.enumerated() where !finished.contains(i) {
            let local = (elapsed - delays[i]) / duration
            guard local > 0 else { continue }      // dieses Fenster wartet noch auf seinen Einsatz
            if local >= 1 {
                finished.insert(i)
                guard let ist = setFrame(m.win, m.to, origin: m.origin) else { continue }
                measured.append((m.win, ist))
                steps.append((m.win, ist))
                if framesDiffer(ist, m.to) {
                    log("Animator: Fenster bleibt bei \(Int(ist.width))×\(Int(ist.height))"
                        + " @(\(Int(ist.minX)),\(Int(ist.minY))) statt \(Int(m.to.width))×\(Int(m.to.height))"
                        + " @(\(Int(m.to.minX)),\(Int(m.to.minY))) — akzeptiert")
                }
                continue
            }
            if started.insert(i).inserted, paths[i].start.size != m.from.size {
                resizeInPlace(m.win, paths[i].start.size)
            }
            let r = lerp(paths[i].start, paths[i].end, eased(local))
            movePosition(m.win, r, origin: m.origin, screenH: screenH)
            steps.append((m.win, r))
        }
        if !steps.isEmpty { onStep?(steps) }
        if finished.count == moves.count {
            running = false
            onFinish?(measured)
            return
        }
        axQueue.asyncAfter(deadline: .now() + FRAME_INTERVAL) { [weak self, started, finished, measured] in
            self?.frame(gen: gen, moves: moves, paths: paths, delays: delays, screenH: screenH,
                        start: start, duration: duration, started: started, finished: finished,
                        measured: measured, onStep: onStep, onFinish: onFinish)
        }
    }
}
