//! The closed vocabulary of a check, its wire values and its typed refusals.
//!
//! A check judge answers one [`CheckVerdict`]: whether a roll is called, who asked for it, why,
//! and, when one is, the [`CheckCall`] (the kind of roll, its ability and skill, the difficulty
//! band, the mode and what grounds it, and which sheet entry it uses). Every field is a closed
//! key checked in code (`judge`); the judge never decides how a roll comes out.

use std::collections::BTreeMap;
use std::fmt;

use df_playground::fixture::RollView;
use df_rules::roll::{
    Ability, AttackKind, DiceExpression, ModifierLabel, RollError, RollMode, Skill,
};
use serde::{Deserialize, Serialize};

use crate::adapters::RulesCheckError;
use crate::persona::judge_route::JudgeRouteError;
use crate::persona::request::PersonaError;

/// Why a check judgement, a line of the dungeon master or a roll was refused.
pub type CheckRouteError = JudgeRouteError<CheckError>;

/// One field of the judge's answer, named in the refusals that concern it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerField {
    CheckRequired,
    Initiator,
    Kind,
    Ability,
    Skill,
    DcBand,
    Mode,
    Grounds,
    Uses,
    Reason,
}

impl AnswerField {
    pub fn key(self) -> &'static str {
        match self {
            Self::CheckRequired => "check_required",
            Self::Initiator => "initiator",
            Self::Kind => "kind",
            Self::Ability => "ability",
            Self::Skill => "skill",
            Self::DcBand => "dc_band",
            Self::Mode => "mode",
            Self::Grounds => "grounds",
            Self::Uses => "uses",
            Self::Reason => "reason",
        }
    }
}

/// The one mood of a flourish of the dungeon master, which the storyteller chooses for its own
/// words and code checks against how things stand (`lines`): a tone that does not fit the outcome
/// is refused, so a flourish never cheers a failure or mourns a success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// A win to cheer.
    Triumph,
    /// A breath let out after a near thing.
    Relief,
    /// A knowing, sidelong remark.
    Wry,
    /// Wonder at something great.
    Awe,
    /// Plain telling, no mood.
    Neutral,
    /// A dry joke.
    DryHumor,
    /// A reverse taken.
    Setback,
    /// Foreboding.
    Dread,
    /// A shake of the head over what might have been.
    Rue,
    /// Held breath before the roll.
    Anticipation,
}

impl Tone {
    pub const ALL: [Self; 10] = [
        Self::Triumph,
        Self::Relief,
        Self::Wry,
        Self::Awe,
        Self::Neutral,
        Self::DryHumor,
        Self::Setback,
        Self::Dread,
        Self::Rue,
        Self::Anticipation,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Triumph => "triumph",
            Self::Relief => "relief",
            Self::Wry => "wry",
            Self::Awe => "awe",
            Self::Neutral => "neutral",
            Self::DryHumor => "dry_humor",
            Self::Setback => "setback",
            Self::Dread => "dread",
            Self::Rue => "rue",
            Self::Anticipation => "anticipation",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tone| tone.key() == key)
    }
}

/// The kind of roll, in the roll vocabulary of the dice gauntlet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    AbilityCheck,
    SavingThrow,
    AttackRoll,
    Initiative,
    DeathSave,
    Passive,
    Contested,
    Damage,
    Healing,
    Table,
}

impl CheckKind {
    pub const ALL: [Self; 10] = [
        Self::AbilityCheck,
        Self::SavingThrow,
        Self::AttackRoll,
        Self::Initiative,
        Self::DeathSave,
        Self::Passive,
        Self::Contested,
        Self::Damage,
        Self::Healing,
        Self::Table,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::AbilityCheck => "ability_check",
            Self::SavingThrow => "saving_throw",
            Self::AttackRoll => "attack_roll",
            Self::Initiative => "initiative",
            Self::DeathSave => "death_save",
            Self::Passive => "passive",
            Self::Contested => "contested",
            Self::Damage => "damage",
            Self::Healing => "healing",
            Self::Table => "table",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// Whether the roll is a d20 test, which a mode (Advantage or Disadvantage) can change.
    pub fn is_d20(self) -> bool {
        !matches!(self, Self::Damage | Self::Healing | Self::Table)
    }
}

/// Who asked for the roll: the player's action or the narrator's moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Initiator {
    Player,
    Narrator,
}

impl Initiator {
    pub const ALL: [Self; 2] = [Self::Player, Self::Narrator];

    pub fn key(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Narrator => "narrator",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|initiator| initiator.key() == key)
    }
}

/// The difficulty ladder (SRD 5.2.1 page 6, Difficulty Class table). On an attack roll the band is
/// the target's Armor Class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum DcBand {
    VeryEasy,
    Easy,
    Medium,
    Hard,
    VeryHard,
    NearlyImpossible,
}

impl DcBand {
    pub const ALL: [Self; 6] = [
        Self::VeryEasy,
        Self::Easy,
        Self::Medium,
        Self::Hard,
        Self::VeryHard,
        Self::NearlyImpossible,
    ];

    pub fn value(self) -> u8 {
        match self {
            Self::VeryEasy => 5,
            Self::Easy => 10,
            Self::Medium => 15,
            Self::Hard => 20,
            Self::VeryHard => 25,
            Self::NearlyImpossible => 30,
        }
    }

    /// The band whose number is `value`, if the ladder has one.
    pub fn from_value(value: i64) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|band| i64::from(band.value()) == value)
    }

    /// The SRD's name of the band, as the narrator may say it.
    pub fn difficulty_name(self) -> &'static str {
        match self {
            Self::VeryEasy => "very easy",
            Self::Easy => "easy",
            Self::Medium => "medium",
            Self::Hard => "hard",
            Self::VeryHard => "very hard",
            Self::NearlyImpossible => "nearly impossible",
        }
    }
}

/// A number that is not on the difficulty ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffLadder(pub u8);

impl fmt::Display for OffLadder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} is not on the difficulty ladder", self.0)
    }
}

impl TryFrom<u8> for DcBand {
    type Error = OffLadder;

    fn try_from(value: u8) -> Result<Self, OffLadder> {
        Self::from_value(i64::from(value)).ok_or(OffLadder(value))
    }
}

impl From<DcBand> for u8 {
    fn from(band: DcBand) -> Self {
        band.value()
    }
}

/// What grounds an Advantage or a Disadvantage. A mode other than normal needs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeGround {
    None,
    /// A feature on the character sheet grants it for this test.
    CharacterFeature,
    /// An ally's help (the Help action).
    AllyHelp,
    /// The situation favours the character.
    FavorableCircumstance,
    /// The situation hinders the character.
    UnfavorableCircumstance,
    /// The opponent does not see the character coming.
    UnawareOpponent,
    /// What the character wears imposes it.
    ArmorPenalty,
}

impl ModeGround {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::CharacterFeature,
        Self::AllyHelp,
        Self::FavorableCircumstance,
        Self::UnfavorableCircumstance,
        Self::UnawareOpponent,
        Self::ArmorPenalty,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::CharacterFeature => "character_feature",
            Self::AllyHelp => "ally_help",
            Self::FavorableCircumstance => "favorable_circumstance",
            Self::UnfavorableCircumstance => "unfavorable_circumstance",
            Self::UnawareOpponent => "unaware_opponent",
            Self::ArmorPenalty => "armor_penalty",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|ground| ground.key() == key)
    }

    /// The mode this ground gives: normal for none.
    pub fn mode(self) -> RollMode {
        match self {
            Self::None => RollMode::Normal,
            Self::CharacterFeature
            | Self::AllyHelp
            | Self::FavorableCircumstance
            | Self::UnawareOpponent => RollMode::Advantage,
            Self::UnfavorableCircumstance | Self::ArmorPenalty => RollMode::Disadvantage,
        }
    }
}

/// The key of a roll mode, as the schemas and the audit lines spell it.
pub fn mode_key(mode: RollMode) -> &'static str {
    match mode {
        RollMode::Normal => "normal",
        RollMode::Advantage => "advantage",
        RollMode::Disadvantage => "disadvantage",
    }
}

/// The mode a key names.
pub fn mode_from_key(key: &str) -> Option<RollMode> {
    [
        RollMode::Normal,
        RollMode::Advantage,
        RollMode::Disadvantage,
    ]
    .into_iter()
    .find(|mode| mode_key(*mode) == key)
}

/// Why the judge called a roll, or did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckReason {
    /// The player's action may fail and failing matters.
    UncertainWithConsequence,
    /// The narrator calls a roll for a hazard the character meets.
    NarratorHazard,
    /// The narrator calls a roll for an obstacle in the way (a lock, a climb).
    NarratorObstacle,
    /// The narrator calls a roll for how someone reacts.
    NarratorReaction,
    /// Anyone would do it; no roll.
    TrivialNoCheck,
    /// It cannot be done; no roll.
    ImpossibleNoCheck,
    /// Nothing rides on it; no roll.
    NoStakes,
    /// The same test was already rolled here.
    AlreadyChecked,
    /// The narrator's intended test makes no sense for the situation; no roll.
    IntentUnsuitable,
}

impl CheckReason {
    pub const ALL: [Self; 9] = [
        Self::UncertainWithConsequence,
        Self::NarratorHazard,
        Self::NarratorObstacle,
        Self::NarratorReaction,
        Self::TrivialNoCheck,
        Self::ImpossibleNoCheck,
        Self::NoStakes,
        Self::AlreadyChecked,
        Self::IntentUnsuitable,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::UncertainWithConsequence => "uncertain_with_consequence",
            Self::NarratorHazard => "narrator_hazard",
            Self::NarratorObstacle => "narrator_obstacle",
            Self::NarratorReaction => "narrator_reaction",
            Self::TrivialNoCheck => "trivial_no_check",
            Self::ImpossibleNoCheck => "impossible_no_check",
            Self::NoStakes => "no_stakes",
            Self::AlreadyChecked => "already_checked",
            Self::IntentUnsuitable => "intent_unsuitable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.key() == key)
    }

    /// The initiator a reason for calling a roll belongs to; none for a reason not to call one.
    pub fn initiator(self) -> Option<Initiator> {
        match self {
            Self::UncertainWithConsequence => Some(Initiator::Player),
            Self::NarratorHazard | Self::NarratorObstacle | Self::NarratorReaction => {
                Some(Initiator::Narrator)
            }
            Self::TrivialNoCheck
            | Self::ImpossibleNoCheck
            | Self::NoStakes
            | Self::AlreadyChecked
            | Self::IntentUnsuitable => None,
        }
    }

    pub fn calls_for_check(self) -> bool {
        self.initiator().is_some()
    }
}

/// A test named by its kind and, when it has them, its ability and skill: one already rolled at a
/// place (so the judge does not call it twice), or the one a narrator intends to call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckTest {
    pub kind: CheckKind,
    #[serde(default)]
    pub ability: Option<Ability>,
    #[serde(default)]
    pub skill: Option<Skill>,
}

/// One weapon of the sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetAttack {
    pub id: String,
    pub name: String,
    pub kind: AttackKind,
    pub ability: Ability,
    pub damage: DiceExpression,
}

/// One healing the sheet has (a Second Wind, a hit die).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetHealing {
    pub id: String,
    pub dice: DiceExpression,
    /// What the flat bonus of `dice` comes from, so the roll names it (`Flat` names nothing).
    pub bonus_label: ModifierLabel,
}

/// A feature of the sheet, in the sheet's words, that the judge reads for what it grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetFeature {
    pub name: String,
    pub text: String,
}

/// What the rolls of a character take from the character sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSheet {
    pub id: String,
    pub proficiency_bonus: u8,
    /// The modifier of each of the six abilities.
    pub ability_modifiers: BTreeMap<Ability, i8>,
    #[serde(default)]
    pub saving_throw_proficiencies: Vec<Ability>,
    #[serde(default)]
    pub skill_proficiencies: Vec<Skill>,
    #[serde(default)]
    pub skill_expertise: Vec<Skill>,
    /// Whether the Proficiency Bonus is added to initiative (the Alert feat).
    #[serde(default)]
    pub initiative_proficient: bool,
    /// The lowest d20 face that is a Critical Hit on a weapon attack: 20, or 19 for a Champion.
    pub critical_from: u8,
    #[serde(default)]
    pub attacks: Vec<SheetAttack>,
    #[serde(default)]
    pub healing: Vec<SheetHealing>,
    #[serde(default)]
    pub equipment: Vec<String>,
    #[serde(default)]
    pub features: Vec<SheetFeature>,
}

impl CheckSheet {
    /// The modifier of `ability`, or why the sheet has none.
    pub fn modifier(&self, ability: Ability) -> Result<i8, CheckError> {
        self.ability_modifiers
            .get(&ability)
            .copied()
            .ok_or(CheckError::SheetIncomplete { ability })
    }

    pub fn attack(&self, id: &str) -> Option<&SheetAttack> {
        self.attacks.iter().find(|attack| attack.id == id)
    }

    pub fn healing_of(&self, id: &str) -> Option<&SheetHealing> {
        self.healing.iter().find(|healing| healing.id == id)
    }
}

/// A station of the gauntlet: the place and the situation, and the test it was designed to
/// provoke. The judge is never told the designed test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckStation {
    pub id: String,
    pub title: String,
    pub kind: CheckKind,
    #[serde(default)]
    pub ability: Option<Ability>,
    #[serde(default)]
    pub skill: Option<Skill>,
    pub setting: String,
    pub situation: String,
}

/// The body of `POST /dev/persona/check/judge`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckJudgeRequest {
    pub character: String,
    pub sheet: CheckSheet,
    /// The moment the judge is asked at: a player's stated action, or the narrator's turn to
    /// decide whether to call a roll before the character acts.
    pub moment: Initiator,
    /// What the character does or says they do; present at a player moment, absent at the
    /// narrator's.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub station: Option<CheckStation>,
    /// The place in words, when the station does not give it.
    #[serde(default)]
    pub place: Option<String>,
    /// More things in sight, beside the station's setting.
    #[serde(default)]
    pub in_sight: Vec<String>,
    #[serde(default)]
    pub checked: Vec<CheckTest>,
    /// The test the narrator has authored for this situation; only a narrator moment has one. The
    /// judge calls it (setting the band, the mode and its grounds) or refuses it with
    /// `intent_unsuitable`.
    #[serde(default)]
    pub intent: Option<CheckTest>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The body of `POST /dev/persona/check/run`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRunRequest {
    pub judge: CheckJudgeRequest,
    /// Whether the dungeon master's lines are voiced; with TTS off they are shown as text only.
    #[serde(default = "voiced_by_default")]
    pub speak: bool,
    /// Spend Heroic Inspiration on the die at this place of the roll's dice (from 0).
    #[serde(default)]
    pub inspiration: Option<u8>,
    /// What the runner has just said aloud (the scene line), which a call must not restate.
    #[serde(default)]
    pub told: Option<String>,
}

fn voiced_by_default() -> bool {
    true
}

/// A validated call for a roll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCall {
    pub kind: CheckKind,
    pub ability: Option<Ability>,
    pub skill: Option<Skill>,
    pub dc_band: Option<DcBand>,
    pub mode: RollMode,
    pub grounds: ModeGround,
    /// The sheet attack or healing the roll uses.
    pub uses: Option<String>,
}

/// The judge's validated answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckVerdict {
    pub initiator: Initiator,
    pub reason: CheckReason,
    pub call: Option<CheckCall>,
}

/// Whether the judged test is the one the station was designed to provoke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Designed {
    pub kind: CheckKind,
    pub matches: bool,
}

/// The answer of `POST /dev/persona/check/judge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckJudged {
    pub verdict: CheckVerdict,
    /// How many times the judge was asked again before its answer stood.
    pub retried: u32,
    pub duration_ms: u64,
    pub designed: Option<Designed>,
}

/// How the dungeon master's line reached the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineVoice {
    /// A clip was cued with the line.
    Played,
    /// The voice failed; the line was shown as text.
    Failed,
    /// The line was shown as text only: not asked for, or TTS is off.
    TextOnly,
}

impl LineVoice {
    pub fn key(self) -> &'static str {
        match self {
            Self::Played => "played",
            Self::Failed => "failed",
            Self::TextOnly => "text_only",
        }
    }
}

/// When each step of a run happened, in milliseconds from its start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckTimes {
    /// The dice were drawn (before the call is spoken; nothing shows them yet).
    pub drawn_ms: u64,
    pub called_ms: u64,
    pub shown_ms: u64,
    pub result_start_ms: u64,
    pub result_end_ms: u64,
    pub cleared_ms: u64,
    /// How long the card was held once the sum was on screen at the latest.
    pub hold_ms: u64,
    pub min_hold_ms: u64,
}

/// A roll that was called, rolled, shown and read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolledCheck {
    pub called_line: String,
    pub result_line: String,
    pub called_voice: LineVoice,
    pub result_voice: LineVoice,
    pub view: RollView,
    pub times: CheckTimes,
}

/// The answer of `POST /dev/persona/check/run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    pub judged: CheckJudged,
    /// The roll, when the judge called one.
    pub rolled: Option<RolledCheck>,
}

/// Why a request is refused, a judge's answer or a line of the dungeon master is refused (and asked
/// again), or a roll could not be made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckError {
    /// The sheet is another character's.
    SheetForOther {
        sheet: String,
        character: String,
    },
    /// The sheet has no modifier for an ability.
    SheetIncomplete {
        ability: Ability,
    },
    /// A player moment has no action.
    NeedsAction,
    /// A player moment is given a narrator's intent.
    IntentNeedsNarrator,
    /// The answer calls another test than the narrator's intent, or refuses it for another reason
    /// than the intent being unsuitable.
    IntentIgnored,
    /// The answer says an intent is unsuitable when the narrator named none.
    NoIntentToRefuse,
    /// A narrator moment is given an action.
    UnwantedAction,
    MalformedAnswer {
        reason: String,
    },
    /// The answer's initiator is not the one the moment asked for.
    InitiatorMismatch {
        moment: Initiator,
        initiator: Initiator,
    },
    /// A reason that calls for a roll with no roll, or the reverse.
    ReasonMismatch {
        check_required: bool,
        reason: CheckReason,
    },
    /// The reason belongs to the other initiator.
    ReasonInitiatorMismatch {
        initiator: Initiator,
        reason: CheckReason,
    },
    UnexpectedField {
        field: AnswerField,
    },
    MissingField {
        field: AnswerField,
    },
    /// The ability is not the one the skill, the weapon or the test uses.
    AbilityMismatch {
        ability: Ability,
        expected: Ability,
    },
    /// The skill is not one this kind of roll can use.
    UnsupportedSkill {
        skill: Skill,
    },
    DcOffLadder {
        value: i64,
    },
    /// The sheet has no attack or healing of this id.
    UnknownUses {
        id: String,
    },
    /// A mode with no ground, or a ground that gives the other mode.
    UngroundedMode {
        mode: RollMode,
        grounds: ModeGround,
    },
    /// The ground needs something the sheet does not have.
    GroundNotInSheet {
        grounds: ModeGround,
    },
    /// A mode on a roll that has no d20.
    ModeOnDice {
        kind: CheckKind,
    },
    /// The same test was already rolled here.
    DoubleCheck,
    /// The answer says a test was already rolled, but none was.
    NothingChecked,
    /// The spoken numbers of a line are not those of the roll.
    NumbersMismatch,
    FlavorHasDigits,
    EmptyFlavor,
    /// The line calls someone by name.
    NamesCalled,
    /// The line speaks of someone in the third person instead of to the party.
    NotSpokenToTheParty,
    /// The tone of the flourish does not fit how things stand.
    ToneNotAllowed {
        tone: Tone,
    },
    /// Heroic Inspiration cannot be spent on this roll.
    InspirationNotPossible {
        kind: CheckKind,
    },
    /// `df-rules` refused the roll.
    Roll {
        cause: RollError,
    },
    /// The dice could not be drawn.
    Rolling(RulesCheckError),
    /// The persona, the dungeon master's box or the scene failed.
    Persona(Box<PersonaError>),
}

impl CheckError {
    /// The error's key, for the audit lines.
    pub fn key(&self) -> &'static str {
        match self {
            Self::SheetForOther { .. } => "sheet_for_other",
            Self::SheetIncomplete { .. } => "sheet_incomplete",
            Self::NeedsAction => "needs_action",
            Self::UnwantedAction => "unwanted_action",
            Self::IntentNeedsNarrator => "intent_needs_narrator",
            Self::IntentIgnored => "intent_ignored",
            Self::NoIntentToRefuse => "no_intent_to_refuse",
            Self::MalformedAnswer { .. } => "malformed_answer",
            Self::InitiatorMismatch { .. } => "initiator_mismatch",
            Self::ReasonMismatch { .. } => "reason_mismatch",
            Self::ReasonInitiatorMismatch { .. } => "reason_initiator_mismatch",
            Self::UnexpectedField { .. } => "unexpected_field",
            Self::MissingField { .. } => "missing_field",
            Self::AbilityMismatch { .. } => "ability_mismatch",
            Self::UnsupportedSkill { .. } => "unsupported_skill",
            Self::DcOffLadder { .. } => "dc_off_ladder",
            Self::UnknownUses { .. } => "unknown_uses",
            Self::UngroundedMode { .. } => "ungrounded_mode",
            Self::GroundNotInSheet { .. } => "ground_not_in_sheet",
            Self::ModeOnDice { .. } => "mode_on_dice",
            Self::DoubleCheck => "double_check",
            Self::NothingChecked => "nothing_checked",
            Self::NumbersMismatch => "numbers_mismatch",
            Self::FlavorHasDigits => "flavor_has_digits",
            Self::EmptyFlavor => "empty_flavor",
            Self::NamesCalled => "names_called",
            Self::NotSpokenToTheParty => "not_spoken_to_the_party",
            Self::ToneNotAllowed { .. } => "tone_not_allowed",
            Self::InspirationNotPossible { .. } => "inspiration_not_possible",
            Self::Roll { .. } => "roll",
            Self::Rolling(_) => "rolling",
            Self::Persona(_) => "persona",
        }
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SheetForOther { sheet, character } => {
                write!(formatter, "the sheet is {sheet}'s, not {character}'s")
            }
            Self::SheetIncomplete { ability } => {
                write!(formatter, "the sheet has no {} modifier", ability.key())
            }
            Self::NeedsAction => formatter.write_str("a player moment needs the action stated"),
            Self::IntentNeedsNarrator => {
                formatter.write_str("only a narrator moment has an intended test")
            }
            Self::IntentIgnored => formatter.write_str(
                "the narrator intends that test: call it, or refuse it as intent_unsuitable",
            ),
            Self::NoIntentToRefuse => {
                formatter.write_str("the narrator named no test, so none is unsuitable")
            }
            Self::UnwantedAction => {
                formatter.write_str("a narrator moment judges the situation and takes no action")
            }
            Self::MalformedAnswer { reason } => write!(formatter, "malformed answer: {reason}"),
            Self::InitiatorMismatch { moment, initiator } => write!(
                formatter,
                "the moment is the {}'s, so the initiator is {}, not {}",
                moment.key(),
                moment.key(),
                initiator.key()
            ),
            Self::ReasonMismatch {
                check_required,
                reason,
            } => write!(
                formatter,
                "reason {} does not go with check_required {check_required}",
                reason.key()
            ),
            Self::ReasonInitiatorMismatch { initiator, reason } => write!(
                formatter,
                "reason {} is not one the {} gives",
                reason.key(),
                initiator.key()
            ),
            Self::UnexpectedField { field } => {
                write!(formatter, "{} must be null for this answer", field.key())
            }
            Self::MissingField { field } => {
                write!(formatter, "{} is needed for this answer", field.key())
            }
            Self::AbilityMismatch { ability, expected } => write!(
                formatter,
                "the ability is {} but this test uses {}",
                ability.key(),
                expected.key()
            ),
            Self::UnsupportedSkill { skill } => write!(
                formatter,
                "{} is not a skill this kind of roll uses",
                skill.key()
            ),
            Self::DcOffLadder { value } => write!(
                formatter,
                "{value} is not on the difficulty ladder (5, 10, 15, 20, 25, 30)"
            ),
            Self::UnknownUses { id } => {
                write!(formatter, "the sheet has no attack or healing '{id}'")
            }
            Self::UngroundedMode { mode, grounds } => write!(
                formatter,
                "mode {} does not follow from grounds {}",
                mode_key(*mode),
                grounds.key()
            ),
            Self::GroundNotInSheet { grounds } => write!(
                formatter,
                "grounds {} needs something the sheet does not have",
                grounds.key()
            ),
            Self::ModeOnDice { kind } => write!(
                formatter,
                "a {} roll has no d20, so its mode is normal",
                kind.key()
            ),
            Self::DoubleCheck => formatter.write_str("this test was already rolled here"),
            Self::NothingChecked => formatter.write_str("nothing was rolled here yet"),
            Self::NumbersMismatch => {
                formatter.write_str("the spoken numbers are not those of the roll")
            }
            Self::FlavorHasDigits => formatter.write_str("the line holds digits: say no numbers"),
            Self::EmptyFlavor => formatter.write_str("a part of the line is empty"),
            Self::NamesCalled => {
                formatter.write_str("the line called someone by name: name nobody")
            }
            Self::NotSpokenToTheParty => formatter.write_str(
                "the line used he, she, they, his, her or their: speak to the party as you, and \
                 name a person by their look or the thing by its name",
            ),
            Self::ToneNotAllowed { tone } => write!(
                formatter,
                "the tone {} does not fit how it stands: choose one of allowed_tones",
                tone.key()
            ),
            Self::InspirationNotPossible { kind } => write!(
                formatter,
                "Heroic Inspiration cannot be spent on a {} roll",
                kind.key()
            ),
            Self::Roll { cause } => write!(formatter, "the roll was refused: {cause}"),
            Self::Rolling(error) => write!(formatter, "the dice could not be drawn: {error:?}"),
            Self::Persona(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for CheckError {}

impl From<PersonaError> for CheckError {
    fn from(error: PersonaError) -> Self {
        Self::Persona(Box::new(error))
    }
}

impl From<RulesCheckError> for CheckError {
    fn from(error: RulesCheckError) -> Self {
        Self::Rolling(error)
    }
}
