//! The music engine's size and time bounds. A bound tied to the supplier's contract says so.

use std::time::Duration;

use df_providers::{MUSIC_MAXIMUM_LENGTH_MS, MUSIC_MINIMUM_LENGTH_MS};

/// Length of a composed track when none is asked, in seconds: two minutes, the owner's aim for
/// one scene's background music.
pub const DEFAULT_LENGTH_SECONDS: u32 = 120;

/// Shortest composition, in seconds. The supplier's contract (`/v1/music` takes 3 s to 10 min).
pub const MINIMUM_LENGTH_SECONDS: u32 = MUSIC_MINIMUM_LENGTH_MS / 1_000;

/// Longest composition, in seconds. The supplier's contract.
pub const MAXIMUM_LENGTH_SECONDS: u32 = MUSIC_MAXIMUM_LENGTH_MS / 1_000;

/// Most characters of a prompt. The supplier's contract.
pub use df_providers::MUSIC_MAXIMUM_PROMPT_CHARACTERS as MAXIMUM_PROMPT_CHARACTERS;

/// The time one composition may take, the answer's audio included. Generous: a ten-minute track
/// takes the supplier a few minutes.
pub const COMPOSE_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// What a composed track's file name adds to its name: the supplier answers MP3.
pub const COMPOSED_EXTENSION: &str = "mp3";
