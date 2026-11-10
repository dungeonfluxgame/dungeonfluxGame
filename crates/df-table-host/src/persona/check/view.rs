//! A resolved roll as the shared screen shows it: the conversion from `df-rules`' `RollResult` to
//! the display's `RollView` (`df_playground::fixture`), which is the only definition of the wire
//! type.
//!
//! Every face, modifier, total, target and verdict is the result's. The only work here is the
//! display's vocabulary: a percentile pair of d10 becomes the one die of 100 sides the card
//! draws, the death-save verdicts the card has no word for map to its success and failure with
//! the natural mark carrying the rest, a contested check reads won or lost, and a roll with no
//! target (damage, initiative, a table) has no verdict. It rolls nothing and checks nothing the
//! page checks again when it arrives.

use df_playground::fixture::{
    DieRole as ViewRole, InspirationReroll, NaturalMark, RollDie, RollKind, RollMode as ViewMode,
    RollModifier, RollOutcome as ViewOutcome, RollTarget, RollView,
};
use df_rules::roll::{
    AttackKind, DiePlace, DieRole, Natural, RollError, RollMode, RollOutcome, RollRequest,
    RollResult, RollSubject, RolledDie,
};

use super::numbers::{ability_word, modifier_word, skill_word, target_word};
use super::request::CheckError;

fn out_of_range(die: &RolledDie) -> CheckError {
    CheckError::Roll {
        cause: RollError::FaceOutOfRange {
            sides: die.sides,
            face: die.face,
        },
    }
}

fn narrow(value: u16, die: &RolledDie) -> Result<u8, CheckError> {
    u8::try_from(value).map_err(|_| out_of_range(die))
}

fn role_of(role: DieRole) -> ViewRole {
    match role {
        DieRole::Test => ViewRole::Test,
        DieRole::Bonus => ViewRole::Bonus,
        DieRole::Damage => ViewRole::Damage,
        DieRole::Healing => ViewRole::Healing,
        DieRole::HitDie => ViewRole::HitDie,
        DieRole::Table => ViewRole::Table,
    }
}

/// The dice the card draws: each die as it was rolled, a tens die and its ones die as the one
/// percentile die of 100 sides, 1 to 100 with "00" as 100 (SRD 5.2.1 page 27).
fn view_dice(dice: &[RolledDie]) -> Result<Vec<RollDie>, CheckError> {
    let mut shown = Vec::with_capacity(dice.len());
    let mut each = dice.iter();
    while let Some(die) = each.next() {
        if die.place == DiePlace::Tens {
            let ones = each.next().ok_or_else(|| out_of_range(die))?;
            let value = match die.counted() + ones.counted() {
                0 => 100,
                value => value,
            };
            shown.push(RollDie {
                sides: 100,
                face: narrow(value, die)?,
                role: role_of(die.role),
                kept: die.kept,
            });
        } else {
            shown.push(RollDie {
                sides: narrow(die.sides, die)?,
                face: narrow(die.face, die)?,
                role: role_of(die.role),
                kept: die.kept,
            });
        }
    }
    Ok(shown)
}

fn kind_of(subject: RollSubject, contested: bool) -> RollKind {
    if contested {
        return RollKind::ContestedCheck;
    }
    match subject {
        RollSubject::AbilityCheck(_) => RollKind::AbilityCheck,
        RollSubject::SkillCheck(_) => RollKind::SkillCheck,
        RollSubject::SavingThrow(_) => RollKind::SavingThrow,
        RollSubject::Attack(AttackKind::Melee) => RollKind::MeleeAttack,
        RollSubject::Attack(AttackKind::Ranged) => RollKind::RangedAttack,
        RollSubject::Initiative => RollKind::Initiative,
        RollSubject::DeathSave => RollKind::DeathSavingThrow,
        RollSubject::PassivePerception => RollKind::PassivePerception,
        RollSubject::Dice(DieRole::Damage) => RollKind::Damage,
        RollSubject::Dice(DieRole::Healing) => RollKind::Healing,
        RollSubject::Dice(DieRole::HitDie) => RollKind::HitDie,
        RollSubject::Dice(DieRole::Table) => RollKind::TableRoll,
        RollSubject::Dice(DieRole::Bonus | DieRole::Test) => RollKind::BonusDie,
    }
}

/// What is tested, as the card's title says it: `Athletics (Strength)`.
fn title_of(subject: RollSubject, contested: bool) -> String {
    let title = match subject {
        RollSubject::SkillCheck(skill) => {
            format!("{} ({})", skill_word(skill), ability_word(skill.ability()))
        }
        RollSubject::AbilityCheck(ability) => format!("{} check", ability_word(ability)),
        RollSubject::SavingThrow(ability) => format!("{} saving throw", ability_word(ability)),
        RollSubject::Attack(AttackKind::Melee) => "Melee attack".to_owned(),
        RollSubject::Attack(AttackKind::Ranged) => "Ranged attack".to_owned(),
        RollSubject::Initiative => "Initiative".to_owned(),
        RollSubject::DeathSave => "Death saving throw".to_owned(),
        RollSubject::PassivePerception => "Passive Perception".to_owned(),
        RollSubject::Dice(DieRole::Damage) => "Damage".to_owned(),
        RollSubject::Dice(DieRole::Healing) => "Healing".to_owned(),
        RollSubject::Dice(DieRole::HitDie) => "Hit die".to_owned(),
        RollSubject::Dice(DieRole::Table) => "Table roll".to_owned(),
        RollSubject::Dice(DieRole::Bonus | DieRole::Test) => "Bonus die".to_owned(),
    };
    if contested {
        format!("{title} contest")
    } else {
        title
    }
}

fn outcome_of(outcome: RollOutcome, contested: bool) -> Option<ViewOutcome> {
    match (outcome, contested) {
        (RollOutcome::Success, true) => Some(ViewOutcome::Won),
        (RollOutcome::Failure, true) => Some(ViewOutcome::Lost),
        (
            RollOutcome::Success | RollOutcome::DeathSaveSuccess | RollOutcome::DeathSaveRevived,
            _,
        ) => Some(ViewOutcome::Success),
        (
            RollOutcome::Failure
            | RollOutcome::DeathSaveFailure
            | RollOutcome::DeathSaveTwoFailures,
            _,
        ) => Some(ViewOutcome::Failure),
        (RollOutcome::Hit, _) => Some(ViewOutcome::Hit),
        (RollOutcome::Miss, _) => Some(ViewOutcome::Miss),
        (RollOutcome::CriticalHit, _) => Some(ViewOutcome::CriticalHit),
        (RollOutcome::Rolled | RollOutcome::Score, _) => None,
    }
}

fn mode_of(mode: RollMode) -> ViewMode {
    match mode {
        RollMode::Normal => ViewMode::Normal,
        RollMode::Advantage => ViewMode::Advantage,
        RollMode::Disadvantage => ViewMode::Disadvantage,
    }
}

fn natural_of(natural: Natural) -> NaturalMark {
    match natural {
        Natural::One => NaturalMark::One,
        Natural::Twenty => NaturalMark::Twenty,
        Natural::None => NaturalMark::Unmarked,
    }
}

/// The card for `result`, named `id`. `contested` says the check is one of the two contests, whose
/// verdict reads won or lost. `request` names the die Heroic Inspiration replaced.
pub(super) fn roll_view(
    id: String,
    request: &RollRequest,
    result: &RollResult,
    contested: bool,
) -> Result<RollView, CheckError> {
    let place = match request {
        RollRequest::D20(request) => request.heroic_inspiration,
        RollRequest::Dice(request) => request.heroic_inspiration,
        RollRequest::PassivePerception(_) => None,
    }
    .map(|spent| usize::from(spent.replaces_die));
    let inspiration_reroll = match (result.inspiration_reroll, place) {
        (Some(was), Some(index)) => Some(InspirationReroll {
            index,
            was: narrow(was.face, &was)?,
        }),
        _ => None,
    };
    Ok(RollView {
        id,
        title: title_of(result.subject, contested),
        kind: kind_of(result.subject, contested),
        mode: mode_of(result.mode),
        dice: view_dice(&result.dice)?,
        modifiers: result
            .modifiers
            .iter()
            .map(|modifier| RollModifier {
                label: modifier_word(modifier.label).to_owned(),
                value: i32::from(modifier.value),
            })
            .collect(),
        total: i32::from(result.total),
        target: result.target.map(|target| RollTarget {
            label: target_word(target).to_owned(),
            value: i32::from(target.value),
        }),
        outcome: outcome_of(result.outcome, contested),
        natural: natural_of(result.natural),
        inspiration_reroll,
    })
}
