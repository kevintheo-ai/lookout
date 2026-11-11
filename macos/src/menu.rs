//! The menu (Main.swift `menuNeedsUpdate`; design A "nach Status gruppiert", Kevin 2026-11-06 evening): a real macOS
//! menu with symbols, status dots, sections with counts, the name in front and the state below it. Screen, arrangement,
//! the parking lot (only with worker names), sessions by status, "More" for the rest, and the preview outlines on
//! hovering (Kevin 2026-09-18). Rebuilt each time it opens. The delegate object is also the target of every menu action.

use std::collections::BTreeSet;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSColor, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags,
    NSFontWeightRegular, NSImage, NSImageSymbolConfiguration, NSMenu, NSMenuDelegate, NSMenuItem,
    NSMenuItemBadge,
};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString};

use lookout_core::config::color_from_hex;
use lookout_core::i18n::tr;
use lookout_core::launch::Favorite;
use lookout_core::layout::{
    active_pair, attention_order, fitted_grid, grid_choices, is_excluded, is_worker,
    resolve_target, screen_names, target_options, target_title,
};
use lookout_core::model::{
    GridSpec, LabelTone, LayoutMode, OnScreenWindow, Pid, ScreenInfo, Target, TargetChoice,
    TermWindow,
};
use lookout_core::motion::{FRAME_TOLERANCE, frames_differ};
use lookout_core::overlay::{
    MENU_LOT_ROWS, MENU_ROWS_MAX, MenuGroup, OnScreenMatch, PreviewBox, locate_on_screen,
    menu_groups, menu_row, more_rows_text, preview_boxes, rows_shown, whereabouts, window_tone,
};
use lookout_core::restore::OpenSession;
use lookout_core::status::windowless_sessions;

use crate::controller::{App, with_app, with_app_now};
use crate::overlay::srgb;
use crate::plus::{self, Choice};
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
/// "Restore sessions": the session id follows.
const PREVIEW_RESTORE: &str = "restore:";
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

        #[unsafe(method(restoreAll:))]
        fn restore_all(&self, _sender: Option<&AnyObject>) {
            with_app(|a| a.restore_sessions(None));
        }

        #[unsafe(method(restoreOne:))]
        fn restore_one(&self, sender: &NSMenuItem) {
            if let Some(id) = key_after(sender, PREVIEW_RESTORE) {
                with_app(move |a| a.restore_sessions(Some(&id)));
            }
        }

        #[unsafe(method(restoreClear:))]
        fn restore_clear(&self, _sender: Option<&AnyObject>) {
            with_app(App::clear_restorable);
        }

        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: Option<&AnyObject>) {
            with_app(App::quit);
        }

        /// The inbox's + and "New session" (`plus::menu`). The task dialog runs here, outside the app slot: it spins a
        /// loop of its own, and the app must stay reachable meanwhile.
        #[unsafe(method(newSessionItem:))]
        fn new_session_item(&self, sender: &NSMenuItem) {
            match item_key(sender).and_then(|k| plus::choice(&k)) {
                Some(Choice::Blank) => with_app(|a| a.new_terminal_window()),
                Some(Choice::Change(slot, field, value)) => {
                    with_app(move |a| a.change_favorite(slot, field, &value));
                }
                Some(Choice::Start(slot)) => {
                    // The favorite as the dialog shows it: changed in the menu meanwhile, it must not start something
                    // else (review quick 2026-11-10).
                    if let Some(f) = with_app_now(|a| a.favorite(slot)).flatten() {
                        ask_and_start(self.mtm(), f);
                    }
                }
                Some(Choice::Custom(f)) => ask_and_start(self.mtm(), f),
                Some(Choice::Team) => ask_and_start_team(self.mtm()),
                Some(Choice::ChangeTeam(role, field, value)) => {
                    with_app(move |a| a.change_team(role, field, &value));
                }
                None => {}
            }
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

/// The task dialog for `f`, then the start of exactly `f` (`newSessionItem:`). No dialog for a choice that no longer
/// fits: the daily model lists can change while a menu is open (review quick 2026-11-10).
fn ask_and_start(mtm: MainThreadMarker, f: Favorite) {
    let (fits, text, cards) =
        with_app_now(|a| (a.resolves(&f), a.favorite_text(&f), a.cards().to_vec()))
            .unwrap_or_else(|| (true, f.label(), vec![]));
    if !fits {
        return;
    }
    if let Some((task, card)) = plus::ask_task(mtm, &text, &cards) {
        with_app(move |a| a.start_favorite(&f, &task, &card));
    }
}

/// Kevin's own title for the card with `ids` (Kevin 2026-11-11), from its right-click menu; outside the app slot, the
/// dialog spins its own loop.
pub(crate) fn rename(ids: Vec<String>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some((id, shown)) = with_app_now(|a| a.title_target(&ids)).flatten() else {
        return;
    };
    if let Some(title) = plus::ask_title(mtm, &shown) {
        with_app(move |a| a.set_title(&id, &title));
    }
}

/// The team dialog, then the start of the team (`newSessionItem:`, Kevin 2026-11-10). The name is checked against the
/// teams open right now, each time Start is clicked.
fn ask_and_start_team(mtm: MainThreadMarker) {
    let Some((settings, setup)) = with_app_now(|a| a.team_dialog_input()) else {
        return;
    };
    // Busy app: no check, so no start either (review quick 2026-11-10); the dialog stays open.
    let check = |request: &crate::team_dialog::TeamRequest| {
        with_app_now(|a| a.check_team(request)).unwrap_or_else(|| {
            Err(tr(
                "Lookout ist gerade beschäftigt, bitte nochmal Starten.",
                "Lookout is busy, please click Start again.",
            )
            .to_owned())
        })
    };
    if let Some(request) = crate::team_dialog::ask_team(mtm, &settings, &setup, check) {
        with_app(move |a| a.start_team(request));
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

/// Sets the item's image so it shows. macOS 27 hides menu images unless the item asks for them (NSMenuItem.h: "AppKit
/// determines the visibility of menu item images, and will typically hide images"; measured 2026-11-06, neither symbols
/// nor drawn dots showed). The dots carry the state and Kevin chose the symbols (draft 2026-11-06). The property came
/// with macOS 27; Lookout runs from 26.0 on, where images show anyway.
fn show_image(item: &NSMenuItem, image: Option<Retained<NSImage>>) {
    let Some(image) = image else { return };
    item.setImage(Some(&image));
    if item.respondsToSelector(sel!(setPreferredImageVisibility:)) {
        // SAFETY: an NSInteger property of NSMenuItem; 1 = NSMenuItemImageVisibilityVisible (macOS 27 SDK).
        let () = unsafe { msg_send![item, setPreferredImageVisibility: 1isize] };
    }
}

/// An SF Symbol; `None` (no image) if this macOS lacks it.
fn symbol(name: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
}

/// The status dot of a session row (draft 2026-11-06): waiting and done in the colours of the frames, working a grey
/// dot, watching a grey ring, the parking lot (`None`) paler. Its description is the state, for VoiceOver.
fn dot(a: &App, tone: Option<LabelTone>) -> Option<Retained<NSImage>> {
    let (name, color) = match tone {
        Some(LabelTone::Waiting) => (
            "circle.fill",
            srgb(color_from_hex(&a.cfg.glow_color_waiting)),
        ),
        Some(LabelTone::Ready) => ("circle.fill", srgb(color_from_hex(&a.cfg.glow_color))),
        Some(LabelTone::Stopped) => ("pause.circle.fill", srgb(crate::overlay::STOPPED)),
        Some(LabelTone::Running) => ("circle.fill", NSColor::secondaryLabelColor()),
        Some(LabelTone::Watching) => ("circle", NSColor::secondaryLabelColor()),
        None => ("circle.fill", NSColor::tertiaryLabelColor()),
    };
    let said = tone.map_or(
        tr("auf dem Parkplatz", "in the parking lot"),
        LabelTone::word,
    );
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(&said.to_lowercase())),
    )?;
    // SAFETY: AppKit's weight constant, a plain number set before any code runs.
    let size = NSImageSymbolConfiguration::configurationWithPointSize_weight(10.0, unsafe {
        NSFontWeightRegular
    });
    let palette = NSImageSymbolConfiguration::configurationWithPaletteColors(
        &NSArray::from_retained_slice(&[color]),
    );
    image.imageWithSymbolConfiguration(&size.configurationByApplyingConfiguration(&palette))
}

/// The small grey capsule on the right (a duration, a grid, a state). AppKit gives it no colour of its own.
fn set_badge(item: &NSMenuItem, text: &str) {
    let badge =
        NSMenuItemBadge::initWithString(NSMenuItemBadge::alloc(), &NSString::from_str(text));
    item.setBadge(Some(&badge));
}

fn count_badge(item: &NSMenuItem, n: usize) {
    if n > 0 {
        let badge = NSMenuItemBadge::initWithCount(NSMenuItemBadge::alloc(), n as isize);
        item.setBadge(Some(&badge));
    }
}

fn set_subtitle(item: &NSMenuItem, text: Option<String>) {
    if let Some(t) = text {
        item.setSubtitle(Some(&NSString::from_str(&t)));
    }
}

/// A session row: status dot and name; its key carries the window's session ids behind `prefix` (what a click does).
fn session_item(
    a: &App,
    w: &TermWindow,
    title: &str,
    tone: Option<LabelTone>,
    action: Sel,
    prefix: &str,
) -> Retained<NSMenuItem> {
    let it = item(a, title, Some(action), "");
    show_image(&it, dot(a, tone));
    keyed(&it, &format!("{prefix}{}", ids_of(w)));
    it
}

/// Opens or closes the peek (Ctrl+Opt+4); hovering shows where its tiles would stand. `lot`: windows on the lot.
fn peek_item(a: &App, title: &str, lot: usize) -> Retained<NSMenuItem> {
    let it = item(
        a,
        title,
        Some(sel!(togglePeek:)),
        if a.cfg.hotkeys { "4" } else { "" },
    );
    it.setKeyEquivalentModifierMask(ctrl_opt());
    it.setEnabled(a.peek || (lot > 0 && !a.paused));
    keyed(&it, PREVIEW_PEEK);
    it
}

/// "Restore sessions": all, one by one, or forget them.
fn restore_menu(a: &App, lost: &[OpenSession]) -> Retained<NSMenu> {
    let sub = submenu(a);
    sub.addItem(&item(
        a,
        tr("Alle wiederherstellen", "Restore all"),
        Some(sel!(restoreAll:)),
        "",
    ));
    sub.addItem(&NSMenuItem::separatorItem(a.mtm));
    for o in lost {
        let (project, name) = match o.name.split_once(" · ") {
            Some((project, task)) => (Some(project.to_owned()), task),
            None => (None, o.name.as_str()),
        };
        let it = item(a, name, Some(sel!(restoreOne:)), "");
        set_subtitle(&it, project);
        keyed(&it, &format!("{PREVIEW_RESTORE}{}", o.id));
        sub.addItem(&it);
    }
    sub.addItem(&NSMenuItem::separatorItem(a.mtm));
    sub.addItem(&item(
        a,
        tr("Liste leeren", "Clear the list"),
        Some(sel!(restoreClear:)),
        "",
    ));
    sub
}

/// The symbol of a screen choice: MacBook, iMac, an external screen, or a control room over two.
fn target_symbol(option: &TargetChoice, infos: &[ScreenInfo]) -> &'static str {
    match option {
        TargetChoice::Screen(id) => match infos.iter().find(|s| s.id == *id) {
            Some(s) if s.builtin && s.laptop => "laptopcomputer",
            Some(s) if s.builtin => "desktopcomputer",
            _ => "display",
        },
        _ => "display.2",
    }
}

fn mode_symbol(m: LayoutMode) -> &'static str {
    match m {
        LayoutMode::Reading => "rectangle.center.inset.filled",
        LayoutMode::Focus2 => "rectangle.split.3x1",
        LayoutMode::Grid => "square.grid.3x2",
        LayoutMode::Inbox => "tray.full",
    }
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
        let it = item(
            a,
            tr(
                "Bedienungshilfen fehlen — klicken zum Freigeben",
                "Accessibility access missing — click to grant",
            ),
            Some(sel!(requestTrust:)),
            "",
        );
        show_image(&it, symbol("exclamationmark.triangle"));
        menu.addItem(&it);
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
        show_image(&it, symbol(target_symbol(&option, &infos)));
        keyed(&it, &format!("{PREVIEW_TARGET}{}", option.raw()));
        menu.addItem(&it);
    }
    // From three screens on: which is work and which control room (Kevin 2026-10-01: "im Menue waehlbar").
    if infos.len() >= 3 {
        let it = item(a, tr("Leitstand-Schirme", "Control room screens"), None, "");
        show_image(&it, symbol("rectangle.2.swap"));
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
        // The grid's shortcut stands in its submenu, the inbox has none.
        let key = if a.cfg.hotkeys && matches!(m, LayoutMode::Reading | LayoutMode::Focus2) {
            (i + 1).to_string()
        } else {
            String::new()
        };
        let grid = if m == LayoutMode::Grid { vis } else { None };
        // An item with a submenu only opens it, its action never comes: the grid is chosen in there (review quick
        // 2026-11-06).
        let action = grid.is_none().then_some(sel!(selectMode:));
        let it = item(a, m.label(), action, &key);
        it.setKeyEquivalentModifierMask(ctrl_opt());
        it.setState(on_off(m == a.mode && !control_room));
        it.setEnabled(!control_room);
        show_image(&it, symbol(mode_symbol(m)));
        keyed(&it, &format!("{PREVIEW_MODE}{}", m.raw()));
        if let Some(v) = grid {
            // The grid in effect stands beside it: Ctrl+Opt+3 takes that one.
            set_badge(&it, &fitted_grid(a.grid, v).label());
            let hotkey = if a.cfg.hotkeys {
                (i + 1).to_string()
            } else {
                String::new()
            };
            it.setSubmenu(Some(&grid_menu(a, v, &hotkey)));
        }
        menu.addItem(&it);
    }
    // Parking lot (Kevin 2026-11-06), only with worker names; also in the control room. Check mark = peeking. Its own
    // block, as in the draft.
    if a.has_lot() {
        menu.addItem(&NSMenuItem::separatorItem(a.mtm));
        let n = a.lot_indices().len();
        let it = peek_item(a, tr("Parkplatz anzeigen", "Show parking lot"), n);
        it.setState(on_off(a.peek));
        count_badge(&it, n);
        show_image(&it, symbol("parkingsign.circle"));
        menu.addItem(&it);
        let hand = item(a, tr("Auf den Parkplatz", "To the parking lot"), None, "");
        show_image(&hand, symbol("arrow.right"));
        hand.setSubmenu(Some(&hand_menu(a)));
        menu.addItem(&hand);
    }

    menu.addItem(&NSMenuItem::separatorItem(a.mtm));
    add_sessions(a, &menu);

    menu.addItem(&NSMenuItem::separatorItem(a.mtm));
    // The inbox's + once more (Kevin 2026-11-10): without a session there is no inbox and no +, and the first session
    // starts from here.
    let new = item(a, tr("Neue Session", "New session"), None, "");
    show_image(&new, symbol("plus"));
    new.setSubmenu(Some(&a.new_session_menu()));
    menu.addItem(&new);
    // After a crash or shutdown (Kevin 2026-11-09); only when there is something to bring back.
    let lost = a.restorable_sessions();
    if !lost.is_empty() {
        let it = item(
            a,
            tr("Sessions wiederherstellen", "Restore sessions"),
            None,
            "",
        );
        count_badge(&it, lost.len());
        show_image(&it, symbol("arrow.counterclockwise"));
        it.setSubmenu(Some(&restore_menu(a, &lost)));
        menu.addItem(&it);
    }
    let more = item(a, tr("Mehr", "More"), None, "");
    show_image(&more, symbol("ellipsis.circle"));
    more.setSubmenu(Some(&more_menu(a)));
    menu.addItem(&more);
    let quit = item(
        a,
        tr("Lookout beenden", "Quit Lookout"),
        Some(sel!(quitApp:)),
        "q",
    );
    show_image(&quit, symbol("power"));
    menu.addItem(&quit);
}

/// Sessions by status (design A, Kevin 2026-11-06 evening): who waits for Kevin on top, then the done ones, each section
/// with its count; the running ones behind one row with an arrow (Kevin 2026-11-09: the menu had outgrown itself); the
/// parking lot apart, one line per window. A row: dot, name, below it project, where the window is and what the session
/// does, the time on the right. A click brings it to the front, hovering lights it up (as before).
fn add_sessions(a: &App, menu: &NSMenu) {
    let order = ai_windows_by_status(a);
    if order.is_empty() {
        menu.addItem(&disabled(a, no_session_text()));
        return;
    }
    let on_screen = on_screen_of(a, &order);
    let parked = a.parked_indices();
    let lot = a.lot_indices();
    let lookout = a.lookout_lines();
    let n = now();
    let row_item = |idx: usize, on_lot: bool| {
        let w = &a.windows[idx];
        let row = menu_row(w, &a.since, &lookout, n);
        let front = sel!(bringToFrontItem:);
        if on_lot {
            let it = session_item(a, w, &row.compact(), None, front, PREVIEW_FRONT);
            set_badge(&it, &row.tone.word().to_lowercase());
            return it;
        }
        let it = session_item(a, w, &row.name, Some(row.tone), front, PREVIEW_FRONT);
        set_subtitle(
            &it,
            row.subtitle(&session_notes(a, idx, on_screen.as_deref(), &parked)),
        );
        if let Some(d) = &row.duration {
            set_badge(&it, d);
        }
        it
    };
    let mut budget = MENU_ROWS_MAX;
    for (group, rows) in menu_groups(&a.windows, &order, &lot) {
        if group == MenuGroup::Working {
            // All of them: a submenu scrolls.
            let list = submenu(a);
            for &idx in &rows {
                list.addItem(&row_item(idx, false));
            }
            let it = item(a, tr("Läuft", "Running"), None, "");
            count_badge(&it, rows.len());
            show_image(&it, dot(a, Some(LabelTone::Running)));
            it.setSubmenu(Some(&list));
            menu.addItem(&it);
            continue;
        }
        menu.addItem(&header(a, &group.header(rows.len())));
        let on_lot = group == MenuGroup::Lot;
        let shown = rows_shown(rows.len(), if on_lot { MENU_LOT_ROWS } else { budget });
        for &idx in &rows[..shown] {
            menu.addItem(&row_item(idx, on_lot));
        }
        let rest = rows.len() - shown;
        if rest > 0 {
            // The rest of the lot stands in the peek; the others had no room left. With the peek open the rest is there
            // already, and a click would close it (review quick 2026-11-06).
            let it = if on_lot {
                let it = peek_item(a, &more_rows_text(rest), lot.len());
                it.setEnabled(!a.peek && !a.paused);
                show_image(&it, dot(a, None));
                it
            } else {
                disabled(a, &more_rows_text(rest))
            };
            menu.addItem(&it);
        }
        if !on_lot {
            budget = budget.saturating_sub(shown);
        }
    }
}

fn ids_of(w: &TermWindow) -> String {
    w.sessions
        .iter()
        .map(|s| s.id.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Notes of a session row: where the window is (`whereabouts`), whether Lookout arranges it, whether it is parked in a
/// list. "In the list" comes from the plan, not from the screen comparison, which counts congruent windows as visible
/// (GP 2). The parking lot has its own section.
fn session_notes(
    a: &App,
    idx: usize,
    on_screen: Option<&[OnScreenWindow]>,
    parked: &BTreeSet<usize>,
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
    show_image(&labels, symbol("tag"));
    sub.addItem(&labels);
    let (side_title, side_symbol) = if a.side == lookout_core::model::StackSide::Right {
        (tr("Streifen rechts", "Strip on the right"), "sidebar.right")
    } else {
        (tr("Streifen links", "Strip on the left"), "sidebar.left")
    };
    let side = item(a, side_title, Some(sel!(toggleSide:)), "");
    show_image(&side, symbol(side_symbol));
    sub.addItem(&side);
    let pause = item(
        a,
        tr("Pause (nichts bewegen)", "Pause (move nothing)"),
        Some(sel!(togglePause:)),
        "",
    );
    pause.setState(on_off(a.paused));
    show_image(&pause, symbol("pause"));
    sub.addItem(&pause);
    sub.addItem(&NSMenuItem::separatorItem(a.mtm));
    let choose = item(a, tr("Fenster auswählen", "Choose windows"), None, "");
    show_image(&choose, symbol("checklist"));
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
        let room = rows_shown(shown.len(), 8);
        for s in &shown[..room] {
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
        // The badge counts them all: say how many the list leaves out (review quick 2026-11-06).
        if room < shown.len() {
            list.addItem(&disabled(a, &more_rows_text(shown.len() - room)));
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
        let it = item(a, tr("Ohne Fenster", "Without window"), None, "");
        count_badge(&it, shown.len());
        show_image(&it, symbol("circle.dashed"));
        it.setSubmenu(Some(&list));
        sub.addItem(&it);
    }
    sub
}

/// "To the parking lot" (Kevin 2026-11-06 evening): check mark = on the lot (a worker by name or sent by hand), a click
/// sends it there or takes it off, workers too ("die nach vorne zu holen"); hovering lights up the window. The window
/// Kevin is in carries Ctrl+Opt+5, which does the same for it. Every window, the menu scrolls: with 20 to 25 workers a
/// cut list left some unreachable (review quick 2026-11-06).
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
    for &idx in &order {
        let w = &a.windows[idx];
        let row = menu_row(w, &a.since, &lookout, n);
        let it = session_item(
            a,
            w,
            &row.name,
            Some(row.tone),
            sel!(toggleHandPark:),
            PREVIEW_HAND,
        );
        let worker = is_worker(w, &a.cfg.worker_names, &a.cfg.lead_names)
            .then(|| tr("Arbeiter", "worker").to_owned());
        set_subtitle(&it, row.project_line(worker.as_slice()));
        if a.cfg.hotkeys && Some(idx) == focused {
            it.setKeyEquivalent(&NSString::from_str("5"));
            it.setKeyEquivalentModifierMask(ctrl_opt());
        }
        it.setState(on_off(a.lot_member(idx)));
        sub.addItem(&it);
    }
    sub
}

/// "Choose windows": check mark = arranged, a click takes the window out or back in, hovering lights up exactly this
/// window (Kevin 2026-09-18). Whoever is not to be seen says so below the name. Every window, as in `hand_menu`.
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
    for &idx in &order {
        let w = &a.windows[idx];
        let place = whereabouts(w, is_on_screen(w, on_screen.as_deref())).label();
        let row = menu_row(w, &a.since, &lookout, n);
        let it = session_item(
            a,
            w,
            &row.name,
            Some(row.tone),
            sel!(toggleExcluded:),
            PREVIEW_WINDOW,
        );
        set_subtitle(&it, row.project_line(&[place.to_owned()]));
        it.setState(on_off(!is_excluded(w, &a.excluded)));
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
            if a.windows
                .iter()
                .position(|x| x.id == w.id)
                .is_some_and(|i| a.lot_member(i))
            {
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
        let mut boxes: Vec<PreviewBox> = arr
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
        // Who finds no tile stands in the list beside them.
        boxes.extend(arr.lists.iter().map(|l| {
            PreviewBox {
                rect: l.rect,
                title: format!("{} ({})", tr("Liste", "List"), l.count()),
                detail: tr(
                    "Klick auf eine Zeile holt in die Mitte",
                    "a click on a row brings it to the middle",
                )
                .to_owned(),
            }
        }));
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
