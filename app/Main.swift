// Lookout — residente Menueleisten-App fuer KI-Terminal-Sessions.
//
// Kein Dock-Icon (accessory), ein NSStatusItem mit Menue. Ein Timer stoesst `tick()` an:
// Prozess-, Log- und Ping-Arbeit auf `workQueue`, AX-Sammelzugriffe auf `axQueue`,
// UI auf main (Plan, Konvention 3). Das Menue wird beim Oeffnen neu aufgebaut.

import ApplicationServices
import Carbon.HIToolbox
import Cocoa
import IOKit.ps

// ---------- Globale Tastenkuerzel (Ctrl+Opt+1…3 = Fokus, Fokus 2, zuletzt gewaehltes Raster) ----------
//
// Carbon `RegisterEventHotKey` statt eines NSEvent-Monitors: nur Carbon VERBRAUCHT die
// Tastenkombination. Ein globaler Monitor wuerde sie zusaetzlich ans Terminal
// durchreichen — dort landet sie als Steuersequenz in der laufenden KI-Session.

private var hotKeyAction: ((Int) -> Void)?
private var hotKeyRefs: [EventHotKeyRef?] = []

private func installHotkeys(_ action: @escaping (Int) -> Void) {
    hotKeyAction = action
    var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
    InstallEventHandler(GetApplicationEventTarget(), { _, event, _ -> OSStatus in
        var hotKeyID = EventHotKeyID()
        let err = GetEventParameter(event, EventParamName(kEventParamDirectObject),
                                    EventParamType(typeEventHotKeyID), nil,
                                    MemoryLayout<EventHotKeyID>.size, nil, &hotKeyID)
        guard err == noErr else { return err }
        DispatchQueue.main.async { hotKeyAction?(Int(hotKeyID.id)) }
        return noErr
    }, 1, &spec, nil, nil)

    // Nur so viele wie Anordnungen: eine Registrierung ohne Aufgabe schluckt die
    // Tastenkombination systemweit (vorher Ctrl+Opt+4).
    let keys = [kVK_ANSI_1, kVK_ANSI_2, kVK_ANSI_3].prefix(LayoutMode.allCases.count).map { UInt32($0) }
    let modifiers = UInt32(controlKey | optionKey)
    for (i, key) in keys.enumerated() {
        var ref: EventHotKeyRef?
        let id = EventHotKeyID(signature: OSType(0x5747_4B59), id: UInt32(i))  // 'WGKY'
        let status = RegisterEventHotKey(key, modifiers, id, GetApplicationEventTarget(), 0, &ref)
        if status == noErr { hotKeyRefs.append(ref) } else {
            log("Hotkey Ctrl+Opt+\(i + 1) nicht registrierbar (Status \(status)) — vermutlich belegt")
        }
    }
    log("Hotkeys registriert: \(hotKeyRefs.count)/\(keys.count)")
}

private func removeHotkeys() {
    for ref in hotKeyRefs { if let ref { UnregisterEventHotKey(ref) } }
    hotKeyRefs = []
    hotKeyAction = nil
}

// ---------- Gedaechtnis + Menue-Kennungen ----------

private let DEFAULTS_MODE = "lookout.mode"
private let DEFAULTS_TARGET = "lookout.target"
private let DEFAULTS_GRID = "lookout.grid"
private let DEFAULTS_EXCLUDED = "lookout.excluded"
private let DEFAULTS_LABELS = "lookout.labels"
/// Leitstand-Schirme, ab drei Schirmen im Menue gewaehlt (Kevin 2026-10-01): Kennungen von Arbeit und Leitstand.
private let DEFAULTS_PAIR_WORK = "lookout.leitstand.work"
private let DEFAULTS_PAIR_CONTROL = "lookout.leitstand.control"
/// Kennungen an den Menue-Eintraegen — die Vorschau erkennt daran, was ueberfahren wird.
private let PREVIEW_TARGET = "target:"
private let PREVIEW_MODE = "mode:"
private let PREVIEW_GRID = "grid:"
/// "Fenster auswählen": dahinter die Session-IDs des Fensters, eine je Zeile. Klick schaltet das Anordnen.
private let PREVIEW_WINDOW = "win:"
/// Session-Liste (Plan 1c): dieselben IDs, eigene Kennung — Klick holt das Fenster nach vorn (R2-4).
private let PREVIEW_FRONT = "front:"
/// Leitstand-Schirme: dahinter die Kennung des Schirms; "auto" = wieder automatisch.
private let PAIR_WORK = "pair-work:"
private let PAIR_CONTROL = "pair-control:"
private let PAIR_AUTO = "pair-auto"

/// Mini-App-Icon als Vorlagebild (Plan 1b, passt sich hell/dunkel an): links eine grosse Kachel, rechts zwei kleine —
/// wie im App-Icon; `filled` fuellt die obere rechte, dort steht im Icon die fertige Session.
private func statusTileImage(filled: Bool) -> NSImage {
    let img = NSImage(size: NSSize(width: 18, height: 14), flipped: false) { _ in
        let left = NSBezierPath(roundedRect: NSRect(x: 1.5, y: 1.5, width: 8, height: 11), xRadius: 2, yRadius: 2)
        let top = NSBezierPath(roundedRect: NSRect(x: 11.5, y: 7.5, width: 5, height: 5), xRadius: 1.5, yRadius: 1.5)
        let bottom = NSBezierPath(roundedRect: NSRect(x: 11.5, y: 1.5, width: 5, height: 5), xRadius: 1.5, yRadius: 1.5)
        NSColor.black.set()
        for p in [left, top, bottom] {
            p.lineWidth = 1.2
            p.stroke()
        }
        if filled { top.fill() }
        return true
    }
    img.isTemplate = true
    return img
}

/// Eingebauter Schirm (MacBook, iMac)? Nur auf dem Main-Thread (NSScreen).
private func isBuiltin(_ s: NSScreen) -> Bool {
    guard let n = s.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { return false }
    return CGDisplayIsBuiltin(n.uint32Value) != 0
}

/// Kennung eines Schirms, die An- und Abstecken ueberdauert (Display-UUID, aus Hersteller, Modell und Seriennummer).
/// Liefert macOS keine (etwa hinter manchen Adaptern), die Display-Nummer — gilt nur bis zum Abstecken, haelt den Schirm
/// aber waehlbar (review quick 2026-10-01, GLM). Nur auf dem Main-Thread (NSScreen).
private func screenID(_ s: NSScreen) -> String {
    guard let n = s.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { return "" }
    guard let uuid = CGDisplayCreateUUIDFromDisplayID(n.uint32Value)?.takeRetainedValue() else { return "display-\(n)" }
    return CFUUIDCreateString(nil, uuid) as String
}

/// Laptop (interner Akku) — dann heisst der eingebaute Schirm MacBook, sonst iMac. Unbekannt: MacBook wie bisher.
private let isLaptop: Bool = {
    guard let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
          let list = IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] else { return true }
    return list.contains { src in
        (IOPSGetPowerSourceDescription(info, src)?.takeUnretainedValue() as? [String: Any])?[kIOPSTypeKey] as? String
            == kIOPSInternalBatteryType
    }
}()

final class LookoutController: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private let animator = Animator()
    private let glow = GlowOverlay()
    private let preview = PreviewOverlay()
    private let focusWatcher = FocusWatcher()
    private let labels = LabelOverlay()
    /// Listen am Fuss der Seiten (Plan 1d): was nicht als Fenster passt.
    private let lists = ListOverlay()
    private let tileEmpty = statusTileImage(filled: false)
    private let tileFull = statusTileImage(filled: true)
    /// Zuletzt gesetztes Menueleisten-Symbol (`updateBadge`).
    private var shownBadge: (filled: Bool, dimmed: Bool)?
    /// Transkripte je Session — NUR im workQueue-Block des Ticks benutzen (angelegt beim ersten Tick, auf main).
    private lazy var transcripts = TranscriptReader(configDirs: cfg.claudeConfigDirs)
    /// Codex-/Grok-/Kimi-Logs zu laufenden Prozessen — ebenso NUR im workQueue-Block.
    private let logFinder = LogSessionFinder()
    /// Hintergrund-Befehle fertiger Claude-Antworten (Plan 1e) — ebenso NUR im workQueue-Block.
    private lazy var background = BackgroundWatch(leads: Set(cfg.leadNames))
    /// Claude-Status aus den Status-Dateien, `claude agents` nur alle 5 Min. (2026-09-27) — ebenso NUR im workQueue-Block.
    private lazy var claudeStatus = ClaudeStatus(dirs: cfg.claudeConfigDirs)
    /// Wartet nur auf andere Session oder Wecker -> ueberwacht (Kevin 2026-09-27) — ebenso NUR im workQueue-Block.
    private let standby = Standby()
    /// Seit wann jede Session in ihrem Zustand ist (Schild-Dauer).
    private var since: [String: SinceEntry] = [:]
    /// Schilder an/aus (Menuepunkt, gemerkt).
    private var labelsOn = true
    private var lastLabelLog = ""
    /// Kennungen der Fenster, die gerade gleiten: ihr Schild blendet waehrend der Fahrt aus (sonst stuende es bis zum
    /// Ende an der alten Stelle ueber fremden Fenstern), ihr Rahmen faehrt mit (`Animator` onStep → `glow.follow`).
    private var movingIds: Set<String> = []
    /// Klicks waehrend eigener Bewegungen vormerken statt verwerfen (v4).
    private var focusGate = MoveGate()
    /// Anordnen waehrend eigener Bewegungen vormerken: die gemerkten Rahmen stimmen dann nicht (2026-09-25).
    private var layoutGate = MoveGate()
    /// Zaehlt Beginn und Ende jeder eigenen Bewegung — ein Tick, dessen Schnappschuss eine davon ueberlappt, zeigt
    /// Zwischenstaende (`keepMeasuredFrames`).
    private var moveEpoch = 0
    private var lastFocusLog = ""
    private var cfg = loadConfig()
    /// Anordnung (Fokus/Fokus 2/Raster), Ziel (welcher Schirm / Leitstand), Raster-Wahl und
    /// ausgeschlossene Sessions ueberleben einen Neustart — nach jedem Neubau muss die App ja
    /// neu gestartet werden.
    private var mode: LayoutMode = .reading
    private var choice: TargetChoice = .external
    /// Leitstand-Schirme (Kennungen), ab drei Schirmen im Menue gewaehlt; nil = automatisch.
    private var pair: (work: String, control: String)?
    private var grid = DEFAULT_GRID
    /// Session-IDs, die Kevin aus der Anordnung genommen hat (Design v3).
    /// ponytail: wird nie ausgeduennt — ein paar Dutzend IDs sind in UserDefaults kein Gewicht.
    private var excluded: Set<String> = []
    private var side: StackSide = .right
    private var paused = false
    private var tickInFlight = false
    private var timer: Timer?
    private var trusted = false

    private var sessions: [Session] = []
    /// Sessions mit laufendem Prozess (bzw. Hintergrund-Agenten) — Grundlage fuer "Ohne Fenster".
    private var liveSessions: [Session] = []
    private var windows: [TermWindow] = []
    private var pidWindows: [pid_t: AXUIElement] = [:]
    private var firstSeen: [(ax: AXUIElement, date: Date)] = []
    /// Seit wann ist ein Fenster fertig — Reihenfolge der linken Spalte in Fokus 2 (wer zuerst
    /// fertig war, steht oben). Faellt ein Fenster zurueck auf "arbeitet", verfaellt der
    /// Zeitpunkt; wird es erneut fertig, stellt es sich hinten an.
    private var readySince: [(ax: AXUIElement, date: Date)] = []
    private var lastSessionFetch = Date.distantPast
    /// Waehrend eigener Bewegungen sind Fokus-Ereignisse keine Nutzer-Klicks (Konvention 7).
    private var selfMove = false
    /// Zuletzt fokussiertes KI-Fenster — bekommt nie einen Rahmen.
    private var focusedAX: AXUIElement?
    /// Session-ID -> wann Kevin ihr Fenster zuletzt vor sich hatte (`answerSeen`, Kevin 2026-10-04). Schreibt nur der Tick.
    private var seenAt: [String: Date] = [:]
    /// Hauptfenster in Fokus, Fokus 2 und Leitstand: das zuletzt angeklickte Fenster, das
    /// Lookout auch anordnet. Ein Klick in ein ausgeschlossenes Fenster laesst es stehen —
    /// sonst saesse ploetzlich ein anderes in der Mitte.
    private var mainAX: AXUIElement?
    /// Reihenfolge der Seitenfenster (Fokus) bzw. Kacheln (Leitstand), gemerkt als AX-Elemente —
    /// damit ein Klick TAUSCHT statt alle neu zu sortieren.
    private var restOrder: [AXUIElement] = []
    private var lastPlanLog = ""
    private var lastGlowLog = ""
    /// Ziele, die ein Fenster verfehlt hat — nicht jeden Tick erneut versuchen (`needsMove`).
    private var moveMemory: [(ax: AXUIElement, memory: MoveMemory)] = []
    /// Prozesse, die noch (oder wieder) gepingt werden muessen (`pingSchedule`).
    private var pingPending: [pid_t: PingState] = [:]
    /// Letzte lesbare Prozessliste — gilt weiter, wenn `ps` einmal haengt (Review 2026-09-25 Fund 10).
    private var lastProcs: [EngineProc] = []
    private var lastRows: [PsRow] = []
    /// Zuletzt angewandte Planung (Plan 1d): geparkte Fenster, Listen, Mitte. Die Fenster sind als AX gemerkt
    /// (`arrangementAX`), damit sie auch in der Pause und bis zum naechsten Tick stimmen (`currentIndex`).
    private var arrangement = Arrangement()
    private var arrangementAX: [AXUIElement] = []
    /// Wo ein Fenster stand, bevor Lookout es parkte (R2-7) — und ein Ruecksprung, der mit der naechsten Anordnung faehrt.
    private var parkOrigin: [(ax: AXUIElement, frame: CGRect)] = []
    private var pendingRestore: (ax: AXUIElement, frame: CGRect)?
    /// Letzte eindeutige Fensternummer der Mitte (R2-1).
    private var mainNumber: (ax: AXUIElement, number: Int)?

    func applicationDidFinishLaunching(_: Notification) {
        uiGerman = prefersGerman(language: cfg.language, preferred: Locale.preferredLanguages)
        greetingName = firstName(NSFullUserName())
        otherWindowNames = cfg.otherWindowNames
        let saved = UserDefaults.standard
        // `parse` statt `init(rawValue:)`: ein gespeicherter Stapel (bis v2) wird zu Fokus.
        mode = LayoutMode.parse(saved.string(forKey: DEFAULTS_MODE) ?? cfg.layout)
        choice = saved.string(forKey: DEFAULTS_TARGET).flatMap(TargetChoice.init(rawValue:)) ?? .external
        if let work = saved.string(forKey: DEFAULTS_PAIR_WORK), let control = saved.string(forKey: DEFAULTS_PAIR_CONTROL) {
            pair = (work, control)
        }
        grid = saved.string(forKey: DEFAULTS_GRID).flatMap { GridSpec($0) } ?? DEFAULT_GRID
        excluded = Set(saved.stringArray(forKey: DEFAULTS_EXCLUDED) ?? [])
        labelsOn = saved.object(forKey: DEFAULTS_LABELS) as? Bool ?? cfg.labels
        side = StackSide(cfg.stackSide)
        menu.delegate = self
        // Ohne das ueberschreibt AppKits Auto-Validierung jedes isEnabled=false:
        // ein Eintrag mit target+action waere trotzdem klickbar (live gesehen 2026-09-18).
        menu.autoenablesItems = false
        statusItem.menu = menu
        statusItem.button?.image = tileEmpty
        statusItem.button?.imagePosition = .imageOnly
        log("--- Lookout gestartet (ziel=\(choice.rawValue) anordnung=\(mode.rawValue) raster=\(grid.key)"
            + " ausgeschlossen=\(excluded.count) poll=\(cfg.pollSec)s sprache=\(uiGerman ? "de" : "en")"
            + " claude-ordner=\(cfg.claudeConfigDirs.count)) ---")

        glow.setTints(ready: colorFromHex(cfg.glowColor), waiting: colorFromHex(cfg.glowColorWaiting))
        labels.setTints(ready: colorFromHex(cfg.glowColor), waiting: colorFromHex(cfg.glowColorWaiting))
        lists.setTints(ready: colorFromHex(cfg.glowColor), waiting: colorFromHex(cfg.glowColorWaiting))
        lists.onClick = { [weak self] ids in self?.bringToFront(Set(ids)) }
        refreshScreenMetrics()
        NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification,
            object: nil, queue: .main) { [weak self] _ in
                refreshScreenMetrics()
                self?.applyLayout()
        }
        // Space-Wechsel: der Glow haengt am sichtbaren Space, sonst blieben Rahmen bis zum
        // naechsten Tick ueber fremden Fenstern stehen.
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.activeSpaceDidChangeNotification,
            object: nil, queue: .main) { [weak self] _ in self?.refreshOverlays() }
        // App-Wechsel: aktiviert Kevin ein Terminal, hebt macOS dessen Fenster UEBER die
        // Rahmen-Panels — die muessen sofort wieder direkt darauf. Ein zweiter Durchgang kurz
        // danach faengt ab, dass macOS das Anheben erst nach der Meldung abschliesst.
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification,
            object: nil, queue: .main) { [weak self] _ in
                self?.refreshOverlays()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { self?.refreshOverlays() }
        }

        trusted = accessibilityTrusted(prompt: true)
        if !trusted { log("WARNUNG: keine Bedienungshilfen-Berechtigung") }

        focusWatcher.onFocusChange = { [weak self] in self?.handleFocusChange() }
        if cfg.hotkeys {
            installHotkeys { [weak self] idx in
                guard let self, idx < LayoutMode.allCases.count else { return }
                // Im Leitstand ist die Anordnung fest — das Menue sperrt sie ebenso. Sonst
                // aenderte der Kuerzel unsichtbar die gespeicherte Anordnung (review quick).
                if case .leitstand? = resolveTarget(self.choice, screens: self.screenInfos(), pair: self.pair) {
                    log("Tastenkuerzel ignoriert: im Leitstand ist die Anordnung fest")
                    return
                }
                self.setMode(LayoutMode.allCases[idx], via: "Tastenkuerzel")
            }
        }
        timer = Timer.scheduledTimer(withTimeInterval: TimeInterval(cfg.pollSec), repeats: true) { [weak self] _ in
            self?.tick()
        }
        tick()
    }

    // ---------- Tick ----------

    private func tick() {
        guard !tickInFlight else { return }
        tickInFlight = true
        let refreshSessions = Date().timeIntervalSince(lastSessionFetch) >= TimeInterval(cfg.sessionPollSec)
        let cachedSessions = sessions
        let registry = firstSeen
        let threshold = cfg.readyThresholdSec
        // Alles, was der Hintergrund-Block braucht, wird HIER auf main abgegriffen —
        // ein `self?.pidWindows` im Block waere ein Zugriff auf main-Zustand von workQueue.
        let cachedPidWindows = pidWindows
        let pending = pingPending
        let knownProcs = lastProcs
        let knownRows = lastRows
        let reader = transcripts
        let finder = logFinder
        let watch = background
        let claude = claudeStatus
        let waits = standby
        let epoch = moveEpoch
        let seenBefore = seenAt
        let shownWindows = windows
        let frontPid = NSWorkspace.shared.frontmostApplication?.processIdentifier

        workQueue.async { [weak self] in
            // R10: Welche Antwort hat Kevin gerade vor sich? Fokus und Titel frisch lesen, nicht den zuletzt gemeldeten
            // Fokus, und VOR der Entscheidung unten — sonst urteilte ein Takt mit dem Stand von vor 3 s. Als Erstes, gleich
            // nach dem Blick auf die vorderste App: `ps` kann bis zu 3 s dauern (review quick 2026-10-04, Codex, GLM,
            // Kimi). Eine Antwort, die erst danach kommt, gilt in diesem Takt als ungesehen. Bei gesperrtem Bildschirm nicht.
            var seen = seenBefore
            if let pid = frontPid, shownWindows.contains(where: { $0.ownerPid == pid }), !screenLocked() {
                // Vorn-Pruefung und Zeitpunkt gehoeren zur Lesung: ist inzwischen eine andere App vorn, zaehlt sie nicht.
                let front = axQueue.sync { () -> (ax: AXUIElement?, title: String?, at: Date) in
                    let at = Date()
                    guard FocusWatcher.focusedAppPid() == pid else { return (nil, nil, at) }
                    let w = FocusWatcher.focusedWindow(pid: pid)
                    return (w, w.flatMap { axString($0, kAXTitleAttribute as String) }, at)
                }
                if let id = sessionInFront(shownWindows, focused: front.ax, title: front.title, frontPid: pid,
                                           onScreen: onScreenWindows(pids: [pid]), screenH: primaryScreenHeight()) {
                    seen[id] = front.at
                }
            }
            // `ps` nicht lesbar: die letzte Liste gilt weiter, statt alle Zuordnungen zu verwerfen (Fund 10).
            let ps = engineProcesses()
            let procs = ps?.procs ?? knownProcs
            let rows = ps?.rows ?? knownRows
            // Plan 1a: angepingt und am Namen zugeordnet wird nur, was unter Warp/Ghostty laeuft. Die volle Liste bleibt
            // fuer Sessions und Log-Finder — sonst fehlten Sessions fremder Terminals unter "Ohne Fenster".
            let terminalProcs = procs.filter(\.inTerminal)
            var raw = refreshSessions
                ? claude.sessions() + finder.sessions(for: procs, thresholdSec: threshold)
                : cachedSessions
            if refreshSessions {
                // Transkripte nur im Session-Takt und nur fuer Claude-Sessions mit Terminal. Je Session ein eigener Pool:
                // sonst bleiben alle gelesenen Bloecke bis zum Ende des Blocks liegen — Spitze 1,1 GB, nachgestellt
                // 296 statt 80 MB bei 8 grossen Transkripten (2026-09-25).
                for i in raw.indices where raw[i].engine == .claude && raw[i].pid != nil {
                    let sid = String(raw[i].id.dropFirst("claude:".count))
                    let pid = raw[i].pid
                    // Ein Start-Prompt (Uebergabe aus einer anderen Session) ist keine Nachricht von Kevin.
                    raw[i].digest = autoreleasepool { reader.digest(sessionId: sid) }
                        .map { launchCorrected($0, arguments: pid.flatMap(processArguments), started: pid.flatMap(processStartDate)) }
                }
                reader.forget(except: Set(raw.map { String($0.id.dropFirst("claude:".count)) }))
                // Plan 1e: `busy`, Antwort fertig, Hintergrund-Befehl ruht seit readyThresholdSec -> fertig.
                raw = watch.adjust(raw, rows: rows, now: Date(), window: TimeInterval(threshold), seen: seen)
                // Fertig, wartet aber nur auf eine andere Session oder einen eigenen Wecker und sagt "nichts noetig".
                raw = waits.adjust(raw, now: Date(), seen: seen)
                // Zuletzt: bittet die Session in ihrer Lookout-Zeile um etwas, wartet sie auf Kevin (Kevin 2026-10-12).
                raw = applyLookoutLines(raw)
            }
            let live = attachProcesses(raw, procs)

            var windows = axQueue.sync {
                collectWindows { w in registry.first { CFEqual($0.ax, w) }?.date }
            }
            // Paare, deren Prozess oder Fenster verschwunden ist, fallen weg — ihr Prozess wird
            // dann neu gepingt. Eine eingeschwungene Lage bleibt still (`pingSchedule`).
            var pidWindows = cachedPidWindows.filter { pid, ax in
                procs.contains { $0.pid == pid } && windows.contains { CFEqual($0.ax, ax) }
            }
            // v4: Fenster am Namen erkennen — eindeutige Titel-Paare gelten sofort (auch ein umgezogener
            // Tab); arbeitende Sessions uebermalen den Ping-Marker. Der Ping bleibt Rueckfall.
            for (pid, idx) in titleMatches(sessions: live, windows: windows, procs: procs)
            where !(pidWindows[pid].map { CFEqual($0, windows[idx].ax) } ?? false) {
                pidWindows[pid] = windows[idx].ax
                log("Zuordnung am Namen: \(titleGlyph(windows[idx].title).text.prefix(60))")
            }
            // Ein Fenster, das der letzte Tick noch nicht kannte, ist vielleicht ein
            // herausgezogener Tab — dann stimmt eine bekannte Zuordnung nicht mehr (Design v3).
            let newWindow = windows.contains { w in !registry.contains { CFEqual($0.ax, w.ax) } }
            let now = Date()
            let trusted = AXIsProcessTrusted()
            let schedule = pingSchedule(pids: terminalProcs.map(\.pid), mapped: Set(pidWindows.keys), pending: pending,
                                        newWindow: newWindow, now: now, trusted: trusted,
                                        windowsReadable: !windows.isEmpty && !screenLocked())
            var nextPending = schedule.pending
            if !schedule.due.isEmpty {
                // Die volle Liste: bekannte Paare bleiben, solange ihr Prozess lebt — auch wenn `inTerminal` einmal kippt
                // (review quick 2026-09-26, GLM). Gepingt werden ohnehin nur die faelligen, also Warp/Ghostty-Prozesse.
                let pinged = pingMap(procs, known: pidWindows, ping: schedule.due)
                pidWindows = pinged.map
                nextPending = pingResult(due: schedule.due, found: pinged.found,
                                         pending: schedule.pending, now: now)
                windows = axQueue.sync {
                    collectWindows { w in registry.first { CFEqual($0.ax, w) }?.date }
                }
            }
            matchSessions(&windows, live, pidWindows: pidWindows)

            DispatchQueue.main.async {
                guard let self else { return }
                if refreshSessions { self.lastSessionFetch = Date() }
                self.sessions = raw
                self.liveSessions = live
                if refreshSessions { self.since = advanceSince(self.since, sessions: live, now: Date()) }

                // Hat Lookout waehrend des Schnappschusses selbst bewegt, zeigt er Zwischenstaende: die gemessenen
                // Rahmen behalten — sonst sprang ein gerade angekommenes Fenster zurueck (2026-09-25).
                var stamped = self.selfMove || self.moveEpoch != epoch
                    ? keepMeasuredFrames(windows, known: self.windows) : windows
                // Fertig-Zeitpunkte fortschreiben, BEVOR die Fenster uebernommen werden.
                var freshRegistry: [(ax: AXUIElement, date: Date)] = []
                for i in stamped.indices where stamped[i].isReady {
                    let known = self.readySince.first { CFEqual($0.ax, stamped[i].ax) }?.date
                    let since = known ?? Date()
                    stamped[i].readySince = since
                    freshRegistry.append((stamped[i].ax, since))
                }
                self.readySince = freshRegistry
                let windows = stamped
                let hadSession = self.windows.filter { !$0.sessions.isEmpty }.map(\.ax)

                self.windows = windows
                // Zusammenfuehren (ein Klick waehrend des Takts steht schon in `seenAt`) und eine Woche behalten statt nach
                // den laufenden Sessions zu filtern: faellt eine Session einen Takt aus der Liste (Status-Quelle gestoert),
                // bliebe ihre gelesene Antwort sonst ungesehen (review quick 2026-10-04, GLM). Danach im Zweifel fertig.
                self.seenAt.merge(seen) { max($0, $1) }
                self.seenAt = self.seenAt.filter { Date().timeIntervalSince($0.value) < 7 * 86_400 }
                self.pidWindows = pidWindows
                self.pingPending = nextPending
                self.lastProcs = procs
                self.lastRows = rows
                self.firstSeen = windows.map { ($0.ax, $0.firstSeen) }
                self.moveMemory.removeAll { m in !windows.contains { CFEqual($0.ax, m.ax) } }
                self.trusted = trusted
                self.tickInFlight = false

                refreshScreenMetrics()
                self.updateBadge()
                self.focusWatcher.rebuild(pids: Set(windows.map(\.ownerPid)))
                // Erst planen, dann zeichnen: Rahmen, Schilder und Listen folgen der neuen Planung (geparkt, Plan 1d).
                if !self.paused { self.applyLayout() }
                self.refreshOverlays()
                // Nach dem Start ist noch keine aktive Session bekannt (Fokus-Meldungen kommen
                // erst beim naechsten Wechsel): das gerade fokussierte KI-Fenster uebernehmen,
                // sonst leuchtet ausgerechnet das Fenster, in dem Kevin arbeitet.
                // R2-9: ebenso, wenn ein Fenster gerade seine erste Session bekam — `onFocused` ignoriert Fenster ohne
                // Session, ein neues Fenster (⌘N, darin claude) laege sonst geparkt VOR der Mitte, bis Kevin es anklickt.
                let firstSession = windows.contains { w in
                    !w.sessions.isEmpty && !hadSession.contains { CFEqual($0, w.ax) }
                }
                if self.focusedAX == nil || firstSession { self.handleFocusChange() }
            }
        }
    }

    /// Menueleisten-Symbol (Plan 1b): obere rechte Kachel gefuellt bei fertig oder Frage, grau ohne Freigabe, kein Text.
    private func updateBadge() {
        let badge = statusBadge(ready: windows.filter { $0.isReady && !$0.isWaiting }.count,
                                asking: windows.filter(\.isWaiting).count, trusted: trusted)
        // Nur bei Aenderung setzen (2026-09-27, gemessen): Bild + appearsDisabled bei jedem Tick loesten in WindowServer je
        // ein "_CGXPackagesSetWindowConstraints: Invalid window" aus — ein Paar alle 3 s, 50,9/min mit Lookout, 12,9/min
        // ohne; mit dieser Pruefung 8,2/min (Rest: andere Apps).
        guard shownBadge.map({ $0 != badge }) ?? true else { return }
        shownBadge = badge
        statusItem.button?.image = badge.filled ? tileFull : tileEmpty
        statusItem.button?.appearsDisabled = badge.dimmed
    }

    // ---------- Fokus: Klick auf ein KI-Fenster ----------

    private func handleFocusChange() {
        // Eigene Bewegungen sind keine Nutzer-Klicks (Konvention 7) — ein echter Klick in dieser Zeit
        // wird aber vorgemerkt und nach der Bewegung ausgewertet (v4; vorher ging er verloren). Auch in
        // der Pause wird die aktive Session nachgefuehrt — der Rahmen gehoert zur Statusanzeige.
        guard trusted else { return }
        guard focusGate.event(selfMove: selfMove) else {
            logFocus("Klick während eigener Bewegung — wird danach ausgewertet")
            return
        }
        guard let pid = focusSource(frontPid: NSWorkspace.shared.frontmostApplication?.processIdentifier,
                                    terminalPids: windows.map(\.ownerPid),
                                    aiTerminalPids: windows.filter { !$0.sessions.isEmpty }.map(\.ownerPid),
                                    activeKnown: focusedAX != nil) else { return }
        axQueue.async { [weak self] in
            let at = Date()
            let focused = FocusWatcher.focusedWindow(pid: pid)
            let title = focused.flatMap { axString($0, kAXTitleAttribute as String) }
            DispatchQueue.main.async { self?.onFocused(focused, title: title, at: at) }
        }
    }

    /// `title`/`at`: mit dem Fokus frisch gelesener Fenstertitel und Zeitpunkt der Lesung — nur dann zaehlt der Klick als
    /// "Antwort gesehen" (R10; auch ein kurzer Blick zwischen zwei Takten, review quick 2026-10-04, Codex). Gestempelt wird
    /// die Lesung, nicht die spaetere Ausfuehrung hier (Kimi): eine Antwort, die erst danach kam, bleibt ungesehen. Die
    /// anderen Aufrufer haben keinen Titel.
    private func onFocused(_ ax: AXUIElement?, title: String? = nil, at: Date? = nil) {
        if let at, !screenLocked(), let pid = NSWorkspace.shared.frontmostApplication?.processIdentifier,
           let id = sessionInFront(windows, focused: ax, title: title, frontPid: pid,
                                   onScreen: onScreenWindows(pids: [pid]), screenH: primaryScreenHeight()) {
            seenAt[id] = max(seenAt[id] ?? at, at)
        }
        guard let ax, let idx = indexOfWindow(ax, in: windows) else { return }
        // Klick-Protokoll (v4): warum ein Klick nichts bewegt, steht im Log statt geraten zu werden.
        guard !windows[idx].sessions.isEmpty else {
            logFocus("Klick ignoriert: Fenster ohne erkannte Session („\(windows[idx].title.prefix(50))“)")
            return
        }

        // Das angeklickte KI-Fenster ist die aktive Session: es verliert sofort seinen Rahmen,
        // und alle Rahmen werden neu eingeordnet (macOS hat es ueber sie gehoben).
        focusedAX = ax
        // Hauptfenster (Mitte in Fokus und Fokus 2, Arbeitsplatz im Leitstand) wird es nur,
        // wenn Lookout es auch anordnet — ein Klick in ein ausgeschlossenes Fenster aendert
        // an der Anordnung nichts.
        let candidates = layoutCandidates(windows, excluded: excluded)
        guard candidates.contains(idx) else {
            logFocus("Klick: \(focusName(idx)) — ausgeschlossen oder nicht beweglich, Anordnung bleibt")
            refreshOverlays()
            return
        }
        let changed = mainAX.map { !CFEqual($0, ax) } ?? true
        if changed { logFocus("Klick: \(focusName(idx)) → Mitte") }
        if changed, let oldMain = mainIndex() ?? candidates.first, oldMain != idx, candidates.contains(oldMain) {
            // Gerechnet auf der Reihenfolge, die GERADE zu sehen ist. Im Leitstand tauscht das angeklickte Fenster den
            // Platz mit dem bisherigen Hauptfenster; in Fokus 1 nur unter Laufenden — Frage und fertig ordnet der
            // Fertig-Zeitpunkt (Plan 1d). Fokus 2 und Raster lesen die Reihenfolge nicht.
            let shown = restSequence(candidates, main: oldMain, hint: restHint())
            var leitstand = false
            if case .leitstand? = resolveTarget(choice, screens: screenInfos(), pair: pair) { leitstand = true }
            let rest = leitstand ? swappedRest(shown, oldMain: oldMain, newMain: idx)
                                 : statusSwap(shown, oldMain: oldMain, newMain: idx, windows: windows)
            restOrder = rest.map { windows[$0].ax }
        }
        mainAX = ax
        refreshOverlays()
        if changed, !paused { applyLayout() }
    }

    /// Name eines Fensters fuers Klick-Protokoll: der Session-Name, wie ihn das Schild zeigt.
    private func focusName(_ idx: Int) -> String {
        let w = windows[idx]
        let s = w.visibleSession ?? w.sessions.first
        return String((s?.digest?.name ?? s?.name ?? w.title).prefix(60))
    }

    /// Eine Zeile je Aenderung — App-Aktivierung meldet denselben Fokus oft doppelt.
    private func logFocus(_ line: String) {
        guard line != lastFocusLog else { return }
        lastFocusLog = line
        log(line)
    }

    private func focusedIndex() -> Int? {
        focusedAX.flatMap { indexOfWindow($0, in: windows) }
    }

    private func mainIndex() -> Int? {
        mainAX.flatMap { indexOfWindow($0, in: windows) }
    }

    private func restHint() -> [Int] {
        restOrder.compactMap { indexOfWindow($0, in: windows) }
    }

    /// Index aus der letzten Planung -> Index im aktuellen Schnappschuss.
    private func currentIndex(_ planned: Int) -> Int? {
        arrangementAX.indices.contains(planned) ? indexOfWindow(arrangementAX[planned], in: windows) : nil
    }

    /// Geparkte Fenster der letzten Planung (Plan 1d): nie Rahmen oder Schild, im Menue "(in der Liste)".
    private func parkedIndices() -> Set<Int> {
        Set(arrangement.parked.compactMap(currentIndex))
    }

    /// Rahmen, Schilder und Listen — dieselben Ausloeser (Tick, Fokus, App-/Space-Wechsel, nach Bewegungen).
    private func refreshOverlays() {
        guard trusted else { glow.hideAll(); labels.hideAll(); lists.hideAll(); return }
        // Eine Fensterliste fuer beides: alle Apps (liegt ein fremdes Fenster zwischen Panel und Terminal? `place`),
        // daraus die Terminal-Fenster (wo liegt was?).
        let stack = onScreenWindows(pids: nil)
        let terminals = Set(windows.map(\.ownerPid))
        let onScreen = stack.map { $0.filter { terminals.contains($0.pid) } }
        let parked = parkedIndices()
        refreshLabels(onScreen, parked: parked, stack: stack)
        refreshLists(onScreen, stack: stack)
        guard cfg.glow else { glow.hideAll(); return }
        // Rahmen gleitender Fenster fahren mit (`glow.follow`) — hier weder versetzen noch ausblenden.
        let targets = glowTargets(windows: windows, onScreen: onScreen, screenH: primaryScreenHeight(),
                                  screens: NSScreen.screens.map(\.visibleFrame), focusedIndex: focusedIndex(),
                                  parked: parked)
            .filter { !movingIds.contains($0.id) }
        // Eine Zeile je Aenderung: der verirrte Rahmen vom 18.09. (Design v3, Fehler 4) laesst
        // sich beim naechsten Auftreten so im Log nachvollziehen (Kennung, Rechteck, Fenster).
        let line = targets.map {
            "\($0.id.suffix(8))@\(Int($0.rect.minX)),\(Int($0.rect.minY)) \(Int($0.rect.width))x\(Int($0.rect.height))"
                + ($0.windowNumber.map { "#\($0)" } ?? "#?")
        }.joined(separator: " ")
        if line != lastGlowLog {
            lastGlowLog = line
            log("Rahmen: " + (line.isEmpty ? "keine" : line))
        }
        glow.update(targets: targets, frozen: movingIds, stack: stack)
    }

    private func refreshLabels(_ onScreen: [OnScreenWindow]?, parked: Set<Int>, stack: [OnScreenWindow]?) {
        guard labelsOn else { labels.hideAll(); return }
        let targets = labelTargets(windows: windows, onScreen: onScreen, screenH: primaryScreenHeight(),
                                   screens: NSScreen.screens.map(\.visibleFrame), focusedIndex: focusedIndex(),
                                   since: since, now: Date(), parked: parked)
            .filter { !movingIds.contains($0.id) }
        // Nur die Menge protokollieren, nicht jeden Textwechsel (der Inhalt aendert sich laufend).
        let line = targets.map { String($0.id.suffix(8)) }.sorted().joined(separator: " ")
        if line != lastLabelLog {
            lastLabelLog = line
            log("Schilder: " + (line.isEmpty ? "keine" : line))
        }
        labels.update(targets, stack: stack)
    }

    /// Listen am Fuss der Seiten (Plan 1d): nur mit Schildern (sie zeigen Namen und Schritte — beim Bildschirmteilen
    /// aus), nie waehrend eigener Bewegungen, nur auf dem Schreibtisch der Mitte und direkt ueber ihr (GP 6, R2-10).
    private func refreshLists(_ onScreen: [OnScreenWindow]?, stack: [OnScreenWindow]?) {
        guard labelsOn, !selfMove, movingIds.isEmpty, !arrangement.lists.isEmpty,
              let main = arrangement.main.flatMap(currentIndex), let number = listAnchor(main, onScreen) else {
            lists.hideAll()
            return
        }
        let now = Date()
        lists.update(arrangement.lists.map { list in
            ListTarget(rect: list.rect, lines: list.entries.compactMap { entry -> ListLine? in
                switch entry {
                case .header(let t): return .header(t)
                case .more(let n): return .more(n)
                case .row(let i):
                    guard let idx = currentIndex(i) else { return nil }
                    let line = sessionLine(windows[idx], since: since, now: now)
                    return .row(ids: windows[idx].sessions.map(\.id), title: line.title, detail: line.subtitle,
                                tone: line.tone)
                }
            }, windowNumber: number)
        }, stack: stack)
    }

    /// Fensternummer der Mitte (R2-1): eindeutig — oder, wenn geparkte Fenster deckungsgleich dahinter liegen, die
    /// vorderste dieser App auf ihrem Rahmen. Ist die Fensterliste nicht lesbar, die letzte eindeutige. Nil = Mitte nicht
    /// auf diesem Schreibtisch: keine Liste, statt dass sie ueber allem schwebt.
    private func listAnchor(_ main: Int, _ onScreen: [OnScreenWindow]?) -> Int? {
        let w = windows[main]
        guard let onScreen else { return mainNumber.flatMap { CFEqual($0.ax, w.ax) ? $0.number : nil } }
        switch locateOnScreen(w, onScreen: onScreen, screenH: primaryScreenHeight()) {
        case .unique(let n):
            mainNumber = (w.ax, n)
            return n
        case .ambiguous:
            return frontmostNumber(w, onScreen: onScreen, screenH: primaryScreenHeight())
        case .none, .unknown:
            return nil
        }
    }

    // ---------- Ziel (welcher Schirm) ----------

    /// Angeschlossene Schirme mit Kennung und Namen (Kevin 2026-10-01: jeder Schirm erkannt und waehlbar).
    private func screenInfos() -> [ScreenInfo] {
        uniqueScreenIDs(NSScreen.screens.map {
            ScreenInfo(visible: $0.visibleFrame, builtin: isBuiltin($0), id: screenID($0), name: $0.localizedName,
                       laptop: isLaptop)
        })
    }

    /// Plan fuer eine Wahl — auch fuer eine nur UEBERFAHRENE (Vorschau), darum mit Parametern.
    private func plan(_ c: TargetChoice, _ m: LayoutMode, grid g: GridSpec? = nil)
        -> (target: Target, arrangement: Arrangement)? {
        let infos = screenInfos()
        guard let target = resolveTarget(c, screens: infos, pair: pair) else { return nil }
        return (target, planArrangement(mode: m, target: target, windows: windows, screens: infos.map(\.visible),
                                        side: side, focusedIndex: mainIndex(), restHint: restHint(),
                                        grid: g ?? grid, excluded: excluded))
    }

    /// Nutzbare Flaeche des Schirms, auf dem "alle auf einen Schirm" gerade landet — nil im Leitstand.
    private func singleTargetVisible() -> CGRect? {
        let infos = screenInfos()
        guard case .single(let s)? = resolveTarget(choice, screens: infos, pair: pair), infos.indices.contains(s) else {
            return nil
        }
        return infos[s].visible
    }

    /// Eine Zeile je Aenderung von Ziel/Anordnung — damit sich das Verhalten auch im
    /// Nachhinein pruefen laesst, ohne danebenzustehen.
    private func logPlan(_ target: Target, count: Int, parked: Int) {
        let line = "Plan: ziel=\(choice.rawValue) -> \(target), anordnung=\(mode.rawValue), \(count) Fenster"
            + (parked > 0 ? ", \(parked) geparkt" : "")
        if line != lastPlanLog {
            lastPlanLog = line
            log(line)
        }
    }

    // ---------- Layout anwenden ----------

    private func applyLayout() {
        // Waehrend einer eigenen Fahrt stimmen die gemerkten Rahmen nicht: vormerken und danach anordnen — sonst sprang
        // ein Fenster an seinen Start zurueck und glitt neu los (GLM+Kimi 2026-09-25).
        guard layoutGate.event(selfMove: selfMove) else { return }
        guard trusted, !windows.isEmpty, let planned = plan(choice, mode) else { return }
        let arr = planned.arrangement
        logPlan(planned.target, count: arr.frames.count, parked: arr.parked.count)
        rememberParking(arr)
        arrangement = arr
        arrangementAX = windows.map(\.ax)
        // Dirty-Check gegen den GEMESSENEN Ist-Frame (Konvention 4) — und kein erneuter Versuch
        // fuer ein Ziel, das dieses Fenster schon einmal verfehlt hat (`needsMove`).
        var moves = arr.frames.compactMap { idx, dest -> WindowMove? in
            guard windows.indices.contains(idx) else { return nil }
            let w = windows[idx]
            let memory = moveMemory.first { CFEqual($0.ax, w.ax) }?.memory
            guard needsMove(current: w.frame, dest: dest, memory: memory) else { return nil }
            return WindowMove(win: w.ax, origin: axOrigin(for: w.bundleId), from: w.frame, to: dest)
        }
        // R2-7: ein geparktes Fenster, das Kevin gerade ausgeschlossen hat, faehrt einmal an seinen letzten sichtbaren
        // Platz — in derselben Fahrt, eine zweite loeste die erste ab. Nur solange es ausgeschlossen ist: wieder
        // aufgenommen (etwa in der Pause), hat es einen geplanten Platz, zwei Ziele in einer Fahrt liessen es zappeln
        // (review quick 2026-09-26, Kimi).
        if let r = pendingRestore, let idx = indexOfWindow(r.ax, in: windows), arr.frames[idx] == nil,
           framesDiffer(windows[idx].frame, r.frame) {
            moves.append(WindowMove(win: r.ax, origin: axOrigin(for: windows[idx].bundleId), from: windows[idx].frame,
                                    to: r.frame))
        }
        pendingRestore = nil
        guard !moves.isEmpty else { return }
        // Lookout verschiebt nur, es legt nie nach hinten (GP 2): parkt diese Fahrt ein Fenster, liegt es danach womoeglich
        // VOR der Mitte. Darum die Mitte am Ende im Terminal anheben (AXRaise ordnet nur innerhalb einer App, R2-6).
        let raise = moves.contains { m in arr.parked.contains { CFEqual(windows[$0].ax, m.win) } }
            ? arr.main.map { windows[$0].ax } : nil
        movingIds = Set(moves.compactMap { m in windows.first { CFEqual($0.ax, m.win) }?.sessions.first?.id })
        refreshOverlays()
        selfMove = true
        moveEpoch += 1
        let height = primaryScreenHeight()
        animator.run(moves, screenH: height, onStep: { [weak self] steps in
            DispatchQueue.main.async { self?.followGlide(steps) }
        }) { [weak self] measured in
            DispatchQueue.main.async {
                guard let self else { return }
                self.selfMove = false
                self.moveEpoch += 1
                // GEMESSENE Ist-Frames uebernehmen, nicht die Soll-Werte: ein geclampt
                // stehengebliebenes Fenster wuerde sonst mit falschem Rahmen leuchten und
                // der Dirty-Check im naechsten Tick gegen eine Luege vergleichen.
                for (win, frame) in measured {
                    if let idx = indexOfWindow(win, in: self.windows) { self.windows[idx].frame = frame }
                    self.moveMemory.removeAll { CFEqual($0.ax, win) }
                    if let target = moves.first(where: { CFEqual($0.win, win) })?.to, framesDiffer(frame, target) {
                        self.moveMemory.append((win, MoveMemory(target: target, reached: frame)))
                    }
                }
                if let raise { axQueue.async { AXUIElementPerformAction(raise, kAXRaiseAction as CFString) } }
                self.movingIds = []
                self.refreshOverlays()
                // Ein Klick waehrend der Fahrt war vorgemerkt — jetzt auswerten (v4); ebenso ein vorgemerktes Anordnen.
                if self.focusGate.moveEnded() { self.handleFocusChange() }
                if self.layoutGate.moveEnded() { self.applyLayout() }
            }
        }
    }

    /// R2-7: wo stand ein Fenster, bevor Lookout es parkt? Gemerkt beim Wechsel in die Rolle "geparkt" (Rahmen vor der
    /// Bewegung), nur wenn es nicht schon auf der Mitte lag. Wer nicht mehr geparkt ist, faellt heraus. VOR dem Uebernehmen
    /// der neuen Planung aufrufen — `parkedIndices` meint hier noch die alte.
    private func rememberParking(_ arr: Arrangement) {
        let wasParked = parkedIndices()
        let mainRect = arr.main.flatMap { arr.frames[$0] }
        let old = parkOrigin
        parkOrigin = arr.parked.compactMap { i in
            let w = windows[i]
            if let kept = old.first(where: { CFEqual($0.ax, w.ax) }) { return kept }
            guard !wasParked.contains(i), let mainRect, framesDiffer(w.frame, mainRect) else { return nil }
            return (w.ax, w.frame)
        }
    }

    /// Rahmen gleitender Fenster auf ihren Zwischenstand setzen — je Bild der Fahrt (Kevin 2026-09-25).
    private func followGlide(_ steps: [(AXUIElement, CGRect)]) {
        guard !movingIds.isEmpty else { return }   // Fahrt schon beendet oder abgebrochen: nichts mehr versetzen
        var rects: [String: CGRect] = [:]
        for (win, rect) in steps {
            if let id = windows.first(where: { CFEqual($0.ax, win) })?.sessions.first?.id { rects[id] = rect }
        }
        glow.follow(rects)
    }

    // ---------- Menue ----------

    /// Menue "wie heute, nur aufgeraeumt" (Plan 1c): Bildschirm, Anordnung, Sessions nach Status, "Mehr" fuer den Rest.
    func menuNeedsUpdate(_ menu: NSMenu) {
        // Die Untermenues haben denselben Delegate (fuer die Vorschau) — nur das Hauptmenue wird hier neu aufgebaut,
        // die Untermenues entstehen dabei frisch mit.
        guard menu === self.menu else { return }
        menu.removeAllItems()

        if !trusted {
            let warn = NSMenuItem(title: tr("⚠︎ Bedienungshilfen fehlen — klicken zum Freigeben",
                                            "⚠︎ Accessibility access missing — click to grant"),
                                  action: #selector(requestTrust), keyEquivalent: "")
            warn.target = self
            menu.addItem(warn)
            menu.addItem(.separator())
        }

        // Wohin: nur Ziele, die mit den angeschlossenen Schirmen gehen. Der Haken sitzt auf
        // dem Ziel, das GERADE wirkt — fehlt der gewaehlte Schirm, auf dem Ersatz.
        // Jeder Schirm einzeln mit seinem echten Namen (Kevin 2026-10-01), dann die Leitstaende.
        let infos = screenInfos()
        let current = resolveTarget(choice, screens: infos, pair: pair)
        menu.addItem(.sectionHeader(title: tr("BILDSCHIRM", "SCREEN")))
        for option in targetOptions(infos) {
            guard let title = targetTitle(option, screens: infos, pair: pair) else { continue }
            let item = NSMenuItem(title: title, action: #selector(selectTarget(_:)), keyEquivalent: "")
            item.target = self
            item.state = resolveTarget(option, screens: infos, pair: pair) == current ? .on : .off
            item.representedObject = PREVIEW_TARGET + option.rawValue
            menu.addItem(item)
        }
        // Ab drei Schirmen: welcher ist Arbeit, welcher Leitstand (Kevin 2026-10-01: "im Menue waehlbar").
        if infos.count >= 3 {
            let item = NSMenuItem(title: tr("Leitstand-Schirme", "Control room screens"), action: nil, keyEquivalent: "")
            item.submenu = pairMenu(infos)
            menu.addItem(item)
        }

        // Anordnung: gilt fuer "alle auf einen Schirm". Der Leitstand ist fest.
        var leitstand = false
        if case .leitstand? = current { leitstand = true }
        menu.addItem(.sectionHeader(title: leitstand ? tr("ANORDNUNG (im Leitstand fest)", "LAYOUT (fixed in the control room)")
                                                     : tr("ANORDNUNG", "LAYOUT")))
        let vis = singleTargetVisible()
        for (i, m) in LayoutMode.allCases.enumerated() {
            // Das Raster traegt die gerade wirksame Wahl im Titel — die nimmt Ctrl+Opt+3.
            let title = m == .grid ? vis.map { "\(m.label) (\(fittedGrid(grid, on: $0).label))" } ?? m.label : m.label
            let item = NSMenuItem(title: title, action: #selector(selectMode(_:)),
                                  keyEquivalent: cfg.hotkeys && m != .grid ? String(i + 1) : "")
            item.keyEquivalentModifierMask = [.control, .option]
            item.target = self
            item.state = (m == mode && !leitstand) ? .on : .off
            item.isEnabled = !leitstand
            item.representedObject = PREVIEW_MODE + m.rawValue
            if m == .grid, let vis {
                item.submenu = gridMenu(vis, hotkeyLabel: cfg.hotkeys ? String(i + 1) : "")
            }
            menu.addItem(item)
        }

        menu.addItem(.separator())
        addSessions(to: menu)

        menu.addItem(.separator())
        let more = NSMenuItem(title: tr("Mehr", "More"), action: nil, keyEquivalent: "")
        more.submenu = moreMenu()
        menu.addItem(more)
        let quit = NSMenuItem(title: tr("Lookout beenden", "Quit Lookout"), action: #selector(quitApp), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
    }

    /// KI-Fenster mit Session, Reihenfolge nach Status (Frage, fertig, laufend — `attentionOrder`).
    private func aiWindowsByStatus() -> [Int] {
        let ai = windows.indices.filter { !windows[$0].sessions.isEmpty }
        return attentionOrder(ai.map { windows[$0] }).map { ai[$0] }
    }

    /// Sessions (Plan 1c): Kopf mit Fragen und Fertigen — "fertig" ohne die Frage (GP 12) —, dann je Fenster eine Zeile
    /// wie sein Schild. Klick holt es nach vorn (`bringToFront`), Maus darueber laesst es aufleuchten.
    private func addSessions(to menu: NSMenu) {
        let order = aiWindowsByStatus()
        let asking = order.filter { windows[$0].isWaiting }.count
        let ready = order.filter { windows[$0].isReady && !windows[$0].isWaiting }.count
        menu.addItem(.sectionHeader(title: sessionsHeader(asking: asking, ready: ready)))
        guard !order.isEmpty else {
            menu.addItem(disabledItem(noSessionText()))
            return
        }
        let onScreen = onScreenWindows(pids: Set(order.map { windows[$0].ownerPid }))
        let parked = parkedIndices()
        let now = Date()
        for idx in order.prefix(20) {
            let w = windows[idx]
            let line = sessionLine(w, since: since, now: now)
            let notes = sessionNotes(idx, onScreen: onScreen, parked: parked)
            let item = NSMenuItem(title: line.title + (notes.isEmpty ? "" : "  (\(notes.joined(separator: ", ")))"),
                                  action: #selector(bringToFrontItem(_:)), keyEquivalent: "")
            item.subtitle = line.subtitle
            item.target = self
            item.representedObject = PREVIEW_FRONT + w.sessions.map(\.id).joined(separator: "\n")
            menu.addItem(item)
        }
    }

    /// Zusaetze einer Session-Zeile: wo das Fenster ist (`whereabouts`), ob Lookout es anordnet, ob es geparkt ist.
    /// "In der Liste" kommt aus der Planung, nicht aus dem Schirm-Abgleich — der zaehlt Deckungsgleiches als sichtbar
    /// (GP 2).
    private func sessionNotes(_ idx: Int, onScreen: [OnScreenWindow]?, parked: Set<Int>) -> [String] {
        let w = windows[idx]
        var notes = [whereabouts(w, onScreen: isOnScreen(w, onScreen)).label].filter { !$0.isEmpty }
        if isExcluded(w, excluded) {
            // R2-7: war es nie sichtbar (Fokus 2 parkt Laufende von Anfang an), liegt es ausgeschlossen hinter der Mitte.
            let behind = mainIndex().map { $0 != idx && !framesDiffer(w.frame, windows[$0].frame) } ?? false
            notes.append(behind ? tr("ausgeschlossen, hinter der Mitte", "excluded, behind the center")
                                : tr("nicht angeordnet", "not arranged"))
        } else if parked.contains(idx) {
            notes.append(tr("in der Liste", "in the list"))
        }
        return notes
    }

    /// "Mehr" (Plan 1c): Schalter, Fenster auswaehlen, Ohne Fenster.
    private func moreMenu() -> NSMenu {
        let sub = submenu()
        let labelItem = NSMenuItem(title: tr("Schilder anzeigen", "Show labels"), action: #selector(toggleLabels),
                                   keyEquivalent: "")
        labelItem.target = self
        labelItem.state = labelsOn ? .on : .off
        sub.addItem(labelItem)
        let sideItem = NSMenuItem(title: side == .right ? tr("Streifen rechts", "Strip on the right")
                                                        : tr("Streifen links", "Strip on the left"),
                                  action: #selector(toggleSide), keyEquivalent: "")
        sideItem.target = self
        sub.addItem(sideItem)
        let pause = NSMenuItem(title: tr("Pause (nichts bewegen)", "Pause (move nothing)"), action: #selector(togglePause),
                               keyEquivalent: "")
        pause.target = self
        pause.state = paused ? .on : .off
        sub.addItem(pause)
        sub.addItem(.separator())
        let choose = NSMenuItem(title: tr("Fenster auswählen", "Choose windows"), action: nil, keyEquivalent: "")
        choose.submenu = windowsMenu()
        sub.addItem(choose)

        // Nur Echtes (Kevin 2026-09-18): beendete Hintergrund-Laeufe und wochenalte Agenten
        // fuellten die Liste auf 35 Eintraege.
        let inWindows = Set(windows.flatMap { $0.sessions.map(\.id) })
        let orphans = windowlessSessions(liveSessions, inWindows: inWindows, now: Date())
        if !orphans.shown.isEmpty || orphans.staleAgents > 0 {
            let list = submenu()
            for s in orphans.shown.prefix(8) {
                list.addItem(disabledItem(
                    "\(s.waiting ? "?" : (s.ready ? "●" : "○")) \(s.engine.rawValue): \(s.digest?.name ?? s.name)"))
            }
            if orphans.staleAgents > 0 {
                list.addItem(disabledItem("\(orphans.staleAgents) " + tr("alte Hintergrund-Agenten (> 24 h) — aufräumen mit „claude agents“",
                                                                         "old background agents (> 24 h) — clean up with “claude agents”")))
            }
            let item = NSMenuItem(title: tr("Ohne Fenster", "Without window") + " (\(orphans.shown.count))", action: nil,
                                  keyEquivalent: "")
            item.submenu = list
            sub.addItem(item)
        }
        return sub
    }

    /// "Fenster auswählen": Haken = wird angeordnet, Klick nimmt das Fenster raus oder wieder rein, die Maus darueber
    /// laesst genau dieses Fenster aufleuchten (Kevin 2026-09-18). Wer nicht zu sehen ist, sagt es im Titel.
    private func windowsMenu() -> NSMenu {
        let sub = submenu()
        let order = aiWindowsByStatus()
        guard !order.isEmpty else {
            sub.addItem(disabledItem(noSessionText()))
            return sub
        }
        let onScreen = onScreenWindows(pids: Set(order.map { windows[$0].ownerPid }))
        let now = Date()
        for idx in order.prefix(20) {
            let w = windows[idx]
            let place = whereabouts(w, onScreen: isOnScreen(w, onScreen))
            let item = NSMenuItem(title: sessionLine(w, since: since, now: now).title
                                      + (place.label.isEmpty ? "" : "  (\(place.label))"),
                                  action: #selector(toggleExcluded(_:)), keyEquivalent: "")
            item.target = self
            item.state = isExcluded(w, excluded) ? .off : .on
            item.representedObject = PREVIEW_WINDOW + w.sessions.map(\.id).joined(separator: "\n")
            sub.addItem(item)
        }
        return sub
    }

    /// "Leitstand-Schirme" (ab drei Schirmen, Kevin 2026-10-01): Arbeit = wo die aktive Session steht, Leitstand = wo die
    /// Laufenden stehen. Haken = das Paar, das gerade gilt; "Automatisch" = wie bisher aus den Groessen.
    private func pairMenu(_ infos: [ScreenInfo]) -> NSMenu {
        let sub = submenu()
        let names = screenNames(infos)
        let eff = effectivePair(infos)
        let roles = [(tr("ARBEIT (aktive Session)", "WORK (active session)"), PAIR_WORK, eff?.work),
                     (tr("LEITSTAND (laufende Sessions)", "CONTROL ROOM (running sessions)"), PAIR_CONTROL, eff?.control)]
        for (header, prefix, chosen) in roles {
            sub.addItem(.sectionHeader(title: header))
            for (i, s) in infos.enumerated() where !s.id.isEmpty {
                let item = NSMenuItem(title: names[i], action: #selector(selectPair(_:)), keyEquivalent: "")
                item.target = self
                item.state = i == chosen ? .on : .off
                item.representedObject = prefix + s.id
                sub.addItem(item)
            }
        }
        sub.addItem(.separator())
        // Dieselbe Aufloesung wie die Anordnung (`activePair`: leere Kennungen zaehlen nicht, Zwillinge schon).
        let auto = NSMenuItem(title: tr("Automatisch", "Automatic"), action: #selector(selectPair(_:)), keyEquivalent: "")
        auto.target = self
        auto.state = activePair(pair, screens: infos) == nil ? .on : .off
        auto.representedObject = PAIR_AUTO
        sub.addItem(auto)
        return sub
    }

    /// Leitstand-Schirme, die gerade gelten (gewaehlt oder automatisch), fuer die aktuelle Leitstand-Variante bzw. Leitstand 1.
    private func effectivePair(_ infos: [ScreenInfo]) -> (work: Int, control: Int)? {
        let variant: TargetChoice = [.both, .both2, .both3].contains(choice) ? choice : .both
        guard case .leitstand(let work, let control, _, _)? = resolveTarget(variant, screens: infos, pair: pair) else {
            return nil
        }
        return (work, control)
    }

    /// Untermenue mit Vorschau beim Ueberfahren: ohne Delegate keine Vorschau, ohne `autoenablesItems = false`
    /// ueberschreibt AppKit jedes `isEnabled` (GP 12).
    private func submenu() -> NSMenu {
        let sub = NSMenu()
        sub.delegate = self
        sub.autoenablesItems = false
        return sub
    }

    /// Feste Raster, die auf den Ziel-Schirm passen. Der Haken sitzt auf dem Raster, das
    /// wirkt (die Wahl auf den Schirm gestutzt); dort steht auch das Tastenkuerzel.
    private func gridMenu(_ vis: CGRect, hotkeyLabel: String) -> NSMenu {
        let sub = submenu()
        let current = fittedGrid(grid, on: vis)
        for g in gridChoices(for: vis) {
            let item = NSMenuItem(title: g.label, action: #selector(selectGrid(_:)),
                                  keyEquivalent: g == current ? hotkeyLabel : "")
            item.keyEquivalentModifierMask = [.control, .option]
            item.target = self
            item.state = (mode == .grid && g == current) ? .on : .off
            item.representedObject = PREVIEW_GRID + g.key
            sub.addItem(item)
        }
        return sub
    }

    /// Liegt das Fenster auf dem sichtbaren Space? Ohne Fensterliste oder bei mehrdeutigem
    /// Treffer gilt es als sichtbar — nur ein klares "nicht da" zaehlt.
    private func isOnScreen(_ w: TermWindow, _ list: [OnScreenWindow]?) -> Bool {
        locateOnScreen(w, onScreen: list, screenH: primaryScreenHeight()) != .none
    }

    private func disabledItem(_ t: String) -> NSMenuItem {
        let i = NSMenuItem(title: t, action: nil, keyEquivalent: "")
        i.isEnabled = false
        return i
    }

    private func noSessionText() -> String {
        tr("— keine Session einem Fenster zugeordnet —", "— no session matched to a window —")
    }

    // ---------- Vorschau beim Ueberfahren (Kevin 2026-09-18) ----------

    /// Faehrt die Maus ueber ein Ziel oder eine Anordnung, zeigt Lookout Umrisse genau dort,
    /// wo die echten Fenster landen WUERDEN. Erst ein Klick bewegt etwas.
    func menu(_: NSMenu, willHighlight item: NSMenuItem?) {
        guard trusted, let key = item?.representedObject as? String else { preview.hide(); return }
        // Session-Eintrag (Sessions oder "Fenster auswählen"): genau dieses Fenster leuchtet auf, dort wo es gerade
        // steht — damit Kevin nicht das falsche erwischt (Kevin 2026-09-18). Beide Listen haben eigene Kennungen, der
        // Text sagt, was der Klick tut (R2-4).
        for prefix in [PREVIEW_FRONT, PREVIEW_WINDOW] where key.hasPrefix(prefix) {
            let ids = Set(sessionIds(key, prefix: prefix))
            // Nur aufleuchten, wo das Fenster wirklich zu sehen ist — ein minimiertes oder auf
            // einem anderen Schreibtisch liegendes hinterliess sonst einen Umriss an leerer Stelle.
            guard let w = windows.first(where: { $0.sessions.contains { ids.contains($0.id) } }),
                  !w.minimized, isOnScreen(w, onScreenWindows(pids: [w.ownerPid])) else {
                preview.hide()
                return
            }
            let detail = prefix == PREVIEW_FRONT ? tr("Klick holt es nach vorn", "Click brings it to the front")
                : isExcluded(w, excluded) ? tr("ausgeschlossen — Klick nimmt es wieder auf", "excluded — click to include it again")
                : tr("wird angeordnet — Klick nimmt es raus", "arranged — click to exclude it")
            preview.show([PreviewBox(rect: w.frame, title: w.sessions.map(\.name).joined(separator: " · "), detail: detail)])
            return
        }
        var c = choice
        var m = mode
        var g: GridSpec?
        if key.hasPrefix(PREVIEW_TARGET), let t = TargetChoice(rawValue: String(key.dropFirst(PREVIEW_TARGET.count))) {
            c = t
        } else if key.hasPrefix(PREVIEW_MODE), let mm = LayoutMode(rawValue: String(key.dropFirst(PREVIEW_MODE.count))) {
            m = mm
        } else if key.hasPrefix(PREVIEW_GRID), let gg = GridSpec(String(key.dropFirst(PREVIEW_GRID.count))) {
            m = .grid
            g = gg
        } else {
            preview.hide()
            return
        }
        guard let planned = plan(c, m, grid: g) else { preview.hide(); return }
        let arr = planned.arrangement
        let active = mainIndex()
        // Geparkte Fenster (GP 2): kein Stapel von Umrissen auf der Mitte, sondern ein Kasten "Liste (N)" an der Listenstelle.
        let boxes = arr.frames.filter { !arr.parked.contains($0.key) }.map { idx, rect in
            let w = windows[idx]
            let state = idx == active ? tr("aktiv", "active") : windowTone(w).word.lowercased()   // Woerter wie auf dem Schild
            return PreviewBox(rect: rect, title: w.sessions.map(\.name).joined(separator: " · "),
                              detail: "\(w.sessions.first?.engine.rawValue ?? w.app) · \(state)")
        }
        preview.show(boxes + arr.lists.map {
            PreviewBox(rect: $0.rect, title: tr("Liste", "List") + " (\($0.count))",
                       detail: tr("Fenster liegen hinter der Mitte", "windows lie behind the center"))
        })
    }

    /// Session-IDs hinter einem Session-Eintrag im Menue.
    private func sessionIds(_ key: String, prefix: String) -> [String] {
        key.dropFirst(prefix.count).split(separator: "\n").map(String.init)
    }

    func menuDidClose(_ closed: NSMenu) {
        // Schliesst ein Untermenue (gleicher Delegate), bleibt sein Obermenue offen: die Vorschau folgt dem dort
        // hervorgehobenen Eintrag, statt zu verschwinden (review quick).
        guard closed !== menu else {
            preview.hide()
            return
        }
        let parent = closed.supermenu ?? menu
        menu(parent, willHighlight: parent.highlightedItem)
    }

    // ---------- Aktionen ----------

    @objc private func requestTrust() {
        _ = accessibilityTrusted(prompt: true)
    }

    @objc private func selectTarget(_ sender: NSMenuItem) {
        guard let key = sender.representedObject as? String, key.hasPrefix(PREVIEW_TARGET),
              var t = TargetChoice(rawValue: String(key.dropFirst(PREVIEW_TARGET.count))) else { return }
        // Derselbe Schirm wie die bisherige Wahl: deren Kennung bleibt ("X#2" statt des kurzen "X", solange der Zwilling
        // abgesteckt ist; review quick 2026-10-01 Runde 2).
        if case .screen(let new) = t, case .screen(let old) = choice, sameScreen(new, old, screens: screenInfos()) { t = choice }
        choice = t
        UserDefaults.standard.set(t.rawValue, forKey: DEFAULTS_TARGET)
        log("Ziel gewechselt: \(t.rawValue)")
        if !paused { applyLayout() }
    }

    /// Leitstand-Schirm gewaehlt (Arbeit oder Leitstand; derselbe Schirm fuer beide Rollen tauscht sie) oder zurueck auf
    /// automatisch. Wer Leitstand-Schirme waehlt, will den Leitstand: steht gerade "Alle auf …", gilt danach Leitstand 1.
    @objc private func selectPair(_ sender: NSMenuItem) {
        guard let key = sender.representedObject as? String else { return }
        if key == PAIR_AUTO {
            pair = nil
        } else {
            let infos = screenInfos()
            guard let eff = effectivePair(infos) else { return }
            // Gilt das gespeicherte Paar, bleiben seine Kennungen (Zwilling "X#2", review quick 2026-10-01 Runde 2, GLM).
            var now = (work: infos[eff.work].id, control: infos[eff.control].id)
            if let p = pair, activePair(p, screens: infos) != nil { now = p }
            if key.hasPrefix(PAIR_WORK) {
                pair = choosePair(current: now, work: String(key.dropFirst(PAIR_WORK.count)), screens: infos)
            } else if key.hasPrefix(PAIR_CONTROL) {
                pair = choosePair(current: now, control: String(key.dropFirst(PAIR_CONTROL.count)), screens: infos)
            }
            if ![.both, .both2, .both3].contains(choice) {
                choice = .both
                UserDefaults.standard.set(choice.rawValue, forKey: DEFAULTS_TARGET)
            }
        }
        let saved = UserDefaults.standard
        if let pair {
            saved.set(pair.work, forKey: DEFAULTS_PAIR_WORK)
            saved.set(pair.control, forKey: DEFAULTS_PAIR_CONTROL)
        } else {
            saved.removeObject(forKey: DEFAULTS_PAIR_WORK)
            saved.removeObject(forKey: DEFAULTS_PAIR_CONTROL)
        }
        log("Leitstand-Schirme: " + (pair.map { "Arbeit \($0.work), Leitstand \($0.control)" } ?? "automatisch")
            + ", ziel=\(choice.rawValue)")
        if !paused { applyLayout() }
    }

    @objc private func selectMode(_ sender: NSMenuItem) {
        guard let key = sender.representedObject as? String, key.hasPrefix(PREVIEW_MODE),
              let m = LayoutMode(rawValue: String(key.dropFirst(PREVIEW_MODE.count))) else { return }
        setMode(m, via: "Menue")
    }

    private func setMode(_ m: LayoutMode, via: String) {
        mode = m
        UserDefaults.standard.set(m.rawValue, forKey: DEFAULTS_MODE)
        log("Anordnung gewechselt (\(via)): \(m.rawValue)" + (m == .grid ? " \(grid.key)" : ""))
        if !paused { applyLayout() }
        refreshOverlays()
    }

    @objc private func selectGrid(_ sender: NSMenuItem) {
        guard let key = sender.representedObject as? String, key.hasPrefix(PREVIEW_GRID),
              let g = GridSpec(String(key.dropFirst(PREVIEW_GRID.count))) else { return }
        grid = g
        UserDefaults.standard.set(g.key, forKey: DEFAULTS_GRID)
        setMode(.grid, via: "Menue")
    }

    /// Fenster aus der Anordnung nehmen oder wieder aufnehmen — gemerkt je Session-ID.
    /// Der Status-Rahmen bleibt (Kevin 2026-09-18).
    @objc private func toggleExcluded(_ sender: NSMenuItem) {
        guard let key = sender.representedObject as? String, key.hasPrefix(PREVIEW_WINDOW) else { return }
        let ids = sessionIds(key, prefix: PREVIEW_WINDOW)
        guard !ids.isEmpty else { return }
        let wasOut = ids.contains(where: excluded.contains)
        if wasOut { excluded.subtract(ids) } else { excluded.formUnion(ids) }
        UserDefaults.standard.set(excluded.sorted(), forKey: DEFAULTS_EXCLUDED)
        log("Ausschliessen: \(ids.joined(separator: ",")) -> \(wasOut ? "wieder drin" : "raus")")
        // R2-7: war es geparkt, faehrt es mit der naechsten Anordnung einmal an seinen letzten sichtbaren Platz; war es nie
        // sichtbar, bleibt es liegen (Menue: "ausgeschlossen, hinter der Mitte").
        if !wasOut, let idx = windows.firstIndex(where: { $0.sessions.contains { ids.contains($0.id) } }),
           parkedIndices().contains(idx), let origin = parkOrigin.first(where: { CFEqual($0.ax, windows[idx].ax) }) {
            pendingRestore = origin
        }
        if !paused { applyLayout() }
    }

    @objc private func bringToFrontItem(_ sender: NSMenuItem) {
        guard let key = sender.representedObject as? String, key.hasPrefix(PREVIEW_FRONT) else { return }
        bringToFront(Set(sessionIds(key, prefix: PREVIEW_FRONT)))
    }

    /// Klick auf eine Session im Menue oder in der Liste (Plan 1c/1d, GP 4, R2-4): Fenster nach vorn — ein minimiertes
    /// zurueck, anheben, Hauptfenster, das Terminal ueber AX nach vorn (`activate` nur als Rueckfall: seit macOS 14 darf
    /// macOS das Aktivieren einer fremden App aus einer Menueleisten-App verweigern [Modellwissen]). Danach frisch lesen
    /// und wie ein Klick hinein werten (`onFocused`: ausgeschlossene und Vollbild-Fenster werden nie angeordnet) — nicht
    /// auf die Fokus-Meldung verlassen, sie prueft gegen den Schnappschuss, in dem das Fenster noch minimiert ist.
    private func bringToFront(_ ids: Set<String>) {
        guard trusted, let idx = windows.firstIndex(where: { $0.sessions.contains { ids.contains($0.id) } }) else { return }
        let w = windows[idx]
        logFocus("Nach vorn: \(focusName(idx))")
        axQueue.async { [weak self] in
            if axBool(w.ax, kAXMinimizedAttribute as String) == true {
                AXUIElementSetAttributeValue(w.ax, kAXMinimizedAttribute as CFString, kCFBooleanFalse)
            }
            AXUIElementPerformAction(w.ax, kAXRaiseAction as CFString)
            AXUIElementSetAttributeValue(w.ax, kAXMainAttribute as CFString, kCFBooleanTrue)
            let front = AXUIElementSetAttributeValue(AXUIElementCreateApplication(w.ownerPid),
                                                     kAXFrontmostAttribute as CFString, kCFBooleanTrue) == .success
            let fresh = rereadWindow(w)
            DispatchQueue.main.async {
                guard let self else { return }
                if !front {
                    log("Nach vorn: Terminal per AX nicht nach vorn — activate als Rueckfall")
                    _ = NSRunningApplication(processIdentifier: w.ownerPid)?.activate()
                }
                // Ein Tick, der schon laeuft, bringt den alten Schnappschuss mit (noch minimiert): Epoche erhoehen wie bei
                // eigenen Bewegungen, dann gelten die frisch gelesenen Werte (`keepMeasuredFrames`).
                self.moveEpoch += 1
                if let i = indexOfWindow(w.ax, in: self.windows) {
                    self.windows[i].frame = fresh.frame
                    self.windows[i].minimized = fresh.minimized
                    self.windows[i].fullscreen = fresh.fullscreen
                }
                self.onFocused(w.ax)
                self.tick()
            }
        }
    }

    @objc private func toggleLabels() {
        labelsOn.toggle()
        UserDefaults.standard.set(labelsOn, forKey: DEFAULTS_LABELS)
        log("Schilder: \(labelsOn ? "an" : "aus")")
        refreshOverlays()
    }

    @objc private func togglePause() {
        paused.toggle()
        log("Pause: \(paused)")
        if paused {
            // Der abgebrochene Lauf ruft sein onFinish nicht mehr — selfMove muss hier
            // zurueckgesetzt werden, sonst ignoriert die App Fokus-Klicks auf Dauer.
            animator.cancel()
            selfMove = false
            // Auch hier: der abgebrochene Lauf meldet sich nie zurueck. Die Schilder kommen mit dem
            // naechsten Tick wieder, der die echten Zwischen-Rahmen neu liest — sofort gezeichnet
            // stuenden sie an den alten Plaetzen (review quick 2026-09-21, Kimi).
            movingIds = []
            layoutGate = MoveGate()                              // Pause heisst: nicht nachtraeglich anordnen
            if focusGate.moveEnded() { handleFocusChange() }   // vorgemerkten Klick nicht verlieren (v4)
        } else {
            // Erst frisch lesen, dann anordnen: nach einer abgebrochenen Fahrt stehen die Fenster irgendwo dazwischen,
            // die gemerkten Rahmen stimmen nicht — ein Plan darauf liess sie springen (review quick 2026-09-25, Kimi).
            tick()
        }
    }

    @objc private func toggleSide() {
        side = (side == .right) ? .left : .right
        log("Streifen-Seite: \(side.rawValue)")
        if !paused { applyLayout() }
    }

    @objc private func quitApp() {
        animator.cancel()
        removeHotkeys()
        glow.hideAll()
        labels.hideAll()
        lists.hideAll()
        preview.hide()
        log("--- Lookout beendet ---")
        NSApplication.shared.terminate(nil)
    }
}

@main
enum LookoutMain {
    static func main() {
        let app = NSApplication.shared
        let controller = LookoutController()
        app.delegate = controller
        app.setActivationPolicy(.accessory)
        app.run()
    }
}
