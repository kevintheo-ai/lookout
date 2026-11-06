//! Lookout's memory across restarts (Main.swift, `UserDefaults.standard`): the keys `lookout.*` of the app's own domain
//! (its bundle id), the same keys and value types as the Swift app, so both read what the other wrote. Any thread
//! (`NSUserDefaults` is thread-safe).

/// `string(forKey:)`: a string value; numbers are not converted (Swift's `string(forKey:)` would; Lookout only stores
/// strings there).
pub fn string(key: &str) -> Option<String> {
    let _ = key;
    todo!()
}

/// `stringArray(forKey:)`: an array whose elements are all strings, else `None`.
pub fn string_array(key: &str) -> Option<Vec<String>> {
    let _ = key;
    todo!()
}

/// `object(forKey:) as? Bool`: a stored boolean (NSNumber); anything else `None`.
pub fn boolean(key: &str) -> Option<bool> {
    let _ = key;
    todo!()
}

pub fn set_string(key: &str, value: &str) {
    let _ = (key, value);
    todo!()
}

pub fn set_string_array(key: &str, value: &[String]) {
    let _ = (key, value);
    todo!()
}

pub fn set_bool(key: &str, value: bool) {
    let _ = (key, value);
    todo!()
}

pub fn remove(key: &str) {
    let _ = key;
    todo!()
}
