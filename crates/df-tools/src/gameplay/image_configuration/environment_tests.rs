//! OS-string boundary regressions without mutating the shared process environment.
use super::*;
use std::ffi::OsString;

#[cfg(windows)]
fn invalid_unicode() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&[0xd800])
}
#[cfg(unix)]
fn invalid_unicode() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![0xff])
}

#[cfg(any(windows, unix))]
#[test]
fn unrelated_non_unicode_environment_is_ignored_without_changing_owned_precedence() {
    let mut unrelated_name = OsString::from("UNRELATED_");
    unrelated_name.push(invalid_unicode());
    let pairs = collect_overlay_environment([
        (unrelated_name, invalid_unicode()),
        (OsString::from("UNRELATED_SECRET"), invalid_unicode()),
        // This setting has a separate host parser and never enters ConfigControl.
        (OsString::from("DF_IMAGERY_CONFIG"), invalid_unicode()),
        (
            OsString::from("DF_GAMEPLAY_IMAGERY_PROVIDER"),
            OsString::from("flux"),
        ),
        (
            OsString::from("DF_IMAGERY_BACKEND"),
            OsString::from("openai"),
        ),
        (OsString::from("DF_IMAGERY_MODE"), OsString::from("off")),
    ])
    .unwrap();
    assert_eq!(pairs.len(), 3);
    let borrowed = pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let control = ConfigControl::build("", &borrowed, &RuntimePatch::empty(), 1).unwrap();
    let effective = control.read_effective();
    assert_eq!(effective.config.mode, GlobalMode::Off);
    assert_eq!(effective.config.default_backend, BackendId::OpenAi);
    assert_eq!(
        effective.provenance.source_of("default_backend"),
        Some(OverlaySource::NewEnv("DF_IMAGERY_BACKEND".to_owned()))
    );
    assert_eq!(
        effective.provenance.source_of("mode"),
        Some(OverlaySource::NewEnv("DF_IMAGERY_MODE".to_owned()))
    );
}

#[cfg(any(windows, unix))]
#[test]
fn relevant_non_unicode_values_return_a_typed_redacted_error() {
    for name in [
        "DF_IMAGERY_MODE",
        "DF_GAMEPLAY_IMAGERY_PROVIDER",
        "DF_IMAGERY_BACKEND",
    ] {
        let mut private = OsString::from("private-fixture-secret-");
        private.push(invalid_unicode());
        let error = collect_overlay_environment([(OsString::from(name), private)]).unwrap_err();
        assert_eq!(
            error,
            df_imagery::OverlayError::InvalidField {
                field: name.to_owned(),
                reason: "environment value is not valid Unicode",
            }
        );
        assert!(!error.to_string().contains("private-fixture-secret"));
    }
}

#[cfg(any(windows, unix))]
#[test]
fn relevant_non_unicode_names_are_refused_without_echoing_the_name_or_value() {
    let mut name = OsString::from("DF_IMAGERY_private-fixture-secret-");
    name.push(invalid_unicode());
    let error = collect_overlay_environment([(name, OsString::from("private-value"))]).unwrap_err();
    assert_eq!(
        error,
        df_imagery::OverlayError::InvalidField {
            field: "imagery.environment".to_owned(),
            reason: "environment name is not valid Unicode",
        }
    );
    let rendered = error.to_string();
    assert!(!rendered.contains("private-fixture-secret"));
    assert!(!rendered.contains("private-value"));
}
