//! The one way the host reads its environment variables.
//!
//! Every setting is read the same way: the value is trimmed, and a variable that is missing,
//! empty or only whitespace is unset. A value that is not Unicode is unset for `text` and
//! `first_text`; `checked_text` reports it instead, for a setting that must not fall back silently.
//! A path is read without the Unicode requirement, so a path the platform allows is kept as it is.
//!
//! Only binaries read the environment, through this module; a library takes its configuration as
//! arguments. The constants naming the variables end in `_VARIABLE` and live beside the setting
//! they configure.

use std::ffi::OsString;
use std::path::PathBuf;

/// A variable that is set to something that is not Unicode text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotTextError {
    /// The variable's name.
    pub name: String,
}

impl std::fmt::Display for NotTextError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the variable {} is not Unicode text", self.name)
    }
}

impl std::error::Error for NotTextError {}

/// The trimmed value of `name`, or `None` when it is unset, empty or not Unicode.
pub fn text(name: &str) -> Option<String> {
    checked_text(name).ok().flatten()
}

/// The trimmed value of `name`, `None` when it is unset or empty, and an error when it is set to
/// something that is not Unicode.
pub fn checked_text(name: &str) -> Result<Option<String>, NotTextError> {
    read_text(name, std::env::var_os(name))
}

/// `value`, the value of `name` or `None` when it is unset, read by the rule of this module.
pub(crate) fn read_text(
    name: &str,
    value: Option<OsString>,
) -> Result<Option<String>, NotTextError> {
    match value.map(OsString::into_string) {
        None => Ok(None),
        Some(Ok(value)) => Ok(non_empty(value.trim())),
        Some(Err(_)) => Err(NotTextError {
            name: name.to_owned(),
        }),
    }
}

/// The value of the first of `names` that is set, read as `text` reads it.
pub fn first_text(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| text(name))
}

/// The value of `name` as a path, or `None` when it is unset or empty. A Unicode value is
/// trimmed; any other value is kept as it is.
pub fn path(name: &str) -> Option<PathBuf> {
    match std::env::var_os(name)?.into_string() {
        Ok(value) => non_empty(value.trim()).map(PathBuf::from),
        Err(value) => Some(PathBuf::from(value)),
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}
