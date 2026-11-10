//! A stage of a move's walk, recorded on the host: `POST /dev/world/walk` takes one note of a sight
//! on the way to a place and writes one audit line, `WORLD walk_stage place=.. stage=.. number=..
//! of=.. outcome=shown|late|failed at_ms=<playground clock> up_ms=<ms|->`. The picture itself is
//! the scene's (`PERSONA frame_timing .. reason=walk`); this line says which sight of which place
//! it was and how it ended.

use serde::{Deserialize, Serialize};

/// How a sight on the way ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalkOutcome {
    /// It was behind the scene.
    Shown,
    /// The place was ready before it, so it was passed over.
    Late,
    /// Making it failed.
    Failed,
}

impl WalkOutcome {
    pub const ALL: [Self; 3] = [Self::Shown, Self::Late, Self::Failed];

    pub fn key(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Late => "late",
            Self::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.key() == key)
    }
}

/// The body of `POST /dev/world/walk`: sight number `number` of `of` on the way to `place`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkStageNote {
    pub place: String,
    pub stage: String,
    pub number: usize,
    pub of: usize,
    pub outcome: WalkOutcome,
    /// How long it was behind the scene, for a sight that was shown and has been replaced.
    #[serde(default)]
    pub up_ms: Option<u64>,
}

/// What `POST /dev/world/walk` answers: when the host recorded it, on the playground's clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkNoted {
    pub at_ms: u64,
}
