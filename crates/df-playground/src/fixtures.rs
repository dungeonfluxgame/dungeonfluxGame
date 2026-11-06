//! Helpers shared by the tests of this crate.

use std::fmt::Debug;

use serde::Serialize;

/// Asserts the closed-key contract of an enum: `key()` is exactly its serde name, and `from_key`
/// finds every value of `all` by its key and nothing by a key it does not have.
pub fn assert_keys_are_serde_names<T: Serialize + Copy + PartialEq + Debug>(
    all: &[T],
    key: impl Fn(T) -> &'static str,
    from_key: impl Fn(&str) -> Option<T>,
) {
    for &value in all {
        let serde_name = serde_json::to_value(value).expect("a closed key serializes");
        assert_eq!(serde_name, serde_json::Value::from(key(value)), "{value:?}");
        assert_eq!(from_key(key(value)), Some(value), "{value:?}");
    }
    assert_eq!(from_key("not_a_key"), None);
}
