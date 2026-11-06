//! The `RULES resolved` and `RULES refused` lines against the resolutions they describe.

use std::collections::BTreeSet;
use std::num::{NonZeroU16, NonZeroU128};

use df_rules::influence::{Attitude, InfluenceSkill, RollMode, Willingness, WordQuality, WordTilt};
use df_table::identity::{OperationId, SeatId};
use df_table::ports::{ActionId, Draw, Resolution};

use super::super::action_book::{ActionBook, DifficultyClass, InfluenceSetup, RulesEntry};
use super::super::hero_profile::{HeroProfile, SeatRoster, Skill};
use super::audit::{
    ResolvedKind, ResolvedLine, ResolvedResult, RulesRefusal, correlation, refused_line,
    resolved_line,
};
use super::{DfRulesOwner, Resolving};

const SEAT: SeatId = SeatId::new(1);

fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("a valid action name")
}

fn d20(value: u16) -> Draw {
    Draw::new(NonZeroU16::new(20).expect("sides"), value).expect("on the die")
}

fn operation() -> OperationId {
    OperationId::from_nonzero_u128(NonZeroU128::new(7).expect("not zero"))
}

/// A hero proficient in Sleight of Hand with Dexterity 14 (+2) and a +2 bonus: +4.
fn owner() -> DfRulesOwner {
    let hero =
        HeroProfile::new([10, 14, 10, 10, 10, 10], 2, [Skill::SleightOfHand]).expect("valid");
    let roster = SeatRoster::default()
        .with_hero(SEAT, hero)
        .expect("one hero");
    DfRulesOwner::new(roster, ActionBook::default())
}

fn fact(resolution: &Resolution, label: &str) -> Option<u64> {
    let Resolution::Applied(resolved) = resolution else {
        return None;
    };
    resolved
        .facts
        .iter()
        .find(|fact| fact.label.as_str() == label)
        .map(|fact| fact.amount)
}

fn resolving(entry: &RulesEntry, draws: &[Draw]) -> Resolving {
    owner()
        .resolve_registered(SEAT, entry, draws)
        .expect("resolves")
}

#[test]
fn a_skill_checks_line_carries_the_die_total_dc_and_result_of_its_facts() {
    let entry = RulesEntry::skill_check(
        Skill::SleightOfHand,
        DifficultyClass::new(15).expect("a DC"),
    );

    for die in [11, 10] {
        let resolved = resolving(&entry, &[d20(die)]);
        let line = resolved.line.expect("a roll has a line");

        assert_eq!(line.kind, ResolvedKind::SkillCheck);
        assert_eq!(
            line.natural.map(u64::from),
            fact(&resolved.resolution, "natural_die")
        );
        assert_eq!(
            line.total.and_then(|total| u64::try_from(total).ok()),
            fact(&resolved.resolution, "total")
        );
        assert_eq!(
            Some(u64::from(line.difficulty_class)),
            fact(&resolved.resolution, "difficulty_class")
        );
        let succeeded = fact(&resolved.resolution, "succeeded") == Some(1);
        assert_eq!(
            line.result,
            if succeeded {
                ResolvedResult::Success
            } else {
                ResolvedResult::Failure
            }
        );
        assert_eq!(line.mode, RollMode::Normal);
    }
    let line = resolving(&entry, &[d20(11)]).line.expect("a line");
    assert_eq!(
        resolved_line(&action("pick_lock"), operation(), &line),
        format!(
            "RULES resolved action=pick_lock operation={operation} kind=skill_check natural=11 \
             total=15 dc=15 result=success mode=normal",
            operation = operation()
        )
    );
}

#[test]
fn an_urging_with_advantage_names_the_counted_die_and_one_without_a_roll_says_so() {
    let urging = |attitude, willingness| {
        RulesEntry::influence(
            InfluenceSetup::new(
                InfluenceSkill::Persuasion,
                attitude,
                10,
                willingness,
                WordQuality {
                    tilt: WordTilt::Neutral,
                    reason: "you named what he is proud of",
                },
            )
            .expect("an urging"),
        )
    };
    let friendly = resolving(
        &urging(Attitude::Friendly, Willingness::Hesitant),
        &[d20(14), d20(6)],
    );
    let willing = resolving(&urging(Attitude::Indifferent, Willingness::Willing), &[]);

    let line = friendly.line.expect("a roll has a line");
    assert_eq!(line.kind, ResolvedKind::Influence);
    assert_eq!(line.mode, RollMode::Advantage);
    assert_eq!(
        line.natural.map(u64::from),
        fact(&friendly.resolution, "natural_die")
    );
    assert_eq!(line.natural, Some(14));
    let no_roll = willing.line.expect("an urging without a roll has a line");
    assert_eq!(no_roll.result, ResolvedResult::NoRoll);
    assert_eq!((no_roll.natural, no_roll.total), (None, None));
    assert!(
        resolved_line(&action("urge"), operation(), &no_roll).contains(" natural=- total=- dc="),
        "{no_roll:?}"
    );
}

#[test]
fn a_call_the_adapter_cannot_answer_has_a_closed_cause_and_a_missing_die_no_line() {
    let entry = RulesEntry::skill_check(
        Skill::SleightOfHand,
        DifficultyClass::new(15).expect("a DC"),
    );

    let two = owner().resolve_registered(SEAT, &entry, &[d20(3), d20(4)]);
    let none = resolving(&entry, &[]);

    assert_eq!(two.err(), Some(RulesRefusal::DrawsUnfit));
    assert!(none.line.is_none());
    assert_eq!(
        refused_line(&action("pick_lock"), operation(), RulesRefusal::DrawsUnfit),
        format!(
            "RULES refused action=pick_lock operation={} cause=draws_unfit",
            operation()
        )
    );
    let keys: BTreeSet<&str> = RulesRefusal::ALL.iter().map(|cause| cause.key()).collect();
    assert_eq!(keys.len(), RulesRefusal::ALL.len());
    assert!(keys.iter().all(|key| {
        key.chars()
            .all(|letter| letter.is_ascii_lowercase() || letter == '_')
    }));
}

#[test]
fn a_resolved_line_is_one_line_of_closed_keys_and_numbers() {
    let line = ResolvedLine {
        kind: ResolvedKind::Influence,
        natural: Some(1),
        total: Some(-2),
        difficulty_class: 15,
        result: ResolvedResult::Failure,
        mode: RollMode::Disadvantage,
    };

    let text = resolved_line(&action("urge"), operation(), &line);

    assert!(!text.contains('\n'));
    assert!(
        text.ends_with(" kind=influence natural=1 total=-2 dc=15 result=failure mode=disadvantage"),
        "{text}"
    );
}

#[test]
fn a_rules_line_names_its_operation_in_the_shared_op_key_too() {
    let line = refused_line(&action("pick_lock"), operation(), RulesRefusal::DrawsUnfit);

    let _scope = df_observe::audit_scope(correlation(operation()));
    let audited = df_observe::audit_line(line);

    let hash = df_observe::operation_hash(operation().as_bytes());
    assert!(audited.contains(&format!(" op={hash} ")), "{audited}");
    assert!(!audited.contains("op=none"), "{audited}");
}
