//! Shared by both AppKit test programs: they run while someone works at the same Mac, so no window of theirs may show
//! on screen or catch a click.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSBackingStoreType, NSPanel, NSWindowStyleMask};
use objc2_foundation::NSRect;

type Init =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSRect, usize, usize, Bool) -> *mut AnyObject;
type GetAlpha = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> f64;
type SetAlpha = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, f64);
type GetIgnores = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool;
type SetIgnores = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Bool);

/// NSWindow's own implementations, saved by the swap.
static INIT: OnceLock<Imp> = OnceLock::new();
static GET_ALPHA: OnceLock<Imp> = OnceLock::new();
static SET_ALPHA: OnceLock<Imp> = OnceLock::new();
static GET_IGNORES: OnceLock<Imp> = OnceLock::new();
static SET_IGNORES: OnceLock<Imp> = OnceLock::new();

thread_local! {
    /// Per window (by address; a new window at an old address starts over at birth): the alpha and the click-through
    /// the code asked for.
    static ASKED: RefCell<HashMap<usize, (f64, bool)>> = RefCell::default();
}

fn original<F: Copy>(saved: &OnceLock<Imp>) -> F {
    assert_eq!(size_of::<F>(), size_of::<Imp>());
    // SAFETY: each caller names the type of the method it saved; both are plain function pointers.
    unsafe { std::mem::transmute_copy(saved.get().expect("saved by the swap")) }
}

/// What the window really has: NSWindow's own getters.
fn real(window: *mut AnyObject) -> (f64, bool) {
    // SAFETY: `window` is an NSWindow and the saved IMPs are NSWindow's own, with these types (also below).
    unsafe {
        (
            original::<GetAlpha>(&GET_ALPHA)(window, sel!(alphaValue)),
            original::<GetIgnores>(&GET_IGNORES)(window, sel!(ignoresMouseEvents)).as_bool(),
        )
    }
}

fn asked(window: *mut AnyObject) -> (f64, bool) {
    ASKED
        .with_borrow(|asked| asked.get(&(window as usize)).copied())
        .unwrap_or_else(|| real(window))
}

/// Remembers what the code asked for and hands the WindowServer transparent and click-through instead (measured
/// 2026-11-11: a window at alpha 0 that takes clicks still catches them).
fn ask(window: *mut AnyObject, now: (f64, bool)) {
    ASKED.with_borrow_mut(|asked| asked.insert(window as usize, now));
    // SAFETY: as in `real`.
    unsafe {
        original::<SetAlpha>(&SET_ALPHA)(window, sel!(setAlphaValue:), 0.0);
        original::<SetIgnores>(&SET_IGNORES)(window, sel!(setIgnoresMouseEvents:), Bool::YES);
    }
}

unsafe extern "C-unwind" fn init(
    this: *mut AnyObject,
    cmd: Sel,
    rect: NSRect,
    style: usize,
    backing: usize,
    defer: Bool,
) -> *mut AnyObject {
    // SAFETY: NSWindow's designated initializer, with exactly this signature.
    let window = unsafe { original::<Init>(&INIT)(this, cmd, rect, style, backing, defer) };
    if !window.is_null() {
        ask(window, real(window));
    }
    window
}

unsafe extern "C-unwind" fn alpha(this: *mut AnyObject, _: Sel) -> f64 {
    asked(this).0
}

unsafe extern "C-unwind" fn set_alpha(this: *mut AnyObject, _: Sel, alpha: f64) {
    ask(this, (alpha, asked(this).1));
}

unsafe extern "C-unwind" fn ignores(this: *mut AnyObject, _: Sel) -> Bool {
    Bool::new(asked(this).1)
}

unsafe extern "C-unwind" fn set_ignores(this: *mut AnyObject, _: Sel, ignores: Bool) {
    ask(this, (asked(this).0, ignores.as_bool()));
}

/// From here on every window is born transparent and click-through (NSWindow's designated initializer, which every
/// panel goes through) and stays so: its alpha and click-through setters hand the WindowServer only 0 and "ignores
/// clicks", while the getters answer what the code asked for, so fades, the list's clicks and the tests' checks run as
/// on screen. Views still lay out, draw and take synthetic events. Call once, before any window.
pub fn keep_windows_invisible(mtm: MainThreadMarker) {
    let window = AnyClass::get(c"NSWindow").expect("NSWindow");
    let swap = |name: Sel, with: Imp, saved: &OnceLock<Imp>| {
        let method = window
            .instance_method(name)
            .unwrap_or_else(|| panic!("NSWindow {name:?}"));
        // SAFETY: `with` has the signature of the method it replaces; swapped once, on the main thread.
        let own = unsafe { method.set_implementation(with) };
        saved.set(own).expect("swapped once");
    };
    // SAFETY: the functions above, typed as the methods they replace, cast to the runtime's untyped IMP. The getters and
    // setters go first: the initializer's replacement calls their saved originals.
    unsafe {
        use std::mem::transmute;
        swap(
            sel!(alphaValue),
            transmute::<GetAlpha, Imp>(alpha),
            &GET_ALPHA,
        );
        swap(
            sel!(setAlphaValue:),
            transmute::<SetAlpha, Imp>(set_alpha),
            &SET_ALPHA,
        );
        swap(
            sel!(ignoresMouseEvents),
            transmute::<GetIgnores, Imp>(ignores),
            &GET_IGNORES,
        );
        swap(
            sel!(setIgnoresMouseEvents:),
            transmute::<SetIgnores, Imp>(set_ignores),
            &SET_IGNORES,
        );
        swap(
            sel!(initWithContentRect:styleMask:backing:defer:),
            transmute::<Init, Imp>(init),
            &INIT,
        );
    }

    // Checked on a panel like the app's (a subclass could bring its own getters), never shown.
    let probe = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::ZERO,
        NSWindowStyleMask::Borderless,
        NSBackingStoreType::Buffered,
        false,
    );
    let raw = Retained::as_ptr(&probe).cast_mut().cast();
    assert_eq!(real(raw), (0.0, true), "born transparent and click-through");
    probe.setAlphaValue(0.6);
    probe.setIgnoresMouseEvents(false);
    assert_eq!(
        (probe.alphaValue(), probe.ignoresMouseEvents()),
        (0.6, false),
        "reads back what it asked for"
    );
    assert_eq!(
        real(raw),
        (0.0, true),
        "the WindowServer still gets nothing to show or click"
    );
}
