use std::{
    env::VarError,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    num::NonZeroU16,
    path::Path,
    str::FromStr,
    time::Duration,
};

const QUALIFICATION_PORT: u16 = 63309;
const MAXIMUM_SESSION_SECONDS: u64 = 365 * 24 * 60 * 60;

/// Raw `DF_GAMEPLAY_*` inputs in the order of [`SessionSettings`] fields.
pub(super) struct SessionInputs {
    pub(super) listen: Result<String, VarError>,
    pub(super) allowed_origins: Result<String, VarError>,
    pub(super) session_seconds: Result<String, VarError>,
    pub(super) call_budget: Result<String, VarError>,
    pub(super) max_connections: Result<String, VarError>,
    pub(super) max_watch_streams: Result<String, VarError>,
}

impl SessionInputs {
    pub(super) fn from_environment() -> Self {
        Self {
            listen: std::env::var("DF_GAMEPLAY_LISTEN"),
            allowed_origins: std::env::var("DF_GAMEPLAY_ALLOWED_ORIGINS"),
            session_seconds: std::env::var("DF_GAMEPLAY_SESSION_SECONDS"),
            call_budget: std::env::var("DF_GAMEPLAY_CALL_BUDGET"),
            max_connections: std::env::var("DF_GAMEPLAY_MAX_CONNECTIONS"),
            max_watch_streams: std::env::var("DF_GAMEPLAY_MAX_WATCH_STREAMS"),
        }
    }
}

/// Listener, origin and lifetime bounds of one demonstration run.
///
/// `allowed_origins` is never empty; its first entry is the origin shown to players.
/// `session` of `None` runs until interrupted. `call_budget` of `None` (plain play with
/// `DF_GAMEPLAY_CALL_BUDGET` unset) is unlimited; `Some` counts every actor call, including
/// view refreshes, as a finite lifetime work limit.
pub(super) struct SessionSettings {
    pub(super) listen: SocketAddr,
    pub(super) allowed_origins: Vec<String>,
    pub(super) session: Option<Duration>,
    pub(super) call_budget: Option<u16>,
    pub(super) max_connections: usize,
    pub(super) max_watch_streams: usize,
}

impl SessionSettings {
    /// The short timed qualification run: fixed loopback address, 290 seconds, 128 calls.
    pub(super) fn qualification() -> Self {
        Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), QUALIFICATION_PORT),
            allowed_origins: vec![format!("http://127.0.0.1:{QUALIFICATION_PORT}")],
            session: Some(Duration::from_secs(290)),
            call_budget: Some(128),
            max_connections: 4,
            max_watch_streams: 6,
        }
    }
}

/// Accepts only an explicit, bounded interactive configuration; invalid input is refused
/// without echoing it. Unset values default to loopback-only play without a timer.
pub(super) fn session_settings(inputs: SessionInputs) -> Result<SessionSettings, io::Error> {
    let explicit_listen = optional(inputs.listen)?;
    let listen = match &explicit_listen {
        Some(value) => SocketAddr::from_str(value).map_err(|_| refused("DF_GAMEPLAY_LISTEN"))?,
        None => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), QUALIFICATION_PORT),
    };
    if listen.port() == 0 {
        return Err(refused("DF_GAMEPLAY_LISTEN"));
    }
    let allowed_origins = match optional(inputs.allowed_origins)? {
        Some(value) => {
            let origins = value
                .split(',')
                .map(|origin| exact_origin(origin).map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()?;
            if origins
                .iter()
                .enumerate()
                .any(|(index, origin)| origins[..index].contains(origin))
            {
                return Err(refused("DF_GAMEPLAY_ALLOWED_ORIGINS"));
            }
            origins
        }
        None if listen.ip().is_loopback() => vec![
            format!("http://{listen}"),
            format!("http://localhost:{}", listen.port()),
        ],
        None => {
            return Err(io::Error::other(
                "DF_GAMEPLAY_ALLOWED_ORIGINS required for a non-loopback listener",
            ));
        }
    };
    let session = match optional(inputs.session_seconds)? {
        Some(value) => {
            let seconds = value
                .parse::<u64>()
                .ok()
                .filter(|seconds| (1..=MAXIMUM_SESSION_SECONDS).contains(seconds))
                .ok_or_else(|| refused("DF_GAMEPLAY_SESSION_SECONDS"))?;
            Some(Duration::from_secs(seconds))
        }
        None => None,
    };
    Ok(SessionSettings {
        listen,
        allowed_origins,
        session,
        call_budget: optional_bounded_count(inputs.call_budget, "DF_GAMEPLAY_CALL_BUDGET")?,
        max_connections: usize::from(bounded_count(
            inputs.max_connections,
            16,
            "DF_GAMEPLAY_MAX_CONNECTIONS",
        )?),
        max_watch_streams: usize::from(bounded_count(
            inputs.max_watch_streams,
            24,
            "DF_GAMEPLAY_MAX_WATCH_STREAMS",
        )?),
    })
}

fn refused(name: &str) -> io::Error {
    io::Error::other(format!("{name} invalid"))
}

fn optional(input: Result<String, VarError>) -> Result<Option<String>, io::Error> {
    match input {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(_) => Err(io::Error::other("gameplay setting not unicode")),
    }
}

fn bounded_count(
    input: Result<String, VarError>,
    default: u16,
    name: &str,
) -> Result<u16, io::Error> {
    Ok(optional_bounded_count(input, name)?.unwrap_or(default))
}

fn optional_bounded_count(
    input: Result<String, VarError>,
    name: &str,
) -> Result<Option<u16>, io::Error> {
    match optional(input)? {
        Some(value) => value
            .parse::<NonZeroU16>()
            .map(|count| Some(count.get()))
            .map_err(|_| refused(name)),
        None => Ok(None),
    }
}

/// A browser `Origin` value: `http` or `https`, a host and optional port, nothing else.
fn exact_origin(origin: &str) -> Result<&str, io::Error> {
    let uri =
        axum::http::Uri::from_str(origin).map_err(|_| refused("DF_GAMEPLAY_ALLOWED_ORIGINS"))?;
    let canonical = match (uri.scheme_str(), uri.authority()) {
        (Some(scheme @ ("http" | "https")), Some(authority)) => {
            format!("{scheme}://{authority}")
        }
        _ => return Err(refused("DF_GAMEPLAY_ALLOWED_ORIGINS")),
    };
    if canonical != origin || origin.contains('@') {
        return Err(refused("DF_GAMEPLAY_ALLOWED_ORIGINS"));
    }
    Ok(origin)
}

pub(super) struct StartupAssets {
    pub(super) glue: bytes::Bytes,
    pub(super) wasm: bytes::Bytes,
    pub(super) art: bytes::Bytes,
    pub(super) campfire: bytes::Bytes,
    pub(super) portrait: bytes::Bytes,
    pub(super) inn: Option<bytes::Bytes>,
}

/// This is the local demonstration's existing byte-load policy, not media decoding.
pub(super) fn load_assets(web_root: &Path, asset_root: &Path) -> Result<StartupAssets, io::Error> {
    let glue = super::bounded_asset(web_root.join("df_tools.js"), 1024 * 1024)?;
    let wasm = super::bounded_asset(web_root.join("df_tools_bg.wasm"), 32 * 1024 * 1024)?;
    let art = super::bounded_asset(
        asset_root.join("scenes/mara-harbor-v4.png"),
        16 * 1024 * 1024,
    )?;
    let campfire = super::bounded_asset(
        asset_root.join("../concept-art/scene-campfire-under-stars.webp"),
        4 * 1024 * 1024,
    )?;
    let portrait = super::bounded_asset(
        asset_root.join("../concept-art/vell-avatar.webp"),
        1024 * 1024,
    )?;
    let inn = match super::bounded_asset(
        asset_root.join("../concept-art/scene-tavern-barkeep-talk-rain.webp"),
        4 * 1024 * 1024,
    ) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    Ok(StartupAssets {
        glue,
        wasm,
        art,
        campfire,
        portrait,
        inn,
    })
}

/// Accepts only the current local database-name input; never a credential or endpoint URI.
pub(super) fn database_config(
    input: Result<String, VarError>,
) -> Result<tokio_postgres::Config, io::Error> {
    let name = match input {
        Ok(value) => value,
        Err(VarError::NotPresent) => "df_gameplay_demo_20261004_r03".to_owned(),
        Err(_) => return Err(io::Error::other("local demonstration database invalid")),
    };
    if name.len() > 63
        || !name.starts_with("df_gameplay_demo_")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(io::Error::other("local demonstration database invalid"));
    }
    let mut configuration = tokio_postgres::Config::new();
    configuration
        .host("127.0.0.1")
        .port(55517)
        .user("df_gameplay_demo_admin_20261004")
        .dbname(&name);
    Ok(configuration)
}

/// Resolve once at startup; missing files are diagnosed by the typed pack loader.
pub(super) fn pack_directory(
    input: Result<String, VarError>,
    current: &Path,
) -> Result<std::path::PathBuf, io::Error> {
    let directory = optional(input)?.unwrap_or_else(|| "content/harbor-journey".to_owned());
    if directory.is_empty() {
        return Err(refused("DF_GAMEPLAY_PACK_DIR"));
    }
    let directory = std::path::PathBuf::from(directory);
    Ok(if directory.is_absolute() {
        directory
    } else {
        current.join(directory)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(values: [Option<&str>; 6]) -> SessionInputs {
        let [listen, origins, seconds, budget, connections, streams] =
            values.map(|value| value.map(str::to_owned).ok_or(VarError::NotPresent));
        SessionInputs {
            listen,
            allowed_origins: origins,
            session_seconds: seconds,
            call_budget: budget,
            max_connections: connections,
            max_watch_streams: streams,
        }
    }

    #[test]
    fn defaults_are_loopback_untimed_unbudgeted_and_bounded() {
        let settings = session_settings(inputs([None; 6])).unwrap();
        assert_eq!(settings.listen, "127.0.0.1:63309".parse().unwrap());
        assert_eq!(
            settings.allowed_origins,
            ["http://127.0.0.1:63309", "http://localhost:63309"]
        );
        assert_eq!(settings.session, None);
        assert_eq!(settings.call_budget, None);
        assert_eq!(settings.max_connections, 16);
        assert_eq!(settings.max_watch_streams, 24);
    }

    #[test]
    fn qualification_settings_keep_the_fixed_original_limits() {
        let settings = SessionSettings::qualification();
        assert_eq!(settings.listen, "127.0.0.1:63309".parse().unwrap());
        assert_eq!(settings.allowed_origins, ["http://127.0.0.1:63309"]);
        assert_eq!(settings.session, Some(Duration::from_secs(290)));
        assert_eq!(settings.call_budget, Some(128));
        assert_eq!(settings.max_connections, 4);
        assert_eq!(settings.max_watch_streams, 6);
    }

    #[test]
    fn every_valid_override_is_applied() {
        let settings = session_settings(inputs([
            Some("127.0.0.1:4000"),
            Some("http://example.test:4000,https://example.test"),
            Some("3600"),
            Some("500"),
            Some("3"),
            Some("5"),
        ]))
        .unwrap();
        assert_eq!(settings.listen.port(), 4000);
        assert_eq!(
            settings.allowed_origins,
            ["http://example.test:4000", "https://example.test"]
        );
        assert_eq!(settings.session, Some(Duration::from_secs(3600)));
        assert_eq!(settings.call_budget, Some(500));
        assert_eq!(settings.max_connections, 3);
        assert_eq!(settings.max_watch_streams, 5);
    }

    #[test]
    fn default_origins_follow_the_listen_address() {
        let ipv6 =
            session_settings(inputs([Some("[::1]:5000"), None, None, None, None, None])).unwrap();
        assert_eq!(
            ipv6.allowed_origins,
            ["http://[::1]:5000", "http://localhost:5000"]
        );
    }

    #[test]
    fn non_loopback_listener_needs_explicit_origins() {
        for listen in ["0.0.0.0:63309", "192.168.1.20:63309", "[::]:63309"] {
            assert!(
                session_settings(inputs([Some(listen), None, None, None, None, None])).is_err()
            );
        }
        let settings = session_settings(inputs([
            Some("0.0.0.0:63309"),
            Some("http://192.168.1.20:63309"),
            None,
            None,
            None,
            None,
        ]))
        .unwrap();
        assert_eq!(settings.allowed_origins, ["http://192.168.1.20:63309"]);
    }

    #[test]
    fn invalid_values_are_refused_without_echoing_input() {
        let cases: [[Option<&str>; 6]; 20] = [
            [Some("localhost:63309"), None, None, None, None, None],
            [Some("127.0.0.1"), None, None, None, None, None],
            [Some("127.0.0.1:0"), None, None, None, None, None],
            [None, Some(""), None, None, None, None],
            [
                None,
                Some("http://127.0.0.1:63309,"),
                None,
                None,
                None,
                None,
            ],
            [
                None,
                Some("http://127.0.0.1:63309/"),
                None,
                None,
                None,
                None,
            ],
            [None, Some("127.0.0.1:63309"), None, None, None, None],
            [None, Some("ftp://127.0.0.1:63309"), None, None, None, None],
            [
                None,
                Some("http://user@127.0.0.1:63309"),
                None,
                None,
                None,
                None,
            ],
            [
                None,
                Some("http://a.test,http://a.test"),
                None,
                None,
                None,
                None,
            ],
            [None, None, Some("0"), None, None, None],
            [None, None, Some("-5"), None, None, None],
            [None, None, Some("soon"), None, None, None],
            [None, None, Some("31536001"), None, None, None],
            [None, None, None, Some("0"), None, None],
            [None, None, None, Some("65536"), None, None],
            [None, None, None, None, Some("0"), None],
            [None, None, None, None, Some("-1"), None],
            [None, None, None, None, None, Some("0")],
            [None, None, None, None, None, Some("many")],
        ];
        for case in cases {
            let error = session_settings(inputs(case)).err().unwrap();
            for value in case.into_iter().flatten().filter(|value| !value.is_empty()) {
                assert!(!error.to_string().contains(value));
            }
        }
    }

    #[test]
    fn non_unicode_input_is_refused() {
        let mut bad = inputs([None; 6]);
        bad.call_budget = Err(VarError::NotUnicode("x".into()));
        assert!(session_settings(bad).is_err());
    }
}
