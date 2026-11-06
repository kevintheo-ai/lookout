//! Lookout's memory across restarts (Main.swift, `UserDefaults.standard`): the keys `lookout.*` of the app's own domain
//! (its bundle id), the same keys and value types as the Swift app, so both read what the other wrote. Any thread
//! (`NSUserDefaults` is thread-safe).

use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSNumber, NSString, NSUserDefaults};

fn defaults() -> Retained<NSUserDefaults> {
    NSUserDefaults::standardUserDefaults()
}

/// `string(forKey:)`: a string value; numbers are not converted (Swift's `string(forKey:)` would; Lookout only stores
/// strings there).
pub fn string(key: &str) -> Option<String> {
    let value = defaults().objectForKey(&NSString::from_str(key))?;
    value.downcast::<NSString>().ok().map(|s| s.to_string())
}

/// `stringArray(forKey:)`: an array whose elements are all strings, else `None`.
pub fn string_array(key: &str) -> Option<Vec<String>> {
    let array = defaults().stringArrayForKey(&NSString::from_str(key))?;
    Some(array.iter().map(|s| s.to_string()).collect())
}

/// `object(forKey:) as? Bool`: a stored boolean (NSNumber); anything else `None`.
pub fn boolean(key: &str) -> Option<bool> {
    let value = defaults().objectForKey(&NSString::from_str(key))?;
    let number = value.downcast::<NSNumber>().ok()?;
    // Swift's NSNumber -> Bool bridge: equal to 1 is true, equal to 0 is false, any other number fails.
    if number.isEqualToNumber(&NSNumber::new_i32(1)) {
        Some(true)
    } else if number.isEqualToNumber(&NSNumber::new_i32(0)) {
        Some(false)
    } else {
        None
    }
}

pub fn set_string(key: &str, value: &str) {
    let value = NSString::from_str(value);
    // SAFETY: an NSString is a property list value.
    unsafe { defaults().setObject_forKey(Some(&value), &NSString::from_str(key)) };
}

pub fn set_string_array(key: &str, value: &[String]) {
    let items: Vec<Retained<NSString>> = value.iter().map(|s| NSString::from_str(s)).collect();
    let array = NSArray::from_retained_slice(&items);
    // SAFETY: an NSArray of NSStrings is a property list value.
    unsafe { defaults().setObject_forKey(Some(&array), &NSString::from_str(key)) };
}

pub fn set_bool(key: &str, value: bool) {
    defaults().setBool_forKey(value, &NSString::from_str(key));
}

pub fn remove(key: &str) {
    defaults().removeObjectForKey(&NSString::from_str(key));
}
