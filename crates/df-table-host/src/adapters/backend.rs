//! The `rules_backend` setting and the ports the dev table gets for each choice.
//!
//! It owns the closed choice (`scripted` or `df-rules`, default `scripted`), reading it from the
//! `DF_RULES_BACKEND` environment variable at the host's edge, and `dev_ports`, which builds the
//! rules owner and the dice the dev engine is started with. The scripted choice is the H0 fakes
//! unchanged (`DevRulesOwner` and the forceable `DevDice`). The `df-rules` choice is the production
//! `DfRulesOwner` over the dev roster and book, with `RecordedDice` seeded for a reproducible dev
//! run, and it has no `dice force` control (a forced value would not be a recorded draw). It never
//! reads a setting a second time and never falls back silently: an unknown value is an
//! `AdapterError` and the engine does not start.

use std::sync::Arc;

use df_table::identity::SeatId;
use df_table::ports::{ActionId, Dice, RulesOwner};

use crate::engine::adapters::{DevDice, DevRulesOwner};
use crate::engine::limits::DEV_SEAT_COUNT;

use super::action_book::{ActionBook, DifficultyClass, RulesEntry};
use super::dev_check::RulesCheckKit;
use super::dice::RecordedDice;
use super::error::AdapterError;
use super::hero_profile::{HeroProfile, SeatRoster, Skill};
use super::rules::DfRulesOwner;

/// The environment variable that carries the `rules_backend` setting.
pub const RULES_BACKEND_VARIABLE: &str = "DF_RULES_BACKEND";

/// The scores of the dev table's stand-in heroes: strength 10, dexterity 15, constitution 13,
/// intelligence 12, wisdom 14, charisma 8, which is the SRD 5.2.1 Standard Array (15, 14, 13, 12,
/// 10, 8, page 21) spread across the abilities.
const DEV_HERO_SCORES: [u8; 6] = [10, 15, 13, 12, 14, 8];
/// A level 1 hero's proficiency bonus (SRD 5.2.1 page 8).
const DEV_HERO_PROFICIENCY_BONUS: u8 = 2;

/// Which rules owner and dice the engine runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RulesBackend {
    Scripted,
    DfRules,
}

impl RulesBackend {
    pub const ALL: [RulesBackend; 2] = [Self::Scripted, Self::DfRules];

    pub fn key(self) -> &'static str {
        match self {
            Self::Scripted => "scripted",
            Self::DfRules => "df-rules",
        }
    }

    /// The backend a setting value names.
    pub fn parse(text: &str) -> Result<Self, AdapterError> {
        Self::ALL
            .into_iter()
            .find(|backend| backend.key() == text)
            .ok_or_else(|| AdapterError::UnknownRulesBackend {
                text: text.to_owned(),
            })
    }

    /// The setting from the environment: `scripted` when it is not set, an error when it is set
    /// to anything but a backend.
    pub fn from_environment() -> Result<Self, AdapterError> {
        match std::env::var(RULES_BACKEND_VARIABLE) {
            Ok(text) => Self::parse(&text),
            Err(std::env::VarError::NotPresent) => Ok(Self::Scripted),
            Err(std::env::VarError::NotUnicode(_)) => Err(AdapterError::RulesBackendNotText),
        }
    }
}

/// The dev table's stand-in heroes, one for each seat, until character creation seats real ones.
pub fn dev_roster() -> Result<SeatRoster, AdapterError> {
    (1..=DEV_SEAT_COUNT).try_fold(SeatRoster::default(), |roster, seat| {
        let hero = HeroProfile::new(
            DEV_HERO_SCORES,
            DEV_HERO_PROFICIENCY_BONUS,
            [Skill::SleightOfHand, Skill::Perception],
        )?;
        roster.with_hero(SeatId::new(seat), hero)
    })
}

/// The dev table's one authored check, as a skill check. The authored beat does not yet carry its
/// skill or DC, so the dev check is Sleight of Hand against the SRD's Medium DC until it does.
pub fn dev_book(check: &ActionId) -> Result<ActionBook, AdapterError> {
    ActionBook::default().with_entry(
        check.clone(),
        RulesEntry::skill_check(Skill::SleightOfHand, DifficultyClass::MEDIUM),
    )
}

/// What the dev engine is started with for one backend.
pub struct DevPorts {
    pub rules: Arc<dyn RulesOwner>,
    pub dice: Arc<dyn Dice>,
    /// The dice the `dice seed` and `dice force` verbs drive: the scripted backend only.
    pub dice_control: Option<Arc<DevDice>>,
    /// What `rules check` runs on: the `df-rules` backend only.
    pub check_kit: Option<Arc<RulesCheckKit>>,
}

/// The rules owner and dice of the dev engine for `backend`. `check` is the authored check the
/// dev table's explore machine offers, and `seed` seeds the dice so a run is reproducible.
pub fn dev_ports(
    backend: RulesBackend,
    check: &ActionId,
    seed: u64,
) -> Result<DevPorts, AdapterError> {
    match backend {
        RulesBackend::Scripted => {
            let dice = Arc::new(DevDice::seeded(seed));
            Ok(DevPorts {
                rules: Arc::new(DevRulesOwner::new(check.clone())),
                dice: dice.clone(),
                dice_control: Some(dice),
                check_kit: None,
            })
        }
        RulesBackend::DfRules => {
            let rules = Arc::new(DfRulesOwner::new(dev_roster()?, dev_book(check)?));
            let dice = Arc::new(RecordedDice::seeded(seed));
            Ok(DevPorts {
                rules: rules.clone(),
                dice: dice.clone(),
                dice_control: None,
                check_kit: Some(Arc::new(RulesCheckKit::new(rules, dice))),
            })
        }
    }
}
