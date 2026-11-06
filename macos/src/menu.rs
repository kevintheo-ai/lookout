//! The menu (Main.swift `menuNeedsUpdate`, plan 1c "wie heute, nur aufgeräumt"): screen, arrangement, the parking lot
//! (only with worker names), sessions by status, "More" for the rest, and the preview outlines on hovering (Kevin
//! 2026-09-18). Rebuilt each time it opens. The delegate object is also the target of every menu action.

use std::collections::BTreeSet;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuDelegate,
    NSMenuItem,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

use lookout_core::i18n::tr;
use lookout_core::layout::{
    active_pair, attention_order, fitted_grid, grid_choices, is_excluded, is_worker,
    resolve_target, screen_names, target_options, target_title,
};
use lookout_core::model::{
    GridSpec, LayoutMode, OnScreenWindow, Pid, ScreenInfo, Target, TargetChoice, TermWindow,
};
use lookout_core::motion::{FRAME_TOLERANCE, frames_differ};
use lookout_core::overlay::{
    OnScreenMatch, PreviewBox, locate_on_screen, preview_boxes, session_line, sessions_header,
    whereabouts, window_tone,
};
use lookout_core::status::windowless_sessions;

use crate::controller::{App, with_app, with_app_now};
use crate::{ax, now, screens};

// Keys on the menu items: the preview tells by them what is hovered.
const PREVIEW_TARGET: &str = "target:";
const PREVIEW_MODE: &str = "mode:";
const PREVIEW_GRID: &str = "grid:";
/// "Choose windows": the window's session ids follow, one per line. A click toggles arranging.
const PREVIEW_WINDOW: &str = "win:";
/// Session list (plan 1c): the same ids, own key; a click brings the window to the front (R2-4).
const PREVIEW_FRONT: &str = "front:";
/// "Show parking lot": the tiles of the peek.
const PREVIEW_PEEK: &str = "peek:";
/// "To the parking lot": the window's session ids follow; a click sends it there by hand or takes it back.
const PREVIEW_HAND: &str = "hand:";
/// Control-room screens: the screen id follows; "auto" = automatic again.
const PAIR_WORK: &str = "pair-work:";
const PAIR_CONTROL: &str = "pair-control:";
const PAIR_AUTO: &str = "pair-auto";

/// A control-room screen choice from the menu.
pub(crate) enum PairKey {
    Auto,
    Work(String),
    Control(String),
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LookoutMenuDelegate"]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSMenuDelegate for Delegate {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            // The submenus share this delegate (for the preview): only the main menu is rebuilt here, its submenus
            // come fresh with it.
            with_app_now(|a| {
                if std::ptr::eq(menu, &*a.menu) {
                    rebuild(a);
                }
            });
        }

        #[unsafe(method(menu:willHighlightItem:))]
        fn menu_will_highlight_item(&self, _menu: &NSMenu, item: Option<&NSMenuItem>) {
            with_app_now(|a| highlight(a, item));
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, closed: &NSMenu) {
            // When a submenu closes its parent stays open: the preview follows the item highlighted there instead of
            // vanishing (review quick).
            with_app_now(|a| {
                if std::ptr::eq(closed, &*a.menu) {
                    a.preview.hide();
                    return;
                }
                // SAFETY: plain getter of the menu tree, main thread.
                let parent = unsafe { closed.supermenu() }.unwrap_or_else(|| a.menu.clone());
                highlight(a, parent.highlightedItem().as_deref());
            });
        }
    }

    impl Delegate {
        #[unsafe(method(requestTrust:))]
        fn request_trust(&self, _sender: Option<&AnyObject>) {
            with_app(App::request_trust);
        }

        #[unsafe(method(selectTarget:))]
        fn select_target(&self, sender: &NSMenuItem) {
            if let Some(t) = key_after(sender, PREVIEW_TARGET).and_then(|k| TargetChoice::from_raw(&k)) {
                with_app(move |a| a.select_target(t));
            }
        }

        #[unsafe(method(selectPair:))]
        fn select_pair(&self, sender: &NSMenuItem) {
            let Some(key) = item_key(sender) else { return };
            let choice = if key == PAIR_AUTO {
                PairKey::Auto
            } else if let Some(id) = key.strip_prefix(PAIR_WORK) {
                PairKey::Work(id.to_owned())
            } else if let Some(id) = key.strip_prefix(PAIR_CONTROL) {
                PairKey::Control(id.to_owned())
            } else {
                return;
            };
            with_app(move |a| a.select_pair(choice));
        }

        #[unsafe(method(selectMode:))]
        fn select_mode(&self, sender: &NSMenuItem) {
            if let Some(m) = key_after(sender, PREVIEW_MODE).and_then(|k| LayoutMode::from_raw(&k)) {
                with_app(move |a| a.set_mode(m, "Menue"));
            }
        }

        #[unsafe(method(selectGrid:))]
        fn select_grid(&self, sender: &NSMenuItem) {
            if let Some(g) = key_after(sender, PREVIEW_GRID).and_then(|k| GridSpec::parse(&k)) {
                with_app(move |a| a.select_grid(g));
            }
        }

        #[unsafe(method(toggleExcluded:))]
        fn toggle_excluded(&self, sender: &NSMenuItem) {
            if let Some(k) = key_after(sender, PREVIEW_WINDOW) {
                let ids = session_ids(&k);
                with_app(move |a| a.toggle_excluded(&ids));
            }
        }

        #[unsafe(method(bringToFrontItem:))]
        fn bring_to_front_item(&self, sender: &NSMenuItem) {
            if let Some(k) = key_after(sender, PREVIEW_FRONT) {
                let ids: BTreeSet<String> = session_ids(&k).into_iter().collect();
                with_app(move |a| a.bring_to_front(&ids));
            }
        }

        #[unsafe(method(togglePeek:))]
        fn toggle_peek(&self, _sender: Option<&AnyObject>) {
            with_app(App::toggle_peek);
        }

        #[unsafe(method(toggleHandPark:))]
        fn toggle_hand_park(&self, sender: &NSMenuItem) {
            if let Some(k) = key_after(sender, PREVIEW_HAND) {
                let ids = session_ids(&k);
                with_app(move |a| a.toggle_hand_park(&ids));
            }
        }

        #[unsafe(method(toggleLabels:))]
        fn toggle_labels(&self, _sender: Option<&AnyObject>) {
            with_app(App::toggle_labels);
        }

        #[unsafe(method(togglePause:))]
        fn toggle_pause(&self, _sender: Option<&AnyObject>) {
            with_app(App::toggle_pause);
        }

        #[unsafe(method(toggleSide:))]
        fn toggle_side(&self, _sender: Option<&AnyObject>) {
            with_app(App::toggle_side);
        }

        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: Option<&AnyObject>) {
            with_app(App::quit);
        }
    }
);

impl Delegate {
    pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Delegate> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// A menu with the delegate (preview on hovering) and without AppKit's auto validation, which overrides every
/// `isEnabled = false` (an item with target and action stayed clickable, live 2026-09-18).
pub(crate) fn new_menu(mtm: MainThreadMarker, delegate: &Delegate) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setDelegate(Some(ProtocolObject::from_ref(delegate)));
    menu.setAutoenablesItems(false);
    menu
}

fn item_key(item: &NSMenuItem) -> Option<String> {
    let obj = item.representedObject()?;
    Some(obj.downcast::<NSString>().ok()?.to_string())
}

fn key_after(item: &NSMenuItem, prefix: &str) -> Option<String> {
    item_key(item)?.strip_prefix(prefix).map(str::to_owned)
}

/// The session ids behind a session item.
fn session_ids(rest: &str) -> Vec<String> {
    rest.split('\n')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn on_off(on: bool) -> isize {
    if on {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    }
}

fn ctrl_opt() -> NSEventModifierFlags {
    NSEventModifierFlags::Control | NSEventModifierFlags::Option
}

/// One menu item; `action` targets the delegate.
fn item(a: &App, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    // SAFETY: designated initializer on a fresh allocation; the action is a method of the delegate.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(a.mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    if action.is_some() {
        // SAFETY: the delegate lives as long as the app, and the menu items are rebuilt before every opening.
        unsafe { item.setTarget(Some(&a.delegate)) };
    }
    item
}

fn keyed(item: &NSMenuItem, key: &str) {
    // SAFETY: an NSString is a valid represented object.
    unsafe { item.setRepresentedObject(Some(&NSString::from_str(key))) };
}

fn header(a: &App, title: &str) -> Retained<NSMenuItem> {
    NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(title), a.mtm)
}

fn disabled(a: &App, title: &str) -> Retained<NSMenuItem> {
    let i = item(a, title, None, "");
    i.setEnabled(false);
    i
}

fn submenu(a: &App) -> Retained<NSMenu> {
    new_menu(a.mtm, &a.delegate)
}

fn no_session_text() -> &'static str {
    tr(
        "— keine Session einem Fenster zugeordnet —",
        "— no session matched to a window —",
    )
}

/// Does the window lie on the visible space? Without a window list or with an ambiguous hit it counts as visible:
/// only a clear "not there" counts.
fn is_on_screen(w: &TermWindow, list: Option<&[OnScreenWindow]>) -> bool {
    locate_on_screen(w, list, screens::primary_height()) != OnScreenMatch::Absent
}

fn on_screen_of(a: &App, order: &[usize]) -> Option<Vec<OnScreenWindow>> {
    let pids: BTreeSet<Pid> = order.iter().map(|&i| a.windows[i].owner_pid).collect();
    ax::on_screen_windows(Some(&pids))
}

/// AI windows with a session, ordered by status (question, done, running: `attention_order`).
fn ai_windows_by_status(a: &App) -> Vec<usize> {
    let ai: Vec<usize> = (0..a.windows.len())
        .filter(|&i| !a.windows[i].sessions.is_empty())
        .collect();
    let ws: Vec<&TermWindow> = ai.iter().map(|&i| &a.windows[i]).collect();
    attention_order(&ws).into_iter().map(|k| ai[k]).collect()
}

/// Usable area of the screen "all on one screen" lands on right now; `None` in the control room.
fn single_target_visible(a: &App, infos: &[ScreenInfo]) -> Option<lookout_core::geom::Rect> {
    match resolve_target(&a.choice, infos, a.pair.as_ref())? {
        Target::Single(s) => infos.get(s).map(|i| i.visible),
        Target::ControlRoom { .. } => None,
    }
}

fn rebuild(a: &mut App) {
    let menu = a.menu.clone();
    menu.removeAllItems();

    if !a.trusted {
        menu.addItem(&item(
            a,
            tr(
                "⚠︎ Bedienungshilfen fehlen — klicken zum Freigeben",
                "⚠︎ Accessibility access missing — click to grant",
            ),
            Some(sel!(requestTrust:)),
            "",
        ));
        menu.addItem(&NSMenuItem::separatorItem(a.mtm));
    }

    // Where: only targets that work with the attached screens. The check mark sits on the target that APPLIES right now;
    // with the chosen screen missing, on its stand-in. Every screen with its real name (Kevin 2026-10-01), then the
    // control rooms.
    let infos = a.screen_infos();
    let current = resolve_target(&a.choice, &infos, a.pair.as_ref());
    menu.addItem(&header(a, tr("BILDSCHIRM", "SCREEN")));
    for option in target_options(&infos) {
        let Some(title) = target_title(&option, &infos, a.pair.as_ref()) else {
            continue;
        };
        let it = item(a, &title, Some(sel!(selectTarget:)), "");
        it.setState(on_off(
            resolve_target(&option, &infos, a.pair.as_ref()) == current,
        ));
        keyed(&it, &format!("{PREVIEW_TARGET}{}", option.raw()));
        menu.addItem(&it);
    }
    // From three screens on: which is work and which control room (Kevin 2026-10-01: "im Menue waehlbar").
    if infos.len() >= 3 {
        let it = item(a, tr("Leitstand-Schirme", "Control room screens"), None, "");
        it.setSubmenu(Some(&pair_menu(a, &infos)));
        menu.addItem(&it);
    }

    // Arrangement: applies to "all on one screen". The control room is fixed.
    let control_room = matches!(current, Some(Target::ControlRoom { .. }));
    menu.addItem(&header(
        a,
        if control_room {
            tr(
                "ANORDNUNG (im Leitstand fest)",
                "LAYOUT (fixed in the control room)",
            )
        } else {
            tr("ANORDNUNG", "LAYOUT")
        },
    ));
    let vis = single_target_visible(a, &infos);
    for (i, m) in LayoutMode::ALL.into_iter().enumerate() {
        // The grid carries the choice in effect in its title: Ctrl+Opt+3 takes that one.
        let title = match (m, vis) {
            (LayoutMode::Grid, Some(v)) => {
                format!("{} ({})", m.label(), fitted_grid(a.grid, v).label())
            }
            _ => m.label().to_owned(),
        };
        let key = if a.cfg.hotkeys && m != LayoutMode::Grid {
            (i + 1).to_string()
        } else {
            String::new()
        };
        let it = item(a, &title, Some(sel!(selectMode:)), &key);
        it.setKeyEquivalentModifierMask(ctrl_opt());
        it.setState(on_off(m == a.mode && !control_room));
        it.setEnabled(!control_room);
        keyed(&it, &format!("{PREVIEW_MODE}{}", m.raw()));
        if m == LayoutMode::Grid
            && let Some(v) = vis
        {
            let hotkey = if a.cfg.hotkeys {
                (i + 1).to_string()
            } else {
                String::new()
            };
            it.setSubmenu(Some(&grid_menu(a, v, &hotkey)));
        }
        menu.addItem(&it);
    }
    // Parking lot (Kevin 2026-11-06), only with worker names; also in the control room. Check mark = peeking.
    if a.has_lot() {
        let n = a.lot_indices().len();
        let title = format!("{} ({n})", tr("Parkplatz anzeigen", "Show parking lot"));
        let key = if a.cfg.hotkeys { "4" } else { "" };
        let it = item(a, &title, Some(sel!(togglePeek:)), key);
        it.setKeyEquivalentModifierMask(ctrl_opt());
        it.setState(on_off(a.peek));
        it.setEnabled(a.peek || (n > 0 && !a.paused));
        keyed(&it, PREVIEW_PEEK);
        menu.addItem(&it);
        let hand = item(a, tr("Auf den Parkplatz", "To the parking lot"), None, "");
        hand.setSubmenu(Some(&hand_menu(a)));
        menu.addItem(&hand);
    }

    menu.addItem(&NSMenuItem::separatorItem(a.mtm));
    add_sessions(a, &menu);

    menu.addItem(&NSMenuItem::separatorItem(a.mtm));
    let more = item(a, tr("Mehr", "More"), None, "");
    more.setSubmenu(Some(&more_menu(a)));
    menu.addItem(&more);
    menu.addItem(&item(
        a,
        tr("Lookout beenden", "Quit Lookout"),
        Some(sel!(quitApp:)),
        "q",
    ));
}

/// Sessions (plan 1c): header with questions and done ones ("done" without the question, GP 12), then one line per
/// window like its label. A click brings it to the front, hovering lights it up.
fn add_sessions(a: &App, menu: &NSMenu) {
    let order = ai_windows_by_status(a);
    let asking = order.iter().filter(|&&i| a.windows[i].is_waiting()).count();
    let ready = order
        .iter()
        .filter(|&&i| a.windows[i].is_ready() && !a.windows[i].is_waiting())
        .count();
    menu.addItem(&header(a, &sessions_header(asking, ready)));
    if order.is_empty() {
        menu.addItem(&disabled(a, no_session_text()));
        return;
    }
    let on_screen = on_screen_of(a, &order);
    let parked = a.parked_indices();
    let lot = a.lot_indices();
    let lookout = a.lookout_lines();
    let n = now();
    for &idx in order.iter().take(20) {
        let w = &a.windows[idx];
        let line = session_line(w, &a.since, &lookout, n);
        let notes = session_notes(a, idx, on_screen.as_deref(), &parked, &lot);
        let title = if notes.is_empty() {
            line.title.clone()
        } else {
            format!("{}  ({})", line.title, notes.join(", "))
        };
        let it = item(a, &title, Some(sel!(bringToFrontItem:)), "");
        if let Some(sub) = &line.subtitle {
            it.setSubtitle(Some(&NSString::from_str(sub)));
        }
        keyed(&it, &format!("{PREVIEW_FRONT}{}", ids_of(w)));
        menu.addItem(&it);
    }
}

fn ids_of(w: &TermWindow) -> String {
    w.sessions
        .iter()
        .map(|s| s.id.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Notes of a session line: where the window is (`whereabouts`), whether Lookout arranges it, whether it is parked or
/// on the parking lot. "In the list" and "on the parking lot" come from the plan, not from the screen comparison,
/// which counts congruent windows as visible (GP 2).
fn session_notes(
    a: &App,
    idx: usize,
    on_screen: Option<&[OnScreenWindow]>,
    parked: &BTreeSet<usize>,
    lot: &BTreeSet<usize>,
) -> Vec<String> {
    let w = &a.windows[idx];
    let mut notes: Vec<String> = [whereabouts(w, is_on_screen(w, on_screen)).label()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if is_excluded(w, &a.excluded) {
        // R2-7: was it never visible (Focus 2 parks running ones from the start), it lies excluded behind the middle.
        let behind = a.main_index().is_some_and(|m| {
            m != idx && !frames_differ(w.frame, a.windows[m].frame, FRAME_TOLERANCE)
        });
        notes.push(
            if behind {
                tr(
                    "ausgeschlossen, hinter der Mitte",
                    "excluded, behind the center",
                )
            } else {
                tr("nicht angeordnet", "not arranged")
            }
            .to_owned(),
        );
    } else if lot.contains(&idx) {
        notes.push(tr("auf dem Parkplatz", "in the parking lot").to_owned());
    } else if parked.contains(&idx) {
        notes.push(tr("in der Liste", "in the list").to_owned());
    }
    notes
}

/// "More" (plan 1c): switches, choose windows, without window.
fn more_menu(a: &App) -> Retained<NSMenu> {
    let sub = submenu(a);
    let labels = item(
        a,
        tr("Schilder anzeigen", "Show labels"),
        Some(sel!(toggleLabels:)),
        "",
    );
    labels.setState(on_off(a.labels_on));
    sub.addItem(&labels);
    let side_title = if a.side == lookout_core::model::StackSide::Right {
        tr("Streifen rechts", "Strip on the right")
    } else {
        tr("Streifen links", "Strip on the left")
    };
    sub.addItem(&item(a, side_title, Some(sel!(toggleSide:)), ""));
    let pause = item(
        a,
        tr("Pause (nichts bewegen)", "Pause (move nothing)"),
        Some(sel!(togglePause:)),
        "",
    );
    pause.setState(on_off(a.paused));
    sub.addItem(&pause);
    sub.addItem(&NSMenuItem::separatorItem(a.mtm));
    let choose = item(a, tr("Fenster auswählen", "Choose windows"), None, "");
    choose.setSubmenu(Some(&windows_menu(a)));
    sub.addItem(&choose);

    // Only what is real (Kevin 2026-09-18): ended background runs and week-old agents filled the list to 35 entries.
    let in_windows: BTreeSet<String> = a
        .windows
        .iter()
        .flat_map(|w| w.sessions.iter().map(|s| s.id.clone()))
        .collect();
    let (shown, stale) = windowless_sessions(&a.live_sessions, &in_windows, now());
    if !shown.is_empty() || stale > 0 {
        let list = submenu(a);
        for s in shown.iter().take(8) {
            let mark = if s.waiting {
                "?"
            } else if s.ready {
                "●"
            } else {
                "○"
            };
            let name = s
                .digest
                .as_ref()
                .and_then(|d| d.name.clone())
                .unwrap_or_else(|| s.name.clone());
            list.addItem(&disabled(a, &format!("{mark} {}: {name}", s.engine.raw())));
        }
        if stale > 0 {
            list.addItem(&disabled(
                a,
                &format!(
                    "{stale} {}",
                    tr(
                        "alte Hintergrund-Agenten (> 24 h) — aufräumen mit „claude agents“",
                        "old background agents (> 24 h) — clean up with “claude agents”"
                    )
                ),
            ));
        }
        let it = item(
            a,
            &format!("{} ({})", tr("Ohne Fenster", "Without window"), shown.len()),
            None,
            "",
        );
        it.setSubmenu(Some(&list));
        sub.addItem(&it);
    }
    sub
}

/// "To the parking lot" (Kevin 2026-11-06 evening): check mark = sent there by hand, a click sends or takes back,
/// hovering lights up the window. Workers are there by name: checked, not clickable. The window Kevin is in carries
/// Ctrl+Opt+5, which does the same for it.
fn hand_menu(a: &App) -> Retained<NSMenu> {
    let sub = submenu(a);
    let order = ai_windows_by_status(a);
    if order.is_empty() {
        sub.addItem(&disabled(a, no_session_text()));
        return sub;
    }
    let lookout = a.lookout_lines();
    let n = now();
    let focused = a.focused_index();
    for &idx in order.iter().take(20) {
        let w = &a.windows[idx];
        let worker = is_worker(w, &a.cfg.worker_names, &a.cfg.lead_names);
        let line = session_line(w, &a.since, &lookout, n);
        let title = if worker {
            format!(
                "{}  ({})",
                line.title,
                tr("Arbeiter, automatisch", "worker, automatic")
            )
        } else {
            line.title
        };
        let key = if a.cfg.hotkeys && Some(idx) == focused {
            "5"
        } else {
            ""
        };
        let it = item(a, &title, Some(sel!(toggleHandPark:)), key);
        it.setKeyEquivalentModifierMask(ctrl_opt());
        it.setState(on_off(
            worker || w.sessions.iter().any(|s| a.hand_parked.contains(&s.id)),
        ));
        it.setEnabled(!worker);
        keyed(&it, &format!("{PREVIEW_HAND}{}", ids_of(w)));
        sub.addItem(&it);
    }
    sub
}

/// "Choose windows": check mark = arranged, a click takes the window out or back in, hovering lights up exactly this
/// window (Kevin 2026-09-18). Whoever is not to be seen says so in the title.
fn windows_menu(a: &App) -> Retained<NSMenu> {
    let sub = submenu(a);
    let order = ai_windows_by_status(a);
    if order.is_empty() {
        sub.addItem(&disabled(a, no_session_text()));
        return sub;
    }
    let on_screen = on_screen_of(a, &order);
    let lookout = a.lookout_lines();
    let n = now();
    for &idx in order.iter().take(20) {
        let w = &a.windows[idx];
        let place = whereabouts(w, is_on_screen(w, on_screen.as_deref())).label();
        let line = session_line(w, &a.since, &lookout, n);
        let title = if place.is_empty() {
            line.title
        } else {
            format!("{}  ({place})", line.title)
        };
        let it = item(a, &title, Some(sel!(toggleExcluded:)), "");
        it.setState(on_off(!is_excluded(w, &a.excluded)));
        keyed(&it, &format!("{PREVIEW_WINDOW}{}", ids_of(w)));
        sub.addItem(&it);
    }
    sub
}

/// "Control room screens" (from three screens on, Kevin 2026-10-01): work = where the active session stands, control
/// room = where the running ones stand. Check mark = the pair in effect; "Automatic" = from the sizes as before.
fn pair_menu(a: &App, infos: &[ScreenInfo]) -> Retained<NSMenu> {
    let sub = submenu(a);
    let names = screen_names(infos);
    let eff = a.effective_pair(infos);
    let roles = [
        (
            tr("ARBEIT (aktive Session)", "WORK (active session)"),
            PAIR_WORK,
            eff.as_ref().map(|p| p.work),
        ),
        (
            tr(
                "LEITSTAND (laufende Sessions)",
                "CONTROL ROOM (running sessions)",
            ),
            PAIR_CONTROL,
            eff.as_ref().map(|p| p.control),
        ),
    ];
    for (title, prefix, chosen) in roles {
        sub.addItem(&header(a, title));
        for (i, s) in infos.iter().enumerate().filter(|(_, s)| !s.id.is_empty()) {
            let it = item(a, &names[i], Some(sel!(selectPair:)), "");
            it.setState(on_off(Some(i) == chosen));
            keyed(&it, &format!("{prefix}{}", s.id));
            sub.addItem(&it);
        }
    }
    sub.addItem(&NSMenuItem::separatorItem(a.mtm));
    // The same resolution as the arrangement (`active_pair`: empty ids do not count, twins do).
    let auto = item(
        a,
        tr("Automatisch", "Automatic"),
        Some(sel!(selectPair:)),
        "",
    );
    auto.setState(on_off(active_pair(a.pair.as_ref(), infos).is_none()));
    keyed(&auto, PAIR_AUTO);
    sub.addItem(&auto);
    sub
}

/// Fixed grids that fit the target screen. The check mark sits on the grid in effect (the choice trimmed to the screen);
/// the shortcut stands there too.
fn grid_menu(a: &App, vis: lookout_core::geom::Rect, hotkey: &str) -> Retained<NSMenu> {
    let sub = submenu(a);
    let current = fitted_grid(a.grid, vis);
    for g in grid_choices(vis) {
        let it = item(
            a,
            &g.label(),
            Some(sel!(selectGrid:)),
            if g == current { hotkey } else { "" },
        );
        it.setKeyEquivalentModifierMask(ctrl_opt());
        it.setState(on_off(a.mode == LayoutMode::Grid && g == current));
        keyed(&it, &format!("{PREVIEW_GRID}{}", g.key()));
        sub.addItem(&it);
    }
    sub
}

/// Hovering a target or arrangement shows outlines exactly where the real windows WOULD land; only a click moves
/// something. A session item lights up exactly that window where it stands now (Kevin 2026-09-18), so Kevin does not
/// catch the wrong one; both lists have their own keys and the text says what the click does (R2-4).
fn highlight(a: &mut App, item: Option<&NSMenuItem>) {
    let Some(key) = item.and_then(item_key).filter(|_| a.trusted) else {
        a.preview.hide();
        return;
    };
    for prefix in [PREVIEW_FRONT, PREVIEW_WINDOW, PREVIEW_HAND] {
        let Some(rest) = key.strip_prefix(prefix) else {
            continue;
        };
        let ids: BTreeSet<String> = session_ids(rest).into_iter().collect();
        // Only where the window really is: a minimized one or one on another desktop left an outline on an empty spot.
        let Some(w) = a
            .windows
            .iter()
            .find(|w| w.sessions.iter().any(|s| ids.contains(&s.id)))
            .filter(|w| {
                !w.minimized
                    && is_on_screen(
                        w,
                        ax::on_screen_windows(Some(&BTreeSet::from([w.owner_pid]))).as_deref(),
                    )
            })
        else {
            a.preview.hide();
            return;
        };
        let detail = if prefix == PREVIEW_FRONT {
            tr("Klick holt es nach vorn", "Click brings it to the front")
        } else if prefix == PREVIEW_HAND {
            if w.sessions.iter().any(|s| a.hand_parked.contains(&s.id)) {
                tr(
                    "auf dem Parkplatz — Klick holt es zurück",
                    "on the parking lot — click brings it back",
                )
            } else {
                tr(
                    "Klick schickt es auf den Parkplatz",
                    "click sends it to the parking lot",
                )
            }
        } else if is_excluded(w, &a.excluded) {
            tr(
                "ausgeschlossen — Klick nimmt es wieder auf",
                "excluded — click to include it again",
            )
        } else {
            tr(
                "wird angeordnet — Klick nimmt es raus",
                "arranged — click to exclude it",
            )
        };
        let title = w
            .sessions
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        a.preview.show(&[PreviewBox {
            rect: w.frame,
            title,
            detail: detail.to_owned(),
        }]);
        return;
    }
    // "Show parking lot": where the tiles would stand (Kevin 2026-11-06: the preview was missing there).
    if key == PREVIEW_PEEK {
        let Some((_, arr)) = a
            .plan(&a.choice.clone(), a.mode, None, true)
            .filter(|_| !a.peek)
        else {
            a.preview.hide();
            return;
        };
        let boxes: Vec<PreviewBox> = arr
            .lot
            .iter()
            .filter(|i| !arr.parked.contains(i))
            .filter_map(|i| {
                let w = a.windows.get(*i)?;
                let engine = w
                    .sessions
                    .first()
                    .map_or(w.app.as_str(), |s| s.engine.raw());
                Some(PreviewBox {
                    rect: arr.frames[i],
                    title: w
                        .sessions
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" · "),
                    detail: format!("{engine} · {}", window_tone(w).word().to_lowercase()),
                })
            })
            .collect();
        a.preview.show(&boxes);
        return;
    }
    let (mut c, mut m, mut g) = (a.choice.clone(), a.mode, None);
    if let Some(t) = key
        .strip_prefix(PREVIEW_TARGET)
        .and_then(TargetChoice::from_raw)
    {
        c = t;
    } else if let Some(mm) = key
        .strip_prefix(PREVIEW_MODE)
        .and_then(LayoutMode::from_raw)
    {
        m = mm;
    } else if let Some(gg) = key.strip_prefix(PREVIEW_GRID).and_then(GridSpec::parse) {
        m = LayoutMode::Grid;
        g = Some(gg);
    } else {
        a.preview.hide();
        return;
    }
    let Some((_, arr)) = a.plan(&c, m, g, a.peek) else {
        a.preview.hide();
        return;
    };
    // Parked windows (GP 2): no pile of outlines on the middle, a box "List (N)" where the list goes.
    let boxes = preview_boxes(&arr, &a.windows, a.main_index());
    a.preview.show(&boxes);
}
