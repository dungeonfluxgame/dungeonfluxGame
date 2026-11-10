//! The time and margin constants of the check flow.

use df_playground::limits::{
    MAXIMUM_ROLL_TUMBLE_MS, MINIMUM_ROLL_TUMBLE_MS, ROLL_CALL_MS, ROLL_EXIT_MS, ROLL_SETTLE_MS,
};

/// The base of the time a spoken line of the dungeon master takes to be heard out; each character
/// adds `cast::routes::line_rest_ms`'s per-character rest (3 s plus 70 ms a character, the
/// narrator's timing in the runbook).
pub(super) const HEARD_OUT_BASE_MS: u64 = 3_000;

/// How long the card stays up after the narrator has finished reading the result.
pub(super) const HOLD_AFTER_LINE_MS: u64 = 2_000;

/// From the card's appearance to the latest moment its sum can be on screen: the call stage, the
/// longest tumble the page picks and the settle. The result line begins then, so the numbers it
/// reads are always visible.
pub(super) const RESULT_LEAD_MS: u64 = ROLL_CALL_MS + MAXIMUM_ROLL_TUMBLE_MS + ROLL_SETTLE_MS;

/// How much later than the earliest the sum can appear the result line may begin: the page picks
/// its tumble between the minimum and maximum, so the sum is on screen between those two moments.
pub(super) const RESULT_SLACK_MS: u64 = MAXIMUM_ROLL_TUMBLE_MS - MINIMUM_ROLL_TUMBLE_MS;

/// How long the card takes to leave once it begins to.
pub(super) const CARD_EXIT_MS: u64 = ROLL_EXIT_MS;

/// A result within this many points of the target was close.
pub(super) const NARROW_MARGIN: i32 = 2;

/// A result within this many points of the target (and not narrow) was clear but not far off.
pub(super) const CLEAR_MARGIN: i32 = 6;

/// How long a line of the dungeon master may take to be worded before it is said as its numbers
/// alone: a call of a model that answers in 5 to 6 s (the 2026-11-10 trial) and two asks again,
/// so a slow model never stalls a station.
pub(super) const LINE_BUDGET_MS: u64 = 14_000;

/// How fast a voice reads a line, in characters a second (the owner's pace for a station).
pub(super) const READ_CHARACTERS_PER_SECOND: u64 = 14;

/// The read time a call is worded for, in seconds, the game's own words included.
pub(super) const CALL_READ_TARGET_S: u64 = 10;

/// The read time a result is worded for, in seconds, the game's own words included.
pub(super) const RESULT_READ_TARGET_S: u64 = 12;

/// How many of the dungeon master's last flourishes are remembered across the run, so the next
/// does not repeat them or their closing phrases; the oldest is forgotten first.
pub(super) const MAXIMUM_REMEMBERED_FLOURISHES: usize = 16;

/// How many of those are told whole to the next flourish, as `said_before` (all of them, about
/// eight stations of a call and a result each, so a scene's stock image is not reused within a
/// scene); the closing sentences of all of them are told as `closers_used`.
pub(super) const FLOURISHES_TOLD_WHOLE: usize = 16;

/// How many times a line is asked of the model: a first ask and one more when the first answer is
/// refused for its content (a tone, a digit, a name, a third-person word). A repair that code can
/// make (a semicolon) is never an ask. A second refusal is not asked a third time, since each ask
/// is about 5 s of the line's budget.
pub(super) const ASKS_PER_LINE: u32 = 2;
