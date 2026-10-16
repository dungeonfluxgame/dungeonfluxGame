use std::{
    env::VarError,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    num::NonZeroU16,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

const QUALIFICATION_PORT: u16 = 63309;
const MAXIMUM_SESSION_SECONDS: u64 = 365 * 24 * 60 * 60;

/// How freely the game's written and spoken content treats violence, horror, strong language
/// and adult themes. `Mature` is the default because this is an adult game; `Restrained`
/// switches the censorship back on. The limits that hold in both modes (no sexual content
/// involving minors or real people, untrusted player words) are not part of this setting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ContentRating {
    #[default]
    Mature,
    Restrained,
}

impl ContentRating {
    /// The value `DF_GAMEPLAY_CONTENT_RATING` spells for this rating.
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Mature => "mature",
            Self::Restrained => "restrained",
        }
    }

    /// The startup status line, `CONTENT rating=<label>`.
    pub(super) fn startup_line(self) -> String {
        format!("CONTENT rating={}", self.label())
    }
}

/// Unset means `mature`; exactly `mature` or `restrained` selects a rating; anything else
/// refuses startup without echoing it.
pub(super) fn content_rating_setting(
    input: Result<String, VarError>,
) -> Result<ContentRating, io::Error> {
    match optional(input)?.as_deref() {
        None | Some("mature") => Ok(ContentRating::Mature),
        Some("restrained") => Ok(ContentRating::Restrained),
        Some(_) => Err(refused("DF_GAMEPLAY_CONTENT_RATING")),
    }
}

static CONTENT_RATING: std::sync::OnceLock<ContentRating> = std::sync::OnceLock::new();

/// Fixes the rating for the whole process, once, before any content is generated.
pub(super) fn install_content_rating(rating: ContentRating) -> Result<(), io::Error> {
    CONTENT_RATING
        .set(rating)
        .map_err(|_| io::Error::other("content rating already installed"))
}

/// The process-wide rating: the installed one, or `Mature` before startup installs it.
pub(super) fn content_rating() -> ContentRating {
    #[cfg(test)]
    if let Some(rating) = TEST_RATING.with(|slot| slot.get()) {
        return rating;
    }
    CONTENT_RATING.get().copied().unwrap_or_default()
}

#[cfg(test)]
thread_local! { static TEST_RATING: std::cell::Cell<Option<ContentRating>> = const { std::cell::Cell::new(None) }; }

/// While alive, [`content_rating`] answers `rating` on this thread only, so tests of the two
/// modes never leak into each other. It restores the previous answer when dropped, also across
/// the `await`s of a single-threaded test runtime.
#[cfg(test)]
pub(super) struct TestRating(Option<ContentRating>);

#[cfg(test)]
impl TestRating {
    pub(super) fn set(rating: ContentRating) -> Self {
        Self(TEST_RATING.with(|slot| slot.replace(Some(rating))))
    }
}

#[cfg(test)]
impl Drop for TestRating {
    fn drop(&mut self) {
        TEST_RATING.with(|slot| slot.set(self.0));
    }
}

/// Runs `run` with [`content_rating`] answering `rating`.
#[cfg(test)]
pub(super) fn with_test_rating<T>(rating: ContentRating, run: impl FnOnce() -> T) -> T {
    let _rating = TestRating::set(rating);
    run()
}

/// Raw `DF_GAMEPLAY_*` inputs in the order of [`SessionSettings`] fields.
pub(super) struct SessionInputs {
    pub(super) listen: Result<String, VarError>,
    pub(super) allowed_origins: Result<String, VarError>,
    pub(super) public_url: Result<String, VarError>,
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
            public_url: std::env::var("DF_GAMEPLAY_PUBLIC_URL"),
            session_seconds: std::env::var("DF_GAMEPLAY_SESSION_SECONDS"),
            call_budget: std::env::var("DF_GAMEPLAY_CALL_BUDGET"),
            max_connections: std::env::var("DF_GAMEPLAY_MAX_CONNECTIONS"),
            max_watch_streams: std::env::var("DF_GAMEPLAY_MAX_WATCH_STREAMS"),
        }
    }
}

/// Listener, origin and lifetime bounds of one demonstration run.
///
/// `allowed_origins` is never empty; its first entry is the origin shown to players. A public
/// URL is that first entry. `join_origin` is the base address players' devices use for the join
/// link and its QR code: the public URL, or else the listener's own address (a wildcard listener
/// is reached through loopback). `join_loopback` is true when that base can only be reached from
/// this machine. `session` of `None` runs until interrupted. `call_budget` of `None` (plain play
/// with `DF_GAMEPLAY_CALL_BUDGET` unset) is unlimited; `Some` counts every actor call, including
/// view refreshes, as a finite lifetime work limit.
pub(super) struct SessionSettings {
    pub(super) listen: SocketAddr,
    pub(super) allowed_origins: Vec<String>,
    pub(super) join_origin: String,
    pub(super) join_loopback: bool,
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
            join_origin: format!("http://127.0.0.1:{QUALIFICATION_PORT}"),
            join_loopback: true,
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
    let public = optional(inputs.public_url)?
        .map(|value| public_origin(&value))
        .transpose()?;
    let mut allowed_origins = match optional(inputs.allowed_origins)? {
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
        // The public origin is itself an allowed origin, so it can stand in for the list.
        None if public.is_some() => Vec::new(),
        None => {
            return Err(io::Error::other(
                "DF_GAMEPLAY_ALLOWED_ORIGINS required for a non-loopback listener",
            ));
        }
    };
    let (join_origin, join_loopback) = match public {
        Some((origin, loopback)) => {
            allowed_origins.retain(|allowed| *allowed != origin);
            allowed_origins.insert(0, origin.clone());
            (origin, loopback)
        }
        None => {
            let host = if listen.ip().is_unspecified() {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            } else {
                listen.ip()
            };
            (
                format!("http://{}", SocketAddr::new(host, listen.port())),
                host.is_loopback(),
            )
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
        join_origin,
        join_loopback,
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

/// Raw inputs of the spoken-narration feature. Only the process environment supplies the
/// ElevenLabs credential.
pub(super) struct VoiceInputs {
    pub(super) credential: Result<String, VarError>,
    pub(super) mode: Result<String, VarError>,
    pub(super) character_budget: Result<String, VarError>,
    pub(super) cache_dir: Result<String, VarError>,
    pub(super) narrator_voice: Result<String, VarError>,
    pub(super) npc_voice: Result<String, VarError>,
    pub(super) casting: Result<String, VarError>,
}

impl VoiceInputs {
    pub(super) fn from_environment() -> Self {
        Self {
            credential: std::env::var("ELEVENLABS_API_KEY"),
            mode: std::env::var("DF_GAMEPLAY_VOICE"),
            character_budget: std::env::var("DF_GAMEPLAY_VOICE_CHAR_BUDGET"),
            cache_dir: std::env::var("DF_GAMEPLAY_VOICE_CACHE"),
            narrator_voice: std::env::var("DF_GAMEPLAY_VOICE_NARRATOR"),
            npc_voice: std::env::var("DF_GAMEPLAY_VOICE_NPC"),
            casting: std::env::var("DF_GAMEPLAY_VOICE_CASTING"),
        }
    }
}

/// Stock ElevenLabs premade voice "George", a warm storyteller for the narration. Speakers
/// are cast from the voice roster instead of one shared NPC voice.
pub(super) const DEFAULT_NARRATOR_VOICE: &str = "JBFqnCBsd6RMkjVDRZzb";
const DEFAULT_VOICE_CASTING: &str = "content/voice-casting.json";
const DEFAULT_VOICE_CHARACTER_BUDGET: u32 = MAXIMUM_VOICE_CHARACTER_BUDGET;
const MAXIMUM_VOICE_CHARACTER_BUDGET: u32 = 1_000_000;
const DEFAULT_VOICE_CACHE: &str = "artifacts/tmp/play/voice-cache";

/// The ElevenLabs credential. It is only readable through [`VoiceCredential::expose`]
/// and never appears in `Debug` output.
pub(super) struct VoiceCredential(String);

impl VoiceCredential {
    pub(super) fn expose(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(super) fn for_test(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Debug for VoiceCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VoiceCredential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum VoiceOff {
    Qualification,
    Disabled,
    NoCredential,
}

impl VoiceOff {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::Qualification => "qualification run",
            Self::Disabled => "DF_GAMEPLAY_VOICE=off",
            Self::NoCredential => "ELEVENLABS_API_KEY not set",
        }
    }
}

#[derive(Debug)]
pub(super) struct VoiceConfiguration {
    pub(super) credential: VoiceCredential,
    pub(super) character_budget: u32,
    pub(super) cache_dir: std::path::PathBuf,
    pub(super) narrator_voice: String,
    /// When set, every NPC line uses this one voice instead of the cast roster voices.
    pub(super) npc_voice_override: Option<String>,
    /// The voice roster file that casts each speaker.
    pub(super) casting_path: std::path::PathBuf,
}

#[derive(Debug)]
pub(super) enum VoiceSettings {
    Off(VoiceOff),
    On(VoiceConfiguration),
}

/// Voice is on when a credential is present and `DF_GAMEPLAY_VOICE` is not `off`. Invalid
/// values are refused without echoing them; empty values count as unset.
pub(super) fn voice_settings(inputs: VoiceInputs) -> Result<VoiceSettings, io::Error> {
    let disabled = match optional(inputs.mode)?.as_deref() {
        None | Some("" | "on") => false,
        Some("off") => true,
        Some(_) => return Err(refused("DF_GAMEPLAY_VOICE")),
    };
    let character_budget = match optional(inputs.character_budget)?.filter(|v| !v.is_empty()) {
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|budget| *budget <= MAXIMUM_VOICE_CHARACTER_BUDGET)
            .ok_or_else(|| refused("DF_GAMEPLAY_VOICE_CHAR_BUDGET"))?,
        None => DEFAULT_VOICE_CHARACTER_BUDGET,
    };
    let cache_dir = optional(inputs.cache_dir)?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_VOICE_CACHE.to_owned());
    let narrator_voice = voice_identifier(
        inputs.narrator_voice,
        DEFAULT_NARRATOR_VOICE,
        "DF_GAMEPLAY_VOICE_NARRATOR",
    )?;
    let npc_voice_override = optional(inputs.npc_voice)?
        .filter(|value| !value.is_empty())
        .map(|value| checked_voice_identifier(value, "DF_GAMEPLAY_VOICE_NPC"))
        .transpose()?;
    let casting_path = optional(inputs.casting)?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_VOICE_CASTING.to_owned());
    if disabled {
        return Ok(VoiceSettings::Off(VoiceOff::Disabled));
    }
    let Some(credential) = optional(inputs.credential)?.filter(|value| !value.is_empty()) else {
        return Ok(VoiceSettings::Off(VoiceOff::NoCredential));
    };
    Ok(VoiceSettings::On(VoiceConfiguration {
        credential: VoiceCredential(credential),
        character_budget,
        cache_dir: cache_dir.into(),
        narrator_voice,
        npc_voice_override,
        casting_path: casting_path.into(),
    }))
}

/// An ElevenLabs voice id: letters, digits, `-` and `_` only, so it is always one safe
/// URL path segment.
pub(super) fn is_voice_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn checked_voice_identifier(value: String, name: &str) -> Result<String, io::Error> {
    if is_voice_identifier(&value) {
        Ok(value)
    } else {
        Err(refused(name))
    }
}

fn voice_identifier(
    input: Result<String, VarError>,
    default: &str,
    name: &str,
) -> Result<String, io::Error> {
    match optional(input)?.filter(|value| !value.is_empty()) {
        Some(value) => checked_voice_identifier(value, name),
        None => Ok(default.to_owned()),
    }
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

/// `DF_GAMEPLAY_PUBLIC_URL`: the address players' devices use, written exactly as a browser
/// origin (`http` or `https`, a host that is not a wildcard address, an optional port) with no
/// path, query, fragment or user information. Returns the origin and whether its host is
/// loopback. The refusal never echoes the value.
fn public_origin(value: &str) -> Result<(String, bool), io::Error> {
    let refused_public = || {
        io::Error::other(
            "DF_GAMEPLAY_PUBLIC_URL refused: expected http:// or https:// with a host and an optional port, and nothing after the port",
        )
    };
    let uri = axum::http::Uri::from_str(value).map_err(|_| refused_public())?;
    let scheme = match uri.scheme_str() {
        Some(scheme @ ("http" | "https")) => scheme,
        _ => return Err(refused_public()),
    };
    let authority = uri.authority().ok_or_else(refused_public)?;
    let host = uri.host().unwrap_or_default();
    let wildcard = unbracketed(host)
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_unspecified());
    // A port that is written must be digits naming 1 to 65535 (`Authority::port` hides overflow).
    let port_ok = match authority.as_str().strip_prefix(authority.host()) {
        Some("") => true,
        Some(rest) => rest.strip_prefix(':').is_some_and(|digits| {
            digits.bytes().all(|byte| byte.is_ascii_digit())
                && digits.parse::<u16>().is_ok_and(|port| port != 0)
        }),
        None => false,
    };
    if format!("{scheme}://{authority}") != value
        || value.contains('@')
        || host.is_empty()
        || wildcard
        || !port_ok
    {
        return Err(refused_public());
    }
    Ok((value.to_owned(), host_is_loopback(host)))
}

fn unbracketed(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host)
}

fn host_is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || unbracketed(host)
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
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
    pub(super) combat: Option<bytes::Bytes>,
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
    let combat = match super::bounded_asset(
        asset_root.join("../concept-art/combat-harbor-docks-wide-lanterns.webp"),
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
        combat,
    })
}

/// Fixed names of the optional generated interface art. A file is looked up as
/// `assets/ui/generated/<stem>.<webp|png|jpg>` and served at `/assets/ui/generated/<stem>`
/// (no extension), so the artwork format can change without touching the browser client.
const GENERATED_STEMS: &[&str] = &[
    "wordmark",
    "panel-frame",
    "button-frame",
    "divider",
    "ribbon-success",
    "ribbon-failure",
    "d20",
    "icon-sword",
    "icon-shield",
    "icon-heart",
    "icon-lantern",
    "icon-speech",
    "icon-compass",
    "icon-bag",
    "icon-book",
    "portrait-hero-1",
    "portrait-hero-2",
    "portrait-hero-3",
    "portrait-bandit",
    "portrait-courier",
    "scene-lobby-harbor",
    "scene-wharf-arrival",
    "scene-whisper-in-the-rain",
    "scene-courier-road",
    "scene-harbor-inn",
    "scene-salt-quay-arrival-photo",
    "figure-hero-1-photo",
    "figure-hero-2-photo",
    "figure-hero-3-photo",
    "sprite-courier-photo",
    "sprite-bandit-photo",
    "sprite-hero-1-photo",
    "sprite-hero-2-photo",
    "sprite-hero-3-photo",
    "icon-attack",
    "icon-second-wind",
    "icon-end-turn",
    "icon-short-rest",
    "icon-journey",
    "icon-talk",
    "icon-investigate",
    "icon-character",
    "icon-journal",
    "icon-map",
    "icon-menu",
    "icon-dice",
    "icon-players",
    "portrait-hero-1-full",
    "portrait-hero-2-full",
    "portrait-hero-3-full",
    "portrait-bandit-full",
    "portrait-courier-full",
];
/// Art for the 3D battlefield: one painted top-down ground per generated location, and the
/// full-body cut-out figures. A missing file is simply not served and the display falls back.
const BATTLEFIELD_STEMS: &[&str] = &[
    "ground-salt-quay",
    "ground-pinewood-trail",
    "ground-windward-pass",
    "ground-lamplit-market",
    "ground-rookhill-gate",
    "ground-reedmoor-boardwalk",
    "ground-thistledown-farm",
    "ground-orrery-library",
    "figure-hero-1",
    "figure-hero-2",
    "figure-hero-3",
    "figure-hero-1-photo",
    "figure-hero-2-photo",
    "figure-hero-3-photo",
    "sprite-courier-photo",
    "sprite-bandit-photo",
    "sprite-hero-1-photo",
    "sprite-hero-2-photo",
    "sprite-hero-3-photo",
    "prop-crates",
    "prop-barrels",
    "prop-rope-nets",
    "prop-boulder",
    "prop-fallen-log",
    "prop-tree-stump",
    "prop-gravestone",
    "prop-stone-pillar",
    "prop-cart",
    "prop-hay-bales",
    "prop-well",
    "prop-market-stall",
    "prop-bookshelf",
    "prop-brazier",
    "prop-fence",
    "prop-snow-rocks",
    "status-blinded",
    "status-charmed",
    "status-deafened",
    "status-exhaustion",
    "status-frightened",
    "status-grappled",
    "status-incapacitated",
    "status-invisible",
    "status-paralyzed",
    "status-petrified",
    "status-poisoned",
    "status-prone",
    "status-restrained",
    "status-stunned",
    "status-unconscious",
    "status-bloodied",
    "status-burning",
    "status-blessed",
    "status-cursed",
    "status-concentrating",
    "status-dodging",
    "status-hidden",
    "status-inspired",
    "status-raging",
    "status-marked",
    "status-dying",
    "status-cover",
    "figure-bandit",
    "figure-ant-bandit",
    "figure-ant-cultist",
    "figure-ant-giant-rat",
    "figure-ant-giant-weasel",
    "figure-ant-goblin-warrior",
    "figure-ant-guard",
    "figure-ant-kobold-warrior",
    "figure-ant-mastiff",
    "figure-ant-scout",
    "figure-ant-skeleton",
    "figure-ant-wolf",
    "figure-ant-zombie",
];
/// Fixed names of the display's sound effects, looked up as
/// `assets/ui/generated/sfx/<name>.mp3` and served at `/assets/ui/generated/sfx/<name>`.
/// A missing file is not served, which the display treats as silence.
const SFX_NAMES: &[&str] = &[
    "dice-rattle",
    "dice-land",
    "hit",
    "miss",
    "critical",
    "turn-chime",
    "hero-ready",
    "party-gathered",
    "scene-transition",
    "victory-fanfare",
    "defeat-toll",
    "bubble-pop",
    "heal-chime",
    "step-wood-1",
    "step-wood-2",
    "step-dirt-1",
    "step-dirt-2",
    "step-stone-1",
    "step-stone-2",
    "sword-swing-1",
    "sword-swing-2",
    "blade-hit-1",
    "blade-hit-2",
    "weapon-parry",
    "bow-draw",
    "bow-release",
    "arrow-flight",
    "arrow-impact",
    "arrow-miss",
    "voice-male-effort",
    "voice-male-effort-2",
    "voice-male-pain",
    "voice-male-down",
    "voice-female-effort",
    "voice-female-effort-2",
    "voice-female-pain",
    "voice-female-down",
    "voice-small-effort",
    "voice-small-pain",
    "voice-small-down",
    "voice-beast-effort",
    "voice-beast-pain",
    "voice-beast-down",
    "voice-undead-effort",
    "voice-undead-pain",
    "voice-undead-down",
    "voice-humanoid-effort",
    "voice-humanoid-pain",
    "voice-humanoid-down",
    "body-fall",
    "heal-breath",
];
const GENERATED_EXTENSIONS: [(&str, &str); 3] = [
    ("webp", "image/webp"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
];
/// Existing authored concept art the browser client references by its original path.
const CONCEPT_FILES: &[&str] = &[
    "dm-avatar.webp",
    "establishing-harbor-canal-rowboat-bridge.webp",
];
const MAXIMUM_EXTRA_ASSET_BYTES: u64 = 8 * 1024 * 1024;
const MAXIMUM_EXTRA_ASSET_TOTAL_BYTES: usize = 96 * 1024 * 1024;

/// Loads the allowlisted optional interface art under the same startup byte bounds as the
/// other public files. A missing file is simply not served; an oversized or unreadable one
/// refuses startup. Nothing outside the fixed name lists is ever read or routed.
pub(super) fn load_extra_assets(
    asset_root: &Path,
) -> Result<Vec<super::public_assets::ExtraAsset>, io::Error> {
    let mut loaded = Vec::new();
    let mut total = 0usize;
    let mut candidates = Vec::new();
    let named: Vec<&str> = GENERATED_STEMS
        .iter()
        .chain(BATTLEFIELD_STEMS)
        .copied()
        .chain(
            super::adventure::generated_stems()
                .iter()
                .map(String::as_str),
        )
        .collect();
    // Every portrait may have a transparent cut-out twin, named by the one shared rule.
    let sprites: Vec<String> = named
        .iter()
        .filter_map(|stem| crate::stage_art::sprite_stem(stem))
        .collect();
    // The 2:3 portrait is the display's fallback when a cut-out is absent.
    let fulls: Vec<String> = named
        .iter()
        .filter(|stem| stem.starts_with("portrait-") && !stem.ends_with("-full"))
        .map(|stem| format!("{stem}-full"))
        .collect();
    let stems = named
        .iter()
        .copied()
        .chain(sprites.iter().map(String::as_str))
        .chain(fulls.iter().map(String::as_str));
    for stem in stems {
        for (extension, mime) in GENERATED_EXTENSIONS {
            candidates.push((
                asset_root
                    .join("generated")
                    .join(format!("{stem}.{extension}")),
                format!("/assets/ui/generated/{stem}"),
                mime,
            ));
        }
    }
    for name in SFX_NAMES {
        candidates.push((
            asset_root.join("generated/sfx").join(format!("{name}.mp3")),
            format!("/assets/ui/generated/sfx/{name}"),
            "audio/mpeg",
        ));
    }
    for file in CONCEPT_FILES {
        candidates.push((
            asset_root.join("../concept-art").join(file),
            format!("/assets/concept-art/{file}"),
            "image/webp",
        ));
    }
    for (path, url, mime) in candidates {
        if loaded
            .iter()
            .any(|asset: &super::public_assets::ExtraAsset| asset.url == url)
        {
            continue;
        }
        match super::bounded_asset(path, MAXIMUM_EXTRA_ASSET_BYTES) {
            Ok(bytes) => {
                total += bytes.len();
                if total > MAXIMUM_EXTRA_ASSET_TOTAL_BYTES {
                    return Err(io::Error::other("owned asset total bound refused"));
                }
                loaded.push(super::public_assets::ExtraAsset { url, mime, bytes });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for asset in load_dice_assets(&dice_dir(std::env::var(DICE_DIR_ENVIRONMENT), asset_root)?)? {
        if loaded
            .iter()
            .any(|existing: &super::public_assets::ExtraAsset| existing.url == asset.url)
        {
            continue;
        }
        total += asset.bytes.len();
        if total > MAXIMUM_EXTRA_ASSET_TOTAL_BYTES {
            return Err(io::Error::other("owned asset total bound refused"));
        }
        loaded.push(asset);
    }
    Ok(loaded)
}

/// Dice-roll clips the display plays over a fight. The directory's own
/// `dice-rolls.json` manifest names exactly the clip files the game serves,
/// extensionless at `/assets/ui/generated/dice/<stem>` with the clip's media
/// type (a request *with* an extension matches no route, by the same rule that
/// hid the drawn die's image). Nothing the manifest does not name is read or
/// routed. This is the field contract a future clip delivery must keep: per
/// clip `webm`, `key_mp4` and `rest_image` file names, `rest_ms`, `face_center`,
/// `face_corners`, `face_rotation_deg`, and the top-level `key_color`.
/// A missing manifest serves nothing (the display falls back to the roll card);
/// an unreadable or invalid one refuses startup.
const DICE_MANIFEST_FILE: &str = "dice-rolls.json";
const DICE_URL_PREFIX: &str = "/assets/ui/generated/dice/";
/// Development override for the dice clip directory: placeholder clips live
/// outside `assets/` and must never be mistaken for delivered assets. Unset in
/// production, when clips load from `<asset_root>/generated/dice/`.
const DICE_DIR_ENVIRONMENT: &str = "DF_GAMEPLAY_DICE_DIR";
/// The manifest names clip files; only these media types are ever served.
const DICE_EXTENSIONS: [(&str, &str); 4] = [
    ("webm", "video/webm"),
    ("mp4", "video/mp4"),
    ("webp", "image/webp"),
    ("json", "application/json"),
];

fn dice_dir(input: Result<String, VarError>, asset_root: &Path) -> Result<PathBuf, io::Error> {
    match input {
        Ok(dir) if !dir.trim().is_empty() => Ok(PathBuf::from(dir)),
        Ok(_) | Err(VarError::NotPresent) => Ok(asset_root.join("generated/dice")),
        Err(_) => Err(refused(DICE_DIR_ENVIRONMENT)),
    }
}

pub(super) fn load_dice_assets(
    dice: &Path,
) -> Result<Vec<super::public_assets::ExtraAsset>, io::Error> {
    let invalid = || io::Error::other("dice manifest invalid");
    let manifest = match std::fs::read_to_string(dice.join(DICE_MANIFEST_FILE)) {
        Ok(manifest) => manifest,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let parsed: serde_json::Value = serde_json::from_str(&manifest).map_err(|_| invalid())?;
    let Some(clips) = parsed.get("clips").and_then(|clips| clips.as_array()) else {
        return Err(invalid());
    };
    let mut names = Vec::new();
    for clip in clips {
        for key in ["webm", "key_mp4", "rest_image"] {
            match clip.get(key).and_then(|name| name.as_str()) {
                Some(name) => names.push(name),
                None => return Err(invalid()),
            }
        }
    }
    names.push(DICE_MANIFEST_FILE);
    let mut loaded = Vec::new();
    for name in names {
        let file = Path::new(name);
        // The manifest names flat files only: no directories, no traversal.
        if file.file_name().and_then(|base| base.to_str()) != Some(name) {
            return Err(invalid());
        }
        let extension = file
            .extension()
            .and_then(|some| some.to_str())
            .unwrap_or("");
        let Some(mime) = DICE_EXTENSIONS
            .iter()
            .find(|(known, _)| *known == extension)
            .map(|(_, mime)| *mime)
        else {
            return Err(invalid());
        };
        let Some(stem) = file.file_stem().and_then(|some| some.to_str()) else {
            return Err(invalid());
        };
        match super::bounded_asset(dice.join(file), MAXIMUM_EXTRA_ASSET_BYTES) {
            Ok(bytes) => loaded.push(super::public_assets::ExtraAsset {
                url: format!("{DICE_URL_PREFIX}{stem}"),
                mime,
                bytes,
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    loaded.sort_by(|first, second| first.url.cmp(&second.url));
    Ok(loaded)
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
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(io::Error::other("local demonstration database invalid"));
    }
    let mut configuration = tokio_postgres::Config::new();
    configuration
        .host("127.0.0.1")
        .port(database_port()?)
        .user("df_gameplay_demo_admin_20261004")
        .dbname(&name);
    Ok(configuration)
}

/// The launcher's PostgreSQL port. PLAY-L01 harness knob: `DF_GAMEPLAY_DEMO_DATABASE_PORT` names a
/// private instance's port (evaluation runs must not share the launcher's); an invalid value is
/// refused without echoing it.
fn database_port() -> Result<u16, io::Error> {
    match std::env::var("DF_GAMEPLAY_DEMO_DATABASE_PORT") {
        Err(VarError::NotPresent) => Ok(55517),
        Ok(value) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port >= 1024)
            .ok_or_else(|| refused("DF_GAMEPLAY_DEMO_DATABASE_PORT")),
        Err(_) => Err(refused("DF_GAMEPLAY_DEMO_DATABASE_PORT")),
    }
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

/// `DF_GAMEPLAY_SPLAT_DIR`: where scan files are served from. Unset means `default`; a relative
/// path is taken from the current directory. The directory may be absent (nothing is routed), but
/// the setting itself must be non-empty text without parent-directory steps.
pub(super) fn splat_directory(
    input: Result<String, VarError>,
    default: &Path,
    current: &Path,
) -> Result<std::path::PathBuf, io::Error> {
    let Some(value) = optional(input)? else {
        return Ok(default.to_owned());
    };
    let path = std::path::PathBuf::from(&value);
    if value.is_empty()
        || value.contains('\0')
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(refused("DF_GAMEPLAY_SPLAT_DIR"));
    }
    Ok(if path.is_absolute() {
        path
    } else {
        current.join(path)
    })
}

/// `DF_GAMEPLAY_BATTLE_SCENE`: the battle scene id used for every fight. Unset leaves the
/// per-location scenes; a set value must name a scene the display has built in (this is how a
/// private-test-only scan is selected: there is no other way to reach one in a live game).
pub(super) fn battle_scene(input: Result<String, VarError>) -> Result<Option<String>, io::Error> {
    let Some(value) = optional(input)? else {
        return Ok(None);
    };
    if value.len() > 48 || !df_render::battle::embedded_ids().any(|known| known == value) {
        return Err(refused("DF_GAMEPLAY_BATTLE_SCENE"));
    }
    Ok(Some(value))
}

/// `DF_GAMEPLAY_TEST_FIGHT=<n>` and `DF_GAMEPLAY_TEST_FOE=<creature id>`: a development setting
/// that starts every fight against `n` enemies, `n` from 1 to the most a fight holds, of the
/// named creature (the adventure's own creature when unnamed). Unset, a game is unchanged. A
/// creature without a count, a count out of range and a creature the rules table lacks are
/// refused at startup rather than replaced.
pub(super) fn test_fight(
    count: Result<String, VarError>,
    foe: Result<String, VarError>,
) -> Result<Option<super::journey::tactical::TestFight>, io::Error> {
    let (count, foe) = (optional(count)?, optional(foe)?);
    let Some(count) = count else {
        return match foe {
            None => Ok(None),
            Some(_) => Err(io::Error::other(
                "DF_GAMEPLAY_TEST_FOE needs DF_GAMEPLAY_TEST_FIGHT",
            )),
        };
    };
    let most = super::journey::tactical::MAXIMUM_ENEMIES;
    let enemies = count
        .parse::<usize>()
        .ok()
        .filter(|enemies| (1..=most).contains(enemies))
        .ok_or_else(|| {
            io::Error::other(format!(
                "DF_GAMEPLAY_TEST_FIGHT invalid: use a whole number from 1 to {most}"
            ))
        })?;
    if let Some(foe) = &foe
        && df_rules::tactical::CombatantSpec::creature(3, foe, df_rules::tactical::Cell(0, 0), 0)
            .is_err()
    {
        return Err(io::Error::other(
            "DF_GAMEPLAY_TEST_FOE invalid: use a creature id of the rules table, for example kobold-warrior",
        ));
    }
    Ok(Some(super::journey::tactical::TestFight { enemies, foe }))
}

/// `DF_GAMEPLAY_BATTLE_FPS`: the battlefield's fixed presentation rate, 24 or 30 (default 30).
/// Anything else is refused at startup rather than silently replaced.
pub(super) fn battle_fps(input: Result<String, VarError>) -> Result<u32, io::Error> {
    let Some(value) = optional(input)? else {
        return Ok(df_render::battle::BattleFps::DEFAULT.hz());
    };
    value
        .parse::<u32>()
        .ok()
        .and_then(df_render::battle::BattleFps::from_hz)
        .map(df_render::battle::BattleFps::hz)
        .ok_or_else(|| {
            io::Error::other("DF_GAMEPLAY_BATTLE_FPS invalid: use 24 or 30 (default 30)")
        })
}

/// The largest `DF_GAMEPLAY_SPLAT_LOD` accepted.
const MAXIMUM_SPLAT_LOD: u32 = 3;
/// The default tier of a streamed scan: 1, the next tier down from the largest scene-authored tier.
const DEFAULT_SPLAT_LOD: u32 = 1;

/// `DF_GAMEPLAY_SPLAT_LOD`: which tier of a streamed scan the battlefield draws as its finest, 0 to
/// 3 (default 1). 0 is the largest tier the scene authors for the scan, 1 the next one down. Anything
/// else is refused at startup rather than silently replaced.
pub(super) fn splat_lod(input: Result<String, VarError>) -> Result<u32, io::Error> {
    let Some(value) = optional(input)? else {
        return Ok(DEFAULT_SPLAT_LOD);
    };
    value
        .parse::<u32>()
        .ok()
        .filter(|level| *level <= MAXIMUM_SPLAT_LOD && value == level.to_string())
        .ok_or_else(|| {
            io::Error::other("DF_GAMEPLAY_SPLAT_LOD invalid: use 0, 1, 2 or 3 (default 1)")
        })
}

/// `DF_GAMEPLAY_BATTLE_GRID`: whether the battlefield on the TV draws its cell grid (hairlines, the
/// reachable region and the legal targets) at all: `on` or `off`, default off. The table reads the
/// fight from rings under the acting figure and the target, the figures walking their paths and the
/// contact shadows; the grid itself is a development view. Anything else is refused at startup.
pub(super) fn battle_grid(input: Result<String, VarError>) -> Result<bool, io::Error> {
    match optional(input)?.as_deref() {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(_) => Err(io::Error::other(
            "DF_GAMEPLAY_BATTLE_GRID invalid: use on or off (default off)",
        )),
    }
}

const DEFAULT_TEXT_MODEL: &str = "gpt-6-luna";
const DEFAULT_TEXT_TOKEN_BUDGET: u64 = MAXIMUM_TEXT_TOKEN_BUDGET;
const MAXIMUM_TEXT_TOKEN_BUDGET: u64 = 100_000_000;
const DEFAULT_TEXT_DEADLINE_SECONDS: u64 = MAXIMUM_TEXT_DEADLINE_SECONDS;
const MAXIMUM_TEXT_DEADLINE_SECONDS: u64 = 60;
const DEFAULT_TEXT_CACHE: &str = "artifacts/tmp/play/text-cache";

/// Raw storyteller inputs. The credential is read from the process environment only.
pub(super) struct StorytellerInputs {
    pub(super) credential: Result<String, VarError>,
    pub(super) mode: Result<String, VarError>,
    pub(super) model: Result<String, VarError>,
    pub(super) token_budget: Result<String, VarError>,
    pub(super) cache: Result<String, VarError>,
    pub(super) deadline_seconds: Result<String, VarError>,
}

/// Jev's optional startup credential and endpoint; no process environment is read per line.
pub(super) struct ApproachSettings {
    pub(super) credential: String,
    pub(super) base_url: String,
}

pub(super) fn approach_settings(
    credential: Result<String, VarError>,
    base_url: Result<String, VarError>,
) -> Result<Option<ApproachSettings>, io::Error> {
    let Some(credential) = optional(credential)? else {
        return Ok(None);
    };
    if credential.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(ApproachSettings {
        credential,
        base_url: optional(base_url)?.unwrap_or_else(|| "https://api.typesafe.ai".to_owned()),
    }))
}

impl StorytellerInputs {
    pub(super) fn from_environment() -> Self {
        Self {
            credential: std::env::var("OPENAI_API_KEY"),
            mode: std::env::var("DF_GAMEPLAY_STORYTELLER"),
            model: std::env::var("DF_GAMEPLAY_TEXT_MODEL"),
            token_budget: std::env::var("DF_GAMEPLAY_TEXT_TOKEN_BUDGET"),
            cache: std::env::var("DF_GAMEPLAY_TEXT_CACHE"),
            deadline_seconds: std::env::var("DF_GAMEPLAY_TEXT_DEADLINE_SECONDS"),
        }
    }
}

/// Everything the storyteller needs to call the text model. Debug never shows the credential.
pub(super) struct StorytellerSettings {
    pub(super) credential: String,
    pub(super) model: String,
    /// Total tokens the server may spend on text generation during one run.
    pub(super) token_budget: u64,
    pub(super) cache_directory: std::path::PathBuf,
    /// Covers every attempt of one expression, including the single retry.
    pub(super) deadline: Duration,
}

impl std::fmt::Debug for StorytellerSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorytellerSettings")
            .finish_non_exhaustive()
    }
}

/// Whether the language model voices the story. `Off` carries the reason shown at startup.
#[derive(Debug)]
pub(super) enum StorytellerMode {
    On(StorytellerSettings),
    Off(&'static str),
}

/// A missing, empty or malformed credential, or `DF_GAMEPLAY_STORYTELLER=off`, turns the
/// storyteller off; a malformed tuning value is refused without echoing it.
pub(super) fn storyteller_mode(
    inputs: StorytellerInputs,
    current: &Path,
) -> Result<StorytellerMode, io::Error> {
    if optional(inputs.mode)?.is_some_and(|mode| mode.trim().eq_ignore_ascii_case("off")) {
        return Ok(StorytellerMode::Off("DF_GAMEPLAY_STORYTELLER=off"));
    }
    let credential = match optional(inputs.credential)? {
        None => return Ok(StorytellerMode::Off("OPENAI_API_KEY not set")),
        Some(value) if value.is_empty() => {
            return Ok(StorytellerMode::Off("OPENAI_API_KEY empty"));
        }
        Some(value)
            if value.len() > 8192 || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) =>
        {
            return Ok(StorytellerMode::Off("OPENAI_API_KEY not usable"));
        }
        Some(value) => value,
    };
    let model = optional(inputs.model)?.unwrap_or_else(|| DEFAULT_TEXT_MODEL.to_owned());
    if model.is_empty()
        || model.len() > 128
        || !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(refused("DF_GAMEPLAY_TEXT_MODEL"));
    }
    let token_budget = match optional(inputs.token_budget)? {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|tokens| (1..=MAXIMUM_TEXT_TOKEN_BUDGET).contains(tokens))
            .ok_or_else(|| refused("DF_GAMEPLAY_TEXT_TOKEN_BUDGET"))?,
        None => DEFAULT_TEXT_TOKEN_BUDGET,
    };
    let deadline_seconds = match optional(inputs.deadline_seconds)? {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=MAXIMUM_TEXT_DEADLINE_SECONDS).contains(seconds))
            .ok_or_else(|| refused("DF_GAMEPLAY_TEXT_DEADLINE_SECONDS"))?,
        None => DEFAULT_TEXT_DEADLINE_SECONDS,
    };
    let cache = optional(inputs.cache)?.unwrap_or_else(|| DEFAULT_TEXT_CACHE.to_owned());
    if cache.is_empty() {
        return Err(refused("DF_GAMEPLAY_TEXT_CACHE"));
    }
    let cache = std::path::PathBuf::from(cache);
    Ok(StorytellerMode::On(StorytellerSettings {
        credential,
        model,
        token_budget,
        cache_directory: if cache.is_absolute() {
            cache
        } else {
            current.join(cache)
        },
        deadline: Duration::from_secs(deadline_seconds),
    }))
}

// ---- PLAY-L01: runtime image generation settings (the content rating is the shared one above) ----

/// Raw `DF_GAMEPLAY_IMAGERY*` inputs.
pub(super) struct ImageryInputs {
    pub(super) credential: Result<String, VarError>,
    pub(super) mode: Result<String, VarError>,
    pub(super) model: Result<String, VarError>,
    pub(super) quality: Result<String, VarError>,
    pub(super) budget: Result<String, VarError>,
    pub(super) interval_seconds: Result<String, VarError>,
    pub(super) deadline_seconds: Result<String, VarError>,
    pub(super) cache: Result<String, VarError>,
}

impl ImageryInputs {
    pub(super) fn from_environment() -> Self {
        Self {
            credential: std::env::var("OPENAI_API_KEY"),
            mode: std::env::var("DF_GAMEPLAY_IMAGERY"),
            model: std::env::var("DF_GAMEPLAY_IMAGERY_MODEL"),
            quality: std::env::var("DF_GAMEPLAY_IMAGERY_QUALITY"),
            budget: std::env::var("DF_GAMEPLAY_IMAGERY_BUDGET"),
            interval_seconds: std::env::var("DF_GAMEPLAY_IMAGERY_INTERVAL_SECONDS"),
            deadline_seconds: std::env::var("DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS"),
            cache: std::env::var("DF_GAMEPLAY_IMAGERY_CACHE"),
        }
    }
}

const DEFAULT_IMAGERY_BUDGET: u32 = MAXIMUM_IMAGERY_BUDGET;
const MAXIMUM_IMAGERY_BUDGET: u32 = 10_000;
/// Scene refreshes are at least this far apart: a repaint every few seconds would cost more
/// than it adds and would never settle.
pub(super) const MINIMUM_IMAGERY_INTERVAL_SECONDS: u64 = 20;
const DEFAULT_IMAGERY_INTERVAL_SECONDS: u64 = MINIMUM_IMAGERY_INTERVAL_SECONDS;
const MAXIMUM_IMAGERY_INTERVAL_SECONDS: u64 = 3_600;
const DEFAULT_IMAGERY_DEADLINE_SECONDS: u64 = MAXIMUM_IMAGERY_DEADLINE_SECONDS;
const MAXIMUM_IMAGERY_DEADLINE_SECONDS: u64 = 170;
const DEFAULT_IMAGERY_MODEL: &str = "gpt-image-2.5-flare";
const DEFAULT_IMAGERY_CACHE: &str = "artifacts/tmp/play/imagery-cache";

/// The OpenAI credential used for image edits; it never appears in `Debug` output.
pub(super) struct ImageryCredential(String);

impl ImageryCredential {
    pub(super) fn expose(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(super) fn for_test(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Debug for ImageryCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ImageryCredential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageryOff {
    Qualification,
    Disabled,
    NoCredential,
}

impl ImageryOff {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::Qualification => "qualification run",
            Self::Disabled => "DF_GAMEPLAY_IMAGERY=off",
            Self::NoCredential => "OPENAI_API_KEY not set",
        }
    }
}

#[derive(Debug)]
pub(super) struct ImageryConfiguration {
    pub(super) credential: ImageryCredential,
    pub(super) model: df_providers::ImageEditModel,
    pub(super) quality: df_providers::ImageQuality,
    /// Provider calls (images) per server run, charged before each call.
    pub(super) budget: u32,
    /// The least time between two scene refreshes.
    pub(super) interval: Duration,
    /// How long one provider call may take.
    pub(super) deadline: Duration,
    pub(super) cache_dir: std::path::PathBuf,
    pub(super) rating: ContentRating,
}

#[derive(Debug)]
pub(super) enum ImagerySettings {
    Off(ImageryOff),
    On(ImageryConfiguration),
}

fn bounded_seconds(
    input: Result<String, VarError>,
    default: u64,
    range: std::ops::RangeInclusive<u64>,
    name: &str,
) -> Result<u64, io::Error> {
    match optional(input)?.filter(|value| !value.is_empty()) {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|seconds| range.contains(seconds))
            .ok_or_else(|| refused(name)),
        None => Ok(default),
    }
}

/// Imagery is on when a credential is present and `DF_GAMEPLAY_IMAGERY` is not `off`. Invalid
/// values are refused without echoing them; empty values count as unset. `rating` is the
/// server's already-installed content rating.
pub(super) fn imagery_settings(
    inputs: ImageryInputs,
    current: &Path,
    rating: ContentRating,
) -> Result<ImagerySettings, io::Error> {
    let disabled = match optional(inputs.mode)?.as_deref() {
        None | Some("" | "on") => false,
        Some("off") => true,
        Some(_) => return Err(refused("DF_GAMEPLAY_IMAGERY")),
    };
    let model = df_providers::ImageEditModel::from_model_id(
        optional(inputs.model)?
            .filter(|value| !value.is_empty())
            .as_deref()
            .unwrap_or(DEFAULT_IMAGERY_MODEL),
    )
    .ok_or_else(|| refused("DF_GAMEPLAY_IMAGERY_MODEL"))?;
    let quality = match optional(inputs.quality)?.as_deref() {
        None | Some("" | "medium") => df_providers::ImageQuality::Medium,
        Some("low") => df_providers::ImageQuality::Low,
        Some("high") => df_providers::ImageQuality::High,
        Some(_) => return Err(refused("DF_GAMEPLAY_IMAGERY_QUALITY")),
    };
    let budget = match optional(inputs.budget)?.filter(|value| !value.is_empty()) {
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|budget| *budget <= MAXIMUM_IMAGERY_BUDGET)
            .ok_or_else(|| refused("DF_GAMEPLAY_IMAGERY_BUDGET"))?,
        None => DEFAULT_IMAGERY_BUDGET,
    };
    let interval = bounded_seconds(
        inputs.interval_seconds,
        DEFAULT_IMAGERY_INTERVAL_SECONDS,
        MINIMUM_IMAGERY_INTERVAL_SECONDS..=MAXIMUM_IMAGERY_INTERVAL_SECONDS,
        "DF_GAMEPLAY_IMAGERY_INTERVAL_SECONDS",
    )?;
    let deadline = bounded_seconds(
        inputs.deadline_seconds,
        DEFAULT_IMAGERY_DEADLINE_SECONDS,
        5..=MAXIMUM_IMAGERY_DEADLINE_SECONDS,
        "DF_GAMEPLAY_IMAGERY_DEADLINE_SECONDS",
    )?;
    let cache = optional(inputs.cache)?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_IMAGERY_CACHE.to_owned());
    if disabled {
        return Ok(ImagerySettings::Off(ImageryOff::Disabled));
    }
    let Some(credential) = optional(inputs.credential)?
        .filter(|value| !value.is_empty())
        .filter(|value| {
            value.len() <= 8192 && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        })
    else {
        return Ok(ImagerySettings::Off(ImageryOff::NoCredential));
    };
    let cache = std::path::PathBuf::from(cache);
    Ok(ImagerySettings::On(ImageryConfiguration {
        credential: ImageryCredential(credential),
        model,
        quality,
        budget,
        interval: Duration::from_secs(interval),
        deadline: Duration::from_secs(deadline),
        cache_dir: if cache.is_absolute() {
            cache
        } else {
            current.join(cache)
        },
        rating,
    }))
}

/// PLAY-V09: whether living scenes make a composed scene plate for every story scene. On unless
/// `DF_GAMEPLAY_PLATES=off`; plates also need imagery on (a credential present). Invalid values
/// are refused without echoing them; an empty value counts as unset.
pub(super) fn plates_enabled(input: Result<String, VarError>) -> Result<bool, io::Error> {
    match optional(input)?.as_deref() {
        None | Some("" | "on") => Ok(true),
        Some("off") => Ok(false),
        Some(_) => Err(refused("DF_GAMEPLAY_PLATES")),
    }
}

/// `DF_GAMEPLAY_CHOICE_HINTS=on|off`: the table's startup choice of whether the numbered choice
/// chips show on the shared display (`on`, the default, or `off`). A stored choice made by a
/// player overrides it. Returns whether the chips start hidden; anything else is refused.
pub(super) fn choice_hints_hidden(input: Result<String, VarError>) -> Result<bool, io::Error> {
    match optional(input)?.as_deref() {
        None | Some("on") => Ok(false),
        Some("off") => Ok(true),
        Some(_) => Err(refused("DF_GAMEPLAY_CHOICE_HINTS")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_assets_serve_listed_sound_effects_and_sprite_twins_only() {
        let root = std::env::temp_dir().join(format!("df-extra-assets-{}", std::process::id()));
        let generated = root.join("generated");
        std::fs::create_dir_all(generated.join("sfx")).unwrap();
        std::fs::write(generated.join("sfx/hit.mp3"), b"hit").unwrap();
        std::fs::write(generated.join("sfx/unlisted.mp3"), b"no").unwrap();
        std::fs::write(generated.join("sprite-hero-1.png"), b"sprite").unwrap();
        std::fs::write(generated.join("sprite-unknown.png"), b"no").unwrap();
        std::fs::write(generated.join("portrait-courier-full.png"), b"full").unwrap();
        let loaded = load_extra_assets(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let served: Vec<(&str, &str)> = loaded
            .iter()
            .map(|asset| (asset.url.as_str(), asset.mime))
            .collect();
        assert_eq!(
            served,
            [
                ("/assets/ui/generated/portrait-courier-full", "image/png"),
                ("/assets/ui/generated/sprite-hero-1", "image/png"),
                ("/assets/ui/generated/sfx/hit", "audio/mpeg"),
            ]
        );
    }

    #[test]
    fn dice_assets_serve_exactly_what_the_manifest_names() {
        let root = std::env::temp_dir().join(format!("df-dice-assets-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("dice-rolls.json"),
            r##"{"key_color":"#ff00ff","clips":[{"id":1,"webm":"roll-1.webm","key_mp4":"roll-1-key.mp4","rest_image":"roll-1-rest.webp","rest_ms":1200,"face_center":[0.5,0.5],"face_corners":[[0.4,0.4],[0.6,0.4],[0.5,0.6]],"face_rotation_deg":0.0}]}"##,
        )
        .unwrap();
        std::fs::write(root.join("roll-1.webm"), b"webm").unwrap();
        std::fs::write(root.join("roll-1-key.mp4"), b"mp4").unwrap();
        std::fs::write(root.join("roll-1-rest.webp"), b"webp").unwrap();
        std::fs::write(root.join("unlisted.webm"), b"no").unwrap();
        let loaded = load_dice_assets(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let served: Vec<(&str, &str)> = loaded
            .iter()
            .map(|asset| (asset.url.as_str(), asset.mime))
            .collect();
        assert_eq!(
            served,
            [
                ("/assets/ui/generated/dice/dice-rolls", "application/json"),
                ("/assets/ui/generated/dice/roll-1", "video/webm"),
                ("/assets/ui/generated/dice/roll-1-key", "video/mp4"),
                ("/assets/ui/generated/dice/roll-1-rest", "image/webp"),
            ]
        );
        // A missing manifest serves nothing; an invalid one refuses startup.
        let missing = std::env::temp_dir().join(format!("df-dice-missing-{}", std::process::id()));
        std::fs::create_dir_all(&missing).unwrap();
        assert!(load_dice_assets(&missing).unwrap().is_empty());
        std::fs::write(missing.join("dice-rolls.json"), b"{nope").unwrap();
        assert!(load_dice_assets(&missing).is_err());
        std::fs::remove_dir_all(&missing).unwrap();
    }

    #[test]
    fn dice_directory_defaults_when_unset_and_refuses_non_unicode_values() {
        let root = Path::new("/srv/assets");
        assert_eq!(
            dice_dir(Err(VarError::NotPresent), root).unwrap(),
            root.join("generated/dice")
        );
        let invalid = VarError::NotUnicode(std::ffi::OsString::from("invalid"));
        assert_eq!(
            dice_dir(Err(invalid), root).unwrap_err().to_string(),
            "DF_GAMEPLAY_DICE_DIR invalid"
        );
    }

    #[test]
    fn choice_hints_default_to_shown_and_accept_only_on_or_off() {
        let value = |text: &str| Ok::<_, VarError>(text.to_owned());
        assert!(!choice_hints_hidden(Err(VarError::NotPresent)).unwrap());
        assert!(!choice_hints_hidden(value("on")).unwrap());
        assert!(choice_hints_hidden(value("off")).unwrap());
        for refused_value in ["", "true", "OFF", "hidden"] {
            let error = choice_hints_hidden(value(refused_value)).unwrap_err();
            assert_eq!(
                error.to_string(),
                "DF_GAMEPLAY_CHOICE_HINTS invalid",
                "{refused_value:?} is refused without being echoed"
            );
        }
    }

    fn storyteller_inputs(values: [Option<&str>; 6]) -> StorytellerInputs {
        let [credential, mode, model, budget, cache, deadline] =
            values.map(|value| value.map(str::to_owned).ok_or(VarError::NotPresent));
        StorytellerInputs {
            credential,
            mode,
            model,
            token_budget: budget,
            cache,
            deadline_seconds: deadline,
        }
    }

    #[test]
    fn storyteller_is_off_without_a_credential_or_when_switched_off() {
        let current = Path::new("/work");
        let missing = storyteller_mode(storyteller_inputs([None; 6]), current).unwrap();
        assert!(matches!(
            missing,
            StorytellerMode::Off("OPENAI_API_KEY not set")
        ));
        let empty = storyteller_mode(
            storyteller_inputs([Some(""), None, None, None, None, None]),
            current,
        )
        .unwrap();
        assert!(matches!(
            empty,
            StorytellerMode::Off("OPENAI_API_KEY empty")
        ));
        for mode in ["off", " OFF "] {
            let off = storyteller_mode(
                storyteller_inputs([Some("sk-test"), Some(mode), None, None, None, None]),
                current,
            )
            .unwrap();
            assert!(matches!(
                off,
                StorytellerMode::Off("DF_GAMEPLAY_STORYTELLER=off")
            ));
        }
    }

    #[test]
    fn storyteller_defaults_and_overrides_are_applied() {
        let current = Path::new("/work");
        let StorytellerMode::On(defaults) = storyteller_mode(
            storyteller_inputs([Some("sk-test"), None, None, None, None, None]),
            current,
        )
        .unwrap() else {
            panic!("storyteller should be on");
        };
        assert_eq!(defaults.model, "gpt-6-luna");
        // No default spend or time limit (owner, 2026-10-15): the defaults are the maximums.
        assert_eq!(defaults.token_budget, MAXIMUM_TEXT_TOKEN_BUDGET);
        assert_eq!(
            defaults.deadline,
            Duration::from_secs(MAXIMUM_TEXT_DEADLINE_SECONDS)
        );
        assert_eq!(
            defaults.cache_directory,
            Path::new("/work/artifacts/tmp/play/text-cache")
        );
        let StorytellerMode::On(custom) = storyteller_mode(
            storyteller_inputs([
                Some("sk-test"),
                Some("on"),
                Some("other-model"),
                Some("20000"),
                Some("/tmp/cache"),
                Some("5"),
            ]),
            current,
        )
        .unwrap() else {
            panic!("storyteller should be on");
        };
        assert_eq!(custom.model, "other-model");
        assert_eq!(custom.token_budget, 20_000);
        assert_eq!(custom.deadline, Duration::from_secs(5));
        assert_eq!(custom.cache_directory, Path::new("/tmp/cache"));
        assert!(!format!("{custom:?}").contains("sk-test"));
    }

    #[test]
    fn invalid_storyteller_values_are_refused_without_echoing_them() {
        let current = Path::new("/work");
        let cases: [[Option<&str>; 6]; 6] = [
            [Some("sk-test"), None, Some("bad model!"), None, None, None],
            [Some("sk-test"), None, None, Some("0"), None, None],
            [Some("sk-test"), None, None, Some("many"), None, None],
            [Some("sk-test"), None, None, None, Some(""), None],
            [Some("sk-test"), None, None, None, None, Some("0")],
            [Some("sk-test"), None, None, None, None, Some("61")],
        ];
        for case in cases {
            let error = storyteller_mode(storyteller_inputs(case), current)
                .err()
                .unwrap();
            for value in case.into_iter().flatten().filter(|value| !value.is_empty()) {
                assert!(!error.to_string().contains(value));
            }
        }
    }

    #[test]
    fn an_unset_content_rating_is_mature_and_exactly_two_values_are_accepted() {
        assert_eq!(
            content_rating_setting(Err(VarError::NotPresent)).unwrap(),
            ContentRating::Mature
        );
        assert_eq!(
            content_rating_setting(Ok("mature".to_owned())).unwrap(),
            ContentRating::Mature
        );
        assert_eq!(
            content_rating_setting(Ok("restrained".to_owned())).unwrap(),
            ContentRating::Restrained
        );
        assert_eq!(ContentRating::default(), ContentRating::Mature);
    }

    #[test]
    fn any_other_content_rating_refuses_startup_without_echoing_it() {
        for value in [
            "",
            "Mature",
            "RESTRAINED",
            " mature",
            "family",
            "off",
            "mature ",
        ] {
            let error = content_rating_setting(Ok(value.to_owned())).unwrap_err();
            assert_eq!(error.to_string(), "DF_GAMEPLAY_CONTENT_RATING invalid");
            assert!(value.is_empty() || !error.to_string().contains(value));
        }
        assert!(content_rating_setting(Err(VarError::NotUnicode("x".into()))).is_err());
    }

    #[test]
    fn the_startup_line_names_the_rating() {
        assert_eq!(
            ContentRating::Mature.startup_line(),
            "CONTENT rating=mature"
        );
        assert_eq!(
            ContentRating::Restrained.startup_line(),
            "CONTENT rating=restrained"
        );
    }

    #[test]
    fn a_test_rating_is_per_thread_nests_and_never_leaks() {
        assert_eq!(content_rating(), ContentRating::Mature);
        with_test_rating(ContentRating::Restrained, || {
            assert_eq!(content_rating(), ContentRating::Restrained);
            with_test_rating(ContentRating::Mature, || {
                assert_eq!(content_rating(), ContentRating::Mature);
            });
            assert_eq!(content_rating(), ContentRating::Restrained);
            let other = std::thread::spawn(content_rating).join().unwrap();
            assert_eq!(other, ContentRating::Mature, "another thread is unaffected");
        });
        assert_eq!(content_rating(), ContentRating::Mature);
    }

    fn inputs(values: [Option<&str>; 6]) -> SessionInputs {
        let [listen, origins, seconds, budget, connections, streams] =
            values.map(|value| value.map(str::to_owned).ok_or(VarError::NotPresent));
        SessionInputs {
            listen,
            allowed_origins: origins,
            public_url: Err(VarError::NotPresent),
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

    fn with_public(listen: Option<&str>, origins: Option<&str>, public: &str) -> SessionInputs {
        let mut value = inputs([listen, origins, None, None, None, None]);
        value.public_url = Ok(public.to_owned());
        value
    }

    #[test]
    fn valid_public_urls_are_accepted_and_refused_ones_are_named_without_their_value() {
        for public in [
            "http://192.168.0.24:63309",
            "http://192.168.0.24",
            "https://game.example.test",
            "http://[fd00::24]:63309",
            "http://localhost:63309",
        ] {
            let settings = session_settings(with_public(None, None, public)).unwrap();
            assert_eq!(settings.join_origin, public);
            assert_eq!(settings.allowed_origins[0], public);
        }
        for public in [
            "",
            "192.168.0.24:63309",
            "ftp://192.168.0.24:63309",
            "http://192.168.0.24:63309/",
            "http://192.168.0.24:63309/gameplay/player",
            "http://192.168.0.24:63309?room=LANTERN",
            "http://192.168.0.24:63309#top",
            "http://player@192.168.0.24:63309",
            "http://0.0.0.0:63309",
            "http://[::]:63309",
            "http://:63309",
            "http://192.168.0.24:0",
            "http://192.168.0.24:99999",
        ] {
            let error = session_settings(with_public(None, None, public))
                .err()
                .unwrap_or_else(|| panic!("{public:?} should be refused"));
            assert!(
                error
                    .to_string()
                    .starts_with("DF_GAMEPLAY_PUBLIC_URL refused")
            );
            if !public.is_empty() {
                assert!(!error.to_string().contains(public));
            }
        }
    }

    #[test]
    fn a_public_url_is_the_join_base_and_the_first_allowed_origin() {
        let settings = session_settings(with_public(
            Some("0.0.0.0:63309"),
            Some("http://192.168.0.9:63309,http://192.168.0.24:63309"),
            "http://192.168.0.24:63309",
        ))
        .unwrap();
        assert_eq!(settings.join_origin, "http://192.168.0.24:63309");
        assert!(!settings.join_loopback);
        assert_eq!(
            settings.allowed_origins,
            ["http://192.168.0.24:63309", "http://192.168.0.9:63309"]
        );
        // The origin check compares the exact string, so the phone's origin is accepted and an
        // unrelated one is not.
        let allowed = |origin: &str| settings.allowed_origins.iter().any(|known| known == origin);
        assert!(allowed("http://192.168.0.24:63309"));
        assert!(!allowed("http://192.168.0.99:63309"));
        assert!(!allowed("http://192.168.0.24:63310"));

        let lan_only = session_settings(with_public(
            Some("0.0.0.0:63309"),
            None,
            "http://192.168.0.24:63309",
        ))
        .unwrap();
        assert_eq!(lan_only.allowed_origins, ["http://192.168.0.24:63309"]);

        let loopback = session_settings(with_public(None, None, "http://127.0.0.1:63309")).unwrap();
        assert!(loopback.join_loopback);
        assert_eq!(
            loopback.allowed_origins,
            ["http://127.0.0.1:63309", "http://localhost:63309"]
        );
    }

    #[test]
    fn without_a_public_url_the_join_base_is_the_listener_or_loopback_for_a_wildcard() {
        let loopback = session_settings(inputs([None; 6])).unwrap();
        assert_eq!(loopback.join_origin, "http://127.0.0.1:63309");
        assert!(loopback.join_loopback);

        let wildcard = session_settings(inputs([
            Some("0.0.0.0:63309"),
            Some("http://192.168.1.20:63309"),
            None,
            None,
            None,
            None,
        ]))
        .unwrap();
        assert_eq!(wildcard.join_origin, "http://127.0.0.1:63309");
        assert!(wildcard.join_loopback);

        let named = session_settings(inputs([
            Some("192.168.1.20:63309"),
            Some("http://192.168.1.20:63309"),
            None,
            None,
            None,
            None,
        ]))
        .unwrap();
        assert_eq!(named.join_origin, "http://192.168.1.20:63309");
        assert!(!named.join_loopback);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AdventureMode {
    Harbor,
    Random,
    Campaign,
}
pub(super) struct AdventureInputs {
    pub(super) mode: Result<String, VarError>,
    pub(super) seed: Result<String, VarError>,
    pub(super) creatures: Result<String, VarError>,
    pub(super) tables: Result<String, VarError>,
    pub(super) chapter_type: Result<String, VarError>,
}
impl AdventureInputs {
    pub(super) fn from_environment() -> Self {
        Self {
            mode: std::env::var("DF_GAMEPLAY_ADVENTURE"),
            seed: std::env::var("DF_GAMEPLAY_ADVENTURE_SEED"),
            creatures: std::env::var("DF_GAMEPLAY_ADVENTURE_CREATURES"),
            tables: std::env::var("DF_GAMEPLAY_ADVENTURE_TABLES"),
            chapter_type: std::env::var("DF_GAMEPLAY_CHAPTER_TYPE"),
        }
    }
}
pub(super) struct AdventureSettings {
    pub(super) mode: AdventureMode,
    pub(super) seed: Option<u64>,
    pub(super) tables: std::path::PathBuf,
    pub(super) rules_mode: super::adventure::RulesMode,
    /// A development override of the random adventure's chapter type (`mystery`, `parley`, ...).
    pub(super) chapter_type: Option<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AdventureSettingsError {
    pub(super) variable: &'static str,
    pub(super) expected: &'static str,
}
impl std::fmt::Display for AdventureSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} must be {}", self.variable, self.expected)
    }
}
impl std::error::Error for AdventureSettingsError {}
fn adventure_input(
    input: Result<String, VarError>,
    variable: &'static str,
) -> Result<Option<String>, AdventureSettingsError> {
    match input {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(_) => Err(AdventureSettingsError {
            variable,
            expected: "Unicode text",
        }),
    }
}
pub(super) fn adventure_seed(value: &str) -> Result<u64, AdventureSettingsError> {
    let (digits, radix) = match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => (hex, 16),
        None => (value, 10),
    };
    if digits.is_empty()
        || !digits.bytes().all(|byte| {
            if radix == 16 {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return Err(AdventureSettingsError {
            variable: "DF_GAMEPLAY_ADVENTURE_SEED",
            expected: "a hexadecimal or decimal u64",
        });
    }
    u64::from_str_radix(digits, radix).map_err(|_| AdventureSettingsError {
        variable: "DF_GAMEPLAY_ADVENTURE_SEED",
        expected: "a hexadecimal or decimal u64",
    })
}
pub(super) fn adventure_creature_mode(
    input: Result<String, VarError>,
) -> Result<super::adventure::RulesMode, AdventureSettingsError> {
    match adventure_input(input, "DF_GAMEPLAY_ADVENTURE_CREATURES")?.as_deref() {
        None | Some("standard") => Ok(super::adventure::RulesMode::Standard),
        Some("all") => Ok(super::adventure::RulesMode::CustomAllowed),
        _ => Err(AdventureSettingsError {
            variable: "DF_GAMEPLAY_ADVENTURE_CREATURES",
            expected: "standard or all",
        }),
    }
}
pub(super) fn adventure_settings(
    inputs: AdventureInputs,
    current: &Path,
) -> Result<AdventureSettings, AdventureSettingsError> {
    let mode = match adventure_input(inputs.mode, "DF_GAMEPLAY_ADVENTURE")?.as_deref() {
        None | Some("harbor") => AdventureMode::Harbor,
        Some("random") => AdventureMode::Random,
        Some("campaign") => AdventureMode::Campaign,
        _ => {
            return Err(AdventureSettingsError {
                variable: "DF_GAMEPLAY_ADVENTURE",
                expected: "harbor, random or campaign",
            });
        }
    };
    let seed = adventure_input(inputs.seed, "DF_GAMEPLAY_ADVENTURE_SEED")?
        .map(|seed| adventure_seed(&seed))
        .transpose()?;
    let rules_mode = adventure_creature_mode(inputs.creatures)?;
    let tables = adventure_input(inputs.tables, "DF_GAMEPLAY_ADVENTURE_TABLES")?
        .unwrap_or_else(|| "content/adventure-tables".into());
    if tables.is_empty() {
        return Err(AdventureSettingsError {
            variable: "DF_GAMEPLAY_ADVENTURE_TABLES",
            expected: "a nonempty directory path",
        });
    }
    let tables = std::path::PathBuf::from(tables);
    let chapter_type = adventure_input(inputs.chapter_type, "DF_GAMEPLAY_CHAPTER_TYPE")?;
    if let Some(kind) = &chapter_type {
        let shaped = !kind.is_empty()
            && kind.len() <= 64
            && kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !shaped || mode != AdventureMode::Random {
            return Err(AdventureSettingsError {
                variable: "DF_GAMEPLAY_CHAPTER_TYPE",
                expected: "a chapter type id such as mystery, with DF_GAMEPLAY_ADVENTURE=random",
            });
        }
    }
    Ok(AdventureSettings {
        mode,
        seed,
        rules_mode,
        tables: if tables.is_absolute() {
            tables
        } else {
            current.join(tables)
        },
        chapter_type,
    })
}
#[cfg(test)]
mod adventure_settings_tests {
    use super::*;
    fn inputs() -> AdventureInputs {
        AdventureInputs {
            mode: Err(VarError::NotPresent),
            seed: Err(VarError::NotPresent),
            creatures: Err(VarError::NotPresent),
            tables: Err(VarError::NotPresent),
            chapter_type: Err(VarError::NotPresent),
        }
    }
    #[test]
    fn chapter_type_override_is_random_only_and_shaped_as_an_id() {
        let default = adventure_settings(inputs(), Path::new("/game")).unwrap();
        assert_eq!(default.chapter_type, None);
        let mut raw = inputs();
        raw.mode = Ok("random".into());
        raw.chapter_type = Ok("mystery".into());
        assert_eq!(
            adventure_settings(raw, Path::new("/game"))
                .unwrap()
                .chapter_type,
            Some("mystery".into())
        );
        for (mode, kind) in [
            (None, "mystery"),
            (Some("campaign"), "mystery"),
            (Some("random"), ""),
            (Some("random"), "Mystery"),
            (Some("random"), "mystery one"),
        ] {
            let mut raw = inputs();
            raw.mode = mode.map_or(Err(VarError::NotPresent), |m| Ok(m.into()));
            raw.chapter_type = Ok(kind.into());
            assert_eq!(
                adventure_settings(raw, Path::new("/game"))
                    .err()
                    .unwrap()
                    .variable,
                "DF_GAMEPLAY_CHAPTER_TYPE",
                "{mode:?} {kind:?}"
            );
        }
    }
    #[test]
    fn defaults_and_seed_spellings_are_explicit() {
        let default = adventure_settings(inputs(), Path::new("/game")).unwrap();
        assert_eq!(default.mode, AdventureMode::Harbor);
        assert_eq!(default.seed, None);
        assert_eq!(
            default.rules_mode,
            super::super::adventure::RulesMode::Standard
        );
        for (text, value) in [
            ("0", 0),
            ("18446744073709551615", u64::MAX),
            ("0xFFFFFFFFFFFFFFFF", u64::MAX),
            ("0X2a", 42),
            ("42", 42),
        ] {
            assert_eq!(adventure_seed(text), Ok(value));
        }
    }
    #[test]
    fn invalid_environment_values_refuse_with_the_variable_name() {
        for text in ["", "-1", "+1", " 1", "0x", "0xz", "18446744073709551616"] {
            assert_eq!(
                adventure_seed(text).unwrap_err().variable,
                "DF_GAMEPLAY_ADVENTURE_SEED"
            );
        }
        let mut raw = inputs();
        raw.mode = Ok("other".into());
        assert_eq!(
            adventure_settings(raw, Path::new("/game"))
                .err()
                .unwrap()
                .variable,
            "DF_GAMEPLAY_ADVENTURE"
        );
        let mut raw = inputs();
        raw.creatures = Ok("other".into());
        assert_eq!(
            adventure_settings(raw, Path::new("/game"))
                .err()
                .unwrap()
                .variable,
            "DF_GAMEPLAY_ADVENTURE_CREATURES"
        );
        let mut raw = inputs();
        raw.tables = Ok(String::new());
        assert_eq!(
            adventure_settings(raw, Path::new("/game"))
                .err()
                .unwrap()
                .variable,
            "DF_GAMEPLAY_ADVENTURE_TABLES"
        );
    }

    #[test]
    fn the_splat_directory_defaults_resolves_relative_paths_and_refuses_odd_values() {
        let default = Path::new("/srv/assets/splat");
        let current = Path::new("/work");
        let set = |value: &str| Ok::<_, VarError>(value.to_owned());
        assert_eq!(
            splat_directory(Err(VarError::NotPresent), default, current).unwrap(),
            default
        );
        assert_eq!(
            splat_directory(set("/data/scans"), default, current).unwrap(),
            Path::new("/data/scans")
        );
        assert_eq!(
            splat_directory(set("artifacts/scans"), default, current).unwrap(),
            Path::new("/work/artifacts/scans")
        );
        for bad in ["", "../scans", "a/../b", "bad\0name"] {
            assert!(
                splat_directory(set(bad), default, current).is_err(),
                "{bad:?}"
            );
        }
        assert!(splat_directory(Err(VarError::NotUnicode("x".into())), default, current).is_err());
    }

    #[test]
    fn the_battle_scene_setting_accepts_only_a_built_in_scene_id() {
        let set = |value: &str| Ok::<_, VarError>(value.to_owned());
        assert_eq!(battle_scene(Err(VarError::NotPresent)).unwrap(), None);
        assert_eq!(
            battle_scene(set("salt-quay")).unwrap(),
            Some("salt-quay".to_owned())
        );
        for bad in ["", "nowhere", "../salt-quay", "SALT-QUAY"] {
            assert!(battle_scene(set(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_test_fight_setting_takes_one_to_the_most_enemies_and_a_creature_of_the_rules_table() {
        let set = |value: &str| Ok::<_, VarError>(value.to_owned());
        let unset = || Err::<String, _>(VarError::NotPresent);
        assert_eq!(test_fight(unset(), unset()).unwrap(), None);
        let most = super::super::journey::tactical::MAXIMUM_ENEMIES;
        for count in 1..=most {
            let chosen = test_fight(set(&count.to_string()), unset())
                .unwrap()
                .unwrap();
            assert_eq!((chosen.enemies, chosen.foe), (count, None));
        }
        let wolves = test_fight(set("2"), set("wolf")).unwrap().unwrap();
        assert_eq!((wolves.enemies, wolves.foe.as_deref()), (2, Some("wolf")));
        for bad in ["", "0", &(most + 1).to_string(), "-1", "two", "1.5", " 1"] {
            assert!(test_fight(set(bad), unset()).is_err(), "count {bad:?}");
        }
        for bad in ["", "dragon", "Wolf", "hero"] {
            assert!(test_fight(set("1"), set(bad)).is_err(), "foe {bad:?}");
        }
        assert!(
            test_fight(unset(), set("wolf")).is_err(),
            "a creature needs a count"
        );
        assert!(test_fight(Err(VarError::NotUnicode("x".into())), unset()).is_err());
    }

    #[test]
    fn the_splat_lod_is_1_by_default_and_only_0_to_3_are_accepted() {
        let set = |value: &str| Ok::<_, VarError>(value.to_owned());
        assert_eq!(splat_lod(Err(VarError::NotPresent)).unwrap(), 1);
        for (text, level) in [("0", 0), ("1", 1), ("2", 2), ("3", 3)] {
            assert_eq!(splat_lod(set(text)).unwrap(), level);
        }
        for bad in ["", "4", "-1", "1.0", "01", "one", "1 ", "99"] {
            assert!(splat_lod(set(bad)).is_err(), "{bad:?}");
        }
        assert!(splat_lod(Err(VarError::NotUnicode("x".into()))).is_err());
    }

    #[test]
    fn the_battle_grid_is_off_by_default_and_only_on_or_off_are_accepted() {
        let set = |value: &str| Ok::<_, VarError>(value.to_owned());
        assert!(!battle_grid(Err(VarError::NotPresent)).unwrap());
        assert!(!battle_grid(set("off")).unwrap());
        assert!(battle_grid(set("on")).unwrap());
        for bad in ["", "ON", "1", "true", "yes", "on ", "off\n"] {
            assert!(battle_grid(set(bad)).is_err(), "{bad:?}");
        }
        assert!(battle_grid(Err(VarError::NotUnicode("x".into()))).is_err());
    }

    #[test]
    fn the_battle_frame_rate_is_30_by_default_and_only_24_or_30_are_accepted() {
        let set = |value: &str| Ok::<_, VarError>(value.to_owned());
        assert_eq!(battle_fps(Err(VarError::NotPresent)).unwrap(), 30);
        assert_eq!(battle_fps(set("30")).unwrap(), 30);
        assert_eq!(battle_fps(set("24")).unwrap(), 24);
        for bad in [
            "", "60", "25", "0", "-30", "30.0", "thirty", "24 ", "1000000",
        ] {
            assert!(battle_fps(set(bad)).is_err(), "{bad:?}");
        }
        assert!(battle_fps(Err(VarError::NotUnicode("x".into()))).is_err());
    }
}
