//! The golden sessions of step C3 and C4: the section G traces 1, 2 and 4 and the three known-bad
//! lines of the observations, each as a script of inputs with what the plan says must come of it.
//!
//! Every expectation below was written from the plan text it cites, in
//! `planning/gameplay-engine-redesign.md`, and not from what the code printed:
//!
//! - G.1, lines 1633 to 1643 (the chandelier), with C.3 (1326), RES26 (2131) and RES48;
//! - G.2, lines 1644 to 1654 (the forged letter), with C.2 (1319) and M2-D19;
//! - G.4, lines 1662 to 1665 ("I attack" beside the innkeeper and a goblin), with C.5 (1372);
//! - the three known-bad lines, named once in G (item 6): "I draw my greatsword and attack Dain
//!   Rusk" (observations, line 221), "I swing my greatsword at Dain Rusk and cut him down" (line
//!   222) and "I draw my greatsword and attack" with no one named (L.3, line 2708).
//!
//! What the plan fixes is the order of the stages (C: ADMIT, INTERPRET, CHECK, DECIDE, APPLY,
//! NARRATE, PROJECT), what each trace ends in, who holds which buttons, which dice are drawn (H.3:
//! one die for each step that rolls and none for a willing guard), the state the machine reports
//! (`Waiting{pending_id}`, C.3 step 3 and RES26) and what each audience is shown (H.17: the
//! snapshot and the per-seat projections; G.1: the ruling shows on the host phone and the TV, C.4:
//! nobody else sees the options). The order of the lines the host releases is not in the plan, so
//! it is asserted on its own: what the input came to and asked of the ports (a die drawn, a
//! decline, a clarification, a plan that stopped part way), then what it changed (a pending
//! opened or closed, a ruling, the effects, `turn.applied`), then the narration, which arrives
//! after the commit. The effect lines carry the table's revision, which the host stamps, and every
//! other line carries the input's own (`check_identity`).
//!
//! What these scripts cannot show, because it needs the real rules owner and not a scripted one:
//! the time a forgery takes and its stored quality, the Influence DC rule of a hesitant guard
//! (the scripted owner says "DC 15" as a typed fact, so only the plumbing of the fact is shown),
//! hit points, and that a hero with the kit and no proficiency proceeds (M2-D19: the scripted
//! owner's answer stands for it). The interpreter is scripted too, so a known-bad line shows that
//! the right reading is carried through, not that a model reads it right; C.5's rule that a line
//! with more than one legal target never executes on a guess belongs to the interpreter adapter,
//! which answers `Ambiguous`, and the pipeline trusts and offers it.

use crate::golden::CountingDice;
use crate::golden::{
    Ending, Expect, GoldenInput, GoldenStep, Identity, Leaves, StateChange, ViewExpect, World,
    check_identity, play, replay_twice,
};
use crate::identity::{RecoveryEpoch, SeatId, SessionRevision};
use crate::outcome::{Decline, DeclineCode, PendingKind};
use crate::pipeline_fixtures_tests::{action, d20, ruling_request, seat};
use crate::ports::{
    ConfirmationReason, Interpretation, Preparation, Resolution, Resolved, RulingAnswer,
    RulingOption, RulingTier,
};
use crate::projection::Viewer;
use crate::testkit::correlation;
use crate::testkit::{RulesCall, ScriptedInterpreter, ScriptedRulesOwner};

const ADMIT: &str = "STEP finished scene=1 machine=scene step=admit";
const INTERPRET: &str = "STEP finished scene=1 machine=scene step=interpret";
const CHECK: &str = "STEP finished scene=1 machine=scene step=check";
const DECIDE: &str = "STEP finished scene=1 machine=scene step=decide";
const APPLY: &str = "STEP finished scene=1 machine=scene step=apply";
const NARRATE: &str = "STEP finished scene=1 machine=scene step=narrate";
const PROJECT_APPLIED: &str = "STEP finished scene=1 machine=scene step=project outcome=applied";
const PROJECT_DECLINED: &str = "STEP finished scene=1 machine=scene step=project outcome=declined";
const PROJECT_PENDING: &str = "STEP finished scene=1 machine=scene step=project outcome=pending";
const PROJECT_CLARIFY: &str =
    "STEP finished scene=1 machine=scene step=project outcome=needs_clarification";
const NARRATION: &str = "NARRATION attached scene=1 machine=scene";
const TURN_APPLIED: &str = "TURN applied scene=1 machine=scene";
const EFFECT: &str = "EFFECT applied scene=1 machine=scene effect=beat_resolved";
const SLOT: &str = "EFFECT applied scene=1 machine=scene effect=active_slot_updated";

const SEAT_1: Viewer = Viewer::Seat {
    seat: SeatId::new(1),
};
const SEAT_2: Viewer = Viewer::Seat {
    seat: SeatId::new(2),
};

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

const DICE_0_11: &str = "DICE drawn scene=1 machine=scene draw=0 sides=20 value=11";
const DICE_1_16: &str = "DICE drawn scene=1 machine=scene draw=1 sides=20 value=16";
const DICE_0_13: &str = "DICE drawn scene=1 machine=scene draw=0 sides=20 value=13";
const DICE_0_15: &str = "DICE drawn scene=1 machine=scene draw=0 sides=20 value=15";
const DICE_0_17: &str = "DICE drawn scene=1 machine=scene draw=0 sides=20 value=17";
const DICE_0_8: &str = "DICE drawn scene=1 machine=scene draw=0 sides=20 value=8";
const DICE_0_9: &str = "DICE drawn scene=1 machine=scene draw=0 sides=20 value=9";

const CHANDELIER_LINE: &str = "I swing from the chandelier and kick the archer.";

// ---------------------------------------------------------------------------------------------
// G.1, plan lines 1633 to 1643
// ---------------------------------------------------------------------------------------------

fn chandelier_absent_world() -> Result<World, crate::golden::WorldError> {
    // "There is no chandelier here. You could run at him, climb the crates, or shoot", with those
    // alternatives "from the legal-choice call": the rules owner's decline also names "fly",
    // which the legal choices do not list, so the line must not offer it.
    let declined = Decline::new(DeclineCode::NotViable, "There is no chandelier here.")
        .and_then(|decline| decline.with_alternative("run at him"))
        .and_then(|decline| decline.with_alternative("climb the crates"))
        .and_then(|decline| decline.with_alternative("shoot"))
        .and_then(|decline| decline.with_alternative("fly"))
        .map_err(|error| crate::golden::WorldError::Machine(Box::new(error)))?;
    let rules = ScriptedRulesOwner::default()
        .with_choices(
            seat(1),
            vec![
                action("run_at_him"),
                action("climb_the_crates"),
                action("shoot"),
                action("unarmed_strike_swing"),
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
            ("climb_the_crates", Leaves::Effect),
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
            stages: &[
                ADMIT,
                INTERPRET,
                "STEP finished scene=1 machine=scene step=check outcome=declined",
                PROJECT_DECLINED,
            ],
            released: &["INPUT declined scene=1 machine=scene code=not_viable"],
            dice: &[],
            readout: "There is no chandelier here. You could run at him, climb the crates, or shoot.",
            options: &[],
            machine: "open",
            pendings: &[],
            views: &[ViewExpect {
                viewer: SEAT_1,
                readout: "There is no chandelier here. You could run at him, climb the crates, or shoot.",
                reason: None,
                offers: &["alternative-1", "alternative-2", "alternative-3"],
            }],
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

const WAITING_FOR_THE_HOST: &str = "Waiting for the host to rule";

#[test]
fn g1_with_a_chandelier_the_host_rules_once_the_strike_resolves_and_the_ruling_is_reused() {
    let first_released: &'static [&'static str] = &[
        "MACHINE transitioned scene=1 machine=scene from=open to=waiting",
        "PENDING opened scene=1 machine=scene pending=1 kind=ruling offers=2",
        SLOT,
        TURN_APPLIED,
    ];
    let second_released: &'static [&'static str] = &[
        DICE_0_15,
        "PENDING answered scene=1 machine=scene pending=1 seat=1",
        "RULING stored scene=1 machine=scene key=swing_from_set_piece tier=ordinary source=host",
        "PENDING resolved scene=1 machine=scene pending=1 kind=ruling",
        "MACHINE transitioned scene=1 machine=scene from=waiting to=open",
        EFFECT,
        SLOT,
        TURN_APPLIED,
        NARRATION,
    ];
    let third_released: &'static [&'static str] = &[
        DICE_0_8,
        "RULING reused scene=1 machine=scene key=swing_from_set_piece tier=ordinary",
        EFFECT,
        SLOT,
        TURN_APPLIED,
        NARRATION,
    ];
    let steps = [
        GoldenStep {
            name: "the source does not say a swing grants advantage, so a ruling is asked for",
            input: free_text(2, CHANDELIER_LINE),
            expect: Expect {
                ending: Ending::Pending(PendingKind::Ruling),
                state: StateChange::Changed,
                stages: &[
                    ADMIT,
                    INTERPRET,
                    "STEP finished scene=1 machine=scene step=check outcome=pending",
                    APPLY,
                    PROJECT_PENDING,
                ],
                released: first_released,
                dice: &[],
                readout: "Needs a ruling.",
                options: &[],
                machine: "waiting",
                pendings: &[
                    "pending=1 kind=ruling status=awaiting_adjudicator offers=seat1:accept_suggestion|decline_ruling|advantage_once;seat2:cancel_ruling",
                ],
                views: &[
                    ViewExpect {
                        viewer: SEAT_1,
                        readout: "The party is at scene 1.",
                        reason: Some(WAITING_FOR_THE_HOST),
                        offers: &["accept_suggestion", "decline_ruling", "advantage_once"],
                    },
                    ViewExpect {
                        viewer: SEAT_2,
                        readout: "Needs a ruling.",
                        reason: Some(WAITING_FOR_THE_HOST),
                        offers: &["cancel_ruling"],
                    },
                    ViewExpect {
                        viewer: Viewer::Shared,
                        readout: "The party is at scene 1.",
                        reason: Some(WAITING_FOR_THE_HOST),
                        offers: &[],
                    },
                ],
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
                stages: &[ADMIT, CHECK, DECIDE, APPLY, NARRATE, PROJECT_APPLIED],
                released: second_released,
                dice: &[(20, 15)],
                readout: "Seat 2: unarmed strike swing done.",
                options: &[],
                machine: "open",
                pendings: &[],
                views: &[
                    ViewExpect {
                        viewer: SEAT_1,
                        readout: "Seat 2: unarmed strike swing done.",
                        reason: None,
                        offers: &[],
                    },
                    ViewExpect {
                        viewer: SEAT_2,
                        readout: "Seat 2: unarmed strike swing done.",
                        reason: None,
                        offers: &[],
                    },
                ],
            },
        },
        GoldenStep {
            name: "the same line later reuses the stored ruling and says so",
            input: free_text(2, CHANDELIER_LINE),
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                stages: &[
                    ADMIT,
                    INTERPRET,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    PROJECT_APPLIED,
                ],
                released: third_released,
                dice: &[(20, 8)],
                readout: "Ruling (earlier): no advantage. Seat 2: unarmed strike swing done.",
                options: &[],
                machine: "open",
                pendings: &[],
                views: &[ViewExpect {
                    viewer: Viewer::Shared,
                    readout: "Ruling (earlier): no advantage. Seat 2: unarmed strike swing done.",
                    reason: None,
                    offers: &[],
                }],
            },
        },
    ];

    assert_golden(|| chandelier_present_world(false), &steps);
}

#[test]
fn g1_under_a_reviewed_automatic_policy_the_ruling_resolves_without_the_host() {
    let released: &'static [&'static str] = &[
        DICE_0_15,
        "RULING stored scene=1 machine=scene key=swing_from_set_piece tier=ordinary source=reviewed_automatic",
        EFFECT,
        SLOT,
        TURN_APPLIED,
        NARRATION,
    ];
    let steps = [GoldenStep {
        name: "a reviewed automatic ruling settles the swing at once",
        input: free_text(2, CHANDELIER_LINE),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            stages: &[
                ADMIT,
                INTERPRET,
                CHECK,
                DECIDE,
                APPLY,
                NARRATE,
                PROJECT_APPLIED,
            ],
            released,
            dice: &[(20, 15)],
            readout: "Seat 2: unarmed strike swing done.",
            options: &[],
            machine: "open",
            pendings: &[],
            views: &[ViewExpect {
                viewer: SEAT_1,
                readout: "Seat 2: unarmed strike swing done.",
                reason: None,
                offers: &[],
            }],
        },
    }];

    assert_golden(|| chandelier_present_world(true), &steps);
}

// ---------------------------------------------------------------------------------------------
// G.2, plan lines 1644 to 1654
// ---------------------------------------------------------------------------------------------

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

/// How the guard stands when the letter is shown (G.2: "a willing guard needs no check, an
/// unwilling one refuses, and a hesitant one is a check against the higher of DC 15 or the
/// guard's Intelligence score").
#[derive(Clone, Copy)]
enum Guard {
    Willing,
    Hesitant,
    Unwilling,
}

fn forgery_rules(kit: bool, guard: Guard) -> Result<ScriptedRulesOwner, crate::golden::WorldError> {
    let rules = ScriptedRulesOwner::default().with_choices(
        seat(1),
        vec![action("forge_letter"), action("present_letter")],
    );
    let invalid =
        |error: crate::outcome::DeclineError| crate::golden::WorldError::Machine(Box::new(error));
    // Proficiency with the kit is not required (M2-D19): a hero with the kit and no proficiency
    // proceeds, so the scripted owner's answer with the kit is `Ready` whatever the hero knows.
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
    match guard {
        // A willing guard needs no check, so no die is drawn for the second step.
        Guard::Willing => Ok(rules.with_action(
            action("present_letter"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )),
        // A hesitant guard is a check: one die, and the DC the rules owner used is a typed fact.
        Guard::Hesitant => {
            let resolved = Resolved::default()
                .with_fact("influence_dc", 15)
                .map_err(|error| crate::golden::WorldError::Machine(Box::new(error)))?;
            Ok(rules.with_action(
                action("present_letter"),
                Preparation::Ready { draws: vec![d20()] },
                Resolution::Applied(resolved),
            ))
        }
        Guard::Unwilling => {
            let refusal = Decline::new(DeclineCode::NotViable, "The guard refuses to read it.")
                .map_err(invalid)?;
            Ok(rules.with_action(
                action("present_letter"),
                Preparation::Declined(refusal.clone()),
                Resolution::Declined(refusal),
            ))
        }
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
            stages: &[
                ADMIT,
                INTERPRET,
                "STEP finished scene=1 machine=scene step=check outcome=declined",
                PROJECT_DECLINED,
            ],
            released: &["INPUT declined scene=1 machine=scene code=missing_requirement"],
            dice: &[],
            readout: "The forgery needs a kit and ink. You have neither.",
            options: &[],
            machine: "open",
            pendings: &[],
            views: &[ViewExpect {
                viewer: SEAT_1,
                readout: "The forgery needs a kit and ink. You have neither.",
                reason: None,
                offers: &[],
            }],
        },
    }];
    let make = || forgery_rules(false, Guard::Willing).and_then(|rules| forgery_world(rules, &[]));

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
fn g2_with_the_kit_and_a_willing_guard_one_die_is_drawn_and_both_steps_are_committed_together() {
    let released: &'static [&'static str] =
        &[DICE_0_11, EFFECT, EFFECT, SLOT, TURN_APPLIED, NARRATION];
    let steps = [GoldenStep {
        name: "the letter is made and presented; a willing guard needs no check (G.2)",
        input: free_text(1, FORGERY_LINE),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            stages: &[
                ADMIT,
                INTERPRET,
                CHECK,
                DECIDE,
                CHECK,
                DECIDE,
                APPLY,
                NARRATE,
                PROJECT_APPLIED,
            ],
            released,
            dice: &[(20, 11)],
            readout: "Seat 1: present letter done.",
            options: &[],
            machine: "open",
            pendings: &[],
            views: &[ViewExpect {
                viewer: SEAT_1,
                readout: "Seat 1: present letter done.",
                reason: None,
                offers: &[],
            }],
        },
    }];

    assert_golden(
        || forgery_rules(true, Guard::Willing).and_then(|rules| forgery_world(rules, &[11, 4])),
        &steps,
    );
}

#[test]
fn g2_with_the_kit_and_a_hesitant_guard_the_check_rolls_its_own_die_and_names_the_dc() {
    let released: &'static [&'static str] = &[
        DICE_0_11,
        DICE_1_16,
        EFFECT,
        EFFECT,
        SLOT,
        TURN_APPLIED,
        NARRATION,
    ];
    let steps = [GoldenStep {
        name: "a hesitant guard is a check: the second step draws a die and its DC is a fact",
        input: free_text(1, FORGERY_LINE),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            stages: &[
                ADMIT,
                INTERPRET,
                CHECK,
                DECIDE,
                CHECK,
                DECIDE,
                APPLY,
                NARRATE,
                PROJECT_APPLIED,
            ],
            released,
            dice: &[(20, 11), (20, 16)],
            readout: "Seat 1: present letter done (influence dc 15).",
            options: &[],
            machine: "open",
            pendings: &[],
            views: &[ViewExpect {
                viewer: SEAT_1,
                readout: "Seat 1: present letter done (influence dc 15).",
                reason: None,
                offers: &[],
            }],
        },
    }];

    assert_golden(
        || forgery_rules(true, Guard::Hesitant).and_then(|rules| forgery_world(rules, &[11, 16])),
        &steps,
    );
}

#[test]
fn g2_with_the_kit_and_an_unwilling_guard_the_letter_is_made_and_the_refusal_is_the_question() {
    let released: &'static [&'static str] = &[
        DICE_0_11,
        "INPUT plan_partly_applied scene=1 machine=scene applied=1 total=2",
        "INPUT clarified scene=1 machine=scene options=2 cause=step_declined",
        EFFECT,
        SLOT,
        TURN_APPLIED,
        NARRATION,
    ];
    let steps = [GoldenStep {
        name: "step one is applied and step two is declined: applied plus one question about step two",
        input: free_text(1, FORGERY_LINE),
        expect: Expect {
            ending: Ending::Applied,
            state: StateChange::Changed,
            stages: &[
                ADMIT,
                INTERPRET,
                CHECK,
                DECIDE,
                "STEP finished scene=1 machine=scene step=check outcome=declined",
                APPLY,
                NARRATE,
                PROJECT_APPLIED,
            ],
            released,
            dice: &[(20, 11)],
            readout: "Seat 1: forge letter done. The guard refuses to read it.",
            options: &["forge_letter", "present_letter"],
            machine: "open",
            pendings: &[],
            views: &[
                ViewExpect {
                    viewer: SEAT_1,
                    readout: "Seat 1: forge letter done. The guard refuses to read it.",
                    reason: None,
                    offers: &["forge_letter", "present_letter"],
                },
                ViewExpect {
                    viewer: Viewer::Shared,
                    readout: "Seat 1: forge letter done. The guard refuses to read it.",
                    reason: None,
                    offers: &[],
                },
            ],
        },
    }];

    assert_golden(
        || forgery_rules(true, Guard::Unwilling).and_then(|rules| forgery_world(rules, &[11])),
        &steps,
    );
}

// ---------------------------------------------------------------------------------------------
// G.4, plan lines 1662 to 1665
// ---------------------------------------------------------------------------------------------

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
        2,
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
            stages: &[
                ADMIT,
                "STEP finished scene=1 machine=scene step=interpret outcome=needs_clarification",
                PROJECT_CLARIFY,
            ],
            released: &["INPUT clarified scene=1 machine=scene options=2 cause=ambiguous"],
            dice: &[],
            readout: "Which did you mean? Pick one:",
            options: &["attack_innkeeper", "attack_goblin"],
            machine: "open",
            pendings: &[],
            // "the phone shows the two targets as buttons": the asking seat holds them, the other
            // phone and the TV hold none.
            views: &[
                ViewExpect {
                    viewer: SEAT_1,
                    readout: "Which did you mean? Pick one:",
                    reason: None,
                    offers: &["attack_innkeeper", "attack_goblin"],
                },
                ViewExpect {
                    viewer: SEAT_2,
                    readout: "The party is at scene 1.",
                    reason: None,
                    offers: &[],
                },
                ViewExpect {
                    viewer: Viewer::Shared,
                    readout: "The party is at scene 1.",
                    reason: None,
                    offers: &[],
                },
            ],
        },
    }
}

fn innkeeper_asked_step() -> GoldenStep {
    GoldenStep {
        name: "the innkeeper is neutral, so the chosen button asks for the seat's yes",
        input: GoldenInput::Tap {
            seat: 1,
            action: "attack_innkeeper",
        },
        expect: Expect {
            ending: Ending::Pending(PendingKind::Choice),
            state: StateChange::Changed,
            stages: &[
                ADMIT,
                "STEP finished scene=1 machine=scene step=check outcome=pending",
                APPLY,
                PROJECT_PENDING,
            ],
            released: &[
                "MACHINE transitioned scene=1 machine=scene from=open to=waiting",
                "PENDING opened scene=1 machine=scene pending=1 kind=choice offers=1",
                SLOT,
                TURN_APPLIED,
            ],
            dice: &[],
            readout: "Waiting for seat 1 to choose.",
            options: &[],
            machine: "waiting",
            pendings: &["pending=1 kind=choice status=open offers=seat1:confirm|cancel"],
            views: &[
                ViewExpect {
                    viewer: SEAT_1,
                    readout: "Waiting for seat 1 to choose.",
                    reason: Some("Waiting for seat 1 to choose"),
                    offers: &["confirm", "cancel"],
                },
                ViewExpect {
                    viewer: SEAT_2,
                    readout: "The party is at scene 1.",
                    reason: Some("Waiting for seat 1 to choose"),
                    offers: &[],
                },
            ],
        },
    }
}

#[test]
fn g4_choosing_the_innkeeper_asks_for_a_one_tap_confirmation_and_the_yes_runs_the_attack() {
    let released: &'static [&'static str] = &[
        DICE_0_13,
        "PENDING answered scene=1 machine=scene pending=1 seat=1",
        "PENDING resolved scene=1 machine=scene pending=1 kind=choice",
        "MACHINE transitioned scene=1 machine=scene from=waiting to=open",
        EFFECT,
        SLOT,
        TURN_APPLIED,
        NARRATION,
    ];
    let steps = [
        clarification_step(),
        innkeeper_asked_step(),
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
                stages: &[ADMIT, CHECK, DECIDE, APPLY, NARRATE, PROJECT_APPLIED],
                released,
                dice: &[(20, 13)],
                readout: "Seat 1: attack innkeeper done.",
                options: &[],
                machine: "open",
                pendings: &[],
                views: &[
                    ViewExpect {
                        viewer: SEAT_1,
                        readout: "Seat 1: attack innkeeper done.",
                        reason: None,
                        offers: &[],
                    },
                    ViewExpect {
                        viewer: Viewer::Shared,
                        readout: "Seat 1: attack innkeeper done.",
                        reason: None,
                        offers: &[],
                    },
                ],
            },
        },
    ];

    assert_golden(attack_world, &steps);
}

#[test]
fn g4_choosing_the_goblin_needs_no_confirmation_and_the_no_drops_the_innkeeper_attack() {
    let goblin_released: &'static [&'static str] =
        &[DICE_0_13, EFFECT, SLOT, TURN_APPLIED, NARRATION];
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
                stages: &[ADMIT, CHECK, DECIDE, APPLY, NARRATE, PROJECT_APPLIED],
                released: goblin_released,
                dice: &[(20, 13)],
                readout: "Seat 1: attack goblin done.",
                options: &[],
                machine: "open",
                pendings: &[],
                views: &[ViewExpect {
                    viewer: SEAT_1,
                    readout: "Seat 1: attack goblin done.",
                    reason: None,
                    offers: &[],
                }],
            },
        },
    ];
    let cancelled = [
        clarification_step(),
        innkeeper_asked_step(),
        GoldenStep {
            name: "the no drops the attack: nothing is rolled and the machine is released",
            input: GoldenInput::Answer {
                seat: 1,
                pending: 1,
                choice: "cancel",
            },
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                stages: &[ADMIT, APPLY, PROJECT_APPLIED],
                released: &[
                    "PENDING answered scene=1 machine=scene pending=1 seat=1",
                    "PENDING resolved scene=1 machine=scene pending=1 kind=choice",
                    "MACHINE transitioned scene=1 machine=scene from=waiting to=open",
                    SLOT,
                    TURN_APPLIED,
                ],
                dice: &[],
                readout: "Cancelled.",
                options: &[],
                machine: "open",
                pendings: &[],
                views: &[ViewExpect {
                    viewer: SEAT_1,
                    readout: "Cancelled.",
                    reason: None,
                    offers: &[],
                }],
            },
        },
    ];

    assert_golden(attack_world, &goblin);
    assert_golden(attack_world, &cancelled);
}

// ---------------------------------------------------------------------------------------------
// The three known-bad lines
// ---------------------------------------------------------------------------------------------

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
    let plan_released: &'static [&'static str] =
        &[DICE_0_17, EFFECT, EFFECT, SLOT, TURN_APPLIED, NARRATION];
    let attack_released: &'static [&'static str] =
        &[DICE_0_9, EFFECT, SLOT, TURN_APPLIED, NARRATION];
    [
        GoldenStep {
            name: "draw and attack Dain Rusk is a plan of two steps, not an Intimidation check",
            input: free_text(1, DRAW_AND_ATTACK_DAIN),
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                stages: &[
                    ADMIT,
                    INTERPRET,
                    CHECK,
                    DECIDE,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    PROJECT_APPLIED,
                ],
                released: plan_released,
                dice: &[(20, 17)],
                readout: "Seat 1: attack dain rusk done.",
                options: &[],
                machine: "open",
                pendings: &[],
                views: &[ViewExpect {
                    viewer: SEAT_1,
                    readout: "Seat 1: attack dain rusk done.",
                    reason: None,
                    offers: &[],
                }],
            },
        },
        GoldenStep {
            name: "swinging at Dain Rusk is the attack, not 'nothing like that is here to act on'",
            input: free_text(1, SWING_AT_DAIN),
            expect: Expect {
                ending: Ending::Applied,
                state: StateChange::Changed,
                stages: &[
                    ADMIT,
                    INTERPRET,
                    CHECK,
                    DECIDE,
                    APPLY,
                    NARRATE,
                    PROJECT_APPLIED,
                ],
                released: attack_released,
                dice: &[(20, 9)],
                readout: "Seat 1: attack dain rusk done.",
                options: &[],
                machine: "open",
                pendings: &[],
                views: &[ViewExpect {
                    viewer: SEAT_1,
                    readout: "Seat 1: attack dain rusk done.",
                    reason: None,
                    offers: &[],
                }],
            },
        },
        GoldenStep {
            name: "draw and attack with no one named and two in reach is asked about, never read as a threat",
            input: free_text(1, DRAW_AND_ATTACK),
            expect: Expect {
                ending: Ending::Clarify,
                state: StateChange::Changed,
                stages: &[
                    ADMIT,
                    "STEP finished scene=1 machine=scene step=interpret outcome=needs_clarification",
                    PROJECT_CLARIFY,
                ],
                released: &["INPUT clarified scene=1 machine=scene options=2 cause=ambiguous"],
                dice: &[],
                readout: "Which did you mean? Pick one:",
                options: &["attack_dain_rusk", "attack_innkeeper"],
                machine: "open",
                pendings: &[],
                views: &[ViewExpect {
                    viewer: SEAT_1,
                    readout: "Which did you mean? Pick one:",
                    reason: None,
                    offers: &["attack_dain_rusk", "attack_innkeeper"],
                }],
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

// ---------------------------------------------------------------------------------------------
// The runner is checked too: a golden that does not match is reported, never swallowed.
// ---------------------------------------------------------------------------------------------

/// Plays the known-bad script after `change` altered what one step expects, and returns the
/// failures.
fn failures_after(change: impl FnOnce(&mut [GoldenStep; 3])) -> Vec<String> {
    let mut steps = known_bad_steps();
    change(&mut steps);
    let mut world = known_bad_lines_world().expect("the world builds");
    play(&mut world, &steps).failures
}

#[test]
fn a_golden_that_does_not_match_is_reported_with_the_step_and_what_differed() {
    let failures = failures_after(|steps| {
        steps[1].expect.readout = "Nothing like that is here to act on.";
    });

    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("swinging at Dain Rusk"));
    assert!(failures[0].contains("readout was"));
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
    assert!(
        failures[1].contains("swinging at Dain Rusk")
            && failures[1].contains("durable state digest")
    );
    assert!(failures[2].contains("no one named") && failures[2].contains("options were"));
}

#[test]
fn the_runner_names_wrong_dice_stages_released_lines_machine_pendings_and_views() {
    let failures = failures_after(|steps| {
        steps[0].expect.dice = &[(20, 18)];
        steps[0].expect.stages = &[ADMIT, DECIDE, CHECK];
        steps[1].expect.released = &[TURN_APPLIED];
        steps[1].expect.machine = "waiting";
        steps[1].expect.pendings = &["pending=1 kind=choice status=open offers=seat1:confirm"];
        steps[2].expect.views = &[ViewExpect {
            viewer: SEAT_1,
            readout: "Which did you mean? Pick one:",
            reason: None,
            offers: &["attack_innkeeper", "attack_dain_rusk"],
        }];
    });

    let all = failures.join("\n");
    assert!(all.contains("dice were [(20, 17)]"), "{all}");
    assert!(all.contains("stage lines were"), "{all}");
    assert!(all.contains("released lines were"), "{all}");
    assert!(all.contains("the machine is \"open\""), "{all}");
    assert!(all.contains("open pendings were"), "{all}");
    assert!(all.contains("holds the offers"), "{all}");
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
            stages: &[],
            released: &[],
            dice: &[],
            readout: "",
            options: &[],
            machine: "open",
            pendings: &[],
            views: &[],
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
