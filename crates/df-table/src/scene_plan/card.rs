//! The establishing card: what the table sees while a scene's picture is not ready (K.2).
//!
//! Waiting for art is never a gate. A scene is playable as soon as its description, cast and set
//! are stored, and after `scene_build_budget_s` play continues on this card: the place name, the
//! stored description voiced as the first narrator line, the party strip, the place music and a
//! small "picture on its way" tag. A background that arrives first shows behind it. The card is
//! built from what is stored and never invents a line: with no description yet it has no narrator
//! line, which is the place card of H.4.
//!
//! It owns the card's content (`EstablishingCard`), the tag as a closed key, and `CardWatch`, the
//! timing of when the card is due. It reads no clock (the caller hands the time in) and never
//! decides how the card is drawn.

use crate::plate::timing::{PlateTiming, SceneBuildKind};
use crate::private_data::Narration;
use crate::scene_plan::brief::SceneBrief;
use crate::scene_plan::{StageOutputs, StoredPicture};
use crate::table_state::{DisplayName, MediaName, TableSnapshot};

/// The tag on the card: a key, never player text, so the projection words it (K.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardTag {
    PictureOnItsWay,
}

impl CardTag {
    pub fn key(self) -> &'static str {
        match self {
            Self::PictureOnItsWay => "picture_on_its_way",
        }
    }
}

/// One hero on the party strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyChip {
    pub name: DisplayName,
    pub hit_points: u32,
    pub maximum_hit_points: u32,
}

/// What the card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstablishingCard {
    pub place: DisplayName,
    /// The stored description, voiced as the first narrator line; absent until it is stored.
    pub narrator_line: Option<Narration>,
    pub party: Vec<PartyChip>,
    pub place_music: Option<MediaName>,
    /// The background, once it has arrived, dissolving in behind the card.
    pub background: Option<StoredPicture>,
    pub tag: CardTag,
}

impl EstablishingCard {
    /// The card of `brief`'s scene from what is stored and the heroes in `snapshot`.
    pub fn compose(brief: &SceneBrief, stored: &StageOutputs, snapshot: &TableSnapshot) -> Self {
        let party = snapshot
            .heroes()
            .filter(|hero| hero.scene() == brief.scene())
            .map(|hero| PartyChip {
                name: hero.name().clone(),
                hit_points: hero.vitals().hit_points(),
                maximum_hit_points: hero.vitals().maximum_hit_points(),
            })
            .collect();
        Self {
            place: brief.place().clone(),
            narrator_line: stored
                .description
                .clone()
                .or_else(|| brief.description().cloned()),
            party,
            place_music: brief.place_music().cloned(),
            background: stored.background,
            tag: CardTag::PictureOnItsWay,
        }
    }
}

/// Where a scene's build stands against its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    /// Within the budget: the table waits for the picture.
    Building { due_in_ms: u64 },
    /// The budget passed and no plate is on screen: play continues on the card.
    Showing { late_by_ms: u64 },
    /// A plate is on screen: the card is done.
    Retired,
}

impl CardState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Building { .. } => "building",
            Self::Showing { .. } => "showing",
            Self::Retired => "retired",
        }
    }
}

/// When a scene's card is due: `scene_build_budget_s` after the build started, or the first-scene
/// hold for the first scene of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardWatch {
    started_ms: u64,
    budget_ms: u64,
}

impl CardWatch {
    pub fn new(started_ms: u64, kind: SceneBuildKind, timing: PlateTiming) -> Self {
        Self {
            started_ms,
            budget_ms: timing.build_budget_ms(kind),
        }
    }

    pub fn started_ms(self) -> u64 {
        self.started_ms
    }

    pub fn due_at_ms(self) -> u64 {
        self.started_ms.saturating_add(self.budget_ms)
    }

    /// The card's state at `now_ms`. `plate_on_screen` is whether the playout buffer shows a plate
    /// of this scene.
    pub fn state(self, now_ms: u64, plate_on_screen: bool) -> CardState {
        if plate_on_screen {
            return CardState::Retired;
        }
        match self.due_at_ms().checked_sub(now_ms) {
            Some(remaining) if remaining > 0 => CardState::Building {
                due_in_ms: remaining,
            },
            _ => CardState::Showing {
                late_by_ms: now_ms.saturating_sub(self.due_at_ms()),
            },
        }
    }
}
