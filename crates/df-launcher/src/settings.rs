//! Launcher settings: what the player chose, stored as TOML in the launcher's data folder, and
//! the exact server environment each choice becomes. Every setting maps onto a variable the game
//! server already reads (see `development/launch-reference.md`); nothing here invents behaviour.
use serde::{Deserialize, Serialize};

use crate::form::Form;

/// Where the table display opens once the game is ready.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayTarget {
    /// A DungeonFlux window owned by the launcher.
    #[default]
    Window,
    /// The default web browser.
    Browser,
    /// Nothing opens; the display link is shown in the launcher.
    Manual,
}

/// Which screen the display window opens on.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayScreen {
    /// A second screen (a TV over HDMI) when one is connected, otherwise this screen.
    #[default]
    Second,
    /// Always the screen the launcher is on.
    Main,
}

/// When the display window fills its screen.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FullScreen {
    #[default]
    OnSecondScreen,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContentRating {
    #[default]
    Mature,
    Restrained,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PictureQuality {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PictureProvider {
    #[default]
    Openai,
    Flux,
}

/// Who runs the game server.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerMode {
    /// The launcher starts and stops its own server and database.
    #[default]
    Managed,
    /// A server someone started separately (for example `development/run-gameplay.sh`).
    External,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub phones_on_network: bool,
    pub game_port: u16,
    pub database_port: u16,

    pub display: DisplayTarget,
    pub display_screen: DisplayScreen,
    pub full_screen: FullScreen,

    pub content_rating: ContentRating,
    pub storyteller: bool,
    pub story_memory: bool,
    pub narration_voice: bool,
    pub music: bool,
    pub speech_input: bool,
    pub choice_hints: bool,

    pub scene_pictures: bool,
    pub scene_plates: bool,
    pub picture_quality: PictureQuality,
    pub picture_provider: PictureProvider,
    pub creator_portraits: bool,
    pub creator_alternate_route: bool,

    pub home_video: bool,
    pub scene_video: bool,
    pub character_video: bool,

    pub keys_from_env_file: bool,

    pub server_mode: ServerMode,
    pub external_url: String,
    pub dev_control: bool,
    pub game_folder: String,
    pub build_folder: String,
    pub postgres_folder: String,
    pub play_folder: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            phones_on_network: true,
            game_port: 63309,
            database_port: 55517,
            display: DisplayTarget::Window,
            display_screen: DisplayScreen::Second,
            full_screen: FullScreen::OnSecondScreen,
            content_rating: ContentRating::Mature,
            storyteller: true,
            story_memory: true,
            narration_voice: true,
            music: true,
            speech_input: true,
            choice_hints: true,
            scene_pictures: true,
            scene_plates: true,
            picture_quality: PictureQuality::Medium,
            picture_provider: PictureProvider::Openai,
            creator_portraits: false,
            creator_alternate_route: false,
            home_video: false,
            scene_video: false,
            character_video: false,
            keys_from_env_file: true,
            server_mode: ServerMode::Managed,
            external_url: String::new(),
            dev_control: false,
            game_folder: String::new(),
            build_folder: String::new(),
            postgres_folder: String::new(),
            play_folder: String::new(),
        }
    }
}

/// A refused field, named by its form field so the page can mark it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}

impl Settings {
    /// Reads the whole settings form. Checkboxes absent from the form are off, so the page
    /// always submits every field. On error nothing is changed.
    pub fn with_form(&self, form: &Form) -> Result<Self, Vec<FieldError>> {
        let mut errors = Vec::new();
        let mut next = self.clone();
        let checked = |name: &str| form.get(name) == Some("on");
        next.phones_on_network = checked("phones_on_network");
        next.game_port = port(form, "game_port", &mut errors).unwrap_or(self.game_port);
        next.database_port = port(form, "database_port", &mut errors).unwrap_or(self.database_port);
        if errors.is_empty() && next.game_port == next.database_port {
            errors.push(FieldError {
                field: "database_port",
                message: "The game and the database need different ports.".to_owned(),
            });
        }
        next.display = choice(
            form,
            "display",
            &mut errors,
            &[
                ("window", DisplayTarget::Window),
                ("browser", DisplayTarget::Browser),
                ("manual", DisplayTarget::Manual),
            ],
        )
        .unwrap_or(self.display);
        next.display_screen = choice(
            form,
            "display_screen",
            &mut errors,
            &[
                ("second", DisplayScreen::Second),
                ("main", DisplayScreen::Main),
            ],
        )
        .unwrap_or(self.display_screen);
        next.full_screen = choice(
            form,
            "full_screen",
            &mut errors,
            &[
                ("on-second-screen", FullScreen::OnSecondScreen),
                ("always", FullScreen::Always),
                ("never", FullScreen::Never),
            ],
        )
        .unwrap_or(self.full_screen);
        next.content_rating = choice(
            form,
            "content_rating",
            &mut errors,
            &[
                ("mature", ContentRating::Mature),
                ("restrained", ContentRating::Restrained),
            ],
        )
        .unwrap_or(self.content_rating);
        next.storyteller = checked("storyteller");
        next.story_memory = checked("story_memory");
        next.narration_voice = checked("narration_voice");
        next.music = checked("music");
        next.speech_input = checked("speech_input");
        next.choice_hints = checked("choice_hints");
        next.scene_pictures = checked("scene_pictures");
        next.scene_plates = checked("scene_plates");
        next.picture_quality = choice(
            form,
            "picture_quality",
            &mut errors,
            &[
                ("low", PictureQuality::Low),
                ("medium", PictureQuality::Medium),
                ("high", PictureQuality::High),
            ],
        )
        .unwrap_or(self.picture_quality);
        next.picture_provider = choice(
            form,
            "picture_provider",
            &mut errors,
            &[
                ("openai", PictureProvider::Openai),
                ("flux", PictureProvider::Flux),
            ],
        )
        .unwrap_or(self.picture_provider);
        next.creator_portraits = checked("creator_portraits");
        next.creator_alternate_route = checked("creator_alternate_route");
        next.home_video = checked("home_video");
        next.scene_video = checked("scene_video");
        next.character_video = checked("character_video");
        next.keys_from_env_file = checked("keys_from_env_file");
        next.server_mode = choice(
            form,
            "server_mode",
            &mut errors,
            &[
                ("managed", ServerMode::Managed),
                ("external", ServerMode::External),
            ],
        )
        .unwrap_or(self.server_mode);
        next.external_url = form.get("external_url").unwrap_or("").trim().to_owned();
        if (next.server_mode == ServerMode::External || !next.external_url.is_empty())
            && let Err(message) = external_origin(&next.external_url)
        {
            errors.push(FieldError {
                field: "external_url",
                message,
            });
        }
        next.dev_control = checked("dev_control");
        for (field, slot) in [
            ("game_folder", &mut next.game_folder),
            ("build_folder", &mut next.build_folder),
            ("postgres_folder", &mut next.postgres_folder),
            ("play_folder", &mut next.play_folder),
        ] {
            match folder_problem(form.get(field).unwrap_or("").trim()) {
                Some(message) => errors.push(FieldError {
                    field,
                    message: message.to_owned(),
                }),
                None => *slot = form.get(field).unwrap_or("").trim().to_owned(),
            }
        }
        if errors.is_empty() {
            Ok(next)
        } else {
            Err(errors)
        }
    }

    /// The game feature switches as server environment variables. Every switch is written
    /// explicitly, so a value inherited from the launcher's own environment never leaks in.
    pub fn feature_environment(&self) -> Vec<(&'static str, String)> {
        let switch = |on: bool| if on { "on" } else { "off" }.to_owned();
        vec![
            (
                "DF_GAMEPLAY_CONTENT_RATING",
                match self.content_rating {
                    ContentRating::Mature => "mature",
                    ContentRating::Restrained => "restrained",
                }
                .to_owned(),
            ),
            ("DF_GAMEPLAY_STORYTELLER", switch(self.storyteller)),
            ("DF_GAMEPLAY_MEMORY", switch(self.story_memory)),
            ("DF_GAMEPLAY_VOICE", switch(self.narration_voice)),
            ("DF_GAMEPLAY_MUSIC", switch(self.music)),
            ("DF_GAMEPLAY_STT", switch(self.speech_input)),
            ("DF_GAMEPLAY_CHOICE_HINTS", switch(self.choice_hints)),
            ("DF_GAMEPLAY_IMAGERY", switch(self.scene_pictures)),
            ("DF_GAMEPLAY_PLATES", switch(self.scene_plates)),
            (
                "DF_GAMEPLAY_IMAGERY_QUALITY",
                match self.picture_quality {
                    PictureQuality::Low => "low",
                    PictureQuality::Medium => "medium",
                    PictureQuality::High => "high",
                }
                .to_owned(),
            ),
            (
                "DF_GAMEPLAY_IMAGERY_PROVIDER",
                match self.picture_provider {
                    PictureProvider::Openai => "openai",
                    PictureProvider::Flux => "flux",
                }
                .to_owned(),
            ),
            (
                "DF_GAMEPLAY_CREATOR_PORTRAITS",
                switch(self.creator_portraits),
            ),
            ("DF_GAMEPLAY_HOME_VIDEO", switch(self.home_video)),
            ("DF_GAMEPLAY_SCENE_VIDEO", switch(self.scene_video)),
            ("DF_GAMEPLAY_CHARACTER_VIDEO", switch(self.character_video)),
            ("DF_GAMEPLAY_DEV_CONTROL", switch(self.dev_control)),
        ]
    }
}

fn port(form: &Form, field: &'static str, errors: &mut Vec<FieldError>) -> Option<u16> {
    let value = form.get(field).unwrap_or("").trim();
    match value.parse::<u16>() {
        Ok(port) if port >= 1024 && value.bytes().all(|byte| byte.is_ascii_digit()) => Some(port),
        _ => {
            errors.push(FieldError {
                field,
                message: "Enter a port number from 1024 to 65535.".to_owned(),
            });
            None
        }
    }
}

fn choice<T: Copy>(
    form: &Form,
    field: &'static str,
    errors: &mut Vec<FieldError>,
    options: &[(&str, T)],
) -> Option<T> {
    let value = form.get(field).unwrap_or("");
    let found = options
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, option)| *option);
    if found.is_none() {
        errors.push(FieldError {
            field,
            message: "Choose one of the listed options.".to_owned(),
        });
    }
    found
}

/// Why a folder setting is refused, if it is. The build and PostgreSQL folders name programs
/// the launcher runs, so network (UNC) paths are refused for all folders.
pub fn folder_problem(value: &str) -> Option<&'static str> {
    if value.len() > 1024 || value.chars().any(char::is_control) {
        Some("Enter a folder path of up to 1,024 characters.")
    } else if value.starts_with("\\\\") || value.starts_with("//") {
        Some("Choose a folder on this PC; network folders are not allowed.")
    } else {
        None
    }
}

/// Setting names whose values differ, for the log (values themselves are not logged).
pub fn changed_names(before: &Settings, after: &Settings) -> Vec<String> {
    let (Ok(before), Ok(after)) = (toml::Table::try_from(before), toml::Table::try_from(after))
    else {
        return Vec::new();
    };
    after
        .iter()
        .filter(|(name, value)| before.get(*name) != Some(value))
        .map(|(name, _)| name.clone())
        .collect()
}

/// An external server is named by its origin only: `http://host:port`, no path or query.
pub fn external_origin(url: &str) -> Result<String, String> {
    let refused = || {
        "Enter the server address as http://host:port, for example http://127.0.0.1:63309."
            .to_owned()
    };
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(refused)?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty()
        || rest.len() > 255
        || rest.contains(['/', '?', '#', '@', ' ', '\\'])
        || rest.chars().any(char::is_control)
    {
        return Err(refused());
    }
    Ok(url.trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_form(overrides: &[(&str, &str)]) -> Form {
        let mut pairs = vec![
            ("phones_on_network", "on"),
            ("game_port", "63309"),
            ("database_port", "55517"),
            ("display", "window"),
            ("display_screen", "second"),
            ("full_screen", "on-second-screen"),
            ("content_rating", "mature"),
            ("storyteller", "on"),
            ("picture_quality", "medium"),
            ("picture_provider", "openai"),
            ("server_mode", "managed"),
        ];
        for (name, value) in overrides {
            pairs.retain(|(existing, _)| existing != name);
            pairs.push((name, value));
        }
        Form::from_pairs(&pairs)
    }

    #[test]
    fn defaults_keep_paid_video_and_portraits_off() {
        let settings = Settings::default();
        let environment = settings.feature_environment();
        let value = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(value("DF_GAMEPLAY_HOME_VIDEO"), Some("off"));
        assert_eq!(value("DF_GAMEPLAY_SCENE_VIDEO"), Some("off"));
        assert_eq!(value("DF_GAMEPLAY_CHARACTER_VIDEO"), Some("off"));
        assert_eq!(value("DF_GAMEPLAY_CREATOR_PORTRAITS"), Some("off"));
        assert_eq!(value("DF_GAMEPLAY_IMAGERY_PROVIDER"), Some("openai"));
        assert_eq!(value("DF_GAMEPLAY_DEV_CONTROL"), Some("off"));
        assert!(settings.phones_on_network);
        assert_eq!(settings.display, DisplayTarget::Window);
    }

    #[test]
    fn unchecked_boxes_turn_features_off_and_choices_are_read() {
        let base = Settings::default();
        let next = base
            .with_form(&full_form(&[
                ("content_rating", "restrained"),
                ("display", "browser"),
                ("picture_quality", "high"),
            ]))
            .unwrap();
        assert!(next.storyteller);
        assert!(!next.music);
        assert!(!next.scene_pictures);
        assert_eq!(next.content_rating, ContentRating::Restrained);
        assert_eq!(next.display, DisplayTarget::Browser);
        assert_eq!(next.picture_quality, PictureQuality::High);
    }

    #[test]
    fn bad_ports_and_unknown_choices_change_nothing() {
        let base = Settings::default();
        for overrides in [
            vec![("game_port", "80")],
            vec![("game_port", "70000")],
            vec![("game_port", "+6330")],
            vec![("database_port", "63309")],
            vec![("display", "projector")],
            vec![("server_mode", "external"), ("external_url", "ftp://x")],
            vec![
                ("server_mode", "external"),
                ("external_url", "http://h:1/path"),
            ],
        ] {
            assert!(
                base.with_form(&full_form(&overrides)).is_err(),
                "{overrides:?}"
            );
        }
    }

    #[test]
    fn folders_on_other_machines_are_refused() {
        assert!(folder_problem("").is_none());
        assert!(folder_problem("C:\\games\\build").is_none());
        for refused in ["\\\\server\\share", "//server/share", "a\nb"] {
            assert!(folder_problem(refused).is_some(), "{refused:?}");
        }
    }

    #[test]
    fn external_origin_accepts_only_bare_origins() {
        assert_eq!(
            external_origin("http://127.0.0.1:63309/").unwrap(),
            "http://127.0.0.1:63309"
        );
        assert!(external_origin("https://table.local").is_ok());
        for refused in [
            "",
            "127.0.0.1:1",
            "http://",
            "http://a b",
            "http://u@h",
            "http://h/x",
        ] {
            assert!(external_origin(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let settings = Settings {
            game_port: 64000,
            display: DisplayTarget::Manual,
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
        assert_eq!(toml::from_str::<Settings>("").unwrap(), Settings::default());
    }
}
