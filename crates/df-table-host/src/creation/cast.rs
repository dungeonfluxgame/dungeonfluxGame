//! A built hero casting a spell outside combat: the rules' checks (`df_rules::hero::plan_cast`)
//! on the hero's sheet, then the slot spent through the one vitals path (`apply_vitals`).
//!
//! Owns `cast_spell` and its outcome. Every cast writes one `RULES cast hero= spell= level=
//! ritual= slots_left=` line and every refusal one `RULES cast_refused hero= spell= level=
//! ritual= cause=` line (`slots_left` counts the slots left of every level). Never does: say what
//! the spell shows (the dungeon master narrates it and Jev judges it against the scene), or spend
//! a slot anywhere but the vitals.
use std::fmt;

use df_rules::hero::{self, HeroSheet, VitalsChange, VitalsError, plan_cast};
use serde::Serialize;

use super::request::CreationRouteError;
use super::rules::HeroRules;
use super::store;
use super::vitals::{apply_vitals, hero_vitals};
use crate::io_cause::IoCause;
use crate::state::AppState;

/// A cast the rules allowed, as the dungeon master is handed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CastOutcome {
    pub hero: u64,
    /// The spell's catalog id (`detect-magic`).
    pub spell: &'static str,
    pub name: &'static str,
    /// The level it was cast at: 0 for a cantrip.
    pub level: u8,
    pub ritual: bool,
    /// The level of the slot spent; none for a cantrip or a Ritual.
    pub slot_spent: Option<u8>,
    /// The spell slots of every level left after the cast.
    pub slots_left: u8,
    /// How long the casting takes, with a Ritual's 10 minutes added ("Action, plus 10 minutes as
    /// a Ritual").
    pub casting_time: String,
    pub concentration: bool,
    pub range: &'static str,
    pub duration: &'static str,
    /// What the spell does, for the dungeon master's narration.
    pub summary: &'static str,
    /// The printed SRD 5.2.1 page of the spell.
    pub page: u16,
}

/// Why a cast was refused; nothing was spent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum CastError {
    /// The rules refused the cast: not a caster, an unknown or unprepared spell, not a ritual, a
    /// level that does not suit it, or no slot left.
    Refused { cause: hero::CastError },
    /// No seat has built this hero, or its build is not legal.
    UnknownHero { hero: u64 },
    /// The vitals refused the slot for a reason other than none being left.
    InvalidVitals { cause: VitalsError },
    /// The creation record or the hero's vitals could not be read or kept.
    Storage { cause: IoCause },
    /// A task stopped without an answer.
    Stopped,
    /// The creation record answered another refusal, kept as it came.
    Creation { cause: CreationRouteError },
}

impl CastError {
    /// The key a refusal names: the rules' cause for a rules refusal (`not_prepared`, `no_slot`).
    pub fn key(&self) -> &'static str {
        match self {
            Self::Refused { cause } => cause.key(),
            Self::UnknownHero { .. } => "unknown_hero",
            Self::InvalidVitals { .. } => "invalid_vitals",
            Self::Storage { .. } => "storage",
            Self::Stopped => "stopped",
            Self::Creation { cause } => cause.key(),
        }
    }
}

impl fmt::Display for CastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { cause } => write!(formatter, "{cause}"),
            Self::UnknownHero { hero } => write!(formatter, "no seat has built hero {hero}"),
            Self::InvalidVitals { cause } => write!(formatter, "{cause}"),
            Self::Storage { cause } => {
                write!(formatter, "the hero's record could not be kept: {cause}")
            }
            Self::Stopped => formatter.write_str("the cast stopped without an answer"),
            Self::Creation { cause } => write!(formatter, "{cause}"),
        }
    }
}

impl std::error::Error for CastError {}

impl From<CreationRouteError> for CastError {
    fn from(error: CreationRouteError) -> Self {
        match error {
            CreationRouteError::UnknownHero { hero } => Self::UnknownHero { hero },
            CreationRouteError::InvalidVitals {
                cause: VitalsError::NoSpellSlot { level },
            } => Self::Refused {
                cause: hero::CastError::NoSlot { level },
            },
            CreationRouteError::InvalidVitals { cause } => Self::InvalidVitals { cause },
            CreationRouteError::Storage { cause } => Self::Storage { cause },
            CreationRouteError::Stopped => Self::Stopped,
            cause @ (CreationRouteError::InvalidBody(_)
            | CreationRouteError::InvalidQuery(_)
            | CreationRouteError::NotOpen
            | CreationRouteError::NoSeats
            | CreationRouteError::UnknownSeat { .. }
            | CreationRouteError::NoHero { .. }
            | CreationRouteError::SeatReady { .. }
            | CreationRouteError::BuildIncomplete { .. }
            | CreationRouteError::NotReady { .. }
            | CreationRouteError::Started
            | CreationRouteError::SeatsNotReady { .. }
            | CreationRouteError::TooManyPicks
            | CreationRouteError::LobbyDeclined { .. }
            | CreationRouteError::HandOffRefused
            | CreationRouteError::Seat(_)) => Self::Creation { cause },
        }
    }
}

/// The sheet of hero `hero`'s finished build, as the rules derive it.
async fn sheet_of(app: &AppState, hero: u64) -> Result<HeroSheet, CastError> {
    let table = store::load(app).await?;
    table
        .as_ref()
        .and_then(|table| table.build_of(hero))
        .and_then(|build| HeroRules::sheet(build.hero.as_ref()?, &build.draft).ok())
        .ok_or(CastError::UnknownHero { hero })
}

/// Hero `hero` casts `spell` at `level` (0 for a cantrip), as a Ritual when `as_ritual`: the
/// rules check the spell is known and prepared (or a Ritual of the spellbook) and the slot, the
/// slot is spent, and the outcome is answered for the dungeon master. Refused with nothing spent.
pub(crate) async fn cast_spell(
    app: &AppState,
    hero: u64,
    spell: &str,
    level: u8,
    as_ritual: bool,
) -> Result<CastOutcome, CastError> {
    let refused = |error: &CastError| {
        format!(
            "RULES cast_refused hero={hero} spell={spell} level={level} ritual={as_ritual} \
                cause={}",
            error.key()
        )
    };
    match cast(app, hero, spell, level, as_ritual).await {
        Ok(outcome) => {
            app.record_line(format!(
                "RULES cast hero={hero} spell={} level={} ritual={} slots_left={}",
                outcome.spell, outcome.level, outcome.ritual, outcome.slots_left
            ));
            Ok(outcome)
        }
        Err(error) => {
            app.record_line(refused(&error));
            Err(error)
        }
    }
}

async fn cast(
    app: &AppState,
    hero: u64,
    spell: &str,
    level: u8,
    as_ritual: bool,
) -> Result<CastOutcome, CastError> {
    let sheet = sheet_of(app, hero).await?;
    let plan =
        plan_cast(&sheet, spell, level, as_ritual).map_err(|cause| CastError::Refused { cause })?;
    let vitals = match plan.slot {
        Some(level) => apply_vitals(app, hero, VitalsChange::SpendSpellSlot { level }).await?,
        None => hero_vitals(app, hero)
            .await?
            .ok_or(CastError::UnknownHero { hero })?,
    };
    let facts = plan.spell;
    let casting_time = if plan.ritual {
        format!(
            "{}, plus {} minutes as a Ritual",
            facts.casting_time, plan.extra_minutes
        )
    } else {
        facts.casting_time.to_owned()
    };
    Ok(CastOutcome {
        hero,
        spell: facts.id,
        name: facts.name,
        level: plan.level,
        ritual: plan.ritual,
        slot_spent: plan.slot,
        slots_left: vitals.spell_slots.iter().map(|slots| slots.left).sum(),
        casting_time,
        concentration: facts.concentration,
        range: facts.range,
        duration: facts.duration,
        summary: facts.summary,
        page: facts.page,
    })
}
