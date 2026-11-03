use std::num::NonZeroU16;

use df_table::identity::SeatId;
use df_table::ports::{ActionId, DrawRequest, Preparation, Resolution};
use df_table::testkit::correlation;

use super::backend::{RULES_BACKEND_VARIABLE, RulesBackend, dev_ports, dev_roster};
use super::error::AdapterError;
use super::hero_profile::Skill;

fn check() -> ActionId {
    ActionId::new("pick_lock").expect("a valid action name")
}

fn d20_request() -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::new(20).expect("sides"),
    }
}

#[test]
fn the_backend_names_are_scripted_and_df_rules_and_nothing_else_parses() {
    assert_eq!(RulesBackend::parse("scripted"), Ok(RulesBackend::Scripted));
    assert_eq!(RulesBackend::parse("df-rules"), Ok(RulesBackend::DfRules));
    for backend in RulesBackend::ALL {
        assert_eq!(RulesBackend::parse(backend.key()), Ok(backend));
    }
    for text in ["", "df_rules", "DF-RULES", "real", " scripted"] {
        assert_eq!(
            RulesBackend::parse(text),
            Err(AdapterError::UnknownRulesBackend {
                text: text.to_owned()
            })
        );
    }
}

#[test]
fn an_unknown_backend_error_names_the_setting_and_the_choices() {
    let message = AdapterError::UnknownRulesBackend {
        text: "real".to_owned(),
    }
    .to_string();

    assert!(message.contains(RULES_BACKEND_VARIABLE), "{message}");
    assert!(message.contains("scripted"), "{message}");
    assert!(message.contains("df-rules"), "{message}");
}

#[test]
fn the_dev_roster_seats_a_hero_for_each_dev_seat_and_for_no_other() {
    let roster = dev_roster().expect("the roster builds");

    for seat in 1..=6 {
        assert!(roster.hero(SeatId::new(seat)).is_some(), "seat {seat}");
    }
    assert!(roster.hero(SeatId::new(0)).is_none());
    assert!(roster.hero(SeatId::new(7)).is_none());
    let bonus = roster
        .hero(SeatId::new(1))
        .expect("a hero")
        .skill_bonus(Skill::SleightOfHand);
    assert_eq!((bonus.ability_modifier, bonus.proficiency_bonus), (2, 2));
}

#[test]
fn the_scripted_backend_keeps_the_forceable_dice_and_has_no_check_kit() {
    let ports = dev_ports(RulesBackend::Scripted, &check(), 7).expect("builds");

    assert!(ports.dice_control.is_some());
    assert!(ports.check_kit.is_none());
    assert_eq!(
        ports.rules.legal_choices(correlation(1), SeatId::new(1)),
        Ok(vec![check()])
    );
}

#[test]
fn the_df_rules_backend_runs_the_dev_check_on_the_production_adapter_and_recording_dice() {
    let ports = dev_ports(RulesBackend::DfRules, &check(), 7).expect("builds");

    assert!(ports.dice_control.is_none());
    assert!(ports.check_kit.is_some());
    let choices = ports
        .rules
        .legal_choices(correlation(1), SeatId::new(1))
        .expect("answers");
    assert_eq!(choices, [check()]);
    let prepared = ports
        .rules
        .prepare(correlation(1), SeatId::new(1), &check())
        .expect("answers");
    assert_eq!(
        prepared,
        Preparation::Ready {
            draws: vec![d20_request()]
        }
    );
    let drawn = ports
        .dice
        .draw(correlation(1), d20_request())
        .expect("a draw");
    let resolved = ports
        .rules
        .resolve(correlation(1), SeatId::new(1), &check(), &[drawn])
        .expect("answers");
    assert!(matches!(resolved, Resolution::Applied(_)));
}

#[test]
fn the_same_seed_gives_the_same_dice_in_two_runs_of_the_df_rules_backend() {
    let first = dev_ports(RulesBackend::DfRules, &check(), 99).expect("builds");
    let second = dev_ports(RulesBackend::DfRules, &check(), 99).expect("builds");

    let rolled = |ports: &super::backend::DevPorts| {
        (1..=8)
            .map(|turn| {
                ports
                    .dice
                    .draw(correlation(turn), d20_request())
                    .expect("a draw")
                    .value()
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(rolled(&first), rolled(&second));
}
