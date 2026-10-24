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

pub fn network(settings: &Settings, address: Option<Ipv4Addr>) -> Network {
    let port = settings.game_port;
    let local = format!("http://127.0.0.1:{port},http://localhost:{port}");
    match address.filter(|_| settings.phones_on_network) {
        Some(address) => {
            let public = format!("http://{address}:{port}");
            Network {
                listen: format!("0.0.0.0:{port}"),
                allowed_origins: format!("{public},{local}"),
                public_url: Some(public),
            }
        }
        None => Network {
            listen: format!("127.0.0.1:{port}"),
            public_url: None,
            allowed_origins: local,
        },
    }
}

/// Inherited names the launcher always replaces or removes, so a developer shell's settings
/// never change a launcher game behind the player's back.
fn inherited_setting(name: &str) -> bool {
    name.starts_with("DF_GAMEPLAY_")
        || name.starts_with("DF_JEV_")
        || name.starts_with("DUNGEONFLUX_")
        || matches!(
            name,
            "OPENAI_API_KEY" | "ELEVENLABS_API_KEY" | "FAL_KEY" | "DF_FAL_KEY" | "BFL_API_KEY"
        )
}

pub struct Launch<'a> {
    pub install: &'a Install,
    pub settings: &'a Settings,
    pub keys: &'a Keys,
    pub network: &'a Network,
    pub database: &'a str,
    pub adventure: Adventure,
    pub stop_dir: &'a Path,
}

pub fn environment(launch: &Launch<'_>) -> Vec<(String, String)> {
    let play = &launch.install.play_root;
    let root = &launch.install.root;
    let path = |value: PathBuf| value.display().to_string();
    let mut environment: Vec<(String, String)> = vec![
        ("DF_GAMEPLAY_DEMO_DATABASE".into(), launch.database.into()),
        (
            "DF_GAMEPLAY_DATABASE_PORT".into(),
            launch.settings.database_port.to_string(),
        ),
        ("DF_GAMEPLAY_STOP_FILE".into(), path(launch.stop_dir.into())),
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
            path(root.join("content/harbor-journey")),
        ),
        (
            "DF_GAMEPLAY_ADVENTURE_TABLES".into(),
            path(root.join("content/adventure-tables")),
        ),
        (
            "DF_GAMEPLAY_VOICE_CASTING".into(),
            path(root.join("content/voice-casting.json")),
        ),
        (
            "DF_GAMEPLAY_TEXT_CACHE".into(),
            path(play.join("text-cache")),
        ),
        (
            "DF_GAMEPLAY_VOICE_CACHE".into(),
            path(play.join("voice-cache")),
        ),
        (
            "DF_GAMEPLAY_HOME_VIDEO_CACHE".into(),
            path(play.join("home-video-cache")),
        ),
        (
            "DF_GAMEPLAY_MUSIC_CACHE".into(),
            path(play.join("music-cache")),
        ),
    ];
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
    ] {
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
    let mut display = None;
    let mut player = None;
    for field in rest.split(' ') {
        if let Some(value) = field.strip_prefix("display=") {
            display = Some(value.to_owned());
        } else if let Some(value) = field.strip_prefix("player=") {
            player = Some(value.to_owned());
        }
    }
    let display_url = display.filter(|url| url.starts_with("http://"))?;
    Some(Ready {
        display_url,
        player_url: player.unwrap_or_default(),
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
    let mut offset = 0u64;
    let mut pending = String::new();
    loop {
        let mut file = std::fs::File::open(log)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut fresh = Vec::new();
        file.read_to_end(&mut fresh)?;
        offset += fresh.len() as u64;
        pending.push_str(&String::from_utf8_lossy(&fresh));
        while let Some(end) = pending.find('\n') {
            let line: String = pending.drain(..=end).collect();
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
        let text = std::fs::read_to_string(log).unwrap_or_default();
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
        assert!(!inherited_setting("PATH"));
        std::fs::remove_dir_all(data).unwrap();
    }
}
