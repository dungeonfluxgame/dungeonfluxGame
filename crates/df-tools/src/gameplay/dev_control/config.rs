//! Development-only control surface: enabled flag, loopback guard and bearer token.
//!
//! The surface exists only when the server starts with
//! `DF_GAMEPLAY_DEV_CONTROL=on`. It listens on loopback only and refuses to
//! start on a non-loopback game listener unless `DF_GAMEPLAY_DEV_CONTROL_PUBLIC=on`
//! is also set. A random token is written to a file in the run's own folder;
//! the client reads that file and the token is never printed.

use std::{env::VarError, io, net::SocketAddr};

/// Whether the development control surface starts with this game listener.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Setting {
    /// The surface stays off: no route, no token file, no behaviour change.
    Off,
    /// The surface starts on the loopback game listener with a fresh token file.
    On,
}

/// Reads `DF_GAMEPLAY_DEV_CONTROL` (`on` enables, unset or `off` disables) and
/// `DF_GAMEPLAY_DEV_CONTROL_PUBLIC` (required to be exactly `on` before the
/// surface may start on a non-loopback listener). Anything else refuses startup.
pub(super) fn setting(
    control: Result<String, VarError>,
    public: Result<String, VarError>,
    listen: SocketAddr,
) -> Result<Setting, io::Error> {
    let enabled = match optional(control)?.as_deref() {
        None | Some("off") => false,
        Some("on") => true,
        Some(_) => return Err(refused("DF_GAMEPLAY_DEV_CONTROL")),
    };
    if !enabled {
        return Ok(Setting::Off);
    }
    if !listen.ip().is_loopback() {
        match optional(public)?.as_deref() {
            Some("on") => {}
            _ => return Err(refused("DF_GAMEPLAY_DEV_CONTROL_PUBLIC")),
        }
    }
    Ok(Setting::On)
}

/// Reads the separate opt-in for development commands that edit battle state.
/// The control surface can be authenticated while these mutations remain off.
pub(super) fn overrides_enabled(input: Result<String, VarError>) -> bool {
    matches!(optional(input).ok().flatten().as_deref(), Some("on"))
}

/// The value of one optional setting: unset is `None`, set is its text. This
/// mirrors the startup configuration convention without depending on it.
fn optional(input: Result<String, VarError>) -> Result<Option<String>, io::Error> {
    match input {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(refused("non-unicode setting")),
    }
}

fn refused(name: &str) -> io::Error {
    io::Error::other(format!("{name} refused"))
}

/// A fresh random bearer token, lowercase hexadecimal.
pub(super) fn fresh_token() -> Result<String, io::Error> {
    let bytes = super::super::model::random::<32>()
        .map_err(|_| io::Error::other("dev control token unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Writes `token` to `<run_dir>/dev-control-token` and returns the file path.
/// The file is created with the process default permissions inside the run's
/// own folder, never in the repository root.
pub(super) fn write_token_file(
    run_dir: &std::path::Path,
    token: &str,
) -> Result<std::path::PathBuf, io::Error> {
    std::fs::create_dir_all(run_dir)?;
    let path = run_dir.join("dev-control-token");
    std::fs::write(&path, format!("{token}\n"))?;
    Ok(path)
}

/// Splits `authorization: Bearer <token>` and reports whether it matches.
pub(super) fn bearer_authorized(header: Option<&str>, token: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    let Some(presented) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_equal(presented.as_bytes(), token.as_bytes())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn loopback() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 63359)
    }

    fn lan() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 24)), 63359)
    }

    fn var(value: &str) -> Result<String, VarError> {
        Ok(value.to_owned())
    }

    fn unset() -> Result<String, VarError> {
        Err(VarError::NotPresent)
    }

    #[test]
    fn control_surface_is_off_unless_explicitly_enabled() {
        assert!(matches!(
            setting(unset(), unset(), loopback()),
            Ok(Setting::Off)
        ));
        assert!(matches!(
            setting(var("off"), unset(), loopback()),
            Ok(Setting::Off)
        ));
    }

    #[test]
    fn control_surface_starts_on_loopback_when_on() {
        assert!(matches!(
            setting(var("on"), unset(), loopback()),
            Ok(Setting::On)
        ));
    }

    #[test]
    fn control_surface_refuses_non_loopback_without_explicit_public() {
        assert!(setting(var("on"), unset(), lan()).is_err());
        assert!(setting(var("on"), var("off"), lan()).is_err());
        assert!(matches!(
            setting(var("on"), var("on"), lan()),
            Ok(Setting::On)
        ));
    }

    #[test]
    fn control_surface_refuses_unknown_values() {
        assert!(setting(var("yes"), unset(), loopback()).is_err());
        assert!(setting(var("ON"), unset(), loopback()).is_err());
    }

    #[test]
    fn gameplay_overrides_are_opt_in() {
        assert!(!overrides_enabled(unset()));
        assert!(!overrides_enabled(var("off")));
        assert!(!overrides_enabled(var("ON")));
        assert!(overrides_enabled(var("on")));
    }

    #[test]
    fn bearer_check_accepts_only_the_exact_token() {
        let token = "abc123";
        assert!(bearer_authorized(Some("Bearer abc123"), token));
        assert!(!bearer_authorized(None, token));
        assert!(!bearer_authorized(Some("abc123"), token));
        assert!(!bearer_authorized(Some("Bearer abc124"), token));
        assert!(!bearer_authorized(Some("Bearer abc12"), token));
    }
}
