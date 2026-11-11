//! "New team" (Kevin 2026-11-10, mockup "Teams"): team name, the team lead and the key account, a tick for the key
//! account ("1a"), and the task. Lead and key account are chosen the way a fresh session is at the + (Kevin: "so wie bei
//! frischer session beim + drücken"): a button opens place › program › model › effort for the lead (its place is the
//! team's) and program › model › effort for the key account. Preset from "Change favorites › Team" (Kevin: "pre chosen
//! ... but need to be open to change"); what is chosen here counts for this team only. `ask_team` runs its own loop like
//! `plus::ask_task`, so never call it from inside the app slot.

use std::cell::RefCell;
use std::collections::BTreeMap;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationOptions, NSButton,
    NSControlStateValueOff, NSControlStateValueOn, NSLineBreakMode, NSMenuItem, NSPopUpButton,
    NSTextAlignment, NSTextField, NSView, NSWorkspace,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use lookout_core::i18n::tr;
use lookout_core::launch::{Favorite, NewSession, favorite_from_key, favorite_key};
use lookout_core::team_start::TeamSetup;

use crate::plus;

/// What the dialog asks for; the key account in the lead's place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamRequest {
    pub name: String,
    pub with_key_account: bool,
    pub lead: Favorite,
    pub key_account: Favorite,
    /// What the lead starts its workers with (Kevin 2026-11-11).
    pub worker: Favorite,
    pub task: String,
}

impl TeamRequest {
    /// The request as a setup for `NewSession::team_starts`.
    pub fn setup(&self) -> TeamSetup {
        TeamSetup {
            lead: self.lead.clone(),
            key_account: self.key_account.clone(),
            worker: self.worker.clone(),
            with_key_account: self.with_key_account,
        }
    }
}

const W: f64 = 420.0;
const H: f64 = 212.0;
const LABEL_W: f64 = 92.0;
const FIELD_X: f64 = 100.0;

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// A label at the left of a row, right-aligned.
fn label(mtm: MainThreadMarker, text: &str, y: f64) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    l.setFrame(rect(0.0, y, LABEL_W, 18.0));
    l.setAlignment(NSTextAlignment::Right);
    l
}

/// A role's choice and the button that shows it.
pub struct PickerIvars {
    choice: RefCell<Favorite>,
    /// Shown text per chosen favorite (`favorite_key`).
    labels: RefCell<BTreeMap<String, String>>,
    button: RefCell<Option<Retained<NSPopUpButton>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `RolePicker` implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LookoutTeamRolePicker"]
    #[ivars = PickerIvars]
    pub struct RolePicker;

    unsafe impl NSObjectProtocol for RolePicker {}

    impl RolePicker {
        /// A favorite chosen at the end of the cascade.
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSMenuItem) {
            let key = sender
                .representedObject()
                .and_then(|o| o.downcast::<NSString>().ok())
                .map(|s| s.to_string());
            if let Some(f) = key.as_deref().and_then(favorite_from_key) {
                self.choose(&f);
            }
        }
    }
);

impl RolePicker {
    /// A pull-down button whose menu is the cascade from `plus::choice_menu` (`place`: the lead's level with places, else
    /// the programs only), `preset` chosen, `shown` naming a favorite on the button.
    fn new(
        mtm: MainThreadMarker,
        settings: &NewSession,
        frame: NSRect,
        (with_places, naming): (bool, bool),
        preset: &Favorite,
        shown: &dyn Fn(&Favorite) -> String,
    ) -> Retained<RolePicker> {
        let this = Self::alloc(mtm).set_ivars(PickerIvars {
            choice: RefCell::new(preset.clone()),
            labels: RefCell::new(BTreeMap::new()),
            button: RefCell::new(None),
        });
        // SAFETY: NSObject's designated initializer.
        let this: Retained<RolePicker> = unsafe { msg_send![super(this), init] };
        let leaf = |title: &str, f: &Favorite| {
            let key = favorite_key(f);
            this.ivars()
                .labels
                .borrow_mut()
                .insert(key.clone(), shown(f));
            let it = NSMenuItem::new(mtm);
            it.setTitle(&NSString::from_str(title));
            // SAFETY: the target is weak in AppKit and lives as long as the dialog's form; an NSString is a valid
            // represented object.
            unsafe {
                it.setTarget(Some(&this));
                it.setAction(Some(sel!(pick:)));
                it.setRepresentedObject(Some(&NSString::from_str(&key)));
            }
            it
        };
        let menu = plus::choice_menu(
            mtm,
            settings,
            (!with_places).then_some(preset.place.as_str()),
            naming,
            &leaf,
        );
        // A pull-down button shows its first item: the choice.
        menu.insertItem_atIndex(&NSMenuItem::new(mtm), 0);
        let button = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), frame, true);
        button.setMenu(Some(&menu));
        *this.ivars().button.borrow_mut() = Some(button);
        this.choose(preset);
        this
    }

    /// `f` chosen: kept, and shown on the button.
    pub fn choose(&self, f: &Favorite) {
        *self.ivars().choice.borrow_mut() = f.clone();
        let shown = self
            .ivars()
            .labels
            .borrow()
            .get(&favorite_key(f))
            .cloned()
            .unwrap_or_else(|| f.label());
        if let Some(b) = self.ivars().button.borrow().as_ref()
            && let Some(first) = b.itemAtIndex(0)
        {
            first.setTitle(&NSString::from_str(&shown));
        }
    }

    pub fn choice(&self) -> Favorite {
        self.ivars().choice.borrow().clone()
    }

    pub fn button(&self) -> Retained<NSPopUpButton> {
        self.ivars().button.borrow().clone().expect("set in new")
    }
}

/// The dialog's fields, built without showing them (tests read them back, `request`).
pub struct TeamForm {
    pub view: Retained<NSView>,
    pub name: Retained<NSTextField>,
    pub lead: Retained<RolePicker>,
    pub key_account: Retained<RolePicker>,
    pub worker: Retained<RolePicker>,
    pub with_key_account: Retained<NSButton>,
    pub task: Retained<NSTextField>,
}

impl TeamForm {
    pub fn new(mtm: MainThreadMarker, settings: &NewSession, setup: &TeamSetup) -> TeamForm {
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, W, H));
        view.addSubview(&label(mtm, tr("Teamname", "Team name"), H - 22.0));
        let name = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            rect(FIELD_X, H - 24.0, W - FIELD_X, 22.0),
        );
        name.setPlaceholderString(Some(&NSString::from_str(tr("z. B. Shop", "e.g. Shop"))));
        view.addSubview(&name);

        view.addSubview(&label(mtm, tr("Team-Lead", "Team lead"), H - 54.0));
        let lead = RolePicker::new(
            mtm,
            settings,
            rect(FIELD_X - 3.0, H - 58.0, W - FIELD_X + 3.0, 26.0),
            (true, true),
            &setup.lead,
            &|f| settings.label(f),
        );
        view.addSubview(&lead.button());

        // SAFETY: no target and no action, the dialog reads the state after it closed.
        let with_key_account = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Key account"),
                None,
                None,
                mtm,
            )
        };
        with_key_account.setFrame(rect(0.0, H - 86.0, LABEL_W + 4.0, 20.0));
        with_key_account.setState(if setup.with_key_account {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        view.addSubview(&with_key_account);
        let key_account = RolePicker::new(
            mtm,
            settings,
            rect(FIELD_X - 3.0, H - 90.0, W - FIELD_X + 3.0, 26.0),
            (false, true),
            &Favorite {
                place: setup.lead.place.clone(),
                ..setup.key_account.clone()
            },
            &|f| settings.role_label(f),
        );
        view.addSubview(&key_account.button());

        // What the lead starts its workers with: any program (Kevin 2026-11-11), the lead's place.
        view.addSubview(&label(mtm, tr("Arbeiter", "Workers"), H - 118.0));
        let worker = RolePicker::new(
            mtm,
            settings,
            rect(FIELD_X - 3.0, H - 122.0, W - FIELD_X + 3.0, 26.0),
            (false, false),
            &Favorite {
                place: setup.lead.place.clone(),
                ..setup.worker.clone()
            },
            &|f| settings.role_label(f),
        );
        view.addSubview(&worker.button());

        view.addSubview(&label(mtm, tr("Auftrag", "Task"), H - 150.0));
        let task = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            rect(FIELD_X, 0.0, W - FIELD_X, H - 150.0 + 16.0),
        );
        task.setPlaceholderString(Some(&NSString::from_str(tr(
            "Was soll das Team erreichen?",
            "What should the team achieve?",
        ))));
        task.setUsesSingleLineMode(false);
        task.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        if let Some(cell) = task.cell() {
            cell.setWraps(true);
            cell.setScrollable(false);
        }
        view.addSubview(&task);
        TeamForm {
            view,
            name,
            lead,
            key_account,
            worker,
            with_key_account,
            task,
        }
    }

    /// What the fields hold now; the key account in the lead's place, whichever place its menu was built for (review
    /// quick 2026-11-10).
    pub fn request(&self) -> TeamRequest {
        let lead = self.lead.choice();
        TeamRequest {
            name: self.name.stringValue().to_string(),
            with_key_account: self.with_key_account.state() == NSControlStateValueOn,
            key_account: Favorite {
                place: lead.place.clone(),
                ..self.key_account.choice()
            },
            worker: Favorite {
                place: lead.place.clone(),
                ..self.worker.choice()
            },
            lead,
            task: self.task.stringValue().to_string(),
        }
    }
}

/// The dialog; `check` turns the request into the team's name or the text to show (`App::check_team`), and the dialog
/// stays open until it fits. `None`: cancelled. On cancel the app that was in front gets the front back.
pub fn ask_team(
    mtm: MainThreadMarker,
    settings: &NewSession,
    setup: &TeamSetup,
    check: impl Fn(&TeamRequest) -> Result<String, String>,
) -> Option<TeamRequest> {
    let previous = NSWorkspace::sharedWorkspace().frontmostApplication();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(tr("Neues Team", "New team")));
    let hint = tr(
        "Startet Team-Lead und Key account in eigenen Warp-Fenstern. Arbeiter startet der Lead selbst.",
        "Starts the team lead and the key account in Warp windows of their own. The lead starts the workers.",
    );
    alert.setInformativeText(&NSString::from_str(hint));
    alert.addButtonWithTitle(&NSString::from_str(tr("Starten", "Start")));
    alert
        .addButtonWithTitle(&NSString::from_str(tr("Abbrechen", "Cancel")))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    let form = TeamForm::new(mtm, settings, setup);
    alert.setAccessoryView(Some(&form.view));
    alert.layout();
    alert.window().setInitialFirstResponder(Some(&form.name));
    NSApplication::sharedApplication(mtm).activate();
    loop {
        if alert.runModal() != NSAlertFirstButtonReturn {
            break;
        }
        let request = form.request();
        match check(&request) {
            Ok(name) => return Some(TeamRequest { name, ..request }),
            Err(why) => alert.setInformativeText(&NSString::from_str(&why)),
        }
    }
    if let Some(app) = previous.filter(|p| p.processIdentifier() != std::process::id() as i32) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
    None
}
