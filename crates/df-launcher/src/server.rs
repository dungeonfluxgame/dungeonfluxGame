//! The game server process: its environment, a detached start that outlives the launcher, the
//! `READY` handshake, and the graceful stop marker the server publishes for its own run.
use std::{
    io::{self, Read, Seek, SeekFrom},
    net::{Ipv4Addr, UdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    credentials::{Credential, Keys},
    install::Install,
    saves::Adventure,
    settings::Settings,
};

/// How phones reach this game.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Network {
    pub listen: String,
    pub public_url: Option<String>,
    pub allowed_origins: String,
}

/// This PC's private-network IPv4 address. Connecting a UDP socket sends no packet; it only
/// asks the routing table which interface would be used.
pub fn private_address() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(address) if address.is_private() => Some(address),
        _ => None,
    }
}

/// The LAN address phones use, when this network reaches them.
pub fn lan_address(network: &Network) -> Option<Ipv4Addr> {
    let public = network.public_url.as_deref()?;
    let authority = public.strip_prefix("http://")?.split('/').next()?;
    authority
        .split(':')
        .next()?
        .parse::<Ipv4Addr>()
        .ok()
        .filter(|address| address.is_private())
}

pub fn network(settings: &Settings, address: Option<Ipv4Addr>) -> Network {
    let port = settings.game_port;
    let localhost_origins = format!("http://127.0.0.1:{port},http://localhost:{port}");
    match address.filter(|_| settings.phones_on_network) {
        Some(address) => {
            let public_url = format!("http://{address}:{port}");
            Network {
                listen: format!("0.0.0.0:{port}"),
                allowed_origins: format!("{public_url},{localhost_origins}"),
                public_url: Some(public_url),
            }
        }
        None => Network {
            listen: format!("127.0.0.1:{port}"),
            public_url: None,
            allowed_origins: localhost_origins,
        },
    }
}

/// Inherited names the launcher always replaces or removes, so a developer shell's settings
/// never change a launcher game behind the player's back.
fn inherited_setting(name: &str) -> bool {
    name.starts_with("DF_GAMEPLAY_")
        || name.starts_with("DF_JEV_")
        || name.starts_with("DF_HOME_VIDEO_")
        || name.starts_with("DUNGEONFLUX_")
        || matches!(
            name,
            "OPENAI_API_KEY"
                | "ELEVENLABS_API_KEY"
                | "FAL_KEY"
                | "DF_FAL_KEY"
                | "BFL_API_KEY"
                | "WORLDLABS_API_KEY"
        )
}

/// Whether any feature this game runs with uses `credential`. A key nothing needs is never
/// handed to the server, so a feature turned off cannot reach its provider by any path.
fn needed(settings: &Settings, credential: Credential) -> bool {
    match credential {
        Credential::Openai => {
            settings.storyteller
                || settings.story_memory
                || settings.scene_pictures
                || settings.character_video
        }
        Credential::Elevenlabs => {
            settings.narration_voice || settings.music || settings.speech_input
        }
        Credential::Fal => {
            settings.home_video
                || settings.scene_video
                || settings.character_video
                // Scene pictures segment their figures (speaker masks) through fal's SAM
                // whichever provider paints them.
                || settings.scene_pictures
        }
        Credential::Jev => settings.action_reading,
        Credential::Worldlabs => settings.splat_worlds,
        // Read only through the creator's own variables below.
        Credential::Bfl => false,
    }
}

pub struct Launch<'a> {
    pub install: &'a Install,
    pub settings: &'a Settings,
    pub keys: &'a Keys,
    pub network: &'a Network,
    pub database: &'a str,
    pub adventure: Adventure,
    pub stop_dir: &'a Path,
    /// The secret the server answers `/gameplay/launcher-status` with (REF-064).
    pub status_token: &'a str,
}

pub fn environment(launch: &Launch<'_>) -> Vec<(String, String)> {
    let play_root = &launch.install.play_root;
    let install_root = &launch.install.root;
    let display_path = |value: PathBuf| value.display().to_string();
    let mut environment: Vec<(String, String)> = vec![
        ("DF_GAMEPLAY_DEMO_DATABASE".into(), launch.database.into()),
        (
            "DF_GAMEPLAY_DATABASE_PORT".into(),
            launch.settings.database_port.to_string(),
        ),
        (
            "DF_GAMEPLAY_STOP_FILE".into(),
            display_path(launch.stop_dir.into()),
        ),
        (
            "DF_GAMEPLAY_LAUNCHER_TOKEN".into(),
            launch.status_token.to_owned(),
        ),
        ("DF_GAMEPLAY_LISTEN".into(), launch.network.listen.clone()),
        (
            "DF_GAMEPLAY_ALLOWED_ORIGINS".into(),
            launch.network.allowed_origins.clone(),
        ),
        (
            "DF_GAMEPLAY_ADVENTURE".into(),
            launch.adventure.server_value().into(),
        ),
        (
            "DF_GAMEPLAY_PACK_DIR".into(),
            display_path(install_root.join("content/harbor-journey")),
        ),
        (
            "DF_GAMEPLAY_ADVENTURE_TABLES".into(),
            display_path(install_root.join("content/adventure-tables")),
        ),
        (
            "DF_GAMEPLAY_VOICE_CASTING".into(),
            display_path(install_root.join("content/voice-casting.json")),
        ),
        (
            "DF_GAMEPLAY_TEXT_CACHE".into(),
            display_path(play_root.join("text-cache")),
        ),
        (
            "DF_GAMEPLAY_VOICE_CACHE".into(),
            display_path(play_root.join("voice-cache")),
        ),
        (
            "DF_GAMEPLAY_HOME_VIDEO_CACHE".into(),
            display_path(play_root.join("home-video-cache")),
        ),
        (
            "DF_GAMEPLAY_MUSIC_CACHE".into(),
            display_path(play_root.join("music-cache")),
        ),
        // Hand-tuned TV values (zoom, margins, focus, thinking), kept across restarts and read on
        // every display load, so changing them needs no rebuild.
        (
            "DF_GAMEPLAY_DISPLAY_TUNING".into(),
            display_path(play_root.join("display-tuning.css")),
        ),
    ];
    if std::env::var_os("DF_RUNTIME_LAUNCH_SECRET").is_some() {
        environment.push(("DF_RUNTIME_SUPERVISOR".into(), "1".into()));
        environment.push((
            "DF_RUNTIME_PHONES_ON_NETWORK".into(),
            if launch.settings.phones_on_network {
                "1".into()
            } else {
                "0".into()
            },
        ));
        if let Some(address) = lan_address(launch.network) {
            environment.push(("DF_RUNTIME_LAN_ADDRESS".into(), address.to_string()));
        }
    }
    if let Some(public) = &launch.network.public_url {
        environment.push(("DF_GAMEPLAY_PUBLIC_URL".into(), public.clone()));
    }
    for (name, value) in launch.settings.feature_environment() {
        environment.push((name.into(), value));
    }
    let keys = launch.keys;
    for (name, credential) in [
        ("OPENAI_API_KEY", Credential::Openai),
        ("ELEVENLABS_API_KEY", Credential::Elevenlabs),
        ("FAL_KEY", Credential::Fal),
        ("DF_JEV_API_KEY", Credential::Jev),
        ("WORLDLABS_API_KEY", Credential::Worldlabs),
    ] {
        if !needed(launch.settings, credential) {
            continue;
        }
        if let Some(value) = keys.value(credential) {
            environment.push((name.into(), value.into()));
        }
    }
    // Character portraits read their own dedicated keys (see launch-reference.md).
    if launch.settings.creator_portraits {
        for (name, credential) in [
            ("DF_GAMEPLAY_CREATOR_OPENAI_KEY", Credential::Openai),
            ("DF_GAMEPLAY_CREATOR_JEV_API_KEY", Credential::Jev),
        ] {
            if let Some(value) = keys.value(credential) {
                environment.push((name.into(), value.into()));
            }
        }
        if launch.settings.creator_alternate_route
            && let Some(value) = keys.value(Credential::Bfl)
        {
            environment.push(("DF_GAMEPLAY_CREATOR_BFL_KEY".into(), value.into()));
        }
    }
    environment
}

/// Settings the server would refuse at startup, caught before anything starts.
pub fn missing_keys(settings: &Settings, keys: &Keys) -> Option<String> {
    let has = |credential| keys.value(credential).is_some();
    if settings.creator_portraits && !(has(Credential::Openai) && has(Credential::Jev)) {
        return Some(
            "Character portraits aren't set up yet. Settings, under Pictures, shows what they need; or turn them off there."
                .to_owned(),
        );
    }
    if settings.creator_portraits && settings.creator_alternate_route && !has(Credential::Bfl) {
        return Some(
            "The alternate portrait route isn't set up yet. Settings, under Pictures, shows what it needs; or turn it off there."
                .to_owned(),
        );
    }
    if settings.picture_provider == crate::settings::PictureProvider::Flux && !has(Credential::Fal)
    {
        return Some(
            "FLUX pictures aren't set up yet. Settings, under Pictures, shows what they need; or choose OpenAI pictures there."
                .to_owned(),
        );
    }
    None
}

/// Starts the server so it keeps running after the launcher closes (REF-005). Output goes to
/// `log`, which the launcher reads for the `READY` line.
pub fn spawn(launch: &Launch<'_>, log: &Path) -> Result<Child, io::Error> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let output = std::fs::File::create(log)?;
    let mut command = Command::new(&launch.install.server_exe);
    command
        .arg("--gameplay-demo")
        .arg(&launch.install.web_dir)
        .arg(&launch.install.assets_dir)
        .current_dir(&launch.install.root)
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output);
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(inherited_setting) {
            command.env_remove(name);
        }
    }
    command.envs(environment(launch));
    detached_spawn(&mut command)
}

#[cfg(windows)]
fn detached_spawn(command: &mut Command) -> Result<Child, io::Error> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    match command.spawn() {
        // A job that forbids breakaway refuses the flag; the server then lives as long as the
        // job does, which is still the launcher's lifetime under Explorer.
        Err(error) if error.raw_os_error() == Some(5) => {
            command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
            command.spawn()
        }
        result => result,
    }
}

#[cfg(not(windows))]
fn detached_spawn(command: &mut Command) -> Result<Child, io::Error> {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    command.spawn()
}

/// The server's `READY display=<url> player=<url> room=<code>` line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ready {
    pub display_url: String,
    pub player_url: String,
}

pub fn parse_ready(line: &str) -> Option<Ready> {
    let rest = line.trim_end().strip_prefix("READY ")?;
    let mut display_url = None;
    let mut player_url = None;
    for field in rest.split(' ') {
        if let Some(value) = field.strip_prefix("display=") {
            display_url = Some(value.to_owned());
        } else if let Some(value) = field.strip_prefix("player=") {
            player_url = Some(value.to_owned());
        }
    }
    let display_url = display_url.filter(|url| url.starts_with("http://"))?;
    Some(Ready {
        display_url,
        player_url: player_url.unwrap_or_default(),
    })
}

pub enum Wait {
    Ready(Ready),
    Exited(Option<i32>),
    TimedOut,
}

/// Follows the log until the server is ready, exits, or `limit` passes. `progress` receives
/// each status line (`ROOM`, `STORYTELLER`, …) so the launcher can show what is happening.
pub fn wait_ready(
    child: &mut Child,
    log: &Path,
    limit: Duration,
    mut progress: impl FnMut(&str),
) -> Result<Wait, io::Error> {
    let started = Instant::now();
    let mut log_offset_bytes = 0u64;
    let mut pending_text = String::new();
    loop {
        let mut file = std::fs::File::open(log)?;
        file.seek(SeekFrom::Start(log_offset_bytes))?;
        let mut new_bytes = Vec::new();
        file.read_to_end(&mut new_bytes)?;
        log_offset_bytes += new_bytes.len() as u64;
        pending_text.push_str(&String::from_utf8_lossy(&new_bytes));
        while let Some(newline_index) = pending_text.find('\n') {
            let line: String = pending_text.drain(..=newline_index).collect();
            if let Some(ready) = parse_ready(&line) {
                return Ok(Wait::Ready(ready));
            }
            progress(line.trim_end());
        }
        if let Some(status) = child.try_wait()? {
            return Ok(Wait::Exited(status.code()));
        }
        if started.elapsed() > limit {
            return Ok(Wait::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The server prints `DEV CONTROL on url=<url> token_file=<path> (token never printed)` just
/// after `READY` when developer control is on.
pub fn dev_control_line(log: &Path, limit: Duration) -> Option<String> {
    let started = Instant::now();
    loop {
        // Lossy, as `wait_ready` reads it: one stray byte in the server's output must not hide
        // the line that follows it.
        let text = std::fs::read(log)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        if let Some(line) = text
            .lines()
            .find(|line| line.starts_with("DEV CONTROL on url="))
        {
            return Some(line.trim().to_owned());
        }
        if started.elapsed() > limit {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `url` and `token_file` from a `DEV CONTROL` line.
pub fn dev_control_parts(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("DEV CONTROL on url=")?;
    let (url, rest) = rest.split_once(" token_file=")?;
    let token_file = rest.strip_suffix(" (token never printed)").unwrap_or(rest);
    Some((url.to_owned(), token_file.trim().to_owned()))
}

/// Asks the server to stop through the exact marker it published in `target.receipt`.
pub fn request_stop(stop_dir: &Path) -> Result<(), io::Error> {
    let receipt = std::fs::read_to_string(stop_dir.join("target.receipt")).map_err(|_| {
        io::Error::other(
            "The game has not published its stop marker, so it cannot be asked to stop.",
        )
    })?;
    let marker = receipt.trim();
    if !valid_marker(marker) {
        return Err(io::Error::other("The game's stop marker is invalid."));
    }
    std::fs::write(stop_dir.join(marker), b"")
}

/// `<16 lowercase hex digits>.stop`, the only form the server writes.
fn valid_marker(marker: &str) -> bool {
    marker.strip_suffix(".stop").is_some_and(|stem| {
        stem.len() == 16
            && stem
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials;

    #[test]
    fn ready_line_parses_and_other_lines_do_not() {
        let ready = parse_ready(
            "READY display=http://192.168.1.9:63309/gameplay/display player=http://192.168.1.9:63309/gameplay/player?room=ABCD room=ABCD\r\n",
        )
        .unwrap();
        assert_eq!(
            ready.display_url,
            "http://192.168.1.9:63309/gameplay/display"
        );
        assert!(ready.player_url.ends_with("room=ABCD"));
        assert!(parse_ready("ROOM ABCD").is_none());
        assert!(parse_ready("READY display=javascript:alert(1)").is_none());
    }

    #[test]
    fn dev_control_line_is_found_even_when_the_log_holds_non_utf8_bytes() {
        let folder =
            std::env::temp_dir().join(format!("df-launcher-devctl-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let log = folder.join("server.log");
        let mut bytes = b"READY display=http://127.0.0.1:63309/gameplay/display\n".to_vec();
        bytes.extend_from_slice(b"postgres: caf\xe9 \xff\n");
        bytes.extend_from_slice(
            b"DEV CONTROL on url=http://127.0.0.1:63310 token_file=t (token never printed)\n",
        );
        std::fs::write(&log, &bytes).unwrap();
        let found = dev_control_line(&log, Duration::from_millis(500));
        assert_eq!(
            found.as_deref(),
            Some("DEV CONTROL on url=http://127.0.0.1:63310 token_file=t (token never printed)")
        );
        assert_eq!(
            dev_control_line(&folder.join("missing.log"), Duration::from_millis(1)),
            None
        );
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn stop_markers_must_be_the_servers_exact_form() {
        assert!(valid_marker("0123456789abcdef.stop"));
        for refused in [
            "",
            "../x.stop",
            "0123456789ABCDEF.stop",
            "0123456789abcde.stop",
            "0123456789abcdef",
        ] {
            assert!(!valid_marker(refused), "{refused}");
        }
        let folder = std::env::temp_dir().join(format!("df-launcher-stop-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        assert!(request_stop(&folder).is_err());
        std::fs::write(folder.join("target.receipt"), b"0123456789abcdef.stop").unwrap();
        request_stop(&folder).unwrap();
        assert!(folder.join("0123456789abcdef.stop").is_file());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn phones_on_network_listen_widely_only_with_a_private_address() {
        let settings = Settings::default();
        let lan = network(&settings, Some(Ipv4Addr::new(192, 168, 1, 9)));
        assert_eq!(lan.listen, "0.0.0.0:63309");
        assert_eq!(lan.public_url.as_deref(), Some("http://192.168.1.9:63309"));
        assert!(lan.allowed_origins.contains("http://127.0.0.1:63309"));
        let none = network(&settings, None);
        assert_eq!(none.listen, "127.0.0.1:63309");
        assert!(none.public_url.is_none());
        let off = Settings {
            phones_on_network: false,
            ..Settings::default()
        };
        assert_eq!(
            network(&off, Some(Ipv4Addr::new(10, 0, 0, 2))).listen,
            "127.0.0.1:63309"
        );
    }

    #[test]
    fn lan_address_for_a_supervised_game_comes_from_the_public_url() {
        let settings = Settings::default();
        let lan = network(&settings, Some(Ipv4Addr::new(192, 168, 0, 96)));
        assert_eq!(lan_address(&lan), Some(Ipv4Addr::new(192, 168, 0, 96)));
        assert_eq!(lan_address(&network(&settings, None)), None);
        let off = Settings {
            phones_on_network: false,
            ..Settings::default()
        };
        assert_eq!(
            lan_address(&network(&off, Some(Ipv4Addr::new(192, 168, 0, 96)))),
            None
        );
    }

    #[test]
    fn environment_carries_settings_and_only_the_needed_keys() {
        let data = std::env::temp_dir().join(format!("df-launcher-env-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let env_file = data.join(".env");
        std::fs::write(
            &env_file,
            "OPENAI_API_KEY=sk-openai\nDF_JEV_API_KEY=jev-key\nBFL_API_KEY=bfl\n",
        )
        .unwrap();
        let keys =
            credentials::resolve(&std::collections::BTreeMap::new(), Some(&env_file)).unwrap();
        let install = Install {
            root: PathBuf::from("/game"),
            server_exe: PathBuf::from("/game/server"),
            web_dir: PathBuf::from("/game/web"),
            assets_dir: PathBuf::from("/game/assets/ui"),
            postgres_bin: PathBuf::from("/pg"),
            play_root: PathBuf::from("/game/play"),
            env_file,
        };
        let mut settings = Settings::default();
        let network = network(&settings, None);
        let launch = |settings: &Settings| {
            environment(&Launch {
                install: &install,
                settings,
                keys: &keys,
                network: &network,
                database: "df_gameplay_demo_play_x",
                adventure: Adventure::Harbor,
                stop_dir: Path::new("/game/play/run/stop.1"),
                status_token: "0123456789abcdef0123456789abcdef",
            })
        };
        let value = |environment: &[(String, String)], name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        let plain = launch(&settings);
        assert_eq!(
            value(&plain, "DF_GAMEPLAY_ADVENTURE").as_deref(),
            Some("harbor")
        );
        assert_eq!(
            value(&plain, "OPENAI_API_KEY").as_deref(),
            Some("sk-openai")
        );
        assert_eq!(value(&plain, "DF_GAMEPLAY_CREATOR_OPENAI_KEY"), None);
        assert_eq!(value(&plain, "FAL_KEY"), None);
        assert_eq!(value(&plain, "DF_GAMEPLAY_PUBLIC_URL"), None);
        assert_eq!(
            value(&plain, "DF_GAMEPLAY_LAUNCHER_TOKEN").as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(missing_keys(&settings, &keys), None);
        settings.creator_portraits = true;
        settings.creator_alternate_route = true;
        let portraits = launch(&settings);
        assert_eq!(
            value(&portraits, "DF_GAMEPLAY_CREATOR_JEV_API_KEY").as_deref(),
            Some("jev-key")
        );
        assert_eq!(
            value(&portraits, "DF_GAMEPLAY_CREATOR_BFL_KEY").as_deref(),
            Some("bfl")
        );
        settings.picture_provider = crate::settings::PictureProvider::Flux;
        assert!(
            missing_keys(&settings, &keys)
                .unwrap()
                .contains("FLUX pictures")
        );
        assert!(inherited_setting("DF_GAMEPLAY_HOME_VIDEO"));
        assert!(inherited_setting("WORLDLABS_API_KEY"));
        assert!(inherited_setting("DF_HOME_VIDEO_KEY_FILE"));
        assert!(!inherited_setting("PATH"));
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn every_feature_off_hands_the_server_no_key_and_every_switch_off() {
        let data = std::env::temp_dir().join(format!("df-launcher-off-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let env_file = data.join(".env");
        std::fs::write(
            &env_file,
            "OPENAI_API_KEY=k1\nELEVENLABS_API_KEY=k2\nFAL_KEY=k3\nDF_JEV_API_KEY=k4\nBFL_API_KEY=k5\nWORLDLABS_API_KEY=k6\n",
        )
        .unwrap();
        let keys =
            credentials::resolve(&std::collections::BTreeMap::new(), Some(&env_file)).unwrap();
        let install = Install {
            root: PathBuf::from("/game"),
            server_exe: PathBuf::from("/game/server"),
            web_dir: PathBuf::from("/game/web"),
            assets_dir: PathBuf::from("/game/assets/ui"),
            postgres_bin: PathBuf::from("/pg"),
            play_root: PathBuf::from("/game/play"),
            env_file,
        };
        let off = Settings {
            storyteller: false,
            story_memory: false,
            narration_voice: false,
            music: false,
            speech_input: false,
            action_reading: false,
            scene_pictures: false,
            scene_plates: false,
            picture_provider: crate::settings::PictureProvider::Flux,
            ..Settings::default()
        };
        let network = network(&off, None);
        let launch = |settings: &Settings| {
            environment(&Launch {
                install: &install,
                settings,
                keys: &keys,
                network: &network,
                database: "df_gameplay_demo_play_x",
                adventure: Adventure::Harbor,
                stop_dir: Path::new("/game/play/run/stop.1"),
                status_token: "0123456789abcdef0123456789abcdef",
            })
        };
        let environment = launch(&off);
        for (name, value) in &environment {
            assert!(
                !["k1", "k2", "k3", "k4", "k5", "k6"].contains(&value.as_str()),
                "{name} carries a key"
            );
        }
        for switch in [
            "DF_GAMEPLAY_STORYTELLER",
            "DF_GAMEPLAY_MEMORY",
            "DF_GAMEPLAY_VOICE",
            "DF_GAMEPLAY_MUSIC",
            "DF_GAMEPLAY_STT",
            "DF_GAMEPLAY_JEV",
            "DF_GAMEPLAY_IMAGERY",
            "DF_GAMEPLAY_PLATES",
            "DF_GAMEPLAY_CREATOR_PORTRAITS",
            "DF_GAMEPLAY_SPLAT_WORLDS",
            "DF_GAMEPLAY_HOME_VIDEO",
            "DF_GAMEPLAY_SCENE_VIDEO",
            "DF_GAMEPLAY_CHARACTER_VIDEO",
        ] {
            let value = environment
                .iter()
                .find(|(name, _)| name == switch)
                .map(|(_, value)| value.as_str());
            assert_eq!(value, Some("off"), "{switch}");
        }
        // Each feature brings exactly the key it needs.
        let with = |change: fn(&mut Settings)| {
            let mut settings = off.clone();
            change(&mut settings);
            let mut names: Vec<String> = launch(&settings)
                .into_iter()
                .filter(|(_, value)| value.starts_with('k') && value.len() == 2)
                .map(|(name, _)| name)
                .collect();
            names.sort();
            names
        };
        assert_eq!(with(|s| s.scene_video = true), ["FAL_KEY"]);
        assert_eq!(with(|s| s.music = true), ["ELEVENLABS_API_KEY"]);
        assert_eq!(with(|s| s.action_reading = true), ["DF_JEV_API_KEY"]);
        assert_eq!(with(|s| s.splat_worlds = true), ["WORLDLABS_API_KEY"]);
        assert_eq!(with(|s| s.storyteller = true), ["OPENAI_API_KEY"]);
        assert_eq!(
            with(|s| s.scene_pictures = true),
            ["FAL_KEY", "OPENAI_API_KEY"]
        );
        std::fs::remove_dir_all(data).unwrap();
    }
}
