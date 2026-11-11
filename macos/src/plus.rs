//! The inbox's + and "New session" in the menu bar menu (Kevin 2026-11-10): three favorites that start a new session
//! with a task (`lookout_core::launch`), a new team (`lookout_core::team_start`, its dialog in `team_dialog`), an empty
//! window, and the favorites and the team's roles changed right in the menu. Also the task dialog, and the hidden main
//! menu that gives its text field the editing keys.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationOptions, NSComboBox,
    NSControlStateValueOff, NSControlStateValueOn, NSLineBreakMode, NSMenu, NSMenuItem,
    NSTextField, NSView, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use lookout_core::i18n::tr;
use lookout_core::launch::{Favorite, Field, NewSession, favorite_from_key, favorite_key};
use lookout_core::team::Role;
use lookout_core::team_start::TeamSetup;

/// What a click does, kept in the item's represented object: start a favorite (its slot follows), start a free choice
/// (`launch::favorite_key` follows), change a favorite (slot, field and value follow, one per line), or open an empty
/// window.
const START: &str = "start:";
const CUSTOM: &str = "custom:";
const CHANGE: &str = "change:";
const BLANK: &str = "blank";
/// A new team; a team role changed (role, field and value follow, one per line).
const TEAM: &str = "team";
const TEAM_CHANGE: &str = "team-change:";

/// The favorites' places in the menu (`launch::FAVORITES`).
const SLOTS: [&str; 3] = ["①", "②", "③"];

/// A click in the menu, read back from its item (`choice`).
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    Start(usize),
    Custom(Favorite),
    Change(usize, Field, String),
    Blank,
    Team,
    ChangeTeam(Role, Field, String),
}

/// A team role in a menu item's key.
fn role_key(role: Role) -> &'static str {
    match role {
        Role::Lead => "lead",
        Role::KeyAccount => "key",
        Role::Worker => "worker",
    }
}

/// What the item with this key does.
pub fn choice(key: &str) -> Option<Choice> {
    if key == BLANK {
        return Some(Choice::Blank);
    }
    if key == TEAM {
        return Some(Choice::Team);
    }
    if let Some(rest) = key.strip_prefix(TEAM_CHANGE) {
        let mut parts = rest.splitn(3, '\n');
        let role = match parts.next()? {
            "lead" => Role::Lead,
            "key" => Role::KeyAccount,
            "worker" => Role::Worker,
            _ => return None,
        };
        let field = Field::from_key(parts.next()?)?;
        return Some(Choice::ChangeTeam(role, field, parts.next()?.to_owned()));
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
    team: &TeamSetup,
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
    menu.addItem(&nest(
        tr("Frei wählen", "Choose freely"),
        choice_menu(mtm, settings, None, false, &leaf),
    ));
    // A new team (Kevin 2026-11-10): needs a program that names its session.
    let new_team = entry(tr("Neues Team …", "New team …"), Some(TEAM.to_owned()));
    if !settings.programs.iter().any(|p| p.takes_name()) {
        new_team.setEnabled(false);
        new_team.setSubtitle(Some(&NSString::from_str(tr(
            "kein Programm mit {name}",
            "no program with {name}",
        ))));
    }
    menu.addItem(&new_team);
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
    // The team's roles: its place, and program, model and effort of lead and key account.
    let choices = |values: Vec<&str>,
                   current: &str,
                   key: &dyn Fn(&str) -> String,
                   label: &dyn Fn(&str) -> String| {
        let options = plain(mtm);
        for value in values {
            let it = entry(&label(value), Some(key(value)));
            it.setState(if current == value {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            options.addItem(&it);
        }
        options
    };
    let roles = plain(mtm);
    let place_key = |v: &str| format!("{TEAM_CHANGE}lead\n{}\n{v}", Field::Place.key());
    let places = choices(
        settings.places.iter().map(|p| p.name.as_str()).collect(),
        &team.lead.place,
        &place_key,
        &|v: &str| v.to_owned(),
    );
    roles.addItem(&nest(field_title(Field::Place), places));
    for (role, title) in [
        (Role::Lead, tr("Team-Lead", "Team lead")),
        (Role::KeyAccount, "Key account"),
        (Role::Worker, tr("Arbeiter", "Workers")),
    ] {
        let f = team.get(role);
        let fields = plain(mtm);
        for field in [Field::Program, Field::Model, Field::Effort] {
            let key = |v: &str| format!("{TEAM_CHANGE}{}\n{}\n{v}", role_key(role), field.key());
            let label = |v: &str| {
                if field == Field::Model {
                    settings.model_label(&f.program, v).to_owned()
                } else {
                    v.to_owned()
                }
            };
            let options = choices(
                settings.team_options(role, f, field),
                f.get(field),
                &key,
                &label,
            );
            let it = nest(field_title(field), options);
            it.setEnabled(it.submenu().is_some_and(|m| m.numberOfItems() > 0));
            fields.addItem(&it);
        }
        roles.addItem(&nest(
            &format!("{title}: {}", settings.role_label(f)),
            fields,
        ));
    }
    change.addItem(&NSMenuItem::separatorItem(mtm));
    change.addItem(&nest("Team", roles));
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

/// Place › program › model › effort, each level with a list as a submenu ("Choose freely", Kevin 2026-11-10 "1b"; the
/// team dialog's roles, Kevin: "so wie bei frischer session beim + drücken"). `leaf` makes the item that chooses its
/// favorite. With `place` the menu starts at the programs (in that place); `naming` keeps only programs that name their
/// session.
pub fn choice_menu(
    mtm: MainThreadMarker,
    settings: &NewSession,
    place: Option<&str>,
    naming: bool,
    leaf: &dyn Fn(&str, &Favorite) -> Retained<NSMenuItem>,
) -> Retained<NSMenu> {
    let nest = |title: &str, sub: Retained<NSMenu>| {
        let it = NSMenuItem::new(mtm);
        it.setTitle(&NSString::from_str(title));
        it.setSubmenu(Some(&sub));
        it
    };
    let programs_in = |place: &str| {
        let programs = plain(mtm);
        for p in settings
            .programs
            .iter()
            .filter(|p| !naming || p.takes_name())
        {
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
                place: place.to_owned(),
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
        programs
    };
    match place {
        Some(place) => programs_in(place),
        None => {
            let places = plain(mtm);
            for place in &settings.places {
                places.addItem(&nest(&place.name, programs_in(&place.name)));
            }
            places
        }
    }
}

/// A menu that keeps the enabled states set here: AppKit's auto validation would override them.
pub(crate) fn plain(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu
}

/// "New task" for the favorite `label` (Kevin 2026-11-10): a text field that wraps, Enter starts, Esc cancels; empty,
/// the session starts without a task. Lookout comes to the front so that the field gets the keys, and on cancel gives
/// the front back to the app that had it. Runs its own loop, so never call it from inside the app slot (`with_app`):
/// the app must stay reachable meanwhile. The tick's timer waits while it is open. Above the task a field "Card" (Kevin
/// 2026-11-11, "3a"): one of `cards` or a new name puts the session on that project's card; empty, none. Answer: task
/// and card.
pub fn ask_task(mtm: MainThreadMarker, label: &str, cards: &[String]) -> Option<(String, String)> {
    let previous = NSWorkspace::sharedWorkspace().frontmostApplication();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(tr("Neuer Auftrag", "New task")));
    alert.setInformativeText(&NSString::from_str(label));
    alert.addButtonWithTitle(&NSString::from_str(tr("Starten", "Start")));
    // NSAlert gives Esc only to a button called "Cancel".
    alert
        .addButtonWithTitle(&NSString::from_str(tr("Abbrechen", "Cancel")))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 100.0)),
    );
    let card = NSComboBox::initWithFrame(
        NSComboBox::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 74.0), NSSize::new(360.0, 26.0)),
    );
    card.setPlaceholderString(Some(&NSString::from_str(tr(
        "Karte (frei lassen: keine)",
        "Card (leave empty: none)",
    ))));
    for c in cards {
        // SAFETY: an NSString is a valid object value for a combo box without a data source.
        unsafe { card.addItemWithObjectValue(&NSString::from_str(c)) };
    }
    card.setCompletes(true);
    view.addSubview(&card);
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
    view.addSubview(&field);
    alert.setAccessoryView(Some(&view));
    alert.layout();
    alert.window().setInitialFirstResponder(Some(&field));
    NSApplication::sharedApplication(mtm).activate();
    if alert.runModal() == NSAlertFirstButtonReturn {
        return Some((
            field.stringValue().to_string(),
            card.stringValue().to_string(),
        ));
    }
    if let Some(app) = previous.filter(|p| p.processIdentifier() != std::process::id() as i32) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
    None
}

/// "Rename" (Kevin 2026-11-11): the title Lookout shows for a session, `shown` filled in; empty gives the session its own
/// name back. Runs its own loop like `ask_task`.
pub fn ask_title(mtm: MainThreadMarker, shown: &str) -> Option<String> {
    let previous = NSWorkspace::sharedWorkspace().frontmostApplication();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(tr("Umbenennen", "Rename")));
    alert.setInformativeText(&NSString::from_str(tr(
        "Titel in Lookout. Mit „Projekt · …“ kommt das Fenster ins Team oder auf die Karte des Projekts, wenn es eines davon gibt. Leer: wieder der eigene Name.",
        "Title in Lookout. With \"Project · …\" the window joins that project's team or card, if it has one. Empty: its own name again.",
    )));
    alert.addButtonWithTitle(&NSString::from_str(tr("Sichern", "Save")));
    alert
        .addButtonWithTitle(&NSString::from_str(tr("Abbrechen", "Cancel")))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 24.0)),
    );
    field.setStringValue(&NSString::from_str(shown));
    alert.setAccessoryView(Some(&field));
    alert.layout();
    alert.window().setInitialFirstResponder(Some(&field));
    NSApplication::sharedApplication(mtm).activate();
    let answer =
        (alert.runModal() == NSAlertFirstButtonReturn).then(|| field.stringValue().to_string());
    if let Some(app) = previous.filter(|p| p.processIdentifier() != std::process::id() as i32) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
    answer
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
