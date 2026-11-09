//! AppKit tests of the small parts (prefs, screens, icon). Runs on the main thread (harness = false): `main` creates
//! the shared NSApplication, runs each test with its panic caught, prints one line per test and fails if any failed.
//! Hotkeys and the click monitor have no live test here: registering Ctrl+Opt+<digit> would take Kevin's keys from his
//! running Lookout.

fn main() {
    #[cfg(target_os = "macos")]
    parts::run();
}

#[cfg(target_os = "macos")]
mod parts {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use objc2::MainThreadMarker;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSScreen};
    use objc2_core_foundation::CGRect;
    use objc2_core_graphics::CGDisplayIsBuiltin;
    use objc2_foundation::{NSArray, NSBundle, NSNumber, NSString, NSUserDefaults, ns_string};

    use lookout_core::geom::Rect;
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
        let tests: [(&str, Test); 3] = [
            ("prefs_round_trip", prefs_round_trip),
            ("screens_match_nsscreen", screens_match_nsscreen),
            ("status_tile_image", status_tile_image),
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
