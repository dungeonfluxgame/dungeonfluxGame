//! The structured d20 roll a combat outcome carries for the table, built from the server's own
//! numbers: the same rule for the table log (which attaches it to the outcome's line) and for
//! the phone's latest-roll card, so a stabilise check or a death save is never worded as an
//! attack with zeros.
//!
//! Attacks, opportunity attacks, death saving throws and Wisdom (Medicine) stabilise checks
//! have a roll; moves, recoveries and a healer's kit (which rolls nothing) have none.
use df_protocol::common as rpc;

/// SRD 5.2.1 p. 17: a Death Saving Throw succeeds on 10 or higher.
const DEATH_SAVE_DC: u32 = 10;

/// The roll an outcome made, or `None` when it made no d20 roll.
pub(crate) fn outcome_roll(outcome: &rpc::CombatOutcome) -> Option<rpc::TableRoll> {
    let kind = rpc::OutcomeKind::try_from(outcome.kind).ok()?;
    match kind {
        rpc::OutcomeKind::Attack | rpc::OutcomeKind::OpportunityAttack => attack_roll(outcome),
        rpc::OutcomeKind::DeathSave => {
            let succeeded = matches!(
                rpc::DeathSaveResult::try_from(outcome.death_save_result),
                Ok(rpc::DeathSaveResult::Success
                    | rpc::DeathSaveResult::Stabilised
                    | rpc::DeathSaveResult::Revived)
            );
            Some(rpc::TableRoll {
                faces: vec![outcome.attack_die],
                kept: outcome.attack_die,
                roll_mode: 0,
                modifier: 0,
                total: i32::try_from(outcome.attack_die).unwrap_or_default(),
                against: rpc::RollAgainst::Dc as i32,
                target_number: DEATH_SAVE_DC,
                succeeded,
                label: format!("{} \u{b7} Death save", outcome.actor_name),
                kind: rpc::RollKind::DeathSave as i32,
            })
        }
        rpc::OutcomeKind::Stabilisation => {
            let result = outcome.stabilization.as_ref()?;
            if rpc::StabilizationMethod::try_from(result.method)
                != Ok(rpc::StabilizationMethod::Medicine)
            {
                return None;
            }
            Some(rpc::TableRoll {
                faces: vec![result.die],
                kept: result.die,
                roll_mode: 0,
                modifier: result.wisdom_modifier,
                total: i32::try_from(result.die)
                    .unwrap_or_default()
                    .saturating_add(result.wisdom_modifier),
                against: rpc::RollAgainst::Dc as i32,
                target_number: result.dc,
                succeeded: result.succeeded,
                label: format!(
                    "{} \u{b7} Medicine (stabilize {})",
                    outcome.actor_name, outcome.target_name
                ),
                kind: rpc::RollKind::Stabilize as i32,
            })
        }
        rpc::OutcomeKind::Unspecified | rpc::OutcomeKind::Move | rpc::OutcomeKind::Recovery => None,
    }
}

/// How the phone's latest-roll card words a stabilise outcome: its own kind of roll, with the
/// Medicine die, the modifier, the total and the DC, or a healer's kit that rolled nothing.
#[cfg(test)]
pub(crate) struct StabiliseCard {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) explanation: String,
    pub(crate) source: String,
    pub(crate) verdict: &'static str,
    pub(crate) succeeded: bool,
}

#[cfg(test)]
pub(crate) fn stabilise_card(outcome: &rpc::CombatOutcome) -> Option<StabiliseCard> {
    if rpc::OutcomeKind::try_from(outcome.kind) != Ok(rpc::OutcomeKind::Stabilisation) {
        return None;
    }
    let result = outcome.stabilization.as_ref()?;
    let (actor, target) = (&outcome.actor_name, &outcome.target_name);
    match rpc::StabilizationMethod::try_from(result.method) {
        Ok(rpc::StabilizationMethod::Medicine) => {
            let roll = outcome_roll(outcome)?;
            Some(StabiliseCard {
                label: roll.label.clone(),
                value: roll.kept.to_string(),
                explanation: format!(
                    "d20 {} \u{b7} {} {} {} = {} vs DC {}",
                    roll.kept,
                    roll.kept,
                    if roll.modifier < 0 { "\u{2212}" } else { "+" },
                    roll.modifier.unsigned_abs(),
                    roll.total,
                    roll.target_number
                ),
                source: if result.succeeded {
                    format!("{target} is stable")
                } else {
                    format!("{target} is still dying")
                },
                verdict: if result.succeeded {
                    "Success"
                } else {
                    "Failure"
                },
                succeeded: result.succeeded,
            })
        }
        _ => Some(StabiliseCard {
            label: format!("{actor} uses a healer's kit on {target}"),
            value: "\u{2014}".to_owned(),
            explanation: "No roll: a healer's kit stabilizes".to_owned(),
            source: format!("{target} is stable"),
            verdict: "Stable",
            succeeded: result.succeeded,
        }),
    }
}

fn attack_roll(outcome: &rpc::CombatOutcome) -> Option<rpc::TableRoll> {
    if outcome.attack_die == 0 {
        return None;
    }
    let weapon = df_render::battle::float_text::attack_name(&outcome.weapon);
    Some(rpc::TableRoll {
        faces: if outcome.faces.is_empty() {
            vec![outcome.attack_die]
        } else {
            outcome.faces.clone()
        },
        kept: outcome.attack_die,
        roll_mode: outcome.roll_mode,
        modifier: outcome.attack_modifier,
        total: outcome.attack_total,
        against: if outcome.target_armor_class > 0 {
            rpc::RollAgainst::Ac
        } else {
            rpc::RollAgainst::Unspecified
        } as i32,
        target_number: outcome.target_armor_class,
        succeeded: outcome.hit,
        label: if weapon.is_empty() {
            outcome.actor_name.clone()
        } else {
            format!("{} \u{b7} {weapon}", outcome.actor_name)
        },
        kind: rpc::RollKind::Attack as i32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_medicine_stabilise_check_is_its_own_roll_never_an_attack_with_zeros() {
        let outcome = rpc::CombatOutcome {
            actor_name: "Sigrun".into(),
            target_name: "Brakka".into(),
            kind: rpc::OutcomeKind::Stabilisation as i32,
            stabilization: Some(rpc::StabilizationResult {
                method: rpc::StabilizationMethod::Medicine as i32,
                die: 13,
                wisdom_modifier: 1,
                dc: 10,
                succeeded: true,
                uses_left: 0,
            }),
            ..Default::default()
        };

        let roll = outcome_roll(&outcome).expect("a Medicine check rolls a die");

        assert_eq!(roll.kind, rpc::RollKind::Stabilize as i32);
        assert_eq!(roll.label, "Sigrun \u{b7} Medicine (stabilize Brakka)");
        assert_eq!((roll.kept, roll.modifier, roll.total), (13, 1, 14));
        assert_eq!(roll.faces, [13]);
        assert_eq!(roll.against, rpc::RollAgainst::Dc as i32);
        assert_eq!(roll.target_number, 10);
        assert!(roll.succeeded);
    }

    #[test]
    fn the_phone_card_for_a_stabilise_check_is_not_an_attack_with_zeros() {
        let outcome = rpc::CombatOutcome {
            actor_name: "Sigrun".into(),
            target_name: "Brakka".into(),
            kind: rpc::OutcomeKind::Stabilisation as i32,
            stabilization: Some(rpc::StabilizationResult {
                method: rpc::StabilizationMethod::Medicine as i32,
                die: 13,
                wisdom_modifier: 1,
                dc: 10,
                succeeded: true,
                uses_left: 0,
            }),
            ..Default::default()
        };

        let card = stabilise_card(&outcome).expect("a stabilise outcome has a card");

        assert_eq!(card.label, "Sigrun \u{b7} Medicine (stabilize Brakka)");
        assert_eq!(card.value, "13");
        assert_eq!(card.explanation, "d20 13 \u{b7} 13 + 1 = 14 vs DC 10");
        assert_eq!(card.verdict, "Success");
        assert_eq!(card.source, "Brakka is stable");
        assert!(!card.explanation.contains("AC"));
        assert!(stabilise_card(&rpc::CombatOutcome::default()).is_none());
    }

    #[test]
    fn a_healers_kit_card_says_no_roll() {
        let outcome = rpc::CombatOutcome {
            actor_name: "Sigrun".into(),
            target_name: "Brakka".into(),
            kind: rpc::OutcomeKind::Stabilisation as i32,
            stabilization: Some(rpc::StabilizationResult {
                method: rpc::StabilizationMethod::HealersKit as i32,
                succeeded: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let card = stabilise_card(&outcome).expect("a card");
        assert_eq!(card.label, "Sigrun uses a healer's kit on Brakka");
        assert_eq!(card.verdict, "Stable");
    }

    #[test]
    fn a_healers_kit_rolls_nothing() {
        let outcome = rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Stabilisation as i32,
            stabilization: Some(rpc::StabilizationResult {
                method: rpc::StabilizationMethod::HealersKit as i32,
                succeeded: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(outcome_roll(&outcome), None);
    }

    #[test]
    fn a_death_save_is_the_heros_roll_against_ten() {
        let outcome = rpc::CombatOutcome {
            actor_name: "Brakka".into(),
            attack_die: 16,
            kind: rpc::OutcomeKind::DeathSave as i32,
            death_save_result: rpc::DeathSaveResult::Success as i32,
            death_successes: 1,
            ..Default::default()
        };

        let roll = outcome_roll(&outcome).expect("a death save rolls a die");

        assert_eq!(roll.kind, rpc::RollKind::DeathSave as i32);
        assert_eq!(roll.label, "Brakka \u{b7} Death save");
        assert_eq!((roll.kept, roll.modifier, roll.total), (16, 0, 16));
        assert_eq!(roll.target_number, 10);
        assert!(roll.succeeded);

        let failed = rpc::CombatOutcome {
            attack_die: 4,
            death_save_result: rpc::DeathSaveResult::Failure as i32,
            ..outcome
        };
        assert!(!outcome_roll(&failed).expect("rolls").succeeded);
    }

    #[test]
    fn an_attack_carries_its_faces_modifier_total_and_armour_class() {
        let outcome = rpc::CombatOutcome {
            actor_name: "Corin".into(),
            kind: rpc::OutcomeKind::OpportunityAttack as i32,
            weapon: "shortsword".into(),
            attack_die: 17,
            faces: vec![17, 5],
            roll_mode: 1,
            attack_modifier: 4,
            attack_total: 21,
            target_armor_class: 16,
            hit: true,
            ..Default::default()
        };

        let roll = outcome_roll(&outcome).expect("an attack rolls a die");

        assert_eq!(roll.kind, rpc::RollKind::Attack as i32);
        assert_eq!(roll.label, "Corin \u{b7} Shortsword");
        assert_eq!(roll.faces, [17, 5]);
        assert_eq!((roll.roll_mode, roll.modifier, roll.total), (1, 4, 21));
        assert_eq!(roll.against, rpc::RollAgainst::Ac as i32);
        assert_eq!(roll.target_number, 16);
        assert!(roll.succeeded);
    }

    #[test]
    fn moves_and_recoveries_make_no_roll() {
        for kind in [rpc::OutcomeKind::Move, rpc::OutcomeKind::Recovery] {
            let outcome = rpc::CombatOutcome {
                kind: kind as i32,
                ..Default::default()
            };
            assert_eq!(outcome_roll(&outcome), None);
        }
    }
}
