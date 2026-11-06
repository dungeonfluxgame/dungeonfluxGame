//! What the Director records of each direction, and how it reads to an operator and to the table
//! (O.14 camera rule 9, G1).
//!
//! Every request becomes a line of the log (queued, applied or dropped, with its reason) and so
//! does every move the Director makes on its own (a return, a pull-back). `director show` reads
//! this same log and the audit lines are rendered from it, so the two cannot differ. The log is
//! bounded: the oldest entries are forgotten first.
//!
//! Redaction: the shared audience never learns that something exists. An entry names its element
//! to a reader other than the operator only when the table could see that element when the entry
//! was made (`Entry::element_visible`), whatever the reason of the entry, so a hidden or unknown
//! name never reaches a line, a report or `director show` (H.3, RES43). A drop for an unknown name
//! is listed as `target_hidden`. Only the operator sees the id.

use serde::{Deserialize, Serialize};

use crate::director::reason::{Reason, WideReason};
use crate::element::ElementId;
use crate::inspect::InspectAudience;
use crate::listed::listed_enum;

listed_enum! {
    /// Why a direction was set aside.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DropReason {
        /// A card shows, so there is no plate and no anchor.
        NoTarget,
        /// The name is not on the stage.
        UnknownTarget,
        /// The target is hidden from the party.
        TargetHidden,
        /// The target has no anchor on the plate now showing.
        NoAnchor,
        /// A higher reason holds the camera, or won the burst.
        LowerPriority,
        /// A later direction of the same or a higher reason replaced it before it started.
        Coalesced,
        /// A K.12 collapsed beat keeps the camera wide.
        CollapsedBeat,
        /// One result named several heroes; the camera stays wide.
        Area,
        /// The camera already holds this target for this reason; only the hold was renewed.
        AlreadyFocused,
        /// `director pause` holds every automatic direction.
        Paused,
        /// `director auto camera off`.
        AutoOff,
        /// The reason ended before the move began (the line was over, the turn passed).
        ReasonEnded,
        /// A new scene or combat started wide, so a waiting direction is void.
        SceneChanged,
    }
}

impl DropReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::NoTarget => "no_target",
            Self::UnknownTarget => "unknown_target",
            Self::TargetHidden => "target_hidden",
            Self::NoAnchor => "no_anchor",
            Self::LowerPriority => "lower_priority",
            Self::Coalesced => "coalesced",
            Self::CollapsedBeat => "collapsed_beat",
            Self::Area => "area",
            Self::AlreadyFocused => "already_focused",
            Self::Paused => "paused",
            Self::AutoOff => "auto_off",
            Self::ReasonEnded => "reason_ended",
            Self::SceneChanged => "scene_changed",
        }
    }
}

/// What a direction or a move of the Director was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum Cause {
    Focus(Reason),
    Wide(WideReason),
}

impl Cause {
    pub fn key(self) -> &'static str {
        match self {
            Self::Focus(reason) => reason.key(),
            Self::Wide(reason) => reason.key(),
        }
    }
}

/// What came of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// Waiting for the spacing, a card or a beat to pass.
    Queued,
    /// A move started. `clamped_from` is the zoom that was asked when the plate could not bear it.
    Applied {
        serial: u64,
        zoom_percent: u16,
        over_ms: u64,
        clamped_from: Option<u16>,
    },
    Dropped {
        why: DropReason,
    },
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub sequence: u64,
    pub at_ms: u64,
    pub cause: Cause,
    pub element: Option<ElementId>,
    /// Whether the whole table could see `element` when this was recorded. An entry made where the
    /// Director holds no view of the screen says `false`, so its id is withheld from every reader
    /// but the operator.
    pub element_visible: bool,
    pub outcome: Outcome,
    /// A person at the dev command line drove it (`driven=true`), not the engine.
    pub driven: bool,
}

impl Entry {
    /// The entry as the audience may read it: the id of an element the table could not see is
    /// withheld, and an unknown name reads as hidden, so a hosted build never says what exists.
    pub fn for_audience(&self, audience: InspectAudience) -> Self {
        let mut shown = self.clone();
        if audience.is_operator() {
            return shown;
        }
        if !self.element_visible {
            shown.element = None;
        }
        if let Outcome::Dropped { why } = &mut shown.outcome
            && *why == DropReason::UnknownTarget
        {
            *why = DropReason::TargetHidden;
        }
        shown
    }

    /// The audit lines of the entry (`CAMERA focus element=ID zoom=250 over=1500 reason=speaker
    /// serial=7`, `CAMERA wide reason=timeout serial=8`, and a `CAMERA clamped` line after a focus
    /// that was cut). One line for every entry except a clamped focus, which has two.
    pub fn lines(&self, audience: InspectAudience) -> Vec<String> {
        let shown = self.for_audience(audience);
        let driven = if shown.driven { " driven=true" } else { "" };
        let element = shown
            .element
            .as_ref()
            .map(|element| format!(" element={element}"))
            .unwrap_or_default();
        match (shown.cause, shown.outcome) {
            (
                Cause::Focus(reason),
                Outcome::Applied {
                    serial,
                    zoom_percent,
                    over_ms,
                    clamped_from,
                },
            ) => {
                let mut lines = vec![format!(
                    "CAMERA focus{element} zoom={zoom_percent} over={over_ms} reason={reason} \
                     serial={serial}{driven}"
                )];
                if let Some(requested) = clamped_from {
                    lines.push(format!(
                        "CAMERA clamped requested={requested} allowed={zoom_percent} serial={serial}"
                    ));
                }
                lines
            }
            (
                Cause::Wide(reason),
                Outcome::Applied {
                    serial, over_ms, ..
                },
            ) => vec![format!(
                "CAMERA wide reason={} serial={serial} over={over_ms}{driven}",
                reason.key()
            )],
            (cause, Outcome::Queued) => vec![format!(
                "CAMERA queued direction={}{element}{driven}",
                cause.key()
            )],
            (cause, Outcome::Dropped { why }) => vec![format!(
                "CAMERA dropped reason={} direction={}{element}{driven}",
                why.key(),
                cause.key()
            )],
        }
    }
}

/// How the focus now held will end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ends", content = "at_ms", rename_all = "snake_case")]
pub enum EndsReport {
    /// A time on the Director's clock.
    At(u64),
    /// When the engine says the reason ended; the clock value is the safety limit.
    WhenReasonEnds(u64),
}

/// The focus the camera holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveReport {
    pub element: Option<ElementId>,
    pub reason: Reason,
    pub serial: u64,
    pub zoom_percent: u16,
    pub since_ms: u64,
    pub ends: EndsReport,
}

/// The one direction waiting for its turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingReport {
    pub element: Option<ElementId>,
    pub reason: Reason,
    pub since_ms: u64,
}

/// What `director show` says: the directions queued, applied and dropped, with reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectorReport {
    pub audience: InspectAudience,
    pub now_ms: u64,
    pub paused: bool,
    pub auto_camera: bool,
    /// How many moves the Director has ever made.
    pub serial: u64,
    /// What the plate on screen can bear (O.14 decision for #1590); the model's largest when no
    /// plate is known.
    pub ceiling_percent: u16,
    pub active: Option<ActiveReport>,
    pub waiting: Option<WaitingReport>,
    pub queued: u64,
    pub applied: u64,
    pub dropped: u64,
    /// The newest entries, oldest first.
    pub log: Vec<Entry>,
}
