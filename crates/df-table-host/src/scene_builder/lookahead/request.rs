//! What crosses the look-ahead dev route: the changes the verbs send and the answer they get.
//!
//! The route is `/dev/engine/scene/lookahead`: a GET reads the planner's report and a POST of a
//! `LookaheadRequest` changes it. Both answer a `SceneReply<LookaheadReply>`, like every scene dev
//! call. Nothing here holds player text: the place of a target is a short display name.

use df_table::lookahead::{Expectation, LookaheadReport};
use df_table::lookahead_settings::LookaheadSetting;
use serde::{Deserialize, Serialize};

use crate::scene_builder::request::{DevStage, SlowStage, TimedLine};

/// A look-ahead setting the route changes: `lookahead_enabled` and the four numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookaheadSettingName {
    /// `lookahead_enabled`; the value is 1 (on) or 0 (off).
    Enabled,
    /// `lookahead_depth`
    Depth,
    /// `lookahead_start_s`
    StartS,
    /// `lookahead_deadline_s`
    DeadlineS,
    /// `lookahead_book_age_s`
    BookAgeS,
}

impl LookaheadSettingName {
    pub const ALL: [Self; 5] = [
        Self::Enabled,
        Self::Depth,
        Self::StartS,
        Self::DeadlineS,
        Self::BookAgeS,
    ];

    /// The word the verb takes, as in `scene lookahead set depth 2`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Depth => "depth",
            Self::StartS => "start_s",
            Self::DeadlineS => "deadline_s",
            Self::BookAgeS => "book_age_s",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|name| name.key() == text)
    }

    /// The numeric setting this names, or `None` for `lookahead_enabled`.
    pub fn numeric(self) -> Option<LookaheadSetting> {
        match self {
            Self::Enabled => None,
            Self::Depth => Some(LookaheadSetting::Depth),
            Self::StartS => Some(LookaheadSetting::StartS),
            Self::DeadlineS => Some(LookaheadSetting::DeadlineS),
            Self::BookAgeS => Some(LookaheadSetting::BookAgeS),
        }
    }
}

/// One change of the look-ahead worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum LookaheadRequest {
    /// Stop starting new builds; builds that are out run on.
    Pause,
    Resume,
    /// The story expects no scene next: running builds are cancelled.
    Clear,
    /// Changes one `lookahead_*` setting; a value outside its range is refused naming the setting.
    Set {
        setting: LookaheadSettingName,
        value: u64,
    },
    /// The story expects the scene the next `scene build` makes, named `place`.
    Target {
        place: String,
        /// Why it is expected; a certain kind is built at once, a likely one after the start delay.
        #[serde(default = "authored_beat")]
        kind: Expectation,
        /// How likely, in thousandths, for a kind that is not certain.
        #[serde(default)]
        likelihood_permille: Option<u16>,
        /// The provider refuses this stage of the look-ahead build (describe or background).
        #[serde(default)]
        fail: Option<DevStage>,
        /// This stage of the look-ahead build takes that many seconds.
        #[serde(default)]
        slow: Option<SlowStage>,
    },
}

fn authored_beat() -> Expectation {
    Expectation::AuthoredBeat
}

/// The answer to every look-ahead verb: the planner as it stands and the audit lines the call made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookaheadReply {
    pub now_ms: u64,
    pub report: LookaheadReport,
    pub lines: Vec<TimedLine>,
}

/// The two stages a look-ahead build makes; a dev stage outside them is refused.
pub fn is_built_ahead(stage: DevStage) -> bool {
    matches!(stage, DevStage::Describe | DevStage::Background)
}
