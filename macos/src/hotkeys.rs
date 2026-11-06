//! Global shortcuts Ctrl+Opt+<digit> (Main.swift `installHotkeys`). Carbon `RegisterEventHotKey` instead of an NSEvent
//! monitor: only Carbon CONSUMES the combination; a global monitor would also pass it on to the terminal, where it lands
//! as a control sequence in the running AI session. A few lines of own FFI to Carbon.framework (no crate).

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::rc::Rc;
use std::sync::Once;

type OSStatus = i32;
type EventTargetRef = *mut c_void;
type EventRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
/// On 64-bit macOS an `EventHandlerUPP` is the plain `EventHandlerProcPtr` (CarbonEventsCore.h `NewEventHandlerUPP`).
type EventHandlerUPP = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerUPP,
        num_types: usize,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> OSStatus;
    fn GetEventParameter(
        event: EventRef,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> OSStatus;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn UnregisterEventHotKey(hot_key: EventHotKeyRef) -> OSStatus;
}

// Constants from the macOS 27 SDK headers (HIToolbox CarbonEvents.h, CarbonEventsCore.h, Events.h; MacTypes.h).
const NO_ERR: OSStatus = 0;
const PARAM_ERR: OSStatus = -50;
const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");
/// `controlKey | optionKey` (`1 << controlKeyBit` = 12, `1 << optionKeyBit` = 11).
const MODIFIERS: u32 = (1 << 12) | (1 << 11);
/// 'WGKY', Swift's `OSType(0x5747_4B59)`.
const SIGNATURE: u32 = u32::from_be_bytes(*b"WGKY");
/// `kVK_ANSI_0` … `kVK_ANSI_9` (Events.h): the digit row is not in order.
const DIGIT_KEYS: [u32; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];

static HANDLER: Once = Once::new();

type Action = Rc<dyn Fn(usize)>;

thread_local! {
    // Main thread only: `on_press` is not Send. An Rc, so a press never holds a borrow while `on_press` runs (it may
    // reinstall the hotkeys).
    static ACTION: RefCell<Option<Action>> = const { RefCell::new(None) };
    static REFS: RefCell<Vec<EventHotKeyRef>> = const { RefCell::new(Vec::new()) };
}

fn key_code(digit: u8) -> Option<u32> {
    DIGIT_KEYS.get(usize::from(digit)).copied()
}

/// Registers Ctrl+Opt+`digits[i]` (e.g. `[1, 2, 3]`, with the parking lot `[1, 2, 3, 4]`); `on_press(i)` runs on the
/// main queue with the index into `digits`. Register only keys that have a task: a registration swallows the combination
/// system-wide. Logs like Swift: per failure "Hotkey Ctrl+Opt+N nicht registrierbar (Status S) — vermutlich belegt",
/// then "Hotkeys registriert: n/m". Main thread.
pub fn install(digits: &[u8], on_press: Box<dyn Fn(usize)>) {
    // A second install replaces the first: the same combination twice in one app would fail as "belegt".
    unregister_all();
    ACTION.with(|a| *a.borrow_mut() = Some(Rc::from(on_press)));
    HANDLER.call_once(|| {
        let spec = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };
        // SAFETY: one valid spec that lives through the call (Swift passes a local too); no user data, no handler ref
        // (never removed). The status is ignored, as in Swift.
        unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                on_hot_key,
                1,
                &spec,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
    });
    let mut registered = 0;
    for (i, &digit) in digits.iter().enumerate() {
        let status = match key_code(digit) {
            Some(key) => {
                let id = EventHotKeyID {
                    signature: SIGNATURE,
                    id: i as u32,
                };
                let mut hot_key = ptr::null_mut();
                // SAFETY: plain values and a valid out pointer; the target is the application's.
                let status = unsafe {
                    RegisterEventHotKey(
                        key,
                        MODIFIERS,
                        id,
                        GetApplicationEventTarget(),
                        0,
                        &mut hot_key,
                    )
                };
                if status == NO_ERR {
                    REFS.with(|r| r.borrow_mut().push(hot_key));
                    registered += 1;
                }
                status
            }
            None => PARAM_ERR,
        };
        if status != NO_ERR {
            crate::log(&format!(
                "Hotkey Ctrl+Opt+{digit} nicht registrierbar (Status {status}) — vermutlich belegt"
            ));
        }
    }
    crate::log(&format!(
        "Hotkeys registriert: {registered}/{}",
        digits.len()
    ));
}

/// Unregisters all (quit). Main thread.
pub fn remove() {
    unregister_all();
    ACTION.with(|a| *a.borrow_mut() = None);
}

fn unregister_all() {
    for hot_key in REFS.with(|r| std::mem::take(&mut *r.borrow_mut())) {
        // SAFETY: a ref RegisterEventHotKey returned, unregistered only once (taken out of the list).
        unsafe { UnregisterEventHotKey(hot_key) };
    }
}

/// Carbon calls this on the main thread; like Swift it hands the press to the main queue.
extern "C" fn on_hot_key(_: EventHandlerCallRef, event: EventRef, _: *mut c_void) -> OSStatus {
    let mut id = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    // SAFETY: Carbon passes a valid hot key event; the buffer is an EventHotKeyID of exactly the size given.
    let err = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            ptr::null_mut(),
            size_of::<EventHotKeyID>(),
            ptr::null_mut(),
            (&raw mut id).cast(),
        )
    };
    if err != NO_ERR {
        return err;
    }
    let index = id.id as usize;
    crate::on_main(move || {
        // Read when it runs, as Swift's `hotKeyAction?`: after `remove` a queued press does nothing.
        let action = ACTION.with(|a| a.borrow().clone());
        if let Some(action) = action {
            action(index);
        }
    });
    NO_ERR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_key_codes_are_the_ansi_ones() {
        let codes: Vec<_> = (0..=9).map(key_code).collect();
        let want = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
        assert_eq!(codes, want.map(Some));
        assert_eq!(key_code(10), None);
    }

    #[test]
    fn carbon_constants_match_swift_and_the_headers() {
        assert_eq!(SIGNATURE, 0x5747_4B59);
        assert_eq!(K_EVENT_CLASS_KEYBOARD, 0x6B65_7962);
        assert_eq!(K_EVENT_PARAM_DIRECT_OBJECT, 0x2D2D_2D2D);
        assert_eq!(TYPE_EVENT_HOT_KEY_ID, 0x686B_6964);
        assert_eq!(MODIFIERS, 0x1800);
        assert_eq!(size_of::<EventHotKeyID>(), 8);
    }
}
