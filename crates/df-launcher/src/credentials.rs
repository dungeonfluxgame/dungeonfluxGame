//! Provider keys. Keys typed into the launcher live in the `[keys]` section of the central
//! configuration file (ignored by Git); a key missing there may come from the game folder's
//! ignored `.env` file, as the shell launcher reads it. Key values are never rendered or logged.
use std::{collections::BTreeMap, io, path::Path};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Credential {
    Openai,
    Elevenlabs,
    Fal,
    Jev,
    Bfl,
    Worldlabs,
}

impl Credential {
    pub const ALL: [Self; 6] = [
        Self::Openai,
        Self::Elevenlabs,
        Self::Fal,
        Self::Jev,
        Self::Bfl,
        Self::Worldlabs,
    ];

    /// The name in the form and in the configuration file's `[keys]` section.
    pub fn id(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Elevenlabs => "elevenlabs",
            Self::Fal => "fal",
            Self::Jev => "jev",
            Self::Bfl => "bfl",
            Self::Worldlabs => "worldlabs",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|credential| credential.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Openai => "OpenAI",
            Self::Elevenlabs => "ElevenLabs",
            Self::Fal => "fal",
            Self::Jev => "Jev (TypeSafe AI)",
            Self::Bfl => "Black Forest Labs",
            Self::Worldlabs => "World Labs",
        }
    }

    /// What the key is used for, in the player's terms.
    pub fn purpose(self) -> &'static str {
        match self {
            Self::Openai => "Storyteller, story memory, scene pictures and character portraits.",
            Self::Elevenlabs => "Narration voices, music and speaking to characters.",
            Self::Fal => "Generated video clips and the FLUX picture option.",
            Self::Jev => "Reading players' choices, and sorting portrait requests.",
            Self::Bfl => "The alternate portrait route for mature character appearances.",
            Self::Worldlabs => "Explorable 3D places.",
        }
    }

    /// Names accepted in `.env`, first match wins (matching `run-gameplay.sh`).
    fn env_file_names(self) -> &'static [&'static str] {
        match self {
            Self::Openai => &["OPENAI_API_KEY"],
            Self::Elevenlabs => &["ELEVENLABS_API_KEY"],
            Self::Fal => &["FAL_KEY", "DF_FAL_KEY"],
            Self::Jev => &["DF_JEV_API_KEY"],
            Self::Bfl => &["BFL_API_KEY"],
            Self::Worldlabs => &["WORLDLABS_API_KEY"],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    Config,
    EnvFile,
    Missing,
}

/// The resolved keys for one game start. Debug output never shows values.
pub struct Keys(BTreeMap<Credential, (String, Source)>);

impl std::fmt::Debug for Keys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Keys").finish_non_exhaustive()
    }
}

impl Keys {
    pub fn value(&self, credential: Credential) -> Option<&str> {
        self.0
            .get(&credential)
            .map(|(value, _)| value.as_str())
            .filter(|value| !value.is_empty())
    }

    pub fn source(&self, credential: Credential) -> Source {
        self.0
            .get(&credential)
            .map_or(Source::Missing, |(_, source)| *source)
    }
}

/// A key the provider would accept on a header: printable ASCII without spaces, bounded.
pub fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// Configured keys first, then `.env` when given. Unusable values are ignored rather than
/// passed on, exactly as the server would ignore them.
pub fn resolve(
    configured: &BTreeMap<Credential, String>,
    env_file: Option<&Path>,
) -> Result<Keys, io::Error> {
    let env_text = match env_file.map(std::fs::read_to_string) {
        Some(Ok(text)) => Some(text),
        Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => None,
    };
    let mut keys = BTreeMap::new();
    for credential in Credential::ALL {
        if let Some(value) = configured.get(&credential).filter(|v| valid_key(v)) {
            keys.insert(credential, (value.clone(), Source::Config));
            continue;
        }
        let from_file = env_text.as_deref().and_then(|text| {
            credential
                .env_file_names()
                .iter()
                .find_map(|name| env_value(text, name))
        });
        if let Some(value) = from_file {
            keys.insert(credential, (value, Source::EnvFile));
        }
    }
    Ok(Keys(keys))
}

/// The first `NAME=value` line, with surrounding quotes and a trailing CR removed (the same
/// reading `run-gameplay.sh` applies with sed).
fn env_value(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix(name)?.strip_prefix('=')?;
        let value: String = value
            .chars()
            .filter(|c| !matches!(c, '"' | '\'' | '\r'))
            .collect();
        valid_key(&value).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_values_match_the_shell_reading() {
        let text = "OPENAI_API_KEY=\"sk-test\"\r\nFAL_KEY=\nDF_FAL_KEY='fal-test'\nBAD=a b\n";
        assert_eq!(
            env_value(text, "OPENAI_API_KEY").as_deref(),
            Some("sk-test")
        );
        assert_eq!(env_value(text, "FAL_KEY"), None);
        assert_eq!(env_value(text, "DF_FAL_KEY").as_deref(), Some("fal-test"));
        assert_eq!(env_value(text, "BAD"), None);
        assert_eq!(env_value(text, "OPENAI_API"), None);
    }

    #[test]
    fn configured_keys_win_over_the_env_file() {
        let folder = std::env::temp_dir().join(format!("df-launcher-keys-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let env_file = folder.join(".env");
        std::fs::write(&env_file, "OPENAI_API_KEY=from-file\nDF_FAL_KEY=fal-file\n").unwrap();
        let configured = BTreeMap::from([
            (Credential::Openai, "configured".to_owned()),
            (Credential::Jev, "has space".to_owned()),
        ]);
        let keys = resolve(&configured, Some(&env_file)).unwrap();
        assert_eq!(keys.value(Credential::Openai), Some("configured"));
        assert_eq!(keys.source(Credential::Openai), Source::Config);
        assert_eq!(keys.value(Credential::Fal), Some("fal-file"));
        assert_eq!(keys.source(Credential::Jev), Source::Missing);
        let keys = resolve(&BTreeMap::new(), None).unwrap();
        assert_eq!(keys.source(Credential::Openai), Source::Missing);
        assert!(!format!("{keys:?}").contains("fal-file"));
        std::fs::remove_dir_all(folder).unwrap();
    }
}
