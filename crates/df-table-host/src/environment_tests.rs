//! The one rule for reading an environment variable, and what it means for `DF_TABLE_PORT`.

use std::ffi::OsString;

use crate::environment::{NotTextError, read_text};
use crate::options::{DEFAULT_PORT, PORT_VARIABLE, parse_dev_arguments};

fn port_value(value: &str) -> Option<String> {
    read_text(PORT_VARIABLE, Some(OsString::from(value))).expect("Unicode")
}

#[test]
fn a_missing_empty_or_blank_variable_is_unset_and_a_padded_one_is_trimmed() {
    assert_eq!(read_text("NAME", None), Ok(None));
    assert_eq!(port_value(""), None);
    assert_eq!(port_value(" \t "), None);
    assert_eq!(port_value(" 4000 \n"), Some("4000".to_owned()));
}

#[test]
fn an_empty_port_variable_runs_against_the_default_port_and_a_padded_one_is_accepted() {
    let words = vec!["canvas".to_owned()];
    let empty = port_value("");
    assert_eq!(
        parse_dev_arguments(&words, empty.as_deref()),
        Ok((DEFAULT_PORT, words.clone()))
    );
    let padded = port_value(" 4000 ");
    assert_eq!(
        parse_dev_arguments(&words, padded.as_deref()),
        Ok((4000, words.clone()))
    );
    let wrong = port_value(" not a port ");
    assert!(parse_dev_arguments(&words, wrong.as_deref()).is_err());
}

#[cfg(windows)]
#[test]
fn a_value_that_is_not_unicode_is_an_error_naming_the_variable() {
    use std::os::windows::ffi::OsStringExt;
    let lone_surrogate = OsString::from_wide(&[0xD800]);
    assert_eq!(
        read_text("NAME", Some(lone_surrogate)),
        Err(NotTextError {
            name: "NAME".to_owned()
        })
    );
}

#[cfg(unix)]
#[test]
fn a_value_that_is_not_unicode_is_an_error_naming_the_variable() {
    use std::os::unix::ffi::OsStringExt;
    let invalid = OsString::from_vec(vec![0xFF]);
    assert_eq!(
        read_text("NAME", Some(invalid)),
        Err(NotTextError {
            name: "NAME".to_owned()
        })
    );
}
