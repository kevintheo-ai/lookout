//! AppKit tests of the small parts (prefs, screens, icon). Runs on the main thread (harness = false): `main` creates
//! the shared NSApplication, runs each test with its panic caught, prints one line per test and fails if any failed.
//! Every window stays invisible and lets clicks through (`invisible`), as in the overlay tests. Hotkeys and the click
//! monitor have no live test here: registering Ctrl+Opt+<digit> would take Kevin's keys from his running Lookout.

#[cfg(target_os = "macos")]
mod invisible;

fn main() {
    #[cfg(target_os = "macos")]
    parts::run();
}

#[cfg(target_os = "macos")]
mod parts {
    use std::collections::BTreeMap;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{
        NSApplication, NSBitmapImageRep, NSControlStateValueOn, NSMenu, NSMenuItem, NSScreen,
    };
    use objc2_core_foundation::CGRect;
    use objc2_core_graphics::CGDisplayIsBuiltin;
    use objc2_foundation::{NSArray, NSBundle, NSNumber, NSString, NSUserDefaults, ns_string};

    use lookout_core::geom::Rect;
    use lookout_core::launch::{Favorite, Field, NewSession, Place, Program};
    use lookout_core::team::Role;
    use lookout_core::team_start::TeamTexts;
    use lookout_macos::plus::{self, Choice};
    use lookout_macos::{icon, prefs, screens};

    type Test = fn(MainThreadMarker);

    pub fn run() {
        // Before anything can log: never Kevin's ~/Library/Logs/Lookout.log.
        let log =
            std::env::temp_dir().join(format!("lookout-appkit-parts-{}.log", std::process::id()));
        // SAFETY: single-threaded so far, nothing reads the environment concurrently.
        unsafe { std::env::set_var("LOOKOUT_LOG", &log) };
        let mtm = MainThreadMarker::new().expect("harness = false runs main on the main thread");
        let _app = NSApplication::sharedApplication(mtm);
        super::invisible::keep_windows_invisible(mtm);
        let tests: [(&str, Test); 6] = [
            ("prefs_round_trip", prefs_round_trip),
            ("screens_match_nsscreen", screens_match_nsscreen),
            ("status_tile_image", status_tile_image),
            ("plus_menu_offers_favorites", plus_menu_offers_favorites),
            ("edit_menu_for_the_task_field", edit_menu_for_the_task_field),
            (
                "team_form_presets_and_reads_back",
                team_form_presets_and_reads_back,
            ),
        ];
        let mut failed = 0;
        for (name, test) in tests {
            let ok = catch_unwind(AssertUnwindSafe(|| test(mtm))).is_ok();
            println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
            failed += usize::from(!ok);
        }
        let _ = std::fs::remove_file(&log);
        if failed > 0 {
            std::process::exit(1);
        }
    }

    /// Removes the test key even when an assertion fails.
    struct Cleanup<'a>(&'a str);

    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            prefs::remove(self.0);
        }
    }

    fn prefs_round_trip(_: MainThreadMarker) {
        // A test binary has no bundle id, so its defaults are its own domain: never Lookout's real memory.
        let bundle = NSBundle::mainBundle()
            .bundleIdentifier()
            .map(|s| s.to_string());
        assert_ne!(bundle.as_deref(), Some("io.github.kevintheo-ai.lookout"));
        // ponytail: with the key removed, cfprefsd still leaves an empty plist in ~/Library/Preferences, one per test
        // binary name appkit_parts-<hash> (measured 2026-11-06; deleting the domain does not remove it); a fixed test
        // bundle id (embedded Info.plist) would keep it to one file.
        let key = format!("lookout-test.{}", std::process::id());
        let _cleanup = Cleanup(&key);
        let defaults = NSUserDefaults::standardUserDefaults();
        let ns_key = NSString::from_str(&key);
        // SAFETY: only property list values are passed.
        let raw = |value: &AnyObject| unsafe { defaults.setObject_forKey(Some(value), &ns_key) };

        assert_eq!(prefs::string(&key), None);
        prefs::set_string(&key, "screen:ABC");
        assert_eq!(prefs::string(&key).as_deref(), Some("screen:ABC"));
        assert_eq!(prefs::string_array(&key), None);
        assert_eq!(prefs::boolean(&key), None);

        prefs::set_string_array(&key, &["b".to_string(), "a".to_string()]);
        assert_eq!(
            prefs::string_array(&key),
            Some(vec!["b".to_string(), "a".to_string()])
        );
        assert_eq!(prefs::string(&key), None);
        prefs::set_string_array(&key, &[]);
        assert_eq!(prefs::string_array(&key), Some(vec![]));
        // stringArray(forKey:): one element that is no string and the whole array is None.
        let (a, one) = (NSString::from_str("a"), NSNumber::new_i32(1));
        raw(&NSArray::<AnyObject>::from_slice(&[
            a.as_ref(),
            one.as_ref(),
        ]));
        assert_eq!(prefs::string_array(&key), None);

        prefs::set_bool(&key, true);
        assert_eq!(prefs::boolean(&key), Some(true));
        prefs::set_bool(&key, false);
        assert_eq!(prefs::boolean(&key), Some(false));
        // Numbers are not converted to strings (unlike NSUserDefaults stringForKey:).
        assert_eq!(prefs::string(&key), None);
        // Swift's `as? Bool` takes the numbers 1 and 0 too, no other number and no string.
        raw(&NSNumber::new_i64(1));
        assert_eq!(prefs::boolean(&key), Some(true));
        raw(&NSNumber::new_f64(0.0));
        assert_eq!(prefs::boolean(&key), Some(false));
        raw(&NSNumber::new_i32(2));
        assert_eq!(prefs::boolean(&key), None);
        raw(&NSString::from_str("YES"));
        assert_eq!(prefs::boolean(&key), None);

        prefs::remove(&key);
        assert_eq!(prefs::string(&key), None);
        assert_eq!(prefs::string_array(&key), None);
        assert_eq!(prefs::boolean(&key), None);
    }

    fn screens_match_nsscreen(mtm: MainThreadMarker) {
        let all = NSScreen::screens(mtm);
        let infos = screens::infos(mtm);
        assert_eq!(infos.len(), all.len());
        let visible: Vec<Rect> = infos.iter().map(|i| i.visible).collect();
        assert_eq!(screens::visible_frames(mtm), visible);
        let battery = has_internal_battery();
        for (info, screen) in infos.iter().zip(all.iter()) {
            let number = screen
                .deviceDescription()
                .objectForKey(ns_string!("NSScreenNumber"))
                .and_then(|n| n.downcast::<NSNumber>().ok())
                .map(|n| n.as_u32())
                .expect("every screen has a display number");
            assert_eq!(info.builtin, CGDisplayIsBuiltin(number), "{info:?}");
            assert!(
                is_uuid(&info.id) || info.id == format!("display-{number}"),
                "{info:?}"
            );
            // A built-in panel has a UUID (2026-11-06, the MacBook's): proves the UUID path, not only the fallback.
            assert!(!info.builtin || is_uuid(&info.id), "{info:?}");
            assert_eq!(info.visible, rect(screen.visibleFrame()), "{info:?}");
            assert_eq!(info.name, screen.localizedName().to_string());
            assert_eq!(info.laptop, battery, "{info:?}");
        }
        let ids: Vec<String> = infos.into_iter().map(|i| i.id).collect();
        let again: Vec<String> = screens::infos(mtm).into_iter().map(|i| i.id).collect();
        assert_eq!(ids, again, "ids stay the same");

        screens::refresh_metrics(mtm);
        let at_zero = all.iter().find(|s| {
            let o = s.frame().origin;
            o.x == 0.0 && o.y == 0.0
        });
        assert_eq!(
            at_zero.is_some(),
            !all.is_empty(),
            "macOS puts the primary screen at 0,0"
        );
        let want = at_zero.map_or(0.0, |s| s.frame().size.height);
        assert_eq!(screens::primary_height(), want);
    }

    fn status_tile_image(_: MainThreadMarker) {
        for filled in [false, true] {
            let image = icon::status_tile_image(filled);
            let size = image.size();
            assert_eq!((size.width, size.height), (18.0, 14.0));
            assert!(image.isTemplate());
            // Rendered: the big tile's left stroke is drawn, the top right tile's inside only when filled.
            let tiff = image.TIFFRepresentation().expect("the image renders");
            let bitmap = NSBitmapImageRep::imageRepWithData(&tiff).expect("a bitmap");
            let scale = bitmap.pixelsWide() as f64 / 18.0;
            // Points from the bottom left, as drawn; bitmap rows count from the top.
            let alpha = |x: f64, y: f64| {
                let (px, py) = ((x * scale) as isize, ((14.0 - y) * scale) as isize);
                bitmap
                    .colorAtX_y(px, py)
                    .map_or(0.0, |c| c.alphaComponent())
            };
            assert!(alpha(1.5, 7.0) > 0.9, "stroke, filled={filled}");
            assert_eq!(alpha(14.2, 9.8) > 0.9, filled, "fill, filled={filled}");
            assert!(
                alpha(14.2, 3.8) < 0.1,
                "bottom tile stays empty, filled={filled}"
            );
        }
    }

    /// The +'s menu, built without opening it (plan 2026-11-10): the favorites in their slots, one that no longer fits
    /// the settings greyed out and "not available", an empty window, and "Change favorites" with the current choice
    /// checked; model and effort of a favorite whose program is gone cannot be opened. Every click reads back.
    fn plus_menu_offers_favorites(mtm: MainThreadMarker) {
        let words = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let settings = NewSession {
            places: vec![
                Place {
                    name: "Acme".into(),
                    folder: "/work/acme".into(),
                    values: BTreeMap::new(),
                },
                Place {
                    name: "Studio".into(),
                    folder: "/work/studio".into(),
                    values: BTreeMap::new(),
                },
            ],
            programs: vec![
                Program {
                    name: "Tool".into(),
                    models: words(&["big", "small"]),
                    efforts: words(&["max", "low"]),
                    command: words(&["tool", "--model", "{model}", "--", "{task}"]),
                    models_command: vec![],
                    model_labels: BTreeMap::new(),
                },
                Program {
                    name: "Plain".into(),
                    models: vec![],
                    efforts: vec![],
                    command: words(&["plain", "--", "{task}"]),
                    models_command: vec![],
                    model_labels: BTreeMap::new(),
                },
            ],
            team: TeamTexts::defaults(),
        };
        let favorite = |place: &str, program: &str, model: &str| Favorite {
            place: place.into(),
            program: program.into(),
            model: model.into(),
            effort: "max".into(),
        };
        let favorites = [
            favorite("Acme", "Tool", "big"),
            favorite("Studio", "Tool", "small"),
            favorite("Acme", "Renamed", "big"),
        ];
        // The daily list names "big" (Kevin 2026-11-10: model names with their number); the favorites keep "big".
        let mut settings = settings;
        settings.set_models("Tool", &["big\tBig 5.5".to_owned(), "small".to_owned()]);
        let menu = plus::menu(mtm, None, &settings, &favorites, &settings.team_setup(None));
        let items = |m: &NSMenu| m.itemArray().iter().collect::<Vec<Retained<NSMenuItem>>>();
        let key = |it: &NSMenuItem| {
            it.representedObject()
                .and_then(|o| o.downcast::<NSString>().ok())
                .map(|s| s.to_string())
        };
        let top = items(&menu);
        assert_eq!(
            top.len(),
            8,
            "3 favorites, choose freely, new team, empty window, line, change"
        );
        assert_eq!(top[0].title().to_string(), "① Acme · Tool · Big 5.5 · max");
        assert_eq!(top[1].title().to_string(), "② Studio · Tool · small · max");
        assert!(top[0].isEnabled() && top[1].isEnabled());
        assert!(!top[2].isEnabled(), "its program is gone");
        assert!(top[2].subtitle().is_some());
        assert_eq!(
            plus::choice(&key(&top[1]).expect("a key")),
            Some(Choice::Start(1))
        );
        assert_eq!(top[0].action(), Some(sel!(newSessionItem:)));
        // Choose freely: place › program › model › effort, a click at the end starts exactly that; a program without
        // models or efforts starts right from its row.
        let free = items(&top[3].submenu().expect("choose freely"));
        assert_eq!(free.len(), 2, "one row per place");
        let programs = items(&free[1].submenu().expect("Studio's programs"));
        assert_eq!(programs.len(), 2);
        let models = items(&programs[0].submenu().expect("Tool's models"));
        assert_eq!(models[0].title().to_string(), "Big 5.5");
        let efforts = items(&models[1].submenu().expect("small's efforts"));
        assert_eq!(efforts[1].title().to_string(), "low");
        assert_eq!(
            plus::choice(&key(&efforts[1]).expect("a key")),
            Some(Choice::Custom(Favorite {
                place: "Studio".into(),
                program: "Tool".into(),
                model: "small".into(),
                effort: "low".into(),
            }))
        );
        assert!(programs[1].submenu().is_none());
        assert_eq!(
            plus::choice(&key(&programs[1]).expect("a key")),
            Some(Choice::Custom(Favorite {
                place: "Studio".into(),
                program: "Plain".into(),
                ..Favorite::default()
            }))
        );
        assert_eq!(
            plus::choice(&key(&top[4]).expect("a key")),
            Some(Choice::Team)
        );
        assert!(
            !top[4].isEnabled() && top[4].subtitle().is_some(),
            "no program here names its session"
        );
        assert_eq!(
            plus::choice(&key(&top[5]).expect("a key")),
            Some(Choice::Blank)
        );
        assert!(top[6].isSeparatorItem());

        let change = top[7].submenu().expect("change favorites");
        let slots = items(&change);
        assert_eq!(slots.len(), 5, "3 favorites, line, team");
        let team = items(&slots[4].submenu().expect("the team"));
        assert_eq!(team.len(), 4, "place, lead, key account, workers");
        let worker_programs = items(
            &items(&team[3].submenu().expect("the workers' fields"))[0]
                .submenu()
                .expect("the workers' programs"),
        );
        assert_eq!(
            worker_programs.len(),
            2,
            "workers may use any program (Tool, Plain), also one that cannot name its session"
        );
        let team_places = items(&team[0].submenu().expect("the team's places"));
        assert_eq!(
            plus::choice(&key(&team_places[1]).expect("a key")),
            Some(Choice::ChangeTeam(
                Role::Lead,
                Field::Place,
                "Studio".into()
            ))
        );
        assert_eq!(
            plus::choice("team-change:key\neffort\nlow"),
            Some(Choice::ChangeTeam(
                Role::KeyAccount,
                Field::Effort,
                "low".into()
            ))
        );
        assert_eq!(plus::choice("team-change:boss\neffort\nlow"), None);
        let fields = items(&slots[1].submenu().expect("the second favorite"));
        assert_eq!(fields.len(), 4, "place, program, model, effort");
        let places = items(&fields[0].submenu().expect("places"));
        let checked: Vec<String> = places
            .iter()
            .filter(|it| it.state() == NSControlStateValueOn)
            .map(|it| it.title().to_string())
            .collect();
        assert_eq!(checked, ["Studio"]);
        assert_eq!(
            plus::choice(&key(&places[0]).expect("a key")),
            Some(Choice::Change(1, Field::Place, "Acme".into()))
        );
        let models = items(&fields[2].submenu().expect("models"));
        assert_eq!(models.len(), 2);
        assert_eq!(models[1].state(), NSControlStateValueOn, "small is chosen");
        assert_eq!(models[0].title().to_string(), "Big 5.5");
        assert_eq!(
            plus::choice(&key(&models[0]).expect("a key")),
            Some(Choice::Change(1, Field::Model, "big".into())),
            "the name is only shown, the model is kept"
        );
        let gone = items(&slots[2].submenu().expect("the third favorite"));
        assert!(gone[0].isEnabled() && gone[1].isEnabled());
        assert!(
            !gone[2].isEnabled() && !gone[3].isEnabled(),
            "no models or efforts without the program"
        );
        assert_eq!(plus::choice("change:x\nplace\nAcme"), None);
        assert_eq!(plus::choice("change:0\nfolder\n/tmp"), None);
        assert_eq!(
            plus::choice("change:2\nmodel\na\nb"),
            Some(Choice::Change(2, Field::Model, "a\nb".into())),
            "a value may hold a line break"
        );
    }

    /// The team dialog's fields, built without showing them (Kevin 2026-11-10: lead and key account "open to choose",
    /// preset): the preset is chosen, a changed choice reads back with the chosen place, a program without efforts
    /// gets none.
    fn team_form_presets_and_reads_back(mtm: MainThreadMarker) {
        use lookout_core::team_start::TeamSetup;
        use lookout_macos::team_dialog::TeamForm;
        use objc2_app_kit::NSControlStateValueOff;

        let words = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let program = |name: &str, models: &[&str], efforts: &[&str], command: &[&str]| Program {
            name: name.into(),
            models: words(models),
            efforts: words(efforts),
            command: words(command),
            models_command: vec![],
            model_labels: BTreeMap::new(),
        };
        let place = |name: &str| Place {
            name: name.into(),
            folder: format!("/work/{name}"),
            values: BTreeMap::new(),
        };
        let settings = NewSession {
            places: vec![place("Acme"), place("Studio")],
            programs: vec![
                program(
                    "Tool",
                    &["big", "small"],
                    &["max", "xhigh"],
                    &[
                        "tool", "--name", "{name}", "--model", "{model}", "-e", "{effort}", "--",
                        "{task}",
                    ],
                ),
                program("Bare", &[], &[], &["bare", "-n", "{name}", "--", "{task}"]),
                program("Plain", &[], &[], &["plain", "--", "{task}"]),
            ],
            team: TeamTexts::defaults(),
        };
        let role = |model: &str, effort: &str| Favorite {
            place: "Studio".into(),
            program: "Tool".into(),
            model: model.into(),
            effort: effort.into(),
        };
        let setup = TeamSetup {
            lead: role("big", "xhigh"),
            key_account: role("small", "xhigh"),
            worker: Favorite {
                place: "Studio".into(),
                program: "Plain".into(),
                ..Favorite::default()
            },
            with_key_account: true,
        };
        let form = TeamForm::new(mtm, &settings, &setup);
        let first = form.request();
        assert_eq!(first.setup(), setup);
        let lead_button = form.lead.button();
        let lead_menu = lead_button.menu().expect("the lead's cascade");
        assert_eq!(
            lead_button
                .itemAtIndex(0)
                .expect("its title")
                .title()
                .to_string(),
            "Studio · Tool · big · xhigh"
        );
        assert_eq!(
            lead_button.title().to_string(),
            "Studio · Tool · big · xhigh"
        );
        assert_eq!(
            lead_menu.numberOfItems(),
            3,
            "the title, then Acme and Studio"
        );
        let key_menu = form
            .key_account
            .button()
            .menu()
            .expect("the key account's cascade");
        assert_eq!(
            key_menu.numberOfItems(),
            3,
            "the title, then the programs that name their session: Tool and Bare, not Plain"
        );

        // Acme › Tool › big › max, as a click in the cascade.
        let sub = |m: &NSMenu, i: isize| {
            m.itemAtIndex(i)
                .and_then(|it| it.submenu())
                .expect("a submenu")
        };
        let big = sub(&sub(&sub(&lead_menu, 1), 0), 0);
        big.performActionForItemAtIndex(0);
        key_menu.performActionForItemAtIndex(2);
        form.with_key_account.setState(NSControlStateValueOff);
        form.name.setStringValue(&NSString::from_str("Shop"));
        let changed = form.request();
        assert_eq!(changed.name, "Shop");
        assert_eq!(
            changed.lead,
            Favorite {
                place: "Acme".into(),
                ..role("big", "max")
            }
        );
        assert_eq!(
            lead_button
                .itemAtIndex(0)
                .expect("its title")
                .title()
                .to_string(),
            "Acme · Tool · big · max"
        );
        assert_eq!(
            lead_button.title().to_string(),
            "Acme · Tool · big · max",
            "what the button shows (review quick 2026-11-10)"
        );
        assert_eq!(
            changed.key_account,
            Favorite {
                place: "Acme".into(),
                program: "Bare".into(),
                ..Favorite::default()
            },
            "a program without models and efforts, in the lead's place"
        );
        assert!(!changed.with_key_account);
    }

    /// Lookout has no menu bar, so the task field needs a main menu for ⌘V and ⌘A (plan review 2026-11-10).
    fn edit_menu_for_the_task_field(mtm: MainThreadMarker) {
        plus::install_edit_menu(mtm);
        let main = NSApplication::sharedApplication(mtm)
            .mainMenu()
            .expect("a main menu");
        let edit = main
            .itemAtIndex(0)
            .and_then(|it| it.submenu())
            .expect("edit menu");
        let actions: Vec<(Option<Sel>, String)> = edit
            .itemArray()
            .iter()
            .map(|it| (it.action(), it.keyEquivalent().to_string()))
            .collect();
        for (action, key) in [
            (sel!(paste:), "v"),
            (sel!(selectAll:), "a"),
            (sel!(copy:), "c"),
            (sel!(cut:), "x"),
            (sel!(undo:), "z"),
        ] {
            assert!(
                actions.contains(&(Some(action), key.to_owned())),
                "{action:?} on {key}"
            );
        }
    }

    fn rect(r: CGRect) -> Rect {
        Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
    }

    /// CFUUIDCreateString's form: 8-4-4-4-12 hex digits.
    fn is_uuid(id: &str) -> bool {
        id.len() == 36
            && id.char_indices().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == '-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
    }

    /// Without IOKit: `pmset -g batt` lists "-InternalBattery-0" on a laptop.
    fn has_internal_battery() -> bool {
        let out = std::process::Command::new("/usr/bin/pmset")
            .args(["-g", "batt"])
            .output()
            .expect("pmset runs");
        String::from_utf8_lossy(&out.stdout).contains("InternalBattery")
    }
}
