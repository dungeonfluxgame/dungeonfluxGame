//! The tempo engine host's size and time bounds. Dev work: every bound only protects against a
//! mistake, so each is generous.

/// Most characters of an impulse's note. The note is shown by `tempo state` and never logged.
pub const MAXIMUM_NOTE_CHARACTERS: usize = 200;

/// Most recent impulses the run remembers for a profile's `recent` condition; the oldest goes
/// first.
pub const MAXIMUM_RECENT_IMPULSES: usize = 64;

/// Most profiles a table holds.
pub const MAXIMUM_PROFILES: usize = 64;

/// Most impulse kinds a table holds.
pub const MAXIMUM_IMPULSE_KINDS: usize = 64;

/// Most characters of a profile id or an impulse kind. A profile's track name adds a prefix and a
/// hash to its id and must still be a plain media name.
pub const MAXIMUM_ID_CHARACTERS: usize = 24;

/// Longest minimum dwell of a cue, in milliseconds: an hour.
pub const MAXIMUM_DWELL_MS: u64 = 3_600_000;

/// Longest window of a `recent` condition, in milliseconds: a day.
pub const MAXIMUM_WINDOW_MS: u64 = 86_400_000;

/// Longest crossfade of a cue, in milliseconds.
pub const MAXIMUM_CROSSFADE_MS: u32 = 60_000;

/// The largest permille: the whole range of an intensity.
pub const MAXIMUM_PERMILLE: u16 = 1_000;

/// Volume steps on each side of a track change. The page plays one track at a time, so a change
/// dips the old track in this many steps, swaps, and raises the new one in this many more.
pub const FADE_STEPS: u32 = 4;

/// Orders the conductor holds (track changes and wake-ups). Beyond it an order is dropped and the
/// next boundary decides again.
pub const ORDER_QUEUE_CAPACITY: usize = 16;
