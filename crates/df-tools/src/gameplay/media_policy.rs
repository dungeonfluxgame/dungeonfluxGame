//! The generation gates: every paid capability the server may use, each allowed or refused by
//! one switch read once at startup (`Gates`), plus cost-sensitive defaults (explicit video
//! opt-in) and deterministic image routing. A generation site asks `gates()`, never the
//! environment, so turning a switch off stops every provider call that capability makes.
use std::{env::VarError, io, sync::OnceLock};

pub(super) fn enabled(input: Result<String, VarError>, name: &str) -> Result<bool, io::Error> {
    switch(input, name, false)
}

/// `on` or `off`; unset or empty takes `default`. Anything else is refused without echoing it.
fn switch(input: Result<String, VarError>, name: &str, default: bool) -> Result<bool, io::Error> {
    match input.as_deref() {
        Err(VarError::NotPresent) | Ok("") => Ok(default),
        Ok("on") => Ok(true),
        Ok("off") => Ok(false),
        _ => Err(io::Error::other(format!("{name} invalid"))),
    }
}

/// One paid capability and the switch that allows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Capability {
    /// The text model that narrates and voices characters (OpenAI).
    Storyteller,
    /// Story memory embeddings and summaries (OpenAI).
    Memory,
    /// Narration and character voices (ElevenLabs).
    Voice,
    /// Generated place music (ElevenLabs), and Jev's planning of it.
    Music,
    /// Speaking to characters through a phone (ElevenLabs).
    SpeechInput,
    /// Scene pictures and the dev photo check (OpenAI, or FLUX through fal).
    Pictures,
    /// Composed scene plates, made with the pictures.
    Plates,
    /// Character creator portraits (their own OpenAI, Jev and Black Forest Labs keys).
    CreatorPortraits,
    /// The home screen's looping video (fal).
    HomeVideo,
    /// Story scene videos (fal).
    SceneVideo,
    /// Character clips and the frames made for them (OpenAI frames, fal video).
    CharacterVideo,
    /// Jev reading players' lines and foreseeing fights (TypeSafe AI).
    Jev,
    /// 3D places generated as splat worlds (World Labs).
    SplatWorlds,
}

impl Capability {
    pub(super) const ALL: [Self; 13] = [
        Self::Storyteller,
        Self::Memory,
        Self::Voice,
        Self::Music,
        Self::SpeechInput,
        Self::Pictures,
        Self::Plates,
        Self::CreatorPortraits,
        Self::HomeVideo,
        Self::SceneVideo,
        Self::CharacterVideo,
        Self::Jev,
        Self::SplatWorlds,
    ];

    pub(super) fn switch_name(self) -> &'static str {
        match self {
            Self::Storyteller => "DF_GAMEPLAY_STORYTELLER",
            Self::Memory => "DF_GAMEPLAY_MEMORY",
            Self::Voice => "DF_GAMEPLAY_VOICE",
            Self::Music => "DF_GAMEPLAY_MUSIC",
            Self::SpeechInput => "DF_GAMEPLAY_STT",
            Self::Pictures => "DF_GAMEPLAY_IMAGERY",
            Self::Plates => "DF_GAMEPLAY_PLATES",
            Self::CreatorPortraits => "DF_GAMEPLAY_CREATOR_PORTRAITS",
            Self::HomeVideo => "DF_GAMEPLAY_HOME_VIDEO",
            Self::SceneVideo => "DF_GAMEPLAY_SCENE_VIDEO",
            Self::CharacterVideo => "DF_GAMEPLAY_CHARACTER_VIDEO",
            Self::Jev => "DF_GAMEPLAY_JEV",
            Self::SplatWorlds => "DF_GAMEPLAY_SPLAT_WORLDS",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Storyteller => "storyteller",
            Self::Memory => "memory",
            Self::Voice => "voice",
            Self::Music => "music",
            Self::SpeechInput => "speech_input",
            Self::Pictures => "pictures",
            Self::Plates => "plates",
            Self::CreatorPortraits => "creator_portraits",
            Self::HomeVideo => "home_video",
            Self::SceneVideo => "scene_video",
            Self::CharacterVideo => "character_video",
            Self::Jev => "jev",
            Self::SplatWorlds => "splat_worlds",
        }
    }

    /// Unset switches: video and creator portraits need an explicit `on`; the rest stay on as
    /// before (each still needs its credential).
    fn default_on(self) -> bool {
        !matches!(
            self,
            Self::CreatorPortraits | Self::HomeVideo | Self::SceneVideo | Self::CharacterVideo
        )
    }
}

/// Which capabilities this server run may use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Gates {
    allowed: [bool; Capability::ALL.len()],
}

impl Gates {
    /// Nothing allowed: what a run gets if the switches were never read.
    const NONE: Self = Self {
        allowed: [false; Capability::ALL.len()],
    };

    pub(super) fn from_environment() -> Result<Self, io::Error> {
        Self::from_lookup(|name| std::env::var(name))
    }

    fn from_lookup(lookup: impl Fn(&str) -> Result<String, VarError>) -> Result<Self, io::Error> {
        let mut gates = Self::NONE;
        for (index, capability) in Capability::ALL.into_iter().enumerate() {
            let name = capability.switch_name();
            gates.allowed[index] = switch(lookup(name), name, capability.default_on())?;
        }
        Ok(gates)
    }

    pub(super) fn allows(self, capability: Capability) -> bool {
        Capability::ALL
            .iter()
            .position(|known| *known == capability)
            .is_some_and(|index| self.allowed[index])
    }

    /// The switch as a site's own parser reads it, so its startup line keeps naming the switch.
    pub(super) fn input(self, capability: Capability) -> Result<String, VarError> {
        Ok(if self.allows(capability) { "on" } else { "off" }.to_owned())
    }

    /// Every capability as `(name, switch, allowed)`, for `GET /dev/gates`.
    pub(super) fn report(self) -> Vec<(&'static str, &'static str, bool)> {
        Capability::ALL
            .into_iter()
            .map(|capability| {
                (
                    capability.label(),
                    capability.switch_name(),
                    self.allows(capability),
                )
            })
            .collect()
    }

    /// `GATES storyteller=on ... splat_worlds=off`: the run's whole generation allowance.
    pub(super) fn startup_line(self) -> String {
        let mut line = "GATES".to_owned();
        for capability in Capability::ALL {
            let state = if self.allows(capability) { "on" } else { "off" };
            line.push_str(&format!(" {}={state}", capability.label()));
        }
        line
    }
}

static INSTALLED: OnceLock<Gates> = OnceLock::new();

/// Fixes this process's gates; later calls keep the first.
pub(super) fn install(gates: Gates) {
    let _ = INSTALLED.set(gates);
}

/// This run's gates. Never installed (a path that skipped startup) reads the environment, and
/// an invalid switch there allows nothing.
pub(super) fn gates() -> Gates {
    *INSTALLED.get_or_init(|| Gates::from_environment().unwrap_or(Gates::NONE))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageProvider {
    OpenAi,
    Flux,
}

pub(super) fn image_provider(
    input: Result<String, VarError>,
    has_fal: bool,
) -> Result<ImageProvider, io::Error> {
    match input.as_deref() {
        Err(VarError::NotPresent) | Ok("" | "openai") => Ok(ImageProvider::OpenAi),
        Ok("flux") if has_fal => Ok(ImageProvider::Flux),
        Ok("flux") => Err(io::Error::other(
            "DF_GAMEPLAY_IMAGERY_PROVIDER=flux needs FAL_KEY",
        )),
        _ => Err(io::Error::other("DF_GAMEPLAY_IMAGERY_PROVIDER refused")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_defaults_to_openai_regardless_of_fal_and_explicit_flux_needs_its_key() {
        for has_fal in [false, true] {
            assert_eq!(
                image_provider(Err(VarError::NotPresent), has_fal).unwrap(),
                ImageProvider::OpenAi
            );
            for choice in ["", "openai"] {
                assert_eq!(
                    image_provider(Ok(choice.to_owned()), has_fal).unwrap(),
                    ImageProvider::OpenAi
                );
            }
        }
        assert_eq!(
            image_provider(Ok("flux".to_owned()), true).unwrap(),
            ImageProvider::Flux
        );
        assert!(image_provider(Ok("flux".to_owned()), false).is_err());
        for choice in ["OpenAI", "openai ", "private-secret"] {
            let error = image_provider(Ok(choice.to_owned()), true).unwrap_err();
            assert!(!error.to_string().contains(choice));
        }
        assert!(image_provider(Err(VarError::NotUnicode("private-secret".into())), true).is_err());
    }

    #[test]
    fn credentials_cannot_enable_video_without_an_explicit_switch() {
        for name in [
            "DF_GAMEPLAY_HOME_VIDEO",
            "DF_GAMEPLAY_SCENE_VIDEO",
            "DF_GAMEPLAY_CHARACTER_VIDEO",
        ] {
            assert!(!enabled(Err(VarError::NotPresent), name).unwrap());
            for value in ["", "off"] {
                assert!(!enabled(Ok(value.to_owned()), name).unwrap());
            }
            assert!(enabled(Ok("on".to_owned()), name).unwrap());
        }
    }
    #[test]
    fn invalid_video_flags_are_refused_without_echoing_values() {
        for value in ["ON", "on ", "true", "private-secret"] {
            let error = enabled(Ok(value.to_owned()), "DF_GAMEPLAY_SCENE_VIDEO").unwrap_err();
            assert!(!error.to_string().contains(value));
        }
        assert!(
            enabled(
                Err(VarError::NotUnicode("private-secret".into())),
                "DF_GAMEPLAY_HOME_VIDEO"
            )
            .is_err()
        );
    }

    #[test]
    fn gates_default_to_on_except_video_and_creator_portraits() {
        let gates = Gates::from_lookup(|_| Err(VarError::NotPresent)).unwrap();
        for capability in Capability::ALL {
            let expected = !matches!(
                capability,
                Capability::CreatorPortraits
                    | Capability::HomeVideo
                    | Capability::SceneVideo
                    | Capability::CharacterVideo
            );
            assert_eq!(gates.allows(capability), expected, "{capability:?}");
        }
    }

    #[test]
    fn every_switch_off_allows_nothing_and_says_so() {
        let gates = Gates::from_lookup(|_| Ok("off".to_owned())).unwrap();
        assert_eq!(gates, Gates::NONE);
        for capability in Capability::ALL {
            assert!(!gates.allows(capability));
            assert_eq!(gates.input(capability), Ok("off".to_owned()));
        }
        let line = gates.startup_line();
        assert!(line.starts_with("GATES storyteller=off "));
        assert!(line.ends_with(" jev=off splat_worlds=off"));
        assert!(!line.contains("=on"));
    }

    #[test]
    fn each_switch_turns_only_its_capability_off() {
        for target in Capability::ALL {
            let gates = Gates::from_lookup(|name| {
                Ok(if name == target.switch_name() {
                    "off"
                } else {
                    "on"
                }
                .to_owned())
            })
            .unwrap();
            for capability in Capability::ALL {
                assert_eq!(gates.allows(capability), capability != target);
            }
        }
    }

    #[test]
    fn the_report_names_every_capability_with_its_switch() {
        let gates = Gates::from_lookup(|name| {
            Ok(if name == "DF_GAMEPLAY_JEV" {
                "off"
            } else {
                "on"
            }
            .to_owned())
        })
        .unwrap();
        let report = gates.report();
        assert_eq!(report.len(), Capability::ALL.len());
        assert!(report.contains(&("jev", "DF_GAMEPLAY_JEV", false)));
        assert!(report.contains(&("storyteller", "DF_GAMEPLAY_STORYTELLER", true)));
    }

    #[test]
    fn an_invalid_switch_refuses_startup_without_echoing_it() {
        for value in ["ON", "true", "private-secret"] {
            let error = Gates::from_lookup(|name| {
                Ok(if name == "DF_GAMEPLAY_JEV" {
                    value
                } else {
                    "on"
                }
                .to_owned())
            })
            .unwrap_err();
            assert_eq!(error.to_string(), "DF_GAMEPLAY_JEV invalid");
        }
    }
}
