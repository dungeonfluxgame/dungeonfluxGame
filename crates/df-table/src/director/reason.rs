//! The reasons the Director focuses the camera, their order and their defaults (O.14, G1).
//!
//! A reason is a named cause with its own zoom, move time and way of ending, all in one table of
//! data (`ReasonTable`), never typed by the model. The order of the reasons is the order of O.14:
//! `hit`, `roll`, `reveal`, `active_creature`, `speaker`, `arrival`, `spotlight`, `requested`. A
//! lower reason never interrupts a higher one inside its hold; `wide` is not a focus and has no
//! place in that order (`WideReason` says why the camera pulls back).
//!
//! The numbers a table may hold are the numbers the dev camera verbs check (`df-playground`'s
//! `limits`: 100 to 10,000 percent, 0 to a day), written here once because this crate takes no
//! dependency on the page crate; the host's tests hold the two copies equal.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::director::error::DirectorError;
use crate::listed::listed_enum;

/// Least zoom of the camera, in percent: 100 is the whole stage (O.14 camera rule 3).
pub const MINIMUM_ZOOM_PERCENT: u16 = 100;
/// Most the camera may ask to zoom in, in percent: a large constant, never the effective ceiling,
/// which comes from the plate (`zoom_ceiling_percent`).
pub const MAXIMUM_ZOOM_PERCENT: u16 = 10_000;
/// Longest a camera move may take, in milliseconds (a day).
pub const MAXIMUM_CAMERA_MS: u64 = 86_400_000;
/// Shortest focus move a person can follow (O.14 camera rule 5), in milliseconds. A cut (0) is the
/// reduced-motion form and is always allowed.
pub(crate) const FOCUS_MOVE_MINIMUM_MS: u64 = 600;
/// Longest focus move a person can follow (O.14 camera rule 5), in milliseconds.
pub(crate) const FOCUS_MOVE_MAXIMUM_MS: u64 = 2_500;
/// How long a pull-back takes when nothing says otherwise, in milliseconds.
pub(crate) const DEFAULT_WIDE_MS: u64 = 1_200;

listed_enum! { keyed
    /// Why the Director focuses the camera, from the highest rank to the lowest.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Reason {
        /// An attack or spell lands on a hero.
        Hit,
        /// A death save or a contested check.
        Roll,
        /// A hidden object, trap or clue is found.
        Reveal,
        /// A turn starts in a scene with more than four figures.
        ActiveCreature,
        /// A non-player character speaks and has an anchor on the plate.
        Speaker,
        /// A figure enters or leaves.
        Arrival,
        /// The K.13 spotlight offer picks a quiet hero.
        Spotlight,
        /// A seat taps "look at me".
        Requested,
    }
}

impl Reason {
    /// The name in logs, verbs and data.
    pub fn key(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Roll => "roll",
            Self::Reveal => "reveal",
            Self::ActiveCreature => "active_creature",
            Self::Speaker => "speaker",
            Self::Arrival => "arrival",
            Self::Spotlight => "spotlight",
            Self::Requested => "requested",
        }
    }

    /// The place in the order of O.14: the larger outranks the smaller.
    pub fn rank(self) -> u8 {
        match self {
            Self::Hit => 8,
            Self::Roll => 7,
            Self::Reveal => 6,
            Self::ActiveCreature => 5,
            Self::Speaker => 4,
            Self::Arrival => 3,
            Self::Spotlight => 2,
            Self::Requested => 1,
        }
    }

    /// The keys of every reason, for a usage line.
    pub fn keys() -> String {
        Self::ALL
            .iter()
            .map(|reason| reason.key())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

listed_enum! {
    /// Why the camera pulls back to the whole stage.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum WideReason {
        /// `focus_hold_s` ran out, or the safety limit of a focus nobody ended.
        Timeout,
        /// The reason ended: the line's dwell, the turn, the result card.
        ReasonEnded,
        /// The target stopped being visible to the table. Outranks everything (rule 1).
        TargetHidden,
        /// The target left the stage.
        TargetGone,
        /// The plate swapped, or a card replaced it, and the target has no anchor on the new one.
        NoAnchor,
        /// One result named several heroes; the camera stays wide until the frame step exists.
        Area,
        /// A new scene starts wide.
        SceneStart,
        /// Combat starts wide.
        CombatStart,
        /// A person at the dev command line asked for it.
        Operator,
        /// A move could not be put on the screen, so the camera is pulled back from what it may
        /// still hold.
        ApplyFailed,
    }
}

impl WideReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::ReasonEnded => "reason_ended",
            Self::TargetHidden => "target_hidden",
            Self::TargetGone => "target_gone",
            Self::NoAnchor => "no_anchor",
            Self::Area => "area",
            Self::SceneStart => "scene_start",
            Self::CombatStart => "combat_start",
            Self::Operator => "operator",
            Self::ApplyFailed => "apply_failed",
        }
    }
}

/// How a focus ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ends {
    /// When the engine says the reason ended (the line's dwell, the turn, the result card); a
    /// focus nobody ends still returns at the safety limit.
    WhenReasonEnds,
    /// `focus_hold_s` after the focus starts (the reason is a moment, so it has ended already).
    AfterHold,
}

/// The default of one reason: a zoom and a move time a Director focus may use, and how it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ReasonSpecRecord", into = "ReasonSpecRecord")]
pub struct ReasonSpec {
    zoom_percent: u16,
    over_ms: u64,
    ends: Ends,
}

/// A `ReasonSpec` as written, read only through `ReasonSpec::new`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReasonSpecRecord {
    zoom_percent: u16,
    over_ms: u64,
    ends: Ends,
}

impl TryFrom<ReasonSpecRecord> for ReasonSpec {
    type Error = DirectorError;

    fn try_from(record: ReasonSpecRecord) -> Result<Self, Self::Error> {
        Self::new(record.zoom_percent, record.over_ms, record.ends)
    }
}

impl From<ReasonSpec> for ReasonSpecRecord {
    fn from(spec: ReasonSpec) -> Self {
        Self {
            zoom_percent: spec.zoom_percent,
            over_ms: spec.over_ms,
            ends: spec.ends,
        }
    }
}

impl ReasonSpec {
    /// A spec whose numbers are inside what a Director focus may use.
    pub fn new(zoom_percent: u16, over_ms: u64, ends: Ends) -> Result<Self, DirectorError> {
        check_zoom(zoom_percent)?;
        check_focus_move(over_ms)?;
        Ok(Self::unchecked(zoom_percent, over_ms, ends))
    }

    /// O.14's own numbers, which `default_specs_are_in_range` holds inside the checked ranges.
    const fn unchecked(zoom_percent: u16, over_ms: u64, ends: Ends) -> Self {
        Self {
            zoom_percent,
            over_ms,
            ends,
        }
    }

    pub fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    pub fn over_ms(&self) -> u64 {
        self.over_ms
    }

    pub fn ends(&self) -> Ends {
        self.ends
    }
}

/// A zoom from the whole stage up to the model's largest.
pub(crate) fn check_zoom(zoom_percent: u16) -> Result<(), DirectorError> {
    if (MINIMUM_ZOOM_PERCENT..=MAXIMUM_ZOOM_PERCENT).contains(&zoom_percent) {
        Ok(())
    } else {
        Err(DirectorError::ZoomOutOfRange { zoom_percent })
    }
}

/// The time of a focus move: a cut, or what a person can follow.
pub(crate) fn check_focus_move(over_ms: u64) -> Result<(), DirectorError> {
    if over_ms == 0 || (FOCUS_MOVE_MINIMUM_MS..=FOCUS_MOVE_MAXIMUM_MS).contains(&over_ms) {
        Ok(())
    } else {
        Err(DirectorError::FocusMoveOutOfRange { over_ms })
    }
}

/// The defaults of every reason, in data: a table can be built from a file later and refused with
/// a typed error where a number is outside its range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReasonTable {
    specs: [ReasonSpec; REASON_COUNT],
}

/// O.14's default of one reason.
fn default_spec(reason: Reason) -> ReasonSpec {
    match reason {
        Reason::Hit => ReasonSpec::unchecked(220, 700, Ends::WhenReasonEnds),
        Reason::Roll => ReasonSpec::unchecked(200, 900, Ends::WhenReasonEnds),
        Reason::Reveal => ReasonSpec::unchecked(250, 1_500, Ends::AfterHold),
        Reason::ActiveCreature => ReasonSpec::unchecked(160, 900, Ends::WhenReasonEnds),
        Reason::Speaker => ReasonSpec::unchecked(200, 1_200, Ends::WhenReasonEnds),
        Reason::Arrival => ReasonSpec::unchecked(180, 1_200, Ends::AfterHold),
        Reason::Spotlight => ReasonSpec::unchecked(180, 1_500, Ends::AfterHold),
        Reason::Requested => ReasonSpec::unchecked(180, 1_200, Ends::AfterHold),
    }
}

/// How many reasons there are, and so how many specs a table holds.
const REASON_COUNT: usize = Reason::ALL.len();

/// Where a reason's spec sits in the table.
const fn slot(reason: Reason) -> usize {
    match reason {
        Reason::Hit => 0,
        Reason::Roll => 1,
        Reason::Reveal => 2,
        Reason::ActiveCreature => 3,
        Reason::Speaker => 4,
        Reason::Arrival => 5,
        Reason::Spotlight => 6,
        Reason::Requested => 7,
    }
}

impl Default for ReasonTable {
    /// O.14's table.
    fn default() -> Self {
        let mut specs = [default_spec(Reason::Hit); REASON_COUNT];
        for reason in Reason::ALL {
            specs[slot(*reason)] = default_spec(*reason);
        }
        Self { specs }
    }
}

impl ReasonTable {
    pub fn spec(&self, reason: Reason) -> ReasonSpec {
        self.specs[slot(reason)]
    }

    /// Every spec the table holds, one for each reason.
    pub fn specs(&self) -> &[ReasonSpec] {
        &self.specs
    }

    /// The same table with one reason's defaults replaced; the spec is checked already.
    pub fn with_spec(mut self, reason: Reason, spec: ReasonSpec) -> Self {
        self.specs[slot(reason)] = spec;
        self
    }
}
