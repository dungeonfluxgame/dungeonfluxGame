//! The central configuration file, `config/dungeonflux.toml` in the game folder.
//!
//! It holds every launcher setting and the provider keys, so it is ignored by Git. The clean
//! copy beside it, `config/dungeonflux.example.toml`, is the same file rendered from the
//! defaults with no keys; a test keeps the two in step. Sections only group the setting names
//! of [`Settings`]: the file and the code use one vocabulary.
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use crate::{credentials::Credential, settings::Settings};

/// Everything the configuration file holds.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Config {
    pub settings: Settings,
    pub keys: BTreeMap<Credential, String>,
}

/// Section, its comment, and its settings each with a one-line comment.
/// A setting's name and its comment line.
type Field = (&'static str, &'static str);

const SECTIONS: &[(&str, &str, &[Field])] = &[
    (
        "network",
        "Players and network",
        &[
            (
                "phones_on_network",
                "Let phones on this PC's network join (true) or keep the game on this PC only (false).",
            ),
            (
                "game_port",
                "Port the table display and phones connect to, 1024-65535.",
            ),
            (
                "database_port",
                "Port of the game's own PostgreSQL database on this PC, 1024-65535.",
            ),
        ],
    ),
    (
        "display",
        "Table display",
        &[
            (
                "display",
                "Where the table display opens when the game is ready: \"window\", \"browser\" or \"manual\".",
            ),
            (
                "display_screen",
                "\"second\": a second screen (a TV) when connected; \"main\": the launcher's screen.",
            ),
            (
                "full_screen",
                "\"on-second-screen\", \"always\" or \"never\".",
            ),
        ],
    ),
    (
        "story",
        "Story and sound",
        &[
            (
                "content_rating",
                "\"mature\" or \"restrained\" (no gore or strong language).",
            ),
            (
                "storyteller",
                "AI narration of the story. Needs an OpenAI key.",
            ),
            (
                "story_memory",
                "The storyteller remembers earlier scenes. Needs an OpenAI key.",
            ),
            (
                "narration_voice",
                "Spoken narration and character voices. Needs an ElevenLabs key.",
            ),
            (
                "music",
                "Generated music for each place. Needs an ElevenLabs key.",
            ),
            (
                "speech_input",
                "Players can talk to characters from their phones. Needs an ElevenLabs key.",
            ),
            (
                "choice_hints",
                "Numbered choice chips on the table display.",
            ),
        ],
    ),
    (
        "pictures",
        "Pictures",
        &[
            (
                "scene_pictures",
                "Pictures of people and places as the story goes. Needs an OpenAI key.",
            ),
            (
                "scene_plates",
                "A composed picture for every scene. Needs scene_pictures.",
            ),
            ("picture_quality", "\"low\", \"medium\" or \"high\"."),
            (
                "picture_provider",
                "\"openai\", or \"flux\" (developer option, needs a fal key).",
            ),
            (
                "creator_portraits",
                "Hero portraits during character creation. Needs OpenAI and Jev keys.",
            ),
            (
                "creator_alternate_route",
                "Send mature hero portraits to Black Forest Labs. Needs a BFL key.",
            ),
        ],
    ),
    (
        "video",
        "Generated video costs much more than pictures; all off by default",
        &[
            ("home_video", "A moving title screen. Needs a fal key."),
            (
                "scene_video",
                "Clips for story moments and finishing blows. Needs a fal key.",
            ),
            (
                "character_video",
                "Clips of heroes attacking and reacting. Needs a fal key.",
            ),
        ],
    ),
    (
        "keys",
        "Provider keys. This file is ignored by Git; never commit it or paste keys elsewhere",
        &[(
            "keys_from_env_file",
            "Also read keys missing below from the game folder's .env file.",
        )],
    ),
    (
        "developer",
        "For working on DungeonFlux; players can leave these alone",
        &[
            (
                "server_mode",
                "\"managed\": the launcher runs its own server; \"external\": connect to one already running.",
            ),
            (
                "external_url",
                "Address of the external server, such as \"http://127.0.0.1:63309\".",
            ),
            (
                "dev_control",
                "Turn on the server's authenticated developer commands (df-transport-fixture dev).",
            ),
            (
                "game_folder",
                "Folder holding content/ and assets/. Empty: found automatically.",
            ),
            (
                "build_folder",
                "Build holding debug/df-transport-fixture.exe and play-web/. Empty: found automatically.",
            ),
            (
                "postgres_folder",
                "Folder holding pg_ctl.exe and initdb.exe. Empty: found automatically.",
            ),
            (
                "play_folder",
                "Saved games' database, caches and logs. Empty: artifacts/tmp/play.",
            ),
        ],
    ),
];

const KEY_COMMENTS: [(Credential, &str); 5] = [
    (
        Credential::Openai,
        "Storyteller, story memory, scene pictures, character portraits.",
    ),
    (
        Credential::Elevenlabs,
        "Narration voices, music, speaking to characters.",
    ),
    (
        Credential::Fal,
        "Generated video and the FLUX picture option.",
    ),
    (
        Credential::Jev,
        "Reading players' choices; sorting portrait requests.",
    ),
    (
        Credential::Bfl,
        "Alternate portrait route for mature appearances.",
    ),
];

pub const FILE_NAME: &str = "dungeonflux.toml";

/// `--config` when given; else `config/dungeonflux.toml` in the game folder; else the
/// launcher's data folder (a launcher installed away from the game).
pub fn locate(explicit: Option<PathBuf>, data: &Path) -> PathBuf {
    explicit.unwrap_or_else(|| {
        crate::install::find_game_folder()
            .map(|root| root.join("config").join(FILE_NAME))
            .unwrap_or_else(|| data.join(FILE_NAME))
    })
}

/// A missing file means the defaults; a damaged file is reported and never overwritten.
pub fn load(path: &Path) -> Result<Config, io::Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error),
    };
    parse(&text).map_err(|message| {
        io::Error::other(format!("{} could not be read: {message}", path.display()))
    })
}

pub fn save(path: &Path, config: &Config) -> Result<(), io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::files::write_atomic(path, render(config)?.as_bytes())
}

fn parse(text: &str) -> Result<Config, String> {
    let table: toml::Table = toml::from_str(text).map_err(|error| error.message().to_owned())?;
    let mut flat = toml::Table::new();
    let mut keys = BTreeMap::new();
    for (section, value) in table {
        let toml::Value::Table(entries) = value else {
            return Err(format!("{section} must be a [section]"));
        };
        for (name, value) in entries {
            match (section.as_str(), Credential::from_id(&name)) {
                ("keys", Some(credential)) => {
                    let toml::Value::String(key) = value else {
                        return Err(format!("keys.{name} must be text"));
                    };
                    if !key.is_empty() {
                        keys.insert(credential, key);
                    }
                }
                _ => {
                    flat.insert(name, value);
                }
            }
        }
    }
    let settings: Settings = toml::Value::Table(flat)
        .try_into()
        .map_err(|error: toml::de::Error| error.message().to_owned())?;
    Ok(Config { settings, keys })
}

/// The whole file with its comments, from the current values.
pub fn render(config: &Config) -> Result<String, io::Error> {
    let values = toml::Table::try_from(&config.settings).map_err(io::Error::other)?;
    let mut out = String::from(
        "# DungeonFlux configuration, read by the DungeonFlux launcher (dungeonflux.exe).\n\
         # config/dungeonflux.example.toml is the clean copy kept in Git; the launcher creates\n\
         # config/dungeonflux.toml from it and saves Settings there. That copy holds your keys\n\
         # and is ignored by Git. Changes apply the next time a game starts or resumes.\n",
    );
    for (section, comment, fields) in SECTIONS {
        out.push_str(&format!("\n# {comment}.\n[{section}]\n"));
        for (field, help) in *fields {
            let value = values
                .get(*field)
                .ok_or_else(|| io::Error::other(format!("setting {field} has no value")))?;
            out.push_str(&format!("# {help}\n{field} = {value}\n"));
        }
        if *section == "keys" {
            for (credential, help) in KEY_COMMENTS {
                let value = config.keys.get(&credential).map_or("", String::as_str);
                out.push_str(&format!(
                    "# {}: {help}\n{} = {}\n",
                    credential.label(),
                    credential.id(),
                    toml::Value::String(value.to_owned())
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{DisplayTarget, PictureQuality};

    const EXAMPLE: &str = include_str!("../../../config/dungeonflux.example.toml");

    /// `cargo test -- --ignored regenerate_clean_copy` rewrites the committed clean copy.
    #[test]
    #[ignore = "writes config/dungeonflux.example.toml"]
    fn regenerate_clean_copy() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/dungeonflux.example.toml"
        );
        std::fs::write(path, render(&Config::default()).unwrap()).unwrap();
    }

    #[test]
    fn the_committed_clean_copy_is_exactly_the_rendered_defaults() {
        let rendered = render(&Config::default()).unwrap();
        assert_eq!(
            EXAMPLE.replace("\r\n", "\n"),
            rendered,
            "regenerate config/dungeonflux.example.toml from config::render"
        );
        assert_eq!(parse(EXAMPLE).unwrap(), Config::default());
    }

    #[test]
    fn every_setting_appears_in_exactly_one_section() {
        let values = toml::Table::try_from(Settings::default()).unwrap();
        let listed: Vec<&str> = SECTIONS
            .iter()
            .flat_map(|(_, _, fields)| fields.iter().map(|(field, _)| *field))
            .collect();
        for name in values.keys() {
            assert_eq!(
                listed.iter().filter(|field| *field == name).count(),
                1,
                "{name}"
            );
        }
        assert_eq!(listed.len(), values.len());
    }

    #[test]
    fn values_and_keys_round_trip_with_comments_kept() {
        let mut config = Config::default();
        config.settings.game_port = 64001;
        config.settings.display = DisplayTarget::Browser;
        config.settings.picture_quality = PictureQuality::High;
        config.settings.build_folder = "C:\\games\\build \"x\"".to_owned();
        config
            .keys
            .insert(Credential::Openai, "sk-synthetic".to_owned());
        let text = render(&config).unwrap();
        assert!(text.contains("[network]\n") && text.contains("# Port the table display"));
        assert_eq!(parse(&text).unwrap(), config);
        let folder =
            std::env::temp_dir().join(format!("df-launcher-config-{}", std::process::id()));
        let path = folder.join("config").join(FILE_NAME);
        assert_eq!(load(&path).unwrap(), Config::default());
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        std::fs::write(&path, "[network]\ngame_port = \"not a port\"\n").unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn unknown_keys_sections_and_empty_keys_are_tolerated() {
        let config = parse("[keys]\nopenai = \"\"\nfal = \"f\"\n[future]\nthing = 1\n").unwrap();
        assert_eq!(config.keys.len(), 1);
        assert!(parse("top = 1\n").is_err());
    }
}
