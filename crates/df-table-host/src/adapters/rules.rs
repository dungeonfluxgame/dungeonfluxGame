//! The production `RulesOwner` over `df-rules`: ability and skill checks and the influence flow,
//! with explicit draws, and a typed `not_supported_yet` decline for everything else (RES31, O.10).
//!
//! It owns the mapping from an action name (through the `ActionBook`) and a seat (through the
//! `SeatRoster`) to a `df-rules` call, the number of d20s a call needs, and the typed facts of a
//! result. It never draws a die (the `Dice` port does, and `resolve` only reads the faces it is
//! given), never decides a difficulty (the book carries the authored one), never reads a clock and
//! never holds state, so the same draws always give the same resolution. The d20 rules are SRD
//! 5.2.1 page 6: the total is the die plus the ability modifier plus the proficiency bonus when
//! proficient, and it meets the DC when it equals or exceeds it; a natural 1 or 20 does not
//! decide an ability check (it does decide an attack roll, which this adapter does not run).
//!
//! What is declined and why, from the RR readiness table (O.16): legal choices of an actor exist
//! only as the book's runnable actions (`df-rules` has no producer of a full action list); the
//! port has no spatial query, rest or time-cost method yet, so there is nothing to answer for
//! them, and a rest, travel, shop, spell or battle action registered in the book declines with
//! the piece that is missing. A call that cannot be answered fails with a `PortError`, never a
//! guess: `invalid_answer` for dice that do not fit the question or a value `df-rules` refuses,
//! with a closed `RulesRefusal` cause behind it.
//!
//! Every call through the port that resolves writes `RULES resolved` with the die, the total, the
//! DC, the result and the roll mode, and every call it cannot answer writes `RULES refused` with
//! the cause (`audit`). A decline and a call for more dice are answers, not lines of their own.

mod audit;

use std::num::NonZeroU16;

use df_rules::ability_check::{self, AbilityCheckInput};
use df_rules::influence::{self, InfluenceInput, InfluenceOutcome, NoRollCause, RollMode};
use df_table::element::NameError;
use df_table::envelope::PendingId;
use df_table::identity::{Correlation, OperationId, SeatId};
use df_table::outcome::{Decline, DeclineCode};
use df_table::ports::{
    ActionId, ConfirmationReason, Draw, DrawRequest, PlanSoFar, PortError, Preparation, Resolution,
    Resolved, Resumption, RulesOwner, RulingRequest,
};

use super::action_book::{
    ActionBook, DifficultyClass, InfluenceSetup, MissingPiece, RulesEntry, Runnable,
};
use super::hero_profile::{HeroProfile, SeatRoster, Skill, SkillBonus};

pub use self::audit::{ResolvedKind, ResolvedLine, ResolvedResult, RulesRefusal};
use self::audit::{record, refused_line, resolved_line};

/// The die of every d20 test: 20 sides (the smallest non-zero number, plus 19).
const D20_SIDES: NonZeroU16 = NonZeroU16::MIN.saturating_add(19);

/// What an action that is not in the book runs: nothing, for want of an entry (O.5).
static UNREGISTERED: RulesEntry = RulesEntry::Unsupported(MissingPiece::UnregisteredAction);

/// The production rules owner. Immutable once built, so it is shared freely.
#[derive(Debug, Clone)]
pub struct DfRulesOwner {
    roster: SeatRoster,
    book: ActionBook,
}

/// Both texts of every decline built here are non-blank constants (a test pins that), so the only
/// error `Decline` can report is impossible; the port still fails typed if it ever happens.
fn not_supported(piece: MissingPiece) -> Result<Decline, RulesRefusal> {
    Decline::with_default_reason(DeclineCode::NotSupportedYet)
        .with_missing_piece(piece.describe())
        .map_err(|_| RulesRefusal::DeclineText)
}

fn seat_has_no_hero() -> Result<Decline, RulesRefusal> {
    Decline::with_default_reason(DeclineCode::MissingRequirement)
        .with_missing_piece("a hero for this seat")
        .map_err(|_| RulesRefusal::DeclineText)
}

/// A resolution, and the numbers of its `RULES resolved` line when it resolved a roll or an
/// urging.
struct Resolving {
    resolution: Resolution,
    line: Option<ResolvedLine>,
}

impl Resolving {
    fn answer(resolution: Resolution) -> Self {
        Self {
            resolution,
            line: None,
        }
    }
}

fn d20_requests(count: usize) -> Vec<DrawRequest> {
    std::iter::repeat_n(DrawRequest { sides: D20_SIDES }, count).collect()
}

/// The faces a call was given, or the dice it still needs.
enum Supply {
    Faces(Vec<u8>),
    Missing(Vec<DrawRequest>),
}

/// Reads the draws as d20 faces. More draws than needed, or a die that is not a d20, does not fit
/// the question and is refused.
fn supply(draws: &[Draw], needed: usize) -> Result<Supply, RulesRefusal> {
    if draws.len() > needed || draws.iter().any(|draw| draw.sides() != D20_SIDES) {
        return Err(RulesRefusal::DrawsUnfit);
    }
    if draws.len() < needed {
        return Ok(Supply::Missing(d20_requests(needed - draws.len())));
    }
    draws
        .iter()
        .map(|draw| u8::try_from(draw.value()).map_err(|_| RulesRefusal::FaceOutOfRange))
        .collect::<Result<Vec<_>, _>>()
        .map(Supply::Faces)
}

fn dice_for(mode: Option<RollMode>) -> usize {
    match mode {
        None => 0,
        Some(RollMode::Normal) => 1,
        Some(RollMode::Advantage | RollMode::Disadvantage) => 2,
    }
}

fn name_refused(_: NameError) -> RulesRefusal {
    RulesRefusal::FactName
}

/// Adds `value` as `positive` when it is zero or more and as `negative` (its size) when below zero,
/// because a fact is a whole number from zero.
fn signed_fact(
    resolved: Resolved,
    positive: &str,
    negative: &str,
    value: i16,
) -> Result<Resolved, RulesRefusal> {
    let label = if value >= 0 { positive } else { negative };
    resolved
        .with_fact(label, u64::from(value.unsigned_abs()))
        .map_err(name_refused)
}

fn flag_fact(resolved: Resolved, label: &str, set: bool) -> Result<Resolved, RulesRefusal> {
    resolved
        .with_fact(label, u64::from(set))
        .map_err(name_refused)
}

impl DfRulesOwner {
    pub fn new(roster: SeatRoster, book: ActionBook) -> Self {
        Self { roster, book }
    }

    /// What a seat must supply for a registered entry: the d20s to roll, or the decline.
    pub fn prepare_entry(
        &self,
        seat: SeatId,
        entry: &RulesEntry,
    ) -> Result<Preparation, PortError> {
        self.prepare_registered(seat, entry)
            .map_err(RulesRefusal::port_error)
    }

    fn prepare_registered(
        &self,
        seat: SeatId,
        entry: &RulesEntry,
    ) -> Result<Preparation, RulesRefusal> {
        let runnable = match entry {
            RulesEntry::Runs(runnable) => runnable,
            RulesEntry::Unsupported(piece) => {
                return Ok(Preparation::Declined(not_supported(*piece)?));
            }
        };
        if self.roster.hero(seat).is_none() {
            return Ok(Preparation::Declined(seat_has_no_hero()?));
        }
        let needed = match runnable {
            Runnable::SkillCheck { .. } => 1,
            Runnable::Influence(setup) => dice_for(setup.roll_mode()),
        };
        Ok(Preparation::Ready {
            draws: d20_requests(needed),
        })
    }

    /// Resolves a registered entry with the dice drawn for it: the result, the decline, or the dice
    /// still missing.
    pub fn resolve_entry(
        &self,
        seat: SeatId,
        entry: &RulesEntry,
        draws: &[Draw],
    ) -> Result<Resolution, PortError> {
        self.resolve_registered(seat, entry, draws)
            .map(|resolving| resolving.resolution)
            .map_err(RulesRefusal::port_error)
    }

    fn resolve_registered(
        &self,
        seat: SeatId,
        entry: &RulesEntry,
        draws: &[Draw],
    ) -> Result<Resolving, RulesRefusal> {
        let runnable = match entry {
            RulesEntry::Runs(runnable) => runnable,
            RulesEntry::Unsupported(piece) => {
                return Ok(Resolving::answer(Resolution::Declined(not_supported(
                    *piece,
                )?)));
            }
        };
        let Some(hero) = self.roster.hero(seat) else {
            return Ok(Resolving::answer(Resolution::Declined(seat_has_no_hero()?)));
        };
        match runnable {
            Runnable::SkillCheck {
                skill,
                difficulty_class,
            } => resolve_skill_check(hero, *skill, *difficulty_class, draws),
            Runnable::Influence(setup) => resolve_influence(hero, setup, draws),
        }
    }

    fn registered(&self, action: &ActionId) -> &RulesEntry {
        self.book.entry(action).unwrap_or(&UNREGISTERED)
    }
}

/// Writes the line of a call through the port: `RULES resolved` for a roll or an urging that
/// resolved, `RULES refused` for a call that could not be answered, nothing for another answer.
fn record_call<T>(
    action: &ActionId,
    operation: OperationId,
    result: &Result<T, RulesRefusal>,
    line: impl FnOnce(&T) -> Option<ResolvedLine>,
) {
    match result {
        Ok(answer) => {
            if let Some(line) = line(answer) {
                record(operation, resolved_line(action, operation, &line));
            }
        }
        Err(cause) => record(operation, refused_line(action, operation, *cause)),
    }
}

fn resolve_skill_check(
    hero: &HeroProfile,
    skill: Skill,
    difficulty_class: DifficultyClass,
    draws: &[Draw],
) -> Result<Resolving, RulesRefusal> {
    let faces = match supply(draws, 1)? {
        Supply::Missing(requests) => {
            return Ok(Resolving::answer(Resolution::NeedsDraws(requests)));
        }
        Supply::Faces(faces) => faces,
    };
    let [die] = faces.as_slice() else {
        return Err(RulesRefusal::DrawsUnfit);
    };
    let SkillBonus {
        ability_modifier,
        proficiency_bonus,
    } = hero.skill_bonus(skill);
    let result = ability_check::resolve(AbilityCheckInput {
        die: *die,
        ability_modifier,
        proficiency_bonus,
        difficulty_class: difficulty_class.get(),
    })
    .map_err(|_| RulesRefusal::CheckRefused)?;
    let line = ResolvedLine {
        kind: ResolvedKind::SkillCheck,
        natural: Some(result.die),
        total: Some(result.total),
        difficulty_class: result.difficulty_class,
        result: if result.succeeded {
            ResolvedResult::Success
        } else {
            ResolvedResult::Failure
        },
        mode: RollMode::Normal,
    };
    let resolved = Resolved::default()
        .with_fact("natural_die", u64::from(result.die))
        .map_err(name_refused)?;
    let resolved = signed_fact(
        resolved,
        "ability_bonus",
        "ability_penalty",
        i16::from(result.ability_modifier),
    )?;
    let resolved = resolved
        .with_fact("proficiency_bonus", u64::from(result.proficiency_bonus))
        .map_err(name_refused)?;
    let resolved = signed_fact(resolved, "total", "negative_total", result.total)?;
    let resolved = resolved
        .with_fact("difficulty_class", u64::from(result.difficulty_class))
        .map_err(name_refused)?;
    Ok(Resolving {
        resolution: Resolution::Applied(flag_fact(resolved, "succeeded", result.succeeded)?),
        line: Some(line),
    })
}

fn resolve_influence(
    hero: &HeroProfile,
    setup: &InfluenceSetup,
    draws: &[Draw],
) -> Result<Resolving, RulesRefusal> {
    let faces = match supply(draws, dice_for(setup.roll_mode()))? {
        Supply::Missing(requests) => {
            return Ok(Resolving::answer(Resolution::NeedsDraws(requests)));
        }
        Supply::Faces(faces) => faces,
    };
    let SkillBonus {
        ability_modifier,
        proficiency_bonus,
    } = hero.skill_bonus(influence_skill(setup));
    let result = influence::resolve(InfluenceInput {
        ability_modifier,
        proficiency_bonus,
        skill: setup.skill(),
        attitude: setup.attitude(),
        monster_intelligence: setup.monster_intelligence(),
        willingness: setup.willingness(),
        words: setup.words(),
        extra_sources: &[],
        faces: &faces,
    })
    .map_err(|_| RulesRefusal::InfluenceRefused)?;
    let line = ResolvedLine {
        kind: ResolvedKind::Influence,
        natural: result.counted,
        total: result.total,
        difficulty_class: result.difficulty_class,
        result: match result.outcome {
            InfluenceOutcome::Success => ResolvedResult::Success,
            InfluenceOutcome::Failure => ResolvedResult::Failure,
            InfluenceOutcome::NoRoll(_) => ResolvedResult::NoRoll,
        },
        mode: result.mode,
    };
    let resolved = match result.outcome {
        InfluenceOutcome::NoRoll(cause) => {
            let resolved = flag_fact(Resolved::default(), "no_roll", true)?;
            flag_fact(resolved, "complied", cause == NoRollCause::Willing)?
        }
        InfluenceOutcome::Success | InfluenceOutcome::Failure => {
            influence_roll_facts(&result, result.outcome == InfluenceOutcome::Success)?
        }
    };
    Ok(Resolving {
        resolution: Resolution::Applied(resolved),
        line: Some(line),
    })
}

fn influence_roll_facts(
    result: &influence::InfluenceResult,
    succeeded: bool,
) -> Result<Resolved, RulesRefusal> {
    let (counted, total) = match (result.counted, result.total) {
        (Some(counted), Some(total)) => (counted, total),
        _ => return Err(RulesRefusal::InfluenceIncomplete),
    };
    let dice_rolled =
        u64::try_from(result.faces.len()).map_err(|_| RulesRefusal::InfluenceIncomplete)?;
    let resolved = Resolved::default()
        .with_fact("dice_rolled", dice_rolled)
        .map_err(name_refused)?
        .with_fact("natural_die", u64::from(counted))
        .map_err(name_refused)?;
    let resolved = flag_fact(resolved, "advantage", result.mode == RollMode::Advantage)?;
    let resolved = flag_fact(
        resolved,
        "disadvantage",
        result.mode == RollMode::Disadvantage,
    )?;
    let resolved = signed_fact(
        resolved,
        "ability_bonus",
        "ability_penalty",
        i16::from(result.ability_modifier),
    )?;
    let resolved = resolved
        .with_fact("proficiency_bonus", u64::from(result.proficiency_bonus))
        .map_err(name_refused)?;
    let resolved = signed_fact(resolved, "total", "negative_total", total)?;
    let resolved = resolved
        .with_fact("difficulty_class", u64::from(result.difficulty_class))
        .map_err(name_refused)?;
    let resolved = flag_fact(resolved, "succeeded", succeeded)?;
    match result.retry_same_request_hours {
        Some(hours) => resolved
            .with_fact("retry_wait_hours", u64::from(hours))
            .map_err(name_refused),
        None => Ok(resolved),
    }
}

/// The skill whose bonus an influence urging adds: the five influence skills are all in the SRD
/// Skills table.
fn influence_skill(setup: &InfluenceSetup) -> Skill {
    match setup.skill() {
        influence::InfluenceSkill::Deception => Skill::Deception,
        influence::InfluenceSkill::Intimidation => Skill::Intimidation,
        influence::InfluenceSkill::Performance => Skill::Performance,
        influence::InfluenceSkill::Persuasion => Skill::Persuasion,
        influence::InfluenceSkill::AnimalHandling => Skill::AnimalHandling,
    }
}

impl RulesOwner for DfRulesOwner {
    fn legal_choices(
        &self,
        _correlation: Correlation,
        seat: SeatId,
    ) -> Result<Vec<ActionId>, PortError> {
        if self.roster.hero(seat).is_none() {
            return Ok(Vec::new());
        }
        Ok(self.book.runnable_actions())
    }

    fn prepare(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
    ) -> Result<Preparation, PortError> {
        let prepared = self.prepare_registered(seat, self.registered(action));
        record_call(action, correlation.operation_id, &prepared, |_| None);
        prepared.map_err(RulesRefusal::port_error)
    }

    fn resolve(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        draws: &[Draw],
    ) -> Result<Resolution, PortError> {
        let resolved = self.resolve_registered(seat, self.registered(action), draws);
        record_call(action, correlation.operation_id, &resolved, |resolving| {
            resolving.line
        });
        resolved
            .map(|resolving| resolving.resolution)
            .map_err(RulesRefusal::port_error)
    }

    // This adapter tracks no action economy, reserves nothing, asks for no confirmation and no
    // ruling, and runs no windows, so each of these says so explicitly.
    fn prepare_in_plan(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        _so_far: &PlanSoFar<'_>,
    ) -> Result<Preparation, PortError> {
        self.prepare(correlation, seat, action)
    }

    fn abandon(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        _action: &ActionId,
        _pending: PendingId,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn confirmation_needed(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        _action: &ActionId,
    ) -> Result<Option<ConfirmationReason>, PortError> {
        Ok(None)
    }

    fn ruling_needed(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        _action: &ActionId,
    ) -> Result<Option<RulingRequest>, PortError> {
        Ok(None)
    }

    fn resume(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        resumption: &Resumption<'_>,
    ) -> Result<Resolution, PortError> {
        self.resolve(correlation, seat, action, resumption.draws)
    }
}

#[cfg(test)]
mod audit_tests;
