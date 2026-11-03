//! The expected next scenes of the look-ahead and the ranked candidates made from them (SB-LA, idea 1
//! of "Ideas to come"): why a scene is expected (`Expectation`), the machine's statement of it
//! (`Expected`), the candidate with the stamp of the facts a build would use, the typed error of the
//! planner, and `lookahead_candidates`, which ranks them. The planner that builds them is in
//! `lookahead`, which re-exports everything here.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::identity::SceneId;
use crate::listed::listed_enum;
use crate::lookahead_stamp::WorldStamp;
use crate::scene_plan_brief::SceneBrief;
use crate::table_state::TableSnapshot;

/// Most scenes the story may name as expected at once. Deliberately huge (owner, 2026-11-02).
pub const MAXIMUM_EXPECTED_SCENES: usize = 1_000_000;
/// The likelihood of a certain scene, in thousandths.
pub const CERTAIN_PERMILLE: u16 = 1_000;

listed_enum! {
    /// Why a scene is expected, in order of certainty (idea 1, "What it predicts").
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Expectation {
        /// The next scene of an authored beat script.
        AuthoredBeat,
        /// The destination of a travel leg that is committed.
        TravelLeg,
        /// An exit of the current place the narration hinted at or the party stated.
        ExitHint,
    }
}

impl Expectation {
    pub fn key(self) -> &'static str {
        match self {
            Self::AuthoredBeat => "authored_beat",
            Self::TravelLeg => "travel_leg",
            Self::ExitHint => "exit_hint",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|value| value.key() == text)
    }

    /// Whether the scene will be entered whatever the party does next.
    pub fn is_certain(self) -> bool {
        matches!(self, Self::AuthoredBeat | Self::TravelLeg)
    }
}

/// A scene the story expects next, as the machine that knows it states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    pub brief: SceneBrief,
    pub expectation: Expectation,
    /// How likely, in thousandths; a certain expectation is always `CERTAIN_PERMILLE`.
    pub likelihood_permille: u16,
}

/// An expected scene with the stamp of the facts a build of it would use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    brief: SceneBrief,
    expectation: Expectation,
    likelihood_permille: u16,
    stamp: WorldStamp,
}

impl Candidate {
    pub fn scene(&self) -> SceneId {
        self.brief.scene()
    }

    pub fn brief(&self) -> &SceneBrief {
        &self.brief
    }

    pub fn expectation(&self) -> Expectation {
        self.expectation
    }

    pub fn likelihood_permille(&self) -> u16 {
        self.likelihood_permille
    }

    pub fn stamp(&self) -> WorldStamp {
        self.stamp
    }

    pub fn is_certain(&self) -> bool {
        self.expectation.is_certain()
    }
}

/// Why the planner refused a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookaheadError {
    /// A likelihood above 1,000 thousandths.
    LikelihoodOutOfRange {
        scene: SceneId,
        permille: u16,
    },
    TooManyExpected {
        given: usize,
    },
    /// A result for a scene that has no build running.
    NoBuildOf {
        scene: SceneId,
    },
    /// A result for a build that was cancelled, retargeted or replaced: it is not stored.
    StaleBuild {
        scene: SceneId,
    },
    /// The book already holds the most scenes it may.
    BookFull {
        maximum: usize,
    },
}

impl fmt::Display for LookaheadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LikelihoodOutOfRange { scene, permille } => write!(
                formatter,
                "the likelihood of scene {scene} is 0 to {CERTAIN_PERMILLE} thousandths, not {permille}"
            ),
            Self::TooManyExpected { given } => write!(
                formatter,
                "{given} expected scenes is more than the {MAXIMUM_EXPECTED_SCENES} the worker holds"
            ),
            Self::NoBuildOf { scene } => write!(formatter, "no build of scene {scene} is running"),
            Self::StaleBuild { scene } => write!(
                formatter,
                "the build of scene {scene} was cancelled, so its result is not kept"
            ),
            Self::BookFull { maximum } => {
                write!(
                    formatter,
                    "the book of built scenes holds at most {maximum}"
                )
            }
        }
    }
}

impl std::error::Error for LookaheadError {}

/// Turns what the story expects into ranked candidates: the current scene is left out, a scene named
/// twice keeps its more certain entry, and the order is certain first, then by likelihood, then by
/// scene id. Each candidate is stamped with the facts a build would use.
pub fn lookahead_candidates(
    snapshot: &TableSnapshot,
    current: Option<SceneId>,
    expected: Vec<Expected>,
) -> Result<Vec<Candidate>, LookaheadError> {
    if expected.len() > MAXIMUM_EXPECTED_SCENES {
        return Err(LookaheadError::TooManyExpected {
            given: expected.len(),
        });
    }
    let mut by_scene: BTreeMap<SceneId, Candidate> = BTreeMap::new();
    for entry in expected {
        let scene = entry.brief.scene();
        let permille = if entry.expectation.is_certain() {
            CERTAIN_PERMILLE
        } else {
            entry.likelihood_permille
        };
        if permille > CERTAIN_PERMILLE {
            return Err(LookaheadError::LikelihoodOutOfRange { scene, permille });
        }
        if current == Some(scene) {
            continue;
        }
        let candidate = Candidate {
            stamp: WorldStamp::of(snapshot, &entry.brief),
            brief: entry.brief,
            expectation: entry.expectation,
            likelihood_permille: permille,
        };
        let better = by_scene.get(&scene).is_none_or(|kept| {
            (candidate.is_certain(), candidate.likelihood_permille)
                > (kept.is_certain(), kept.likelihood_permille)
        });
        if better {
            by_scene.insert(scene, candidate);
        }
    }
    let mut ranked: Vec<Candidate> = by_scene.into_values().collect();
    ranked.sort_by_key(|candidate| {
        (
            Reverse(candidate.is_certain()),
            Reverse(candidate.likelihood_permille),
            candidate.scene(),
        )
    });
    Ok(ranked)
}
