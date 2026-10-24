//! Games started from the launcher, so Resume can list them by name and last-played time.
//! The game state itself lives in the game's PostgreSQL database; this list only names it.
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

/// Which adventure a game plays. Resuming must use the same one.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Adventure {
    /// The fixed harbor adventure.
    Harbor,
    /// A freshly rolled adventure.
    Random,
}

impl Adventure {
    pub fn server_value(self) -> &'static str {
        match self {
            Self::Harbor => "harbor",
            Self::Random => "random",
        }
    }

    /// For compact lists.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Harbor => "Harbor adventure",
            Self::Random => "Rolled adventure",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Harbor => "The harbor adventure",
            Self::Random => "A newly rolled adventure",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SavedGame {
    pub database: String,
    pub name: String,
    pub adventure: Adventure,
    /// Unix seconds.
    pub created: i64,
    /// Unix seconds of the last start or resume.
    pub last_played: i64,
    /// Seconds played across every session (REF-064). Games saved before this was kept start
    /// at zero.
    #[serde(default)]
    pub played_seconds: u64,
}

#[derive(Default, Deserialize, Serialize)]
struct SavedGames {
    #[serde(default, rename = "game")]
    games: Vec<SavedGame>,
}

const SAVES_FILE: &str = "saved-games.toml";

pub fn load(data: &Path) -> Result<Vec<SavedGame>, io::Error> {
    let text = match std::fs::read_to_string(data.join(SAVES_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut games = toml::from_str::<SavedGames>(&text)
        .map_err(|_| io::Error::other(format!("{} is damaged.", data.join(SAVES_FILE).display())))?
        .games;
    games.retain(|game| valid_database(&game.database));
    games.sort_by_key(|game| std::cmp::Reverse(game.last_played));
    Ok(games)
}

/// Adds the game or updates its last-played time and play time.
pub fn record(data: &Path, game: &SavedGame) -> Result<(), io::Error> {
    let mut games = load(data)?;
    match games
        .iter_mut()
        .find(|known| known.database == game.database)
    {
        Some(known) => {
            known.last_played = game.last_played;
            known.played_seconds = game.played_seconds;
        }
        None => games.push(game.clone()),
    }
    let text = toml::to_string_pretty(&SavedGames { games }).map_err(io::Error::other)?;
    crate::files::write_atomic(&data.join(SAVES_FILE), text.as_bytes())
}

/// The game after a session that began at its `last_played` and ended `now`: the session's
/// length is added to its play time and `now` becomes its last-played time.
pub fn after_session(game: &SavedGame, now: i64) -> SavedGame {
    let session = u64::try_from(now.saturating_sub(game.last_played)).unwrap_or(0);
    SavedGame {
        played_seconds: game.played_seconds.saturating_add(session),
        last_played: now,
        ..game.clone()
    }
}

/// The game server's database rule (see `run-gameplay.sh`): `df_gameplay_demo_` followed by
/// letters, digits, underscores or hyphens, at most 63 bytes.
pub fn valid_database(name: &str) -> bool {
    name.len() <= 63
        && name.len() > "df_gameplay_demo_".len()
        && name.starts_with("df_gameplay_demo_")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A fresh database name in the shell launcher's format, unique per start.
pub fn new_database(now: chrono::DateTime<chrono::Utc>, nonce: u32) -> String {
    format!(
        "df_gameplay_demo_play_{}_{:04x}",
        now.format("%Y%m%d%H%M%S"),
        nonce & 0xffff
    )
}

const QUALITIES: [&str; 24] = [
    "Amber",
    "Ashen",
    "Brazen",
    "Copper",
    "Drowned",
    "Ember",
    "Gilded",
    "Hollow",
    "Iron",
    "Lantern",
    "Lost",
    "Moonlit",
    "Oaken",
    "Pale",
    "Quiet",
    "Red",
    "Salt",
    "Silver",
    "Sunken",
    "Thorned",
    "Twice-Sworn",
    "Velvet",
    "Wandering",
    "Winter",
];
const THINGS: [&str; 24] = [
    "Anchor",
    "Bell",
    "Bargain",
    "Candle",
    "Crown",
    "Ferry",
    "Gate",
    "Harbor",
    "Hound",
    "Key",
    "Lantern Road",
    "Map",
    "Mill",
    "Oath",
    "Raven",
    "Reliquary",
    "Sigil",
    "Spire",
    "Tide",
    "Tower",
    "Vault",
    "Wake",
    "Well",
    "Wolf",
];

/// An automatic name such as "The Gilded Tide", never one already in use (REF-009).
pub fn new_name(seed: u64, existing: &[SavedGame]) -> String {
    let taken = |name: &str| existing.iter().any(|game| game.name == name);
    let pairs = QUALITIES.len() * THINGS.len();
    for step in 0..pairs {
        let index = (seed as usize).wrapping_add(step.wrapping_mul(7919)) % pairs;
        let quality = QUALITIES[index % QUALITIES.len()];
        let thing = THINGS[index / QUALITIES.len()];
        let name = format!("The {quality} {thing}");
        if !taken(&name) {
            return name;
        }
    }
    let mut number = 2;
    loop {
        let name = format!("The Gilded Tide {number}");
        if !taken(&name) {
            return name;
        }
        number += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(database: &str, name: &str, last_played: i64) -> SavedGame {
        SavedGame {
            database: database.to_owned(),
            name: name.to_owned(),
            adventure: Adventure::Harbor,
            created: 1,
            last_played,
            played_seconds: 0,
        }
    }

    #[test]
    fn database_names_follow_the_server_rule() {
        let now = chrono::DateTime::from_timestamp(1_792_800_000, 0).unwrap();
        let name = new_database(now, 0x1_beef);
        assert!(valid_database(&name), "{name}");
        assert!(name.ends_with("_beef"));
        for refused in [
            "df_gameplay_demo_",
            "other_x",
            "df_gameplay_demo_a;b",
            &"x".repeat(64),
        ] {
            assert!(!valid_database(refused), "{refused}");
        }
    }

    #[test]
    fn names_stay_unique_past_every_combination() {
        let mut existing = Vec::new();
        for seed in 0..600 {
            let name = new_name(seed, &existing);
            assert!(!existing.iter().any(|known: &SavedGame| known.name == name));
            assert!(name.starts_with("The "));
            existing.push(game(&format!("df_gameplay_demo_{seed}"), &name, 0));
        }
    }

    #[test]
    fn record_updates_last_played_and_lists_newest_first() {
        let data = std::env::temp_dir().join(format!("df-launcher-saves-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        record(&data, &game("df_gameplay_demo_a", "The Amber Bell", 10)).unwrap();
        record(&data, &game("df_gameplay_demo_b", "The Iron Gate", 20)).unwrap();
        record(&data, &game("df_gameplay_demo_a", "ignored rename", 30)).unwrap();
        let games = load(&data).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].name, "The Amber Bell");
        assert_eq!(games[0].last_played, 30);
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn a_session_adds_its_length_to_the_play_time_and_survives_a_save() {
        let data =
            std::env::temp_dir().join(format!("df-launcher-playtime-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let started = game("df_gameplay_demo_a", "The Amber Bell", 1_000);
        let ended = after_session(&started, 1_000 + 3_725);
        assert_eq!((ended.played_seconds, ended.last_played), (3_725, 4_725));
        let again = after_session(
            &SavedGame {
                last_played: 9_000,
                ..ended.clone()
            },
            9_060,
        );
        assert_eq!(again.played_seconds, 3_785);
        // A clock that went backwards adds nothing.
        assert_eq!(after_session(&again, 0).played_seconds, 3_785);
        record(&data, &started).unwrap();
        record(&data, &again).unwrap();
        assert_eq!(load(&data).unwrap()[0].played_seconds, 3_785);
        // A saves file written before play time was kept still loads, at zero.
        std::fs::write(
            data.join(SAVES_FILE),
            "[[game]]\ndatabase = \"df_gameplay_demo_old\"\nname = \"The Old Gate\"\n\
             adventure = \"harbor\"\ncreated = 1\nlast_played = 2\n",
        )
        .unwrap();
        assert_eq!(load(&data).unwrap()[0].played_seconds, 0);
        std::fs::remove_dir_all(data).unwrap();
    }
}
