//! The action book: which rules entry each action name runs (O.5: an action is keyed to a rules
//! entry, and an action without one is `not_supported_yet`).
//!
//! It owns the closed set of entries the adapter can run today (a skill check with an authored
//! difficulty, an influence urging with its authored setup), the closed list of rules pieces that
//! are known to be missing (RES31, O.16 RR table) and the book that maps `ActionId`s to them. It
//! never rolls and never decides a difficulty: the authored beat or conversation sets the DC, and
//! the SRD says the GM sets it (page 6). It fails with `AdapterError` when an entry or a duplicate
//! registration is invalid.

use std::collections::BTreeMap;
use std::num::NonZeroU8;

use df_rules::influence::{
    Attitude, InfluenceSkill, RollMode, Willingness, WordQuality, WordTilt, attitude_source,
    combine_sources, word_source,
};
use df_table::ports::ActionId;

use super::error::AdapterError;
use super::hero_profile::Skill;

/// A difficulty class, which is at least 1 (`df-rules` refuses zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DifficultyClass(NonZeroU8);

impl DifficultyClass {
    /// DC 15, "Medium" in the SRD 5.2.1 Typical Difficulty Classes table (page 6).
    pub const MEDIUM: Self = Self(NonZeroU8::MIN.saturating_add(14));

    pub fn new(value: u8) -> Result<Self, AdapterError> {
        NonZeroU8::new(value)
            .map(Self)
            .ok_or(AdapterError::DifficultyClassIsZero)
    }

    pub fn get(self) -> u8 {
        self.0.get()
    }
}

/// The rules that are known not to exist yet, each named so a decline says what is missing (RES31).
/// The list is read from the RR readiness table (O.16); a piece leaves it when its rules land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissingPiece {
    UnregisteredAction,
    Spellcasting,
    ReadyAction,
    Hide,
    Search,
    Study,
    GrappleAndShove,
    TravelLeg,
    HazardsTrapsAndFalling,
    CellLight,
    Exhaustion,
    Identify,
    PurseAndStock,
    LevelFromExperience,
    FleeAndSurrender,
    RestRecovery,
    BattleState,
}

impl MissingPiece {
    pub const ALL: [MissingPiece; 17] = [
        Self::UnregisteredAction,
        Self::Spellcasting,
        Self::ReadyAction,
        Self::Hide,
        Self::Search,
        Self::Study,
        Self::GrappleAndShove,
        Self::TravelLeg,
        Self::HazardsTrapsAndFalling,
        Self::CellLight,
        Self::Exhaustion,
        Self::Identify,
        Self::PurseAndStock,
        Self::LevelFromExperience,
        Self::FleeAndSurrender,
        Self::RestRecovery,
        Self::BattleState,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::UnregisteredAction => "unregistered_action",
            Self::Spellcasting => "spellcasting",
            Self::ReadyAction => "ready_action",
            Self::Hide => "hide",
            Self::Search => "search",
            Self::Study => "study",
            Self::GrappleAndShove => "grapple_and_shove",
            Self::TravelLeg => "travel_leg",
            Self::HazardsTrapsAndFalling => "hazards_traps_and_falling",
            Self::CellLight => "cell_light",
            Self::Exhaustion => "exhaustion",
            Self::Identify => "identify",
            Self::PurseAndStock => "purse_and_stock",
            Self::LevelFromExperience => "level_from_experience",
            Self::FleeAndSurrender => "flee_and_surrender",
            Self::RestRecovery => "rest_recovery",
            Self::BattleState => "battle_state",
        }
    }

    /// What is missing, in a sentence a developer reads in a decline.
    pub fn describe(self) -> &'static str {
        match self {
            Self::UnregisteredAction => "a rules entry for this action in the action book",
            Self::Spellcasting => "spellcasting rules (df-rules coverage: spellcasting is No)",
            Self::ReadyAction => "the Ready action (no reaction trigger rules)",
            Self::Hide => "the Hide action (no hiding or Stealth contest rules)",
            Self::Search => "the Search action (no hidden-object or passive check rules)",
            Self::Study => "the Study action (no lore check rules)",
            Self::GrappleAndShove => "grapple and shove rules",
            Self::TravelLeg => {
                "travel rules: pace, terrain, forage and navigation (no df-rules travel module)"
            }
            Self::HazardsTrapsAndFalling => "hazard, trap and falling rules",
            Self::CellLight => "per-cell light and vision rules",
            Self::Exhaustion => "Exhaustion rules",
            Self::Identify => "an item identify step and an item catalog beyond weapons",
            Self::PurseAndStock => {
                "a purse and shop stock held by the table, and an atomic item-and-coin move"
            }
            Self::LevelFromExperience => "a level-from-experience table (SRD page 23)",
            Self::FleeAndSurrender => "flee and surrender outcomes",
            Self::RestRecovery => {
                "hero hit points, Hit Dice and Second Wind uses held by the table, and a standalone rest"
            }
            Self::BattleState => {
                "a tactical battle held by the table (df-rules tactical needs one)"
            }
        }
    }
}

/// An authored urging of a creature that is hesitant, willing or unwilling (SRD 5.2.1 page 184).
/// The port carries no per-input context, so the conversation sets this up when it registers the
/// action: who is asked (attitude, Intelligence, willingness) and how the words were judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfluenceSetup {
    skill: InfluenceSkill,
    attitude: Attitude,
    monster_intelligence: u8,
    willingness: Willingness,
    word_tilt: WordTilt,
    word_reason: String,
}

impl InfluenceSetup {
    pub fn new(
        skill: InfluenceSkill,
        attitude: Attitude,
        monster_intelligence: u8,
        willingness: Willingness,
        words: WordQuality<'_>,
    ) -> Result<Self, AdapterError> {
        if words.reason.trim().is_empty() {
            return Err(AdapterError::InfluenceReasonIsBlank);
        }
        Ok(Self {
            skill,
            attitude,
            monster_intelligence,
            willingness,
            word_tilt: words.tilt,
            word_reason: words.reason.to_owned(),
        })
    }

    pub(super) fn skill(&self) -> InfluenceSkill {
        self.skill
    }

    pub(super) fn attitude(&self) -> Attitude {
        self.attitude
    }

    pub(super) fn monster_intelligence(&self) -> u8 {
        self.monster_intelligence
    }

    pub(super) fn willingness(&self) -> Willingness {
        self.willingness
    }

    pub(super) fn words(&self) -> WordQuality<'_> {
        WordQuality {
            tilt: self.word_tilt,
            reason: &self.word_reason,
        }
    }

    /// How the sources of this urging combine, which is how many d20s it needs (SRD page 8): none
    /// for a willing or unwilling creature, one, or two with Advantage or Disadvantage.
    pub(super) fn roll_mode(&self) -> Option<RollMode> {
        match self.willingness {
            Willingness::Willing | Willingness::Unwilling => None,
            Willingness::Hesitant => {
                let sources: Vec<_> = attitude_source(self.attitude)
                    .into_iter()
                    .chain(word_source(&self.words()))
                    .collect();
                Some(combine_sources(&sources).0)
            }
        }
    }
}

/// The rules an action runs today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runnable {
    /// One d20 plus the skill's ability modifier and the proficiency bonus when proficient,
    /// against an authored DC (SRD page 6). No Advantage, Disadvantage or Help: `df-rules` has
    /// none for a plain check.
    SkillCheck {
        skill: Skill,
        difficulty_class: DifficultyClass,
    },
    Influence(InfluenceSetup),
}

/// What an action does in the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesEntry {
    Runs(Runnable),
    /// Registered so the action is known, and declined because this piece is missing.
    Unsupported(MissingPiece),
}

impl RulesEntry {
    pub fn skill_check(skill: Skill, difficulty_class: DifficultyClass) -> Self {
        Self::Runs(Runnable::SkillCheck {
            skill,
            difficulty_class,
        })
    }

    pub fn influence(setup: InfluenceSetup) -> Self {
        Self::Runs(Runnable::Influence(setup))
    }
}

/// The actions the adapter knows, in name order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionBook {
    entries: BTreeMap<ActionId, RulesEntry>,
}

impl ActionBook {
    pub fn with_entry(mut self, action: ActionId, entry: RulesEntry) -> Result<Self, AdapterError> {
        if self.entries.contains_key(&action) {
            return Err(AdapterError::ActionAlreadyRegistered { action });
        }
        self.entries.insert(action, entry);
        Ok(self)
    }

    pub fn entry(&self, action: &ActionId) -> Option<&RulesEntry> {
        self.entries.get(action)
    }

    /// The actions that run today (everything but `Unsupported`), in name order.
    pub fn runnable_actions(&self) -> Vec<ActionId> {
        self.entries
            .iter()
            .filter(|(_, entry)| matches!(entry, RulesEntry::Runs(_)))
            .map(|(action, _)| action.clone())
            .collect()
    }
}
