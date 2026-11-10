//! The roll card's data: the closed keys, the contract's round trip, the checks a live change
//! passes, the schedule of a card, the tumble of its dice and how the overlay and the draw plan
//! treat it.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::controls::Controls;
use crate::draw_plan::{DrawPlan, Part, Snapshot};
use crate::fixture::{
    DieRole, Fixture, InspirationReroll, NaturalMark, RollDie, RollKind, RollMode, RollModifier,
    RollOutcome, RollTarget, RollView,
};
use crate::fixtures::assert_keys_are_serde_names;
use crate::limits::{
    MAXIMUM_ROLL_DICE, MAXIMUM_ROLL_TUMBLE_MS, MINIMUM_ROLL_HOLD_MS, MINIMUM_ROLL_TUMBLE_MS,
    ROLL_CALL_MS, ROLL_EXIT_MS, ROLL_SETTLE_MS,
};
use crate::roll::{RollPhase, RollSchedule, ShownRoll, TumblePlan, covers_panels};
use crate::scene::{SceneChange, SceneError, SceneOverlay};

fn base() -> Fixture {
    Fixture::parse(concat!(
        r#"{"name":"B","tv":{"place":"P","status":{"kind":"ok"},"elements":[]},"#,
        r#""phone":{"hero":"H","status":{"kind":"ok"}}}"#
    ))
    .expect("reads")
}

fn die(sides: u8, face: u8, role: DieRole, kept: bool) -> RollDie {
    RollDie {
        sides,
        face,
        role,
        kept,
    }
}

/// Athletics with proficiency: a 14 and 3 and 2 against DC 15.
fn athletics() -> RollView {
    RollView {
        id: "r1".to_owned(),
        title: "Athletics (Strength)".to_owned(),
        kind: RollKind::SkillCheck,
        mode: RollMode::Normal,
        dice: vec![die(20, 14, DieRole::Test, true)],
        modifiers: vec![
            RollModifier {
                label: "Strength".to_owned(),
                value: 3,
            },
            RollModifier {
                label: "Proficiency".to_owned(),
                value: 2,
            },
        ],
        total: 19,
        target: Some(RollTarget {
            label: "DC".to_owned(),
            value: 15,
        }),
        outcome: Some(RollOutcome::Success),
        natural: NaturalMark::Unmarked,
        inspiration_reroll: None,
    }
}

fn drawn(overlay: &SceneOverlay) -> Fixture {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

fn snapshot(fixture: &Fixture) -> Snapshot {
    Snapshot {
        names: Vec::new(),
        controls: Controls::default(),
        loaded_name: None,
        shown: Some(Rc::new(Ok(fixture.clone()))),
        connection_problem: None,
        live_changes: false,
        audio_unlocked: false,
    }
}

#[test]
fn the_closed_keys_of_a_roll_are_their_serde_names() {
    assert_keys_are_serde_names(&RollKind::ALL, RollKind::key, RollKind::from_key);
    assert_keys_are_serde_names(&RollMode::ALL, RollMode::key, RollMode::from_key);
    assert_keys_are_serde_names(&DieRole::ALL, DieRole::key, DieRole::from_key);
    assert_keys_are_serde_names(&NaturalMark::ALL, NaturalMark::key, NaturalMark::from_key);
    assert_keys_are_serde_names(&RollOutcome::ALL, RollOutcome::key, RollOutcome::from_key);
}

#[test]
fn a_roll_survives_the_wire_and_a_misspelt_key_is_refused() {
    let mut roll = athletics();
    roll.mode = RollMode::Advantage;
    roll.dice = vec![
        die(20, 14, DieRole::Test, true),
        die(20, 6, DieRole::Test, false),
    ];
    roll.inspiration_reroll = Some(InspirationReroll { index: 1, was: 3 });
    let text = serde_json::to_string(&roll).expect("writes");
    assert_eq!(
        serde_json::from_str::<RollView>(&text).expect("reads"),
        roll
    );
    let misspelt = text.replace("\"total\"", "\"totall\"");
    assert!(serde_json::from_str::<RollView>(&misspelt).is_err());
}

#[test]
fn a_roll_with_only_the_required_keys_reads_with_the_documented_defaults() {
    let roll: RollView = serde_json::from_str(concat!(
        r#"{"id":"r","title":"Fireball","kind":"damage","#,
        r#""dice":[{"sides":6,"face":4,"role":"damage"}],"total":4}"#
    ))
    .expect("reads");
    assert_eq!(roll.mode, RollMode::Normal);
    assert!(roll.dice[0].kept);
    assert_eq!((roll.target, roll.outcome), (None, None));
    assert_eq!(roll.natural, NaturalMark::Unmarked);
    assert_eq!(roll.inspiration_reroll, None);
}

#[test]
fn a_natural_mark_is_written_as_none_when_there_is_none() {
    assert_eq!(
        serde_json::to_string(&NaturalMark::Unmarked).expect("writes"),
        "\"none\""
    );
}

#[test]
fn a_drawable_roll_passes_its_check() {
    assert_eq!(athletics().check(), Ok(()));
}

#[test]
fn a_percentile_die_may_land_on_one_hundred() {
    let mut roll = athletics();
    roll.kind = RollKind::TableRoll;
    roll.dice = vec![die(100, 100, DieRole::Table, true)];
    roll.modifiers = Vec::new();
    roll.total = 100;
    assert_eq!(roll.check(), Ok(()));
}

#[test]
fn an_id_that_is_not_a_plain_name_is_refused() {
    let mut roll = athletics();
    roll.id = "roll one".to_owned();
    assert_eq!(
        roll.check(),
        Err(SceneError::InvalidRollId("roll one".to_owned()))
    );
}

#[test]
fn a_die_the_art_has_no_picture_of_is_refused() {
    let mut roll = athletics();
    roll.dice = vec![die(7, 3, DieRole::Test, true)];
    roll.total = 8;
    assert_eq!(
        roll.check(),
        Err(SceneError::InvalidDie { sides: 7, face: 3 })
    );
}

#[test]
fn a_face_the_die_does_not_have_is_refused() {
    for (sides, face) in [(20, 21), (20, 0), (6, 7)] {
        let mut roll = athletics();
        roll.dice = vec![die(sides, face, DieRole::Test, true)];
        assert_eq!(
            roll.check(),
            Err(SceneError::InvalidDie { sides, face }),
            "face {face} of {sides}"
        );
    }
}

#[test]
fn a_total_that_is_not_the_kept_dice_and_modifiers_is_refused() {
    let mut roll = athletics();
    roll.total = 21;
    assert_eq!(
        roll.check(),
        Err(SceneError::RollTotalMismatch { sum: 19, total: 21 })
    );
}

#[test]
fn a_die_left_out_does_not_count_in_the_sum() {
    let mut roll = athletics();
    roll.mode = RollMode::Disadvantage;
    roll.dice = vec![
        die(20, 14, DieRole::Test, false),
        die(20, 9, DieRole::Test, true),
    ];
    roll.total = 14;
    assert_eq!(roll.check(), Ok(()));
}

#[test]
fn a_reroll_must_name_a_die_of_the_roll_and_a_face_it_has() {
    let mut roll = athletics();
    roll.inspiration_reroll = Some(InspirationReroll { index: 1, was: 3 });
    assert_eq!(roll.check(), Err(SceneError::InvalidReroll));
    roll.inspiration_reroll = Some(InspirationReroll { index: 0, was: 21 });
    assert_eq!(roll.check(), Err(SceneError::InvalidReroll));
    roll.inspiration_reroll = Some(InspirationReroll { index: 0, was: 3 });
    assert_eq!(roll.check(), Ok(()));
}

#[test]
fn a_roll_of_more_dice_than_the_limit_is_refused() {
    let mut roll = athletics();
    roll.dice = vec![die(6, 1, DieRole::Damage, true); MAXIMUM_ROLL_DICE + 1];
    assert!(matches!(
        roll.check(),
        Err(SceneError::TooMany {
            what: "a roll's dice",
            ..
        })
    ));
}

#[test]
fn the_card_waits_for_the_call_then_tumbles_then_writes_the_sum_after_the_dice_lie_still() {
    let schedule = RollSchedule::new(1_500, 0);
    assert_eq!(schedule.tumble_start_ms(), ROLL_CALL_MS);
    assert_eq!(schedule.tumble_end_ms(), ROLL_CALL_MS + 1_500);
    assert_eq!(
        schedule.breakdown_ms(),
        schedule.tumble_end_ms() + ROLL_SETTLE_MS
    );
    assert!(schedule.breakdown_ms() > schedule.tumble_end_ms());
    assert!(schedule.verdict_ms() > schedule.breakdown_ms());
}

#[test]
fn the_call_is_about_two_seconds_and_the_tumble_stays_between_one_point_two_and_one_point_eight() {
    assert_eq!(ROLL_CALL_MS, 2_000);
    assert_eq!(RollSchedule::new(10, 0).tumble_ms(), MINIMUM_ROLL_TUMBLE_MS);
    assert_eq!(
        RollSchedule::new(10_000, 0).tumble_ms(),
        MAXIMUM_ROLL_TUMBLE_MS
    );
    assert_eq!(RollSchedule::new(1_500, 0).tumble_ms(), 1_500);
}

#[test]
fn the_verdict_is_held_at_least_six_seconds_and_never_less_than_the_hold_asked_for() {
    assert_eq!(RollSchedule::new(1_500, 0).hold_ms(), MINIMUM_ROLL_HOLD_MS);
    assert_eq!(RollSchedule::new(1_500, 5_999).hold_ms(), 6_000);
    assert_eq!(RollSchedule::new(1_500, 9_300).hold_ms(), 9_300);
    let schedule = RollSchedule::new(1_500, 9_300);
    assert_eq!(schedule.off_ms() - schedule.verdict_ms(), 9_300);
    assert!(schedule.off_ms() - schedule.verdict_ms() >= MINIMUM_ROLL_HOLD_MS);
    assert_eq!(schedule.gone_ms() - schedule.off_ms(), ROLL_EXIT_MS);
}

#[test]
fn a_card_is_in_its_stage_at_each_moment_of_its_run() {
    let schedule = RollSchedule::new(1_500, 0);
    let at = |ms| schedule.phase_at(ms);
    assert_eq!(at(0), RollPhase::Call);
    assert_eq!(at(ROLL_CALL_MS - 1), RollPhase::Call);
    assert_eq!(at(ROLL_CALL_MS), RollPhase::Tumble);
    assert_eq!(at(schedule.tumble_end_ms() - 1), RollPhase::Tumble);
    assert_eq!(at(schedule.tumble_end_ms()), RollPhase::Settle);
    assert_eq!(at(schedule.breakdown_ms()), RollPhase::Breakdown);
    assert_eq!(at(schedule.off_ms() - 1), RollPhase::Breakdown);
    assert_eq!(at(schedule.off_ms()), RollPhase::Leaving);
    assert_eq!(at(schedule.gone_ms()), RollPhase::Gone);
}

#[test]
fn the_card_covers_the_other_panels_until_it_begins_to_leave() {
    let covering = [
        RollPhase::Call,
        RollPhase::Tumble,
        RollPhase::Settle,
        RollPhase::Breakdown,
    ];
    assert!(covering.iter().all(|phase| phase.covers_panels()));
    assert!(!RollPhase::Leaving.covers_panels());
    assert!(!RollPhase::Gone.covers_panels());
}

#[test]
fn the_panels_step_aside_for_a_roll_the_page_has_not_seen_leave() {
    let shown = ShownRoll {
        view: athletics(),
        min_hold_ms: 0,
        serial: 0,
        began_ms: 0,
        elapsed_ms: 0,
    };
    assert!(!covers_panels(None, None));
    assert!(covers_panels(Some(&shown), None));
    assert!(covers_panels(Some(&shown), Some(4)));
    assert!(!covers_panels(Some(&shown), Some(0)));
}

#[test]
fn a_page_that_opens_after_a_roll_ran_its_course_does_not_show_it() {
    let schedule = RollSchedule::new(1_500, 0);
    let mut shown = ShownRoll {
        view: athletics(),
        min_hold_ms: 0,
        serial: 3,
        began_ms: 0,
        elapsed_ms: schedule.gone_ms() - 1,
    };
    assert!(!shown.is_over(schedule));
    shown.elapsed_ms = schedule.gone_ms();
    assert!(shown.is_over(schedule));
}

#[test]
fn the_same_seed_tumbles_the_same_way_and_other_seeds_do_not() {
    let plan = TumblePlan::new(7, 3);
    assert_eq!(plan, TumblePlan::new(7, 3));
    let others: BTreeSet<String> = (0..20)
        .map(|seed| format!("{:?}", TumblePlan::new(seed, 3).dice))
        .collect();
    assert_eq!(others.len(), 20, "twenty seeds, twenty different throws");
}

#[test]
fn every_die_lands_before_the_tumble_ends_and_the_tumble_keeps_its_bounds() {
    for seed in 0..500_u64 {
        let plan = TumblePlan::new(seed, 6);
        assert!((MINIMUM_ROLL_TUMBLE_MS..=MAXIMUM_ROLL_TUMBLE_MS).contains(&plan.tumble_ms));
        assert_eq!(plan.dice.len(), 6);
        for tumble in &plan.dice {
            assert!(tumble.delay_ms < MINIMUM_ROLL_TUMBLE_MS / 2, "seed {seed}");
            assert!(tumble.spin_degrees.abs() >= 540 && tumble.spin_degrees.abs() <= 1_260);
            assert!((4..=26).contains(&tumble.rest_degrees.abs()));
            assert!((4..=18).contains(&tumble.rest_x_percent.abs()));
            assert!((2..=6).contains(&tumble.rest_y_percent.abs()));
            assert!(tumble.from_x_cqw.abs() <= 25 && tumble.from_y_cqw <= -1);
        }
    }
}

#[test]
fn a_tumble_never_depends_on_the_faces() {
    let one = TumblePlan::new(11, 2);
    let mut roll = athletics();
    roll.dice = vec![
        die(20, 1, DieRole::Test, true),
        die(20, 20, DieRole::Test, true),
    ];
    assert_eq!(one, TumblePlan::new(11, roll.dice.len()));
}

#[test]
fn a_roll_set_by_the_host_is_stamped_with_the_next_serial_and_the_time_it_began() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply_at(
            SceneChange::Roll {
                view: athletics(),
                min_hold_ms: 9_000,
            },
            &base(),
            1_250,
        )
        .expect("shows");
    let shown = drawn(&overlay).tv.roll.expect("a roll");
    assert_eq!(
        (shown.serial, shown.began_ms, shown.min_hold_ms),
        (1, 1_250, 9_000)
    );
    overlay
        .apply_at(
            SceneChange::Roll {
                view: athletics(),
                min_hold_ms: 0,
            },
            &base(),
            2_000,
        )
        .expect("shows");
    assert_eq!(drawn(&overlay).tv.roll.expect("a roll").serial, 2);
    assert_eq!(overlay.changed_slots(), ["roll"]);
}

#[test]
fn a_page_reading_the_overlay_later_is_told_how_far_along_the_roll_is() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply_at(
            SceneChange::Roll {
                view: athletics(),
                min_hold_ms: 0,
            },
            &base(),
            1_000,
        )
        .expect("shows");
    let stamped = overlay.at(4_500);
    assert_eq!(
        stamped.roll.set_value().map(|roll| roll.elapsed_ms),
        Some(3_500)
    );
}

#[test]
fn a_roll_that_fails_its_check_leaves_the_overlay_as_it_was() {
    let mut overlay = SceneOverlay::default();
    let mut roll = athletics();
    roll.total = 99;
    assert!(
        overlay
            .apply(
                SceneChange::Roll {
                    view: roll,
                    min_hold_ms: 0
                },
                &base()
            )
            .is_err()
    );
    assert!(overlay.is_empty());
    assert_eq!(overlay.roll_count, 0);
}

#[test]
fn the_roll_goes_when_cleared_and_the_count_of_rolls_survives_a_reset() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Roll {
                view: athletics(),
                min_hold_ms: 0,
            },
            &base(),
        )
        .expect("shows");
    overlay
        .apply(SceneChange::ClearRoll {}, &base())
        .expect("clears");
    assert_eq!(drawn(&overlay).tv.roll, None);
    overlay
        .apply(
            SceneChange::Roll {
                view: athletics(),
                min_hold_ms: 0,
            },
            &base(),
        )
        .expect("shows");
    assert_eq!(overlay.cleared().roll_count, 2);
    assert!(overlay.cleared().is_empty());
}

#[test]
fn only_the_roll_slot_is_redrawn_for_a_new_roll_and_the_hosts_stamps_redraw_nothing() {
    let before = snapshot(&base());
    let mut overlay = SceneOverlay::default();
    overlay
        .apply_at(
            SceneChange::Roll {
                view: athletics(),
                min_hold_ms: 0,
            },
            &base(),
            100,
        )
        .expect("shows");
    let shown = snapshot(&drawn(&overlay.at(100)));
    let plan = DrawPlan::between(Some(&before), &shown);
    assert_eq!(plan.parts, BTreeSet::from([Part::TvRoll]));
    assert!(!plan.devices && !plan.stage);
    let later = snapshot(&drawn(&overlay.at(3_100)));
    assert!(
        DrawPlan::between(Some(&shown), &later).is_empty(),
        "a roll under way is carried by its animations, not redrawn"
    );
    overlay
        .apply_at(
            SceneChange::Roll {
                view: athletics(),
                min_hold_ms: 0,
            },
            &base(),
            4_000,
        )
        .expect("shows");
    let next = snapshot(&drawn(&overlay.at(4_000)));
    assert_eq!(
        DrawPlan::between(Some(&later), &next).parts,
        BTreeSet::from([Part::TvRoll])
    );
}

#[test]
fn a_fixture_may_carry_a_roll_and_the_host_stamps_are_optional() {
    let fixture = Fixture::parse(
        r#"{"name":"B","tv":{"place":"P","status":{"kind":"ok"},"elements":[],
            "roll":{"view":{"id":"r","title":"Fireball","kind":"damage",
            "dice":[{"sides":6,"face":4,"role":"damage"}],"total":4}}},
            "phone":{"hero":"H","status":{"kind":"ok"}}}"#,
    )
    .expect("reads");
    let roll = fixture.tv.roll.expect("a roll");
    assert_eq!((roll.serial, roll.min_hold_ms), (0, 0));
}

#[test]
fn the_inventory_panel_steps_aside_for_the_roll_card_as_for_a_full_frame_picture() {
    use crate::controls::PlateTimer;
    use crate::fixture::PlateKind;
    use crate::inventory::is_covered;
    let plate = PlateTimer::default();
    assert!(!is_covered(&plate, Some(PlateKind::Room), false));
    assert!(is_covered(&plate, Some(PlateKind::Room), true));
    assert!(is_covered(&plate, Some(PlateKind::CloseUp), false));
}

#[test]
fn the_roll_is_a_timeline_kind_that_the_log_writes_by_its_key() {
    use crate::timeline::TimelineKind;
    assert_eq!(TimelineKind::Roll.key(), "roll");
    assert_eq!(TimelineKind::from_key("roll"), Some(TimelineKind::Roll));
}

fn fixture_with_roll(roll: &str) -> Result<Fixture, crate::fixture::FixtureError> {
    Fixture::parse(&format!(
        r#"{{"name":"B","tv":{{"place":"P","status":{{"kind":"ok"}},"elements":[],"roll":{roll}}},
            "phone":{{"hero":"H","status":{{"kind":"ok"}}}}}}"#
    ))
}

#[test]
fn a_fixtures_roll_that_adds_up_wrongly_or_carries_the_hosts_numbers_is_refused() {
    let view = concat!(
        r#""view":{"id":"r","title":"T","kind":"damage","#,
        r#""dice":[{"sides":6,"face":4,"role":"damage"}],"total":9}"#
    );
    let error = fixture_with_roll(&format!("{{{view}}}")).expect_err("the total is wrong");
    assert!(error.message.contains("add up"), "{error}");
    let view = view.replace("\"total\":9", "\"total\":4");
    let error = fixture_with_roll(&format!("{{{view},\"serial\":3}}")).expect_err("host owned");
    assert!(error.message.contains("serial"), "{error}");
    assert!(fixture_with_roll(&format!("{{{view}}}")).is_ok());
}

#[test]
fn two_rolls_of_the_same_test_land_in_clearly_different_places_and_slants() {
    let landings: BTreeSet<(i32, i32, i32)> = (0..40)
        .map(|seed| {
            let die = TumblePlan::new(seed, 1).dice[0];
            (die.rest_degrees, die.rest_x_percent, die.rest_y_percent)
        })
        .collect();
    assert!(
        landings.len() >= 36,
        "{} different landings in 40",
        landings.len()
    );
    let slants: BTreeSet<i32> = landings.iter().map(|landing| landing.0).collect();
    assert!(slants.iter().any(|slant| *slant > 15) && slants.iter().any(|slant| *slant < -15));
}

#[test]
fn a_die_is_thrown_from_the_edge_of_the_tray_or_from_over_it_and_not_from_off_the_card() {
    for seed in 0..200_u64 {
        let die = TumblePlan::new(seed, 1).dice[0];
        assert!(die.from_x_cqw.abs() <= 25, "seed {seed}");
        assert!(die.from_y_cqw.abs() <= 11, "seed {seed}");
    }
}
