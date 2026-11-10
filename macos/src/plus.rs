//! The inbox's + and "New session" in the menu bar menu (Kevin 2026-11-10): three favorites that start a new session
//! with a task (`lookout_core::launch`), an empty window, and the favorites changed right in the menu. Also the task
//! dialog, and the hidden main menu that gives its text field the editing keys.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationOptions,
    NSControlStateValueOff, NSControlStateValueOn, NSLineBreakMode, NSMenu, NSMenuItem,
    NSTextField, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use lookout_core::i18n::tr;
use lookout_core::launch::{Favorite, Field, NewSession, favorite_from_key, favorite_key};

/// What a click does, kept in the item's represented object: start a favorite (its slot follows), start a free choice
/// (`launch::favorite_key` follows), change a favorite (slot, field and value follow, one per line), or open an empty
/// window.
const START: &str = "start:";
const CUSTOM: &str = "custom:";
const CHANGE: &str = "change:";
const BLANK: &str = "blank";

/// The favorites' places in the menu (`launch::FAVORITES`).
const SLOTS: [&str; 3] = ["①", "②", "③"];

/// A click in the menu, read back from its item (`choice`).
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    Start(usize),
    Custom(Favorite),
    Change(usize, Field, String),
    Blank,
}

/// What the item with this key does.
pub fn choice(key: &str) -> Option<Choice> {
    if key == BLANK {
        return Some(Choice::Blank);
    }
    if let Some(slot) = key.strip_prefix(START) {
        return slot.parse().ok().map(Choice::Start);
    }
    if let Some(f) = key.strip_prefix(CUSTOM) {
        return favorite_from_key(f).map(Choice::Custom);
    }
    let mut parts = key.strip_prefix(CHANGE)?.splitn(3, '\n');
    let slot = parts.next()?.parse().ok()?;
    let field = Field::from_key(parts.next()?)?;
    Some(Choice::Change(slot, field, parts.next()?.to_owned()))
}

/// The favorites (one that no longer fits the settings is greyed out, "not available"), "Choose freely" (place, program,
/// model and effort for one start, the favorites stay), an empty window, and "Change favorites" with place, program,
/// model and effort of each, the current one checked. A click sends `newSessionItem:` to `target` (the menu
/// delegate); `None` in tests.
pub fn menu(
    mtm: MainThreadMarker,
    target: Option<&AnyObject>,
    settings: &NewSession,
    favorites: &[Favorite],
) -> Retained<NSMenu> {
    let entry = |title: &str, key: Option<String>| {
        // SAFETY: designated initializer on a fresh allocation; the action is a method of the target.
        let it = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                key.is_some().then_some(sel!(newSessionItem:)),
                &NSString::new(),
            )
        };
        if let Some(key) = key {
            // SAFETY: the target is weak in AppKit and lives as long as the app; an NSString is a valid represented
            // object.
            unsafe {
                it.setTarget(target);
                it.setRepresentedObject(Some(&NSString::from_str(&key)));
            }
        }
        it
    };
    let slot_title =
        |k: usize, f: &Favorite| format!("{} {}", SLOTS.get(k).unwrap_or(&"·"), settings.label(f));

    let menu = plain(mtm);
    for (k, f) in favorites.iter().enumerate() {
        let it = entry(&slot_title(k, f), Some(format!("{START}{k}")));
        if settings.resolve(f).is_none() {
            it.setEnabled(false);
            it.setSubtitle(Some(&NSString::from_str(tr(
                "nicht verfügbar",
                "not available",
            ))));
        }
        menu.addItem(&it);
    }

    // Choose freely (Kevin 2026-11-10, "1b"): each level with a list gets a submenu, the last choice starts.
    let leaf =
        |title: &str, f: &Favorite| entry(title, Some(format!("{CUSTOM}{}", favorite_key(f))));
    let nest = |title: &str, sub: Retained<NSMenu>| {
        let it = entry(title, None);
        it.setSubmenu(Some(&sub));
        it
    };
    let custom = plain(mtm);
    for place in &settings.places {
        let programs = plain(mtm);
        for p in &settings.programs {
            let with_effort = |title: &str, f: Favorite| {
                if p.efforts.is_empty() {
                    return leaf(title, &f);
                }
                let efforts = plain(mtm);
                for e in &p.efforts {
                    let f = Favorite {
                        effort: e.clone(),
                        ..f.clone()
                    };
                    efforts.addItem(&leaf(e, &f));
                }
                nest(title, efforts)
            };
            let f = Favorite {
                place: place.name.clone(),
                program: p.name.clone(),
                ..Favorite::default()
            };
            let it = if p.models.is_empty() {
                with_effort(&p.name, f)
            } else {
                let models = plain(mtm);
                for m in &p.models {
                    let f = Favorite {
                        model: m.clone(),
                        ..f.clone()
                    };
                    models.addItem(&with_effort(settings.model_label(&p.name, m), f));
                }
                nest(&p.name, models)
            };
            programs.addItem(&it);
        }
        custom.addItem(&nest(&place.name, programs));
    }
    menu.addItem(&nest(tr("Frei wählen", "Choose freely"), custom));
    menu.addItem(&entry(
        tr("Leeres Fenster", "Empty window"),
        Some(BLANK.to_owned()),
    ));
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let change = plain(mtm);
    for (k, f) in favorites.iter().enumerate() {
        let fields = plain(mtm);
        for field in Field::ALL {
            let options = plain(mtm);
            for value in settings.options(f, field) {
                let title = if field == Field::Model {
                    settings.model_label(&f.program, value)
                } else {
                    value
                };
                let it = entry(
                    title,
                    Some(format!("{CHANGE}{k}\n{}\n{value}", field.key())),
                );
                it.setState(if f.get(field) == value {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                options.addItem(&it);
            }
            let it = entry(field_title(field), None);
            it.setEnabled(options.numberOfItems() > 0);
            it.setSubmenu(Some(&options));
            fields.addItem(&it);
        }
        let it = entry(&slot_title(k, f), None);
        it.setSubmenu(Some(&fields));
        change.addItem(&it);
    }
    let it = entry(tr("Favoriten ändern", "Change favorites"), None);
    it.setSubmenu(Some(&change));
    menu.addItem(&it);
    menu
}

fn field_title(field: Field) -> &'static str {
    match field {
        Field::Place => tr("Ort", "Place"),
        Field::Program => tr("Programm", "Program"),
        Field::Model => tr("Modell", "Model"),
        Field::Effort => "Effort",
    }
}

/// A menu that keeps the enabled states set here: AppKit's auto validation would override them.
fn plain(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu
}

/// "New task" for the favorite `label` (Kevin 2026-11-10): a text field that wraps, Enter starts, Esc cancels; empty,
/// the session starts without a task. Lookout comes to the front so that the field gets the keys, and on cancel gives
/// the front back to the app that had it. Runs its own loop, so never call it from inside the app slot (`with_app`):
/// the app must stay reachable meanwhile. The tick's timer waits while it is open.
pub fn ask_task(mtm: MainThreadMarker, label: &str) -> Option<String> {
    let previous = NSWorkspace::sharedWorkspace().frontmostApplication();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(tr("Neuer Auftrag", "New task")));
    alert.setInformativeText(&NSString::from_str(label));
    alert.addButtonWithTitle(&NSString::from_str(tr("Starten", "Start")));
    // NSAlert gives Esc only to a button called "Cancel".
    alert
        .addButtonWithTitle(&NSString::from_str(tr("Abbrechen", "Cancel")))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 66.0)),
    );
    field.setPlaceholderString(Some(&NSString::from_str(tr(
        "Was soll die Session tun? Leer: ohne Auftrag",
        "What should the session do? Empty: no task",
    ))));
    field.setUsesSingleLineMode(false);
    field.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    if let Some(cell) = field.cell() {
        cell.setWraps(true);
        cell.setScrollable(false);
    }
    alert.setAccessoryView(Some(&field));
    alert.layout();
    alert.window().setInitialFirstResponder(Some(&field));
    NSApplication::sharedApplication(mtm).activate();
    if alert.runModal() == NSAlertFirstButtonReturn {
        return Some(field.stringValue().to_string());
    }
    if let Some(app) = previous.filter(|p| p.processIdentifier() != std::process::id() as i32) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
    None
}

/// Lookout has no menu bar of its own, so the editing keys in the task field had nothing to go through (plan review
/// 2026-11-10): a main menu with "Edit" gives it ⌘Z, ⌘X, ⌘C, ⌘V and ⌘A. A menu bar app's main menu never shows.
pub fn install_edit_menu(mtm: MainThreadMarker) {
    let edit = NSMenu::new(mtm);
    for (title, action, key) in [
        (tr("Widerrufen", "Undo"), sel!(undo:), "z"),
        (tr("Ausschneiden", "Cut"), sel!(cut:), "x"),
        (tr("Kopieren", "Copy"), sel!(copy:), "c"),
        (tr("Einsetzen", "Paste"), sel!(paste:), "v"),
        (tr("Alles auswählen", "Select All"), sel!(selectAll:), "a"),
    ] {
        // SAFETY: designated initializer on a fresh allocation; no target, the action goes to the first responder.
        let it = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&it);
    }
    let it = NSMenuItem::new(mtm);
    it.setSubmenu(Some(&edit));
    let main = NSMenu::new(mtm);
    main.addItem(&it);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&main));
}
