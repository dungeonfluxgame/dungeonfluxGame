//! The golden sessions of step C3: the section G traces 1, 2 and 4 and the three known-bad lines of
//! the observations, each as an explicit expected sequence (RES52).
//!
//! Each trace below was read against the plan text it names (`planning/gameplay-engine-redesign.md`
//! section G, C.3, C.5, C.6, RES26, RES47, RES48, M2-D19) before its expectations were written, and
//! nothing here is recorded from a run. Two things in the plan cannot be shown yet and are said
//! where they come up: the rules owner's own effects (the time a forgery takes, its stored quality,
//! hit points) are not table effects, so a committed change appears as a `beat_resolved` effect
//! named after the action; and a pending ruling is held in the table with the action as its
//! continuation, not as a `Waiting` of the machine (RES26 says AwaitingAdjudicator is a state of
//! the ruling and not of a machine).
//!
//! The order of the event lines is the order the host sees them: what the stages said while they
//! ran (`step.finished`, `dice.drawn`, the decline or clarification), then what explains the change
//! (the pending and ruling lines), then the table's own effects, `turn.applied`, and last the
//! narration once the narrator answers. The stage lines come first because they are recorded as
//! the work is done; every line that says a table changed is held and released after the commit
//! (O.16 S1). The effect lines carry the table's revision, which the host stamps, and every other
//! line carries the input's own (`check_identity`).

use crate::golden::CountingDice;
use crate::golden::{
    Ending, Expect, GoldenInput, GoldenStep, Identity, Leaves, StateChange, World, check_identity,
    play, replay_twice,
};
use crate::identity::{RecoveryEpoch, SessionRevision};
use crate::outcome::{Decline, DeclineCode, PendingKind};
use crate::pipeline_fixtures_tests::{action, d20, ruling_request, seat};
use crate::ports::{
    ConfirmationReason, Interpretation, Preparation, Resolution, RulingAnswer, RulingOption,
    RulingTier,
};
use crate::testkit::correlation;
use crate::testkit::{RulesCall, ScriptedInterpreter, ScriptedRulesOwner};

const ADMIT: &str = "STEP finished scene=1 machine=scene step=admit";
const INTERPRET: &str = "STEP finished scene=1 machine=scene step=interpret";
const CHECK: &str = "STEP finished scene=1 machine=scene step=check";
const DECIDE: &str = "STEP finished scene=1 machine=scene step=decide";
const APPLY: &str = "STEP finished scene=1 machine=scene step=apply";
const NARRATE: &str = "STEP finished scene=1 machine=scene step=narrate";
const NARRATION: &str = "NARRATION attached scene=1 machine=scene";
const TURN_APPLIED: &str = "TURN applied scene=1 machine=scene";
const PROJECT_APPLIED: &str = "STEP finished scene=1 machine=scene step=project outcome=applied";
const PROJECT_DECLINED: &str = "STEP finished scene=1 machine=scene step=project outcome=declined";
const PROJECT_PENDING: &str = "STEP finished scene=1 machine=scene step=project outcome=pending";
const PROJECT_CLARIFY: &str =
    "STEP finished scene=1 machine=scene step=project outcome=needs_clarification";
const EFFECT: &str = "EFFECT applied scene=1 machine=scene effect=beat_resolved";
const SLOT: &str = "EFFECT applied scene=1 machine=scene effect=active_slot_updated";

fn world_with(
    actions: &[(&str, Leaves)],
    seats: u64,
    rules: ScriptedRulesOwner,
    interpreter: ScriptedInterpreter,
    dice: &[u16],
) -> Result<World, crate::golden::WorldError> {
    let mut world = World::with_scene(actions, seats, None)?;
    world.rules = rules;
    world.interpreter = interpreter;
    world.dice = CountingDice::new(dice.iter().copied());
    world.table = world.table.clone().hosted_by(seat(1));
    Ok(world)
}

fn free_text(seat: u64, line: &'static str) -> GoldenInput {
    GoldenInput::Text { seat, line }
}

fn assert_golden(
    make: impl Fn() -> Result<World, crate::golden::WorldError>,
    steps: &[GoldenStep],
) {
    let report = replay_twice(make, steps);
    assert!(report.failures.is_empty(), "{}", report.failures.join("\n"));
    assert_eq!(report.records.len(), steps.len());
}

const CHANDELIER_LINE: &str = "I swing from the chandelier and kick the archer.";

fn chandelier_absent_world() -> Result<World, crate::golden::WorldError> {
    let declined = Decline::new(DeclineCode::NotViable, "There is no chandelier here.")
        .and_then(|decline| decline.with_alternative("run at him"))
        .and_then(|decline| decline.with_alternative("climb the crates"))
        .and_then(|decline| decline.with_alternative("shoot"))
        .map_err(|error| crate::golden::WorldError::Machine(error.to_string()))?;
    let rules = ScriptedRulesOwner::default()
        .with_choices(
            seat(1),
            vec![
                action("run_at_him"),
                action("climb_crates"),
                action("shoot"),
            ],
        )
        .with_action(
            action("unarmed_strike_swing"),
            Preparation::Declined(declined.clone()),
            Resolution::Declined(declined),
        );
    let interpreter = ScriptedInterpreter::default().with_line(
        CHANDELIER_LINE,
        Interpretation::Action(action("unarmed_strike_swing")),
    );
    world_with(
        &[
            ("unarmed_strike_swing", Leaves::Effect),
            ("run_at_him", Leaves::Effect),
            ("climb_crates", Leaves::Effect),
            ("shoot", Leaves::Effect),
        ],
        1,
        rules,
        interpreter,
        &[],
    )
}

#[test]
fn g1_without_a_chandelier_the_line_is_declined_not_viable_with_the_legal_alternatives() {
    let steps = [GoldenStep {
        name: "the line names a chandelier that is not in the set",
        input: free_text(1, CHANDELIER_LINE),
        expect: Expect {
            ending: Ending::Declined(DeclineCode::NotViable),
            state: StateChange::Unchanged,
            events: &[
                ADMIT,
                INTERPRET,
                "STEP finished scene=1 machine=scene step=check outcome=declined",
                "INPUT declined scene=1 machine=scene code=not_viable",
                PROJECT_DECLINED,
            ],
            readout: "There is no chandelier here. You could run at him, climb the crates, or shoot.",
            options: &[],
        },
    }];

    assert_golden(chandelier_absent_world, &steps);
}

fn swing_ruling(reviewed_automatic: bool) -> crate::ports::RulingRequest {
    let mut request = ruling_request("swing_from_set_piece", "no_advantage", RulingTier::Ordinary);
    request.alternatives = vec![RulingOption {
        answer: RulingAnswer::new("advantage_once").expect("valid"),
        tier: RulingTier::Advantage,
    }];
    request.reviewed_automatic = reviewed_automatic;
    request
}

fn chandelier_present_world(reviewed_automatic: bool) -> Result<World, crate::golden::WorldError> {
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(2), vec![action("unarmed_strike_swing")])
        .with_action(
            action("unarmed_strike_swing"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_ruling(
            action("unarmed_strike_swing"),
            swing_ruling(reviewed_automatic),
        );
    let interpreter = ScriptedInterpreter::default().with_line(
        CHANDELIER_LINE,
        Interpretation::Action(action("unarmed_strike_swing")),
    );
    world_with(
        &[("unarmed_strike_swing", Leaves::Effect)],
        2,
        rules,
        interpreter,
        &[15, 8],
    )
}

#[test]
fn g1_with_a_chandelier_the_host_rules_once_the_strike_resolves_and_the_ruling_is_reused() {
    let steps = [
        GoldenStep {
            name: "the source does not say a swing grants advantage, so a ruling is asked for",
            input: free_text(2, CHANDELIER_LINE),
            expect: Expect {
                ending: Ending::Pending(PendingKind::Ruling),
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    INTERPRET,
                    "STEP finished scene=1 machine=scene step=check outcome=pending",
                    APPLY,
                    PROJECT_PENDING,
                    "PENDING opened scene=1 machine=scene pending=1 kind=ruling offers=2",
                    TURN_APPLIED,
                ],
                readout: "Needs a ruling.",
                options: &[],
            },
        },
        GoldenStep {
            name: "the host taps the suggestion, the strike rolls the table's die and is committed",
            input: GoldenInput::Answer {
                seat: 1,
                pending: 1,
                choice: "accept_suggestion",
            },
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    "DICE drawn scene=1 machine=scene count=1",
                    PROJECT_APPLIED,
                    "PENDING answered scene=1 machine=scene pending=1 seat=1",
                    "RULING stored scene=1 machine=scene key=swing_from_set_piece tier=ordinary source=host",
                    "PENDING resolved scene=1 machine=scene pending=1 kind=ruling",
                    EFFECT,
                    SLOT,
                    TURN_APPLIED,
                    NARRATION,
                ],
                readout: "Seat 2: unarmed strike swing done.",
                options: &[],
            },
        },
        GoldenStep {
            name: "the same line later reuses the stored ruling and says so",
            input: free_text(2, CHANDELIER_LINE),
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    INTERPRET,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    "DICE drawn scene=1 machine=scene count=1",
                    PROJECT_APPLIED,
                    "RULING reused scene=1 machine=scene key=swing_from_set_piece tier=ordinary",
                    EFFECT,
                    SLOT,
                    TURN_APPLIED,
                    NARRATION,
                ],
                readout: "Ruling (earlier): no advantage. Seat 2: unarmed strike swing done.",
                options: &[],
            },
        },
    ];

    assert_golden(|| chandelier_present_world(false), &steps);
}

#[test]
fn g1_under_a_reviewed_automatic_policy_the_ruling_resolves_without_the_host() {
    let steps = [GoldenStep {
        name: "a reviewed automatic ruling settles the swing at once",
        input: free_text(2, CHANDELIER_LINE),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            events: &[
                ADMIT,
                INTERPRET,
                CHECK,
                DECIDE,
                APPLY,
                NARRATE,
                "DICE drawn scene=1 machine=scene count=1",
                PROJECT_APPLIED,
                "RULING stored scene=1 machine=scene key=swing_from_set_piece tier=ordinary source=reviewed_automatic",
                EFFECT,
                SLOT,
                TURN_APPLIED,
                NARRATION,
            ],
            readout: "Seat 2: unarmed strike swing done.",
            options: &[],
        },
    }];

    assert_golden(|| chandelier_present_world(true), &steps);
}

const FORGERY_LINE: &str = "I tell the guard I am the inspector and hand him a letter I forged.";

fn forgery_world(
    rules: ScriptedRulesOwner,
    dice: &[u16],
) -> Result<World, crate::golden::WorldError> {
    let interpreter = ScriptedInterpreter::default().with_line(
        FORGERY_LINE,
        Interpretation::Plan(vec![action("forge_letter"), action("present_letter")]),
    );
    world_with(
        &[
            ("forge_letter", Leaves::Effect),
            ("present_letter", Leaves::Effect),
        ],
        1,
        rules,
        interpreter,
        dice,
    )
}

fn forgery_rules(
    kit: bool,
    guard_willing: bool,
) -> Result<ScriptedRulesOwner, crate::golden::WorldError> {
    let rules = ScriptedRulesOwner::default().with_choices(
        seat(1),
        vec![action("forge_letter"), action("present_letter")],
    );
    let invalid =
        |error: crate::outcome::DeclineError| crate::golden::WorldError::Machine(error.to_string());
    let rules = if kit {
        rules.with_action(
            action("forge_letter"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
    } else {
        let decline = Decline::new(
            DeclineCode::MissingRequirement,
            "The forgery needs a kit and ink. You have neither.",
        )
        .and_then(|decline| decline.with_missing_piece("a forgery kit and ink"))
        .map_err(invalid)?;
        rules.with_action(
            action("forge_letter"),
            Preparation::Declined(decline.clone()),
            Resolution::Declined(decline),
        )
    };
    if guard_willing {
        Ok(rules.with_action(
            action("present_letter"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        ))
    } else {
        let refusal = Decline::new(DeclineCode::NotViable, "The guard refuses to read it.")
            .map_err(invalid)?;
        Ok(rules.with_action(
            action("present_letter"),
            Preparation::Declined(refusal.clone()),
            Resolution::Declined(refusal),
        ))
    }
}

#[test]
fn g2_without_a_kit_and_ink_the_plan_is_declined_missing_requirement_and_step_two_never_runs() {
    let steps = [GoldenStep {
        name: "the forgery needs a kit and ink and the hero has neither",
        input: free_text(1, FORGERY_LINE),
        expect: Expect {
            ending: Ending::Declined(DeclineCode::MissingRequirement),
            state: StateChange::Unchanged,
            events: &[
                ADMIT,
                INTERPRET,
                "STEP finished scene=1 machine=scene step=check outcome=declined",
                "INPUT declined scene=1 machine=scene code=missing_requirement",
                PROJECT_DECLINED,
            ],
            readout: "The forgery needs a kit and ink. You have neither.",
            options: &[],
        },
    }];
    let make = || forgery_rules(false, true).and_then(|rules| forgery_world(rules, &[]));

    assert_golden(make, &steps);

    let mut world = make().expect("the world builds");
    play(&mut world, &steps);
    assert!(
        !world
            .rules
            .calls()
            .contains(&RulesCall::Prepare(action("present_letter"))),
        "the second step is not even checked once the first is declined"
    );
}

#[test]
fn g2_with_the_kit_and_a_willing_guard_both_steps_are_committed_together() {
    let steps = [GoldenStep {
        name: "the letter is made and presented; no proficiency is needed (M2-D19)",
        input: free_text(1, FORGERY_LINE),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            events: &[
                ADMIT,
                INTERPRET,
                CHECK,
                DECIDE,
                CHECK,
                DECIDE,
                APPLY,
                NARRATE,
                "DICE drawn scene=1 machine=scene count=2",
                PROJECT_APPLIED,
                EFFECT,
                EFFECT,
                SLOT,
                TURN_APPLIED,
                NARRATION,
            ],
            readout: "Seat 1: present letter done.",
            options: &[],
        },
    }];

    assert_golden(
        || forgery_rules(true, true).and_then(|rules| forgery_world(rules, &[11, 16])),
        &steps,
    );
}

#[test]
fn g2_with_the_kit_and_an_unwilling_guard_the_letter_is_made_and_the_refusal_is_the_question() {
    let steps = [GoldenStep {
        name: "step one is applied and step two is declined by the rules owner",
        input: free_text(1, FORGERY_LINE),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            events: &[
                ADMIT,
                INTERPRET,
                CHECK,
                DECIDE,
                "STEP finished scene=1 machine=scene step=check outcome=declined",
                APPLY,
                "INPUT plan_partly_applied scene=1 machine=scene applied=1 total=2",
                NARRATE,
                "DICE drawn scene=1 machine=scene count=1",
                PROJECT_APPLIED,
                EFFECT,
                SLOT,
                TURN_APPLIED,
                NARRATION,
            ],
            readout: "Seat 1: forge letter done. The guard refuses to read it.",
            options: &[],
        },
    }];

    assert_golden(
        || forgery_rules(true, false).and_then(|rules| forgery_world(rules, &[11])),
        &steps,
    );
}

fn attack_world() -> Result<World, crate::golden::WorldError> {
    let rules = ScriptedRulesOwner::default()
        .with_choices(
            seat(1),
            vec![action("attack_innkeeper"), action("attack_goblin")],
        )
        .with_action(
            action("attack_innkeeper"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_action(
            action("attack_goblin"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_confirmation(
            action("attack_innkeeper"),
            ConfirmationReason::HarmsANeutral,
        );
    let interpreter = ScriptedInterpreter::default().with_line(
        "I attack",
        Interpretation::Ambiguous {
            candidates: vec![action("attack_innkeeper"), action("attack_goblin")],
        },
    );
    world_with(
        &[
            ("attack_innkeeper", Leaves::Effect),
            ("attack_goblin", Leaves::Effect),
        ],
        1,
        rules,
        interpreter,
        &[13],
    )
}

fn clarification_step() -> GoldenStep {
    GoldenStep {
        name: "an attack with an innkeeper and a goblin in reach is asked about, not guessed",
        input: free_text(1, "I attack"),
        expect: Expect {
            ending: Ending::Clarify,
            state: StateChange::Changed,
            events: &[
                ADMIT,
                "STEP finished scene=1 machine=scene step=interpret outcome=needs_clarification",
                "INPUT clarified scene=1 machine=scene options=2 cause=ambiguous",
                PROJECT_CLARIFY,
            ],
            readout: "Which did you mean? Pick one:",
            options: &["attack_innkeeper", "attack_goblin"],
        },
    }
}

#[test]
fn g4_choosing_the_innkeeper_asks_for_a_one_tap_confirmation_and_the_yes_runs_the_attack() {
    let steps = [
        clarification_step(),
        GoldenStep {
            name: "the innkeeper is neutral, so the chosen button asks for the seat's yes",
            input: GoldenInput::Tap {
                seat: 1,
                action: "attack_innkeeper",
            },
            expect: Expect {
                ending: Ending::Pending(PendingKind::Choice),
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    "STEP finished scene=1 machine=scene step=check outcome=pending",
                    APPLY,
                    PROJECT_PENDING,
                    "PENDING opened scene=1 machine=scene pending=1 kind=choice offers=1",
                    TURN_APPLIED,
                ],
                readout: "Waiting for seat 1 to choose.",
                options: &[],
            },
        },
        GoldenStep {
            name: "the yes runs the attack with the table's die",
            input: GoldenInput::Answer {
                seat: 1,
                pending: 1,
                choice: "confirm",
            },
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    "DICE drawn scene=1 machine=scene count=1",
                    PROJECT_APPLIED,
                    "PENDING answered scene=1 machine=scene pending=1 seat=1",
                    "PENDING resolved scene=1 machine=scene pending=1 kind=choice",
                    EFFECT,
                    SLOT,
                    TURN_APPLIED,
                    NARRATION,
                ],
                readout: "Seat 1: attack innkeeper done.",
                options: &[],
            },
        },
    ];

    assert_golden(attack_world, &steps);
}

#[test]
fn g4_choosing_the_goblin_needs_no_confirmation_and_the_no_drops_the_innkeeper_attack() {
    let goblin = [
        clarification_step(),
        GoldenStep {
            name: "the goblin is a foe, so the chosen button runs at once",
            input: GoldenInput::Tap {
                seat: 1,
                action: "attack_goblin",
            },
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    "DICE drawn scene=1 machine=scene count=1",
                    PROJECT_APPLIED,
                    EFFECT,
                    SLOT,
                    TURN_APPLIED,
                    NARRATION,
                ],
                readout: "Seat 1: attack goblin done.",
                options: &[],
            },
        },
    ];
    let cancelled = [
        clarification_step(),
        GoldenStep {
            name: "the innkeeper is chosen",
            input: GoldenInput::Tap {
                seat: 1,
                action: "attack_innkeeper",
            },
            expect: Expect {
                ending: Ending::Pending(PendingKind::Choice),
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    "STEP finished scene=1 machine=scene step=check outcome=pending",
                    APPLY,
                    PROJECT_PENDING,
                    "PENDING opened scene=1 machine=scene pending=1 kind=choice offers=1",
                    TURN_APPLIED,
                ],
                readout: "Waiting for seat 1 to choose.",
                options: &[],
            },
        },
        GoldenStep {
            name: "the no drops the attack, nothing is rolled and nothing changes but the pending",
            input: GoldenInput::Answer {
                seat: 1,
                pending: 1,
                choice: "cancel",
            },
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    APPLY,
                    PROJECT_APPLIED,
                    "PENDING answered scene=1 machine=scene pending=1 seat=1",
                    "PENDING resolved scene=1 machine=scene pending=1 kind=choice",
                    TURN_APPLIED,
                ],
                readout: "Cancelled.",
                options: &[],
            },
        },
    ];

    assert_golden(attack_world, &goblin);
    assert_golden(attack_world, &cancelled);
}

const DRAW_AND_ATTACK_DAIN: &str = "I draw my greatsword and attack Dain Rusk";
const SWING_AT_DAIN: &str = "I swing my greatsword at Dain Rusk and cut him down";
const DRAW_AND_ATTACK: &str = "I draw my greatsword and attack";

fn known_bad_lines_world() -> Result<World, crate::golden::WorldError> {
    let attack = |rules: ScriptedRulesOwner, name: &str| {
        rules.with_action(
            action(name),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
    };
    let rules = ScriptedRulesOwner::default()
        .with_choices(
            seat(1),
            vec![action("attack_dain_rusk"), action("attack_innkeeper")],
        )
        .with_action(
            action("draw_greatsword"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        );
    let rules = attack(attack(rules, "attack_dain_rusk"), "attack_innkeeper");
    let interpreter = ScriptedInterpreter::default()
        .with_line(
            DRAW_AND_ATTACK_DAIN,
            Interpretation::Plan(vec![action("draw_greatsword"), action("attack_dain_rusk")]),
        )
        .with_line(
            SWING_AT_DAIN,
            Interpretation::Action(action("attack_dain_rusk")),
        )
        .with_line(
            DRAW_AND_ATTACK,
            Interpretation::Ambiguous {
                candidates: vec![action("attack_dain_rusk"), action("attack_innkeeper")],
            },
        );
    world_with(
        &[
            ("draw_greatsword", Leaves::Effect),
            ("attack_dain_rusk", Leaves::Effect),
            ("attack_innkeeper", Leaves::Effect),
        ],
        1,
        rules,
        interpreter,
        &[17, 9],
    )
}

fn known_bad_steps() -> [GoldenStep; 3] {
    [
        GoldenStep {
            name: "draw and attack Dain Rusk is a plan of two steps, not an Intimidation check",
            input: free_text(1, DRAW_AND_ATTACK_DAIN),
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    INTERPRET,
                    CHECK,
                    DECIDE,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    "DICE drawn scene=1 machine=scene count=1",
                    PROJECT_APPLIED,
                    EFFECT,
                    EFFECT,
                    SLOT,
                    TURN_APPLIED,
                    NARRATION,
                ],
                readout: "Seat 1: attack dain rusk done.",
                options: &[],
            },
        },
        GoldenStep {
            name: "swinging at Dain Rusk is the attack, not 'nothing like that is here to act on'",
            input: free_text(1, SWING_AT_DAIN),
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    INTERPRET,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    "DICE drawn scene=1 machine=scene count=1",
                    PROJECT_APPLIED,
                    EFFECT,
                    SLOT,
                    TURN_APPLIED,
                    NARRATION,
                ],
                readout: "Seat 1: attack dain rusk done.",
                options: &[],
            },
        },
        GoldenStep {
            name: "draw and attack with no one named and two in reach is asked about, never read as a threat",
            input: free_text(1, DRAW_AND_ATTACK),
            expect: Expect {
                ending: Ending::Clarify,
                state: StateChange::Changed,
                events: &[
                    ADMIT,
                    "STEP finished scene=1 machine=scene step=interpret outcome=needs_clarification",
                    "INPUT clarified scene=1 machine=scene options=2 cause=ambiguous",
                    PROJECT_CLARIFY,
                ],
                readout: "Which did you mean? Pick one:",
                options: &["attack_dain_rusk", "attack_innkeeper"],
            },
        },
    ]
}

#[test]
fn the_three_known_bad_lines_become_an_attack_a_plan_and_a_question_never_an_intimidation() {
    let steps = known_bad_steps();

    assert_golden(known_bad_lines_world, &steps);

    let mut world = known_bad_lines_world().expect("the world builds");
    play(&mut world, &steps);
    let prepared: Vec<String> = world
        .rules
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            RulesCall::Prepare(prepared) => Some(prepared.as_str().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(
        prepared,
        ["draw_greatsword", "attack_dain_rusk", "attack_dain_rusk"],
        "the rules owner is never asked about a social check, and the ambiguous line asks nothing"
    );
}

#[test]
fn a_golden_that_does_not_match_is_reported_with_the_step_and_what_differed() {
    let mut steps = known_bad_steps();
    steps[1].expect.readout = "Nothing like that is here to act on.";
    let mut world = known_bad_lines_world().expect("the world builds");

    let report = play(&mut world, &steps);

    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].contains("swinging at Dain Rusk"));
    assert!(report.failures[0].contains("readout was"));
}

/// Plays the known-bad script after `change` altered what one step expects, and returns the failures.
fn failures_after(change: impl FnOnce(&mut [GoldenStep; 3])) -> Vec<String> {
    let mut steps = known_bad_steps();
    change(&mut steps);
    let mut world = known_bad_lines_world().expect("the world builds");
    play(&mut world, &steps).failures
}

#[test]
fn the_runner_names_a_wrong_ending_a_wrong_state_change_and_wrong_options() {
    let failures = failures_after(|steps| {
        steps[0].expect.ending = Ending::Clarify;
        steps[1].expect.state = StateChange::Unchanged;
        steps[2].expect.options = &["attack_innkeeper", "attack_dain_rusk"];
    });

    assert_eq!(failures.len(), 3, "{failures:?}");
    assert!(failures[0].contains("draw and attack Dain Rusk") && failures[0].contains("ended"));
    assert!(failures[1].contains("swinging at Dain Rusk") && failures[1].contains("state digest"));
    assert!(failures[2].contains("no one named") && failures[2].contains("options were"));
}

#[test]
fn the_runner_catches_events_in_the_wrong_order_and_a_missing_event() {
    let swapped: &'static [&'static str] = &[ADMIT, DECIDE, CHECK];
    let failures = failures_after(|steps| {
        steps[0].expect.events = swapped;
    });

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("events were") && failures[0].contains("expected"));
}

#[test]
fn the_runner_names_a_line_that_carries_another_inputs_identity() {
    let mut world = known_bad_lines_world().expect("the world builds");
    let steps = known_bad_steps();
    let before = world.table.facts().snapshot().revision();
    play(&mut world, &steps[..1]);
    let events = world.events();
    let own = Identity {
        input: correlation(1),
        table_revision: before,
    };

    let mut failures = Vec::new();
    check_identity(&steps[0], &events, own, &mut failures);
    assert!(failures.is_empty(), "{failures:?}");

    let other_input = Identity {
        input: correlation(2),
        table_revision: before,
    };
    check_identity(&steps[0], &events, other_input, &mut failures);
    assert!(
        failures
            .iter()
            .any(|failure| failure.contains("turn_seq=2")),
        "a line of input 1 must not pass as input 2: {failures:?}"
    );

    let other_table = Identity {
        input: correlation(1),
        table_revision: SessionRevision::new(RecoveryEpoch::FIRST, 77),
    };
    let mut table_failures = Vec::new();
    check_identity(&steps[0], &events, other_table, &mut table_failures);
    assert!(
        table_failures
            .iter()
            .all(|failure| failure.contains("EFFECT")),
        "only the table's own effect lines carry the table's revision: {table_failures:?}"
    );
    assert!(!table_failures.is_empty());
}

#[test]
fn a_script_that_cannot_run_is_reported_not_swallowed() {
    let steps = [GoldenStep {
        name: "a timer with a name the engine cannot hold",
        input: GoldenInput::Timer("not a valid timer name!"),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Unchanged,
            events: &[],
            readout: "",
            options: &[],
        },
    }];
    let mut world = known_bad_lines_world().expect("the world builds");

    let report = play(&mut world, &steps);

    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert!(report.failures[0].contains("bad timer name"));
    assert!(report.records.is_empty());
}

#[test]
fn two_replays_of_the_same_script_agree_and_a_diverging_world_is_reported() {
    let same = replay_twice(known_bad_lines_world, &known_bad_steps());
    assert!(same.failures.is_empty(), "{:?}", same.failures);

    let flip = std::cell::Cell::new(false);
    let diverging = replay_twice(
        || {
            let dice: &[u16] = if flip.replace(true) {
                &[3, 4]
            } else {
                &[17, 9]
            };
            let mut world = known_bad_lines_world()?;
            world.dice = CountingDice::new(dice.iter().copied());
            Ok(world)
        },
        &known_bad_steps(),
    );

    assert!(
        diverging
            .failures
            .iter()
            .any(|failure| failure.contains("second replay")),
        "{:?}",
        diverging.failures
    );
}
