//! The director's choice of cue at one boundary: pure, over the tempo, the table and which tracks
//! are ready, so a test drives it with numbers.
//!
//! The tempo wants the first profile of the table whose `when` holds (the base when none does).
//! What plays changes to it only when all of these hold:
//!
//! - the playing profile no longer applies: it does not match even with the table's hold margin
//!   widening its intensity bounds, or a profile before it in the table now matches strictly (a
//!   profile earlier in the table outranks one later);
//! - the playing profile has played its minimum dwell;
//! - the wanted profile's track is ready. A profile that is not is skipped, never awaited.
//!
//! The engine's own hysteresis already keeps the phase from flickering at a threshold; the margin
//! and the dwell keep the music from following every wobble of the intensity.

use super::request::{DecisionReason, Momentum};
use super::run::Moment;
use super::table::{Condition, MusicTable};

/// What plays, as the director decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Playing {
    /// The profile's place in the table.
    pub(super) index: usize,
    /// The run's time the cue began at.
    pub(super) since_ms: u64,
}

/// What to do at a boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Keep(DecisionReason),
    Switch(DecisionReason),
    /// A change is wanted and may happen from this time of the run.
    Hold {
        until_ms: u64,
    },
    Skip,
}

/// The director's decision: the profile the tempo wants and what to do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Decision {
    pub(super) wanted: usize,
    pub(super) choice: Choice,
}

/// The profile the tempo wants now: the first whose `when` holds, else the base.
pub(super) fn wanted(table: &MusicTable, moment: &Moment<'_>) -> usize {
    table
        .profiles
        .iter()
        .position(|profile| applies(&profile.when, moment, 0))
        .unwrap_or_else(|| table.base_index())
}

/// Decides at a boundary. `ready[i]` is whether the track of profile `i` is in the library.
pub(super) fn decide(
    table: &MusicTable,
    moment: &Moment<'_>,
    playing: Option<Playing>,
    ready: &[bool],
) -> Decision {
    let wanted = wanted(table, moment);
    let is_ready = ready.get(wanted).copied().unwrap_or(false);
    let Some(playing) = playing else {
        let choice = if is_ready {
            Choice::Switch(DecisionReason::Start)
        } else {
            Choice::Skip
        };
        return Decision { wanted, choice };
    };
    let choice = if playing.index == wanted {
        Choice::Keep(DecisionReason::Wanted)
    } else if playing.index < wanted && holds(table, moment, playing.index) {
        Choice::Keep(DecisionReason::Margin)
    } else if moment.now_ms < playing.since_ms.saturating_add(table.director.min_dwell_ms) {
        Choice::Hold {
            until_ms: playing.since_ms.saturating_add(table.director.min_dwell_ms),
        }
    } else if is_ready {
        Choice::Switch(DecisionReason::Changed)
    } else {
        Choice::Skip
    };
    Decision { wanted, choice }
}

/// Whether the playing profile still applies within the table's hold margin.
fn holds(table: &MusicTable, moment: &Moment<'_>, index: usize) -> bool {
    table
        .profiles
        .get(index)
        .is_some_and(|profile| applies(&profile.when, moment, table.director.hold_margin_permille))
}

/// Whether `when` holds at `moment`, the intensity bounds widened by `margin` permille.
fn applies(when: &Condition, moment: &Moment<'_>, margin: u16) -> bool {
    let intensity = moment.intensity;
    when.phases
        .as_ref()
        .is_none_or(|phases| phases.contains(&moment.phase))
        && when
            .intensity_min
            .is_none_or(|low| intensity.saturating_add(margin) >= low)
        && when
            .intensity_max
            .is_none_or(|high| intensity <= high.saturating_add(margin))
        && when
            .momentum
            .is_none_or(|momentum: Momentum| momentum == moment.momentum)
        && when.recent.as_ref().is_none_or(|recent| {
            moment.recent.iter().any(|impulse| {
                recent.kinds.contains(&impulse.kind)
                    && moment.now_ms.saturating_sub(impulse.at_ms) <= recent.within_ms
            })
        })
}
