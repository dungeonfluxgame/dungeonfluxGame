use crate::effects::{ApplyOutcome, BeatName, EffectCause, EffectError, TableEffect};
use crate::element::ElementId;
use crate::identity::{
    MachineId, RecoveryEpoch, SceneId, SeatId, SessionRevision, TableId, TableScope,
};
use crate::machine::slot::MachineSlot;
use crate::table_state::{
    CastChange, CastEntry, CastKind, ConditionName, Controller, DisplayName, Placement, Presence,
    SceneGeneration, SplitPolicy, TableState, TableStatus, Visibility, Vitals,
};
use crate::testkit::correlation;

pub(crate) fn seat(number: u64) -> SeatId {
    SeatId::new(number)
}

pub(crate) fn scene(number: u64) -> SceneId {
    SceneId::new(number)
}

pub(crate) fn scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
}

pub(crate) fn element(name: &str) -> ElementId {
    ElementId::new(name).expect("valid element id")
}

pub(crate) fn machine(name: &str) -> MachineId {
    MachineId::new(name).expect("valid machine id")
}

pub(crate) fn display_name(text: &str) -> DisplayName {
    DisplayName::new(text).expect("valid display name")
}

pub(crate) fn place(x_units: u32, y_units: u32) -> Placement {
    Placement::from_milli_units(x_units * 1000, y_units * 1000).expect("on the canvas")
}

/// A hero of `seat` standing in `scene` at (`x`, `y`).
pub(crate) fn hero(id: &str, name: &str, seat_number: u64, scene_number: u64, x: u32) -> CastEntry {
    CastEntry::new(
        element(id),
        CastKind::Party,
        Controller::Seat {
            seat: seat(seat_number),
        },
        scene(scene_number),
        display_name(name),
        Vitals::new(7, 9).expect("valid vitals"),
    )
    .at(place(x, 20))
}

/// A creature the engine plays, standing in `scene` at (`x`, 30).
pub(crate) fn creature(id: &str, name: &str, scene_number: u64, x: u32) -> CastEntry {
    CastEntry::new(
        element(id),
        CastKind::Npc,
        Controller::Engine,
        scene(scene_number),
        display_name(name),
        Vitals::new(12, 19).expect("valid vitals"),
    )
    .at(place(x, 30))
}

pub(crate) fn apply(
    table: &mut TableState,
    turn: u64,
    effects: &[TableEffect],
) -> Result<ApplyOutcome, EffectError> {
    table.apply(&scope(), correlation(turn), effects)
}

pub(crate) fn refusal(result: Result<ApplyOutcome, EffectError>) -> EffectCause {
    result.expect_err("the commit is refused").cause
}

/// A table of four places with seats 1 and 2 in scene 10.
pub(crate) fn table_in_scene_ten() -> TableState {
    let mut table = TableState::seated(4, &[seat(1), seat(2)]).expect("valid table");
    apply(
        &mut table,
        1,
        &[TableEffect::SceneEntered { scene: scene(10) }],
    )
    .expect("the party enters");
    table
}

fn party_that_may_split() -> TableState {
    table_in_scene_ten().with_split_policy(SplitPolicy::Allowed)
}

#[test]
fn a_seat_that_joins_after_a_scene_started_stands_in_the_focus_scene() {
    let mut table = table_in_scene_ten();
    apply(&mut table, 2, &[TableEffect::SeatJoined { seat: seat(3) }]).expect("joins");
    let snapshot = table.snapshot();
    let state = snapshot.seat(seat(3)).expect("seated");
    assert_eq!(state.scene(), Some(scene(10)));
    assert_eq!(state.presence(), Presence::Connected);
}

#[test]
fn a_seat_at_a_table_with_no_scene_yet_is_in_none() {
    let table = TableState::seated(4, &[seat(1)]).expect("valid table");
    let snapshot = table.snapshot();
    assert_eq!(snapshot.seat(seat(1)).expect("seated").scene(), None);
    assert_eq!(snapshot.focus_scene(), None);
}

#[test]
fn presence_changes_for_a_seated_seat_and_is_refused_for_anyone_else() {
    let mut table = table_in_scene_ten();
    for (turn, presence) in (2..).zip([
        Presence::Sleeping,
        Presence::Disconnected,
        Presence::VoluntaryAfk,
        Presence::Connected,
    ]) {
        apply(
            &mut table,
            turn,
            &[TableEffect::SeatPresenceChanged {
                seat: seat(1),
                presence,
            }],
        )
        .expect("changes");
        assert_eq!(table.snapshot().seat(seat(1)).unwrap().presence(), presence);
    }
    let unseated = apply(
        &mut table,
        9,
        &[TableEffect::SeatPresenceChanged {
            seat: seat(9),
            presence: Presence::Sleeping,
        }],
    );
    assert_eq!(refusal(unseated), EffectCause::SeatNotSeated);
}

#[test]
fn entering_a_scene_moves_every_seat_into_it_and_gives_it_a_new_generation() {
    let mut table = table_in_scene_ten();
    let first = table.snapshot().scene(scene(10)).unwrap().generation();
    apply(
        &mut table,
        2,
        &[TableEffect::SceneEntered { scene: scene(11) }],
    )
    .expect("enters");
    let snapshot = table.snapshot();
    assert!(snapshot.scene(scene(10)).is_none(), "the old scene closed");
    let second = snapshot.scene(scene(11)).unwrap().generation();
    assert!(second > first);
    assert_eq!(snapshot.focus_scene(), Some(scene(11)));
    for seat_number in [1, 2] {
        let state = snapshot.seat(seat(seat_number)).unwrap();
        assert_eq!(state.scene(), Some(scene(11)));
    }
}

#[test]
fn a_scene_entered_again_never_gets_a_generation_it_had_before() {
    let mut table = table_in_scene_ten();
    let mut seen = vec![table.snapshot().scene(scene(10)).unwrap().generation()];
    for (turn, number) in [(2, 11), (3, 10), (4, 10)] {
        apply(
            &mut table,
            turn,
            &[TableEffect::SceneEntered {
                scene: scene(number),
            }],
        )
        .expect("enters");
        let generation = table.snapshot().scene(scene(number)).unwrap().generation();
        assert!(!seen.contains(&generation), "{generation} repeated");
        seen.push(generation);
    }
    assert_eq!(
        seen,
        [1, 2, 3, 4].map(SceneGeneration::new),
        "one counter, only ever up"
    );
}

#[test]
fn entering_a_scene_keeps_the_heroes_and_drops_every_other_creature() {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered {
                entry: creature("goblin", "Goblin", 10, 60),
            },
        ],
    )
    .expect("the cast enters");
    apply(
        &mut table,
        3,
        &[TableEffect::SceneEntered { scene: scene(11) }],
    )
    .expect("enters");
    let snapshot = table.snapshot();
    let pip = snapshot
        .cast_entry(&element("pip"))
        .expect("the hero came along");
    assert_eq!(pip.scene(), scene(11));
    assert!(snapshot.cast_entry(&element("goblin")).is_none());
}

#[test]
fn a_beat_resolves_only_in_an_open_scene_and_entering_another_scene_clears_it() {
    let mut table = table_in_scene_ten();
    let beat = BeatName::new("locked_gate").expect("valid beat");
    let missing = apply(
        &mut table,
        2,
        &[TableEffect::BeatResolved {
            scene: scene(99),
            beat: beat.clone(),
        }],
    );
    assert_eq!(refusal(missing), EffectCause::SceneNotActive);
    apply(
        &mut table,
        3,
        &[TableEffect::BeatResolved {
            scene: scene(10),
            beat: beat.clone(),
        }],
    )
    .expect("resolves");
    assert_eq!(table.active_beat(), Some(&beat));
    apply(
        &mut table,
        4,
        &[TableEffect::SceneEntered { scene: scene(11) }],
    )
    .expect("enters");
    assert_eq!(table.active_beat(), None);
}

#[test]
fn while_the_party_stays_together_one_seat_cannot_go_to_another_scene() {
    let mut table = table_in_scene_ten();
    let before = table.clone();
    let split = apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(1),
            scene: scene(11),
        }],
    );
    assert_eq!(refusal(split), EffectCause::SplitPartyDeclined);
    assert_eq!(table, before);
}

#[test]
fn a_move_that_keeps_the_whole_party_in_one_scene_is_not_a_split() {
    let mut table = TableState::seated(4, &[seat(1)]).expect("valid table");
    apply(
        &mut table,
        1,
        &[TableEffect::SceneEntered { scene: scene(10) }],
    )
    .expect("enters");
    apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(1),
            scene: scene(11),
        }],
    )
    .expect("a lone seat takes the party with it");
    let snapshot = table.snapshot();
    assert!(snapshot.scene(scene(10)).is_none());
    assert_eq!(snapshot.focus_scene(), Some(scene(11)));
}

#[test]
fn moving_a_seat_to_the_scene_it_is_in_changes_nothing() {
    let mut table = table_in_scene_ten();
    let before = table.clone();
    apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(1),
            scene: scene(10),
        }],
    )
    .expect("applies");
    assert!(table.same_world_as(&before));
}

#[test]
fn a_table_that_allows_a_split_opens_the_second_scene_with_its_own_generation() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(2),
            scene: scene(11),
        }],
    )
    .expect("the seat goes alone");
    let snapshot = table.snapshot();
    let scenes: Vec<_> = snapshot.scenes().map(|(id, _)| id).collect();
    assert_eq!(scenes, [scene(10), scene(11)]);
    let first = snapshot.scene(scene(10)).unwrap().generation();
    let second = snapshot.scene(scene(11)).unwrap().generation();
    assert!(second > first);
    assert_eq!(snapshot.seat(seat(1)).unwrap().scene(), Some(scene(10)));
    assert_eq!(snapshot.seat(seat(2)).unwrap().scene(), Some(scene(11)));
    assert_eq!(
        snapshot.focus_scene(),
        Some(scene(10)),
        "the focus stays put"
    );
}

#[test]
fn a_seat_that_moves_alone_takes_its_hero_and_leaves_the_others_behind() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered {
                entry: hero("vex", "Vex", 2, 10, 50),
            },
            TableEffect::SeatMoved {
                seat: seat(2),
                scene: scene(11),
            },
        ],
    )
    .expect("applies");
    let snapshot = table.snapshot();
    assert_eq!(
        snapshot.cast_entry(&element("vex")).unwrap().scene(),
        scene(11)
    );
    assert_eq!(
        snapshot.cast_entry(&element("pip")).unwrap().scene(),
        scene(10)
    );
}

#[test]
fn a_scene_closes_when_its_last_seat_leaves_it_and_the_focus_follows() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[
            TableEffect::SeatMoved {
                seat: seat(2),
                scene: scene(11),
            },
            TableEffect::CastEntered {
                entry: creature("ogre", "Ogre", 11, 40),
            },
        ],
    )
    .expect("applies");
    apply(
        &mut table,
        3,
        &[TableEffect::SeatMoved {
            seat: seat(1),
            scene: scene(11),
        }],
    )
    .expect("regroups");
    let snapshot = table.snapshot();
    assert!(
        snapshot.scene(scene(10)).is_none(),
        "scene 10 emptied and closed"
    );
    assert_eq!(snapshot.focus_scene(), Some(scene(11)));
    assert!(
        snapshot.cast_entry(&element("ogre")).is_some(),
        "scene 11 kept its cast"
    );
}

#[test]
fn closing_a_scene_drops_the_creatures_that_stood_in_it() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[
            TableEffect::SeatMoved {
                seat: seat(2),
                scene: scene(11),
            },
            TableEffect::CastEntered {
                entry: creature("ogre", "Ogre", 11, 40),
            },
        ],
    )
    .expect("applies");
    apply(
        &mut table,
        3,
        &[TableEffect::SeatMoved {
            seat: seat(2),
            scene: scene(10),
        }],
    )
    .expect("regroups");
    assert!(table.snapshot().cast_entry(&element("ogre")).is_none());
}

#[test]
fn a_seat_that_leaves_takes_its_cast_with_it_and_closes_the_scene_it_emptied() {
    let mut table = TableState::seated(4, &[seat(1)]).expect("valid table");
    apply(
        &mut table,
        1,
        &[
            TableEffect::SceneEntered { scene: scene(10) },
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered {
                entry: creature("goblin", "Goblin", 10, 60),
            },
        ],
    )
    .expect("applies");
    apply(&mut table, 2, &[TableEffect::SeatLeft { seat: seat(1) }]).expect("leaves");
    let snapshot = table.snapshot();
    assert!(snapshot.cast_entry(&element("pip")).is_none());
    assert!(snapshot.cast_entry(&element("goblin")).is_none());
    assert_eq!(snapshot.scenes().count(), 0);
    assert_eq!(snapshot.focus_scene(), None);
}

#[test]
fn a_seat_that_leaves_while_others_stay_leaves_the_scene_open() {
    let mut table = table_in_scene_ten();
    apply(&mut table, 2, &[TableEffect::SeatLeft { seat: seat(1) }]).expect("leaves");
    let snapshot = table.snapshot();
    assert!(snapshot.scene(scene(10)).is_some());
    assert_eq!(snapshot.focus_scene(), Some(scene(10)));
}

#[test]
fn the_world_clock_adds_up_and_refuses_to_overflow() {
    let mut table = table_in_scene_ten();
    for (turn, advance_s) in [(2, 6), (3, 3_600)] {
        apply(&mut table, turn, &[TableEffect::TimeAdvanced { advance_s }]).expect("advances");
    }
    assert_eq!(table.snapshot().world_time_s(), 3_606);
    let overflow = apply(
        &mut table,
        4,
        &[TableEffect::TimeAdvanced {
            advance_s: u64::MAX,
        }],
    );
    assert_eq!(refusal(overflow), EffectCause::WorldTimeOverflow);
    assert_eq!(table.snapshot().world_time_s(), 3_606);
}

#[test]
fn a_paused_table_freezes_the_clock_and_resuming_lets_it_move_again() {
    let mut table = table_in_scene_ten();
    apply(&mut table, 2, &[TableEffect::TablePaused]).expect("pauses");
    let frozen = apply(&mut table, 3, &[TableEffect::TimeAdvanced { advance_s: 6 }]);
    assert_eq!(refusal(frozen), EffectCause::TablePaused);
    assert_eq!(table.snapshot().world_time_s(), 0);
    apply(&mut table, 4, &[TableEffect::TableResumed]).expect("resumes");
    apply(&mut table, 5, &[TableEffect::TimeAdvanced { advance_s: 6 }]).expect("advances");
    assert_eq!(table.snapshot().world_time_s(), 6);
}

#[test]
fn an_idle_table_still_moves_its_clock_because_the_world_is_not_paused() {
    let mut table = table_in_scene_ten();
    apply(&mut table, 2, &[TableEffect::TableIdled]).expect("idles");
    apply(&mut table, 3, &[TableEffect::TimeAdvanced { advance_s: 6 }]).expect("advances");
    assert_eq!(table.snapshot().world_time_s(), 6);
}

#[test]
fn the_status_moves_only_along_the_allowed_edges() {
    let mut table = table_in_scene_ten();
    let status = |table: &TableState| table.snapshot().status();
    assert_eq!(status(&table), TableStatus::Active);
    assert_eq!(
        refusal(apply(&mut table, 2, &[TableEffect::TableResumed])),
        EffectCause::TableNotPausedOrIdle
    );
    apply(&mut table, 3, &[TableEffect::TableIdled]).expect("idles");
    assert_eq!(status(&table), TableStatus::Idle);
    assert_eq!(
        refusal(apply(&mut table, 4, &[TableEffect::TableIdled])),
        EffectCause::TableNotActive
    );
    apply(&mut table, 5, &[TableEffect::TablePaused]).expect("an idle table can be paused");
    assert_eq!(status(&table), TableStatus::Paused);
    assert_eq!(
        refusal(apply(&mut table, 6, &[TableEffect::TablePaused])),
        EffectCause::AlreadyPaused
    );
    assert_eq!(
        refusal(apply(&mut table, 7, &[TableEffect::TableIdled])),
        EffectCause::TableNotActive
    );
    apply(&mut table, 8, &[TableEffect::TableResumed]).expect("resumes");
    assert_eq!(status(&table), TableStatus::Active);
    apply(&mut table, 9, &[TableEffect::TableIdled]).expect("idles");
    apply(&mut table, 10, &[TableEffect::TableResumed]).expect("an idle table wakes");
    assert_eq!(status(&table), TableStatus::Active);
}

#[test]
fn a_pause_remembers_the_revision_of_the_commit_that_made_it() {
    let mut table = table_in_scene_ten();
    assert_eq!(table.snapshot().last_paused_at(), None);
    apply(&mut table, 7, &[TableEffect::TablePaused]).expect("pauses");
    let paused_at = table.snapshot().last_paused_at().expect("remembered");
    assert_eq!(paused_at, correlation(7).revision);
    apply(&mut table, 8, &[TableEffect::TableResumed]).expect("resumes");
    assert_eq!(table.snapshot().last_paused_at(), Some(paused_at));
}

fn pip_enters(table: &mut TableState, turn: u64) {
    apply(
        table,
        turn,
        &[TableEffect::CastEntered {
            entry: hero("pip", "Pip", 1, 10, 30),
        }],
    )
    .expect("the hero enters");
}

#[test]
fn a_cast_entry_enters_changes_and_leaves() {
    let mut table = table_in_scene_ten();
    pip_enters(&mut table, 2);
    let changes = [
        CastChange::Visibility {
            visibility: Visibility::Hidden,
        },
        CastChange::Placement {
            placement: Some(place(70, 40)),
        },
        CastChange::HitPoints { hit_points: 3 },
        CastChange::Conditions {
            conditions: vec![ConditionName::new("poisoned").unwrap()],
        },
    ];
    for (turn, change) in (3..).zip(changes) {
        apply(
            &mut table,
            turn,
            &[TableEffect::CastChanged {
                cast: element("pip"),
                change,
            }],
        )
        .expect("changes");
    }
    let snapshot = table.snapshot();
    let pip = snapshot.cast_entry(&element("pip")).unwrap();
    assert_eq!(pip.visibility(), Visibility::Hidden);
    assert_eq!(pip.placement(), Some(place(70, 40)));
    assert_eq!(pip.vitals().hit_points(), 3);
    assert_eq!(pip.vitals().maximum_hit_points(), 9);
    assert_eq!(pip.conditions().len(), 1);
    apply(
        &mut table,
        8,
        &[TableEffect::CastLeft {
            cast: element("pip"),
        }],
    )
    .expect("leaves");
    assert!(table.snapshot().cast_entry(&element("pip")).is_none());
}

#[test]
fn a_cast_entry_that_breaks_a_rule_is_refused_with_its_own_cause() {
    let mut table = table_in_scene_ten();
    pip_enters(&mut table, 2);
    let cases = [
        (
            hero("pip", "Pip again", 1, 10, 30),
            EffectCause::CastEntryExists,
        ),
        (
            creature("goblin", "Goblin", 77, 30),
            EffectCause::SceneNotActive,
        ),
        (
            hero("ghost", "Ghost", 9, 10, 30),
            EffectCause::ControllerNotSeated,
        ),
        (
            CastEntry::new(
                element("lord"),
                CastKind::Party,
                Controller::Engine,
                scene(10),
                display_name("Lord"),
                Vitals::new(1, 1).unwrap(),
            ),
            EffectCause::PartyNeedsSeatController,
        ),
    ];
    for (turn, (entry, expected)) in (3..).zip(cases) {
        let result = apply(&mut table, turn, &[TableEffect::CastEntered { entry }]);
        assert_eq!(refusal(result), expected);
    }
}

#[test]
fn a_hero_must_stand_in_the_scene_its_seat_is_in() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(2),
            scene: scene(11),
        }],
    )
    .expect("the seat goes alone");
    let astray = apply(
        &mut table,
        3,
        &[TableEffect::CastEntered {
            entry: hero("vex", "Vex", 2, 10, 30),
        }],
    );
    assert_eq!(refusal(astray), EffectCause::ControlledOutsideSeatScene);
}

#[test]
fn hit_points_cannot_be_raised_above_the_maximum_and_unknown_entries_cannot_change() {
    let mut table = table_in_scene_ten();
    pip_enters(&mut table, 2);
    let too_many = apply(
        &mut table,
        3,
        &[TableEffect::CastChanged {
            cast: element("pip"),
            change: CastChange::HitPoints { hit_points: 10 },
        }],
    );
    assert_eq!(refusal(too_many), EffectCause::HitPointsAboveMaximum);
    let unknown = apply(
        &mut table,
        4,
        &[TableEffect::CastChanged {
            cast: element("nobody"),
            change: CastChange::HitPoints { hit_points: 1 },
        }],
    );
    assert_eq!(refusal(unknown), EffectCause::CastEntryUnknown);
    let gone = apply(
        &mut table,
        5,
        &[TableEffect::CastLeft {
            cast: element("nobody"),
        }],
    );
    assert_eq!(refusal(gone), EffectCause::CastEntryUnknown);
}

pub(crate) fn slot_with(machine_name: &str, phase: &str) -> MachineSlot {
    MachineSlot::from_parts(
        machine(machine_name),
        1,
        serde_json::json!({"phase": phase}),
    )
}

fn slot(machine_name: &str) -> MachineSlot {
    slot_with(machine_name, "presenting")
}

fn push(scene_number: u64, machine_name: &str) -> TableEffect {
    TableEffect::HandOffPushed {
        scene: scene(scene_number),
        machine: machine(machine_name),
        slot: slot(machine_name),
    }
}

pub(crate) fn set_active(scene_number: u64, machine_name: &str) -> TableEffect {
    TableEffect::ActiveMachineSet {
        home: crate::table_state::MachineHome::Scene {
            scene: scene(scene_number),
        },
        machine: machine(machine_name),
        slot: slot(machine_name),
    }
}

fn stack_names(table: &TableState) -> Vec<String> {
    let snapshot = table.snapshot();
    snapshot
        .scene(scene(10))
        .unwrap()
        .hand_off_stack()
        .iter()
        .map(|frame| frame.machine().as_str().to_owned())
        .collect()
}

fn active_name(table: &TableState) -> String {
    let snapshot = table.snapshot();
    let name = snapshot.scene(scene(10)).unwrap().active_machine();
    name.map(|machine| machine.as_str().to_owned())
        .unwrap_or_default()
}

#[test]
fn a_hand_off_suspends_the_active_machine_and_activates_the_next_in_one_step() {
    let mut table = table_in_scene_ten();
    apply(&mut table, 2, &[set_active(10, "explore")]).expect("sets");
    apply(&mut table, 3, &[push(10, "dialogue")]).expect("pushes");
    assert_eq!(active_name(&table), "dialogue");
    assert_eq!(stack_names(&table), ["explore"]);
    let snapshot = table.snapshot();
    let frame = &snapshot.scene(scene(10)).unwrap().hand_off_stack()[0];
    assert_eq!(frame.pushed_at(), correlation(3).revision);
    assert_eq!(frame.slot(), &slot("explore"));
}

#[test]
fn the_hand_off_stack_holds_three_frames_and_a_pop_resumes_the_suspended_machine_with_its_slot() {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[TableEffect::ActiveMachineSet {
            home: crate::table_state::MachineHome::Scene { scene: scene(10) },
            machine: machine("explore"),
            slot: slot_with("explore", "awaiting_roll"),
        }],
    )
    .expect("sets");
    for (turn, name) in [(3, "dialogue"), (4, "shop"), (5, "rest")] {
        apply(&mut table, turn, &[push(10, name)]).expect("pushes");
    }
    let full = apply(&mut table, 6, &[push(10, "travel")]);
    assert_eq!(refusal(full), EffectCause::HandOffStackFull);
    assert_eq!(
        active_name(&table),
        "rest",
        "a refused push changes nothing"
    );
    assert_eq!(stack_names(&table), ["explore", "dialogue", "shop"]);
    apply(
        &mut table,
        7,
        &[TableEffect::HandOffPopped { scene: scene(10) }],
    )
    .expect("pops");
    assert_eq!(active_name(&table), "shop");
    assert_eq!(stack_names(&table), ["explore", "dialogue"]);
    for turn in [8, 9] {
        apply(
            &mut table,
            turn,
            &[TableEffect::HandOffPopped { scene: scene(10) }],
        )
        .expect("pops");
    }
    assert_eq!(active_name(&table), "explore");
    let snapshot = table.snapshot();
    let resumed = snapshot.scene(scene(10)).unwrap().active().unwrap();
    assert_eq!(resumed.slot(), &slot_with("explore", "awaiting_roll"));
}

#[test]
fn a_hand_off_needs_an_active_machine_a_new_one_and_a_slot_of_its_own() {
    let mut table = table_in_scene_ten();
    let none_running = apply(&mut table, 2, &[push(10, "explore")]);
    assert_eq!(refusal(none_running), EffectCause::NoActiveMachine);
    apply(&mut table, 3, &[set_active(10, "explore")]).expect("sets");
    let same = apply(&mut table, 4, &[push(10, "explore")]);
    assert_eq!(refusal(same), EffectCause::MachineAlreadyOnStack);
    apply(&mut table, 5, &[push(10, "dialogue")]).expect("pushes");
    let below = apply(&mut table, 6, &[push(10, "explore")]);
    assert_eq!(refusal(below), EffectCause::MachineAlreadyOnStack);
    assert_eq!(active_name(&table), "dialogue");
    assert_eq!(stack_names(&table), ["explore"]);
    let mismatched = apply(
        &mut table,
        7,
        &[TableEffect::HandOffPushed {
            scene: scene(10),
            machine: machine("shop"),
            slot: slot("rest"),
        }],
    );
    assert_eq!(refusal(mismatched), EffectCause::FrameSlotMismatch);
    let nowhere = apply(&mut table, 8, &[push(99, "shop")]);
    assert_eq!(refusal(nowhere), EffectCause::SceneNotActive);
}

#[test]
fn an_empty_stack_cannot_be_popped() {
    let mut table = table_in_scene_ten();
    apply(&mut table, 2, &[set_active(10, "explore")]).expect("sets");
    let empty = apply(
        &mut table,
        3,
        &[TableEffect::HandOffPopped { scene: scene(10) }],
    );
    assert_eq!(refusal(empty), EffectCause::HandOffStackEmpty);
    assert_eq!(active_name(&table), "explore");
}

#[test]
fn a_scene_has_an_active_machine_with_its_slot_once_one_was_set_and_a_step_updates_the_slot() {
    let mut table = table_in_scene_ten();
    assert!(
        table
            .snapshot()
            .scene(scene(10))
            .unwrap()
            .active()
            .is_none()
    );
    apply(&mut table, 2, &[set_active(10, "explore")]).expect("sets");
    assert_eq!(active_name(&table), "explore");
    apply(
        &mut table,
        3,
        &[TableEffect::ActiveSlotUpdated {
            home: crate::table_state::MachineHome::Scene { scene: scene(10) },
            slot: slot_with("explore", "resolved"),
        }],
    )
    .expect("updates");
    let snapshot = table.snapshot();
    let active = snapshot.scene(scene(10)).unwrap().active().unwrap();
    assert_eq!(active.machine(), &machine("explore"));
    assert_eq!(active.slot(), &slot_with("explore", "resolved"));
    let wrong = apply(
        &mut table,
        4,
        &[TableEffect::ActiveSlotUpdated {
            home: crate::table_state::MachineHome::Scene { scene: scene(10) },
            slot: slot("dialogue"),
        }],
    );
    assert_eq!(refusal(wrong), EffectCause::FrameSlotMismatch);
    let nowhere = apply(&mut table, 5, &[set_active(99, "explore")]);
    assert_eq!(refusal(nowhere), EffectCause::SceneNotActive);
    let mismatched = apply(
        &mut table,
        6,
        &[TableEffect::ActiveMachineSet {
            home: crate::table_state::MachineHome::Scene { scene: scene(10) },
            machine: machine("shop"),
            slot: slot("rest"),
        }],
    );
    assert_eq!(refusal(mismatched), EffectCause::FrameSlotMismatch);
}

#[test]
fn a_slot_update_for_a_scene_with_no_active_machine_is_refused() {
    let mut table = table_in_scene_ten();
    let none = apply(
        &mut table,
        2,
        &[TableEffect::ActiveSlotUpdated {
            home: crate::table_state::MachineHome::Scene { scene: scene(10) },
            slot: slot("explore"),
        }],
    );
    assert_eq!(refusal(none), EffectCause::NoActiveMachine);
}

#[test]
fn setting_the_active_machine_replaces_it_and_leaves_the_stack_alone() {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[set_active(10, "explore"), push(10, "dialogue")],
    )
    .expect("applies");
    apply(&mut table, 3, &[set_active(10, "lobby")]).expect("replaces");
    assert_eq!(active_name(&table), "lobby");
    assert_eq!(stack_names(&table), ["explore"]);
}

fn scene_closed_lines(outcome: ApplyOutcome) -> Vec<String> {
    let ApplyOutcome::Applied(events) = outcome else {
        panic!("applied");
    };
    events
        .iter()
        .map(|event| event.render_audit_line())
        .filter(|line| line.starts_with("SCENE closed"))
        .collect()
}

#[test]
fn a_scene_that_closes_with_a_machine_and_a_stack_says_so_in_an_event() {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[set_active(10, "explore"), push(10, "dialogue")],
    )
    .expect("applies");
    let outcome = apply(
        &mut table,
        3,
        &[TableEffect::SceneEntered { scene: scene(11) }],
    )
    .expect("enters");
    let lines = scene_closed_lines(outcome);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].ends_with("closed_scene=10 generation=1 active=dialogue stack_depth=1"),
        "{}",
        lines[0]
    );
}

#[test]
fn a_scene_entered_again_reports_the_scene_it_replaced_and_a_first_scene_reports_none() {
    let mut table = TableState::seated(2, &[seat(1)]).expect("valid table");
    let first = apply(
        &mut table,
        1,
        &[TableEffect::SceneEntered { scene: scene(10) }],
    )
    .unwrap();
    assert!(scene_closed_lines(first).is_empty());
    let again = apply(
        &mut table,
        2,
        &[TableEffect::SceneEntered { scene: scene(10) }],
    )
    .unwrap();
    let lines = scene_closed_lines(again);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].ends_with("closed_scene=10 generation=1 stack_depth=0"),
        "{}",
        lines[0]
    );
}

#[test]
fn a_scene_emptied_by_a_seat_leaving_it_is_reported_closed() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(2),
            scene: scene(11),
        }],
    )
    .expect("applies");
    let outcome = apply(
        &mut table,
        3,
        &[TableEffect::SeatMoved {
            seat: seat(2),
            scene: scene(10),
        }],
    )
    .expect("regroups");
    let lines = scene_closed_lines(outcome);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("closed_scene=11 generation=2"),
        "{}",
        lines[0]
    );
}

#[test]
fn a_commit_with_one_refused_effect_changes_none_of_the_new_state_either() {
    let mut table = table_in_scene_ten();
    let before = table.clone();
    let effects = [
        TableEffect::TimeAdvanced { advance_s: 60 },
        TableEffect::CastEntered {
            entry: hero("pip", "Pip", 1, 10, 30),
        },
        TableEffect::TablePaused,
        TableEffect::SeatLeft { seat: seat(9) },
    ];
    assert!(apply(&mut table, 2, &effects).is_err());
    assert_eq!(table, before);
    assert_eq!(table.snapshot().world_time_s(), 0);
}

#[test]
fn the_revision_of_the_table_is_the_newest_commit_applied_and_starts_at_genesis() {
    let genesis = SessionRevision::new(RecoveryEpoch::FIRST, 0);
    let mut table = TableState::seated(2, &[seat(1)]).expect("valid table");
    assert_eq!(table.snapshot().revision(), genesis);
    apply(&mut table, 5, &[]).expect("an empty commit applies");
    assert_eq!(table.snapshot().revision(), correlation(5).revision);
    apply(&mut table, 3, &[TableEffect::TableIdled]).expect("applies");
    assert_eq!(
        table.snapshot().revision(),
        correlation(5).revision,
        "an older commit never moves it back"
    );
    let refused = apply(&mut table, 9, &[TableEffect::SeatLeft { seat: seat(9) }]);
    assert!(refused.is_err());
    assert_eq!(table.snapshot().revision(), correlation(5).revision);
}

#[test]
fn the_table_machine_and_a_scenes_machine_each_have_their_own_home_and_the_focus_prefers_the_scene()
{
    let mut table = table_in_scene_ten();
    assert!(table.snapshot().focus_machine().is_none());
    let table_home = crate::table_state::MachineHome::Table;
    apply(
        &mut table,
        2,
        &[TableEffect::ActiveMachineSet {
            home: table_home,
            machine: machine("lobby"),
            slot: slot_with("lobby", "gathering"),
        }],
    )
    .expect("sets");
    let snapshot = table.snapshot();
    assert_eq!(
        snapshot.table_machine().unwrap().machine(),
        &machine("lobby")
    );
    assert_eq!(
        snapshot.focus_machine().unwrap().machine(),
        &machine("lobby")
    );
    apply(&mut table, 3, &[set_active(10, "explore")]).expect("sets");
    let snapshot = table.snapshot();
    assert_eq!(
        snapshot.focus_machine().unwrap().machine(),
        &machine("explore")
    );
    assert_eq!(
        snapshot.table_machine().unwrap().machine(),
        &machine("lobby")
    );
    apply(
        &mut table,
        4,
        &[TableEffect::ActiveSlotUpdated {
            home: table_home,
            slot: slot_with("lobby", "started"),
        }],
    )
    .expect("updates");
    let snapshot = table.snapshot();
    assert_eq!(
        snapshot.table_machine().unwrap().slot(),
        &slot_with("lobby", "started")
    );
    let wrong = apply(
        &mut table,
        5,
        &[TableEffect::ActiveSlotUpdated {
            home: table_home,
            slot: slot_with("explore", "x"),
        }],
    );
    assert_eq!(refusal(wrong), EffectCause::FrameSlotMismatch);
}

#[test]
fn a_slot_update_for_a_table_with_no_table_machine_is_refused() {
    let mut table = table_in_scene_ten();
    let none = apply(
        &mut table,
        2,
        &[TableEffect::ActiveSlotUpdated {
            home: crate::table_state::MachineHome::Table,
            slot: slot_with("lobby", "gathering"),
        }],
    );
    assert_eq!(refusal(none), EffectCause::NoActiveMachine);
}

#[test]
fn a_seat_may_play_one_hero_and_the_next_waits_for_the_first_to_leave() {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[TableEffect::CastEntered {
            entry: hero("pip", "Pip", 1, 10, 30),
        }],
    )
    .expect("the hero enters");
    assert_eq!(
        table.snapshot().seat(seat(1)).unwrap().hero(),
        Some(&element("pip"))
    );
    let second = apply(
        &mut table,
        3,
        &[TableEffect::CastEntered {
            entry: hero("pop", "Pop", 1, 10, 40),
        }],
    );
    assert_eq!(refusal(second), EffectCause::SeatHasHero);
    apply(
        &mut table,
        4,
        &[
            TableEffect::CastLeft {
                cast: element("pip"),
            },
            TableEffect::CastEntered {
                entry: hero("pop", "Pop", 1, 10, 40),
            },
        ],
    )
    .expect("the replacement enters after the old hero leaves");
    let snapshot = table.snapshot();
    assert_eq!(
        snapshot.seat(seat(1)).unwrap().hero(),
        Some(&element("pop"))
    );
    assert_eq!(snapshot.hero_of(seat(1)).unwrap().name().as_str(), "Pop");
    assert!(snapshot.hero_of(seat(2)).is_none());
}

fn horse() -> CastEntry {
    CastEntry::new(
        element("horse"),
        CastKind::Mount,
        Controller::Seat { seat: seat(1) },
        scene(10),
        display_name("Horse"),
        Vitals::new(20, 20).expect("valid vitals"),
    )
}

#[test]
fn a_seats_mount_travels_with_its_hero_when_the_party_enters_a_scene() {
    let mut table = table_in_scene_ten();
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered { entry: horse() },
            TableEffect::CastEntered {
                entry: creature("goblin", "Goblin", 10, 60),
            },
        ],
    )
    .expect("the cast enters");
    apply(
        &mut table,
        3,
        &[TableEffect::SceneEntered { scene: scene(11) }],
    )
    .expect("enters");
    let snapshot = table.snapshot();
    assert_eq!(
        snapshot.cast_entry(&element("horse")).unwrap().scene(),
        scene(11)
    );
    assert_eq!(
        snapshot.cast_entry(&element("pip")).unwrap().scene(),
        scene(11)
    );
    assert!(snapshot.cast_entry(&element("goblin")).is_none());
}

#[test]
fn a_seats_mount_goes_with_it_when_it_moves_alone_and_is_not_dropped_with_the_old_scene() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered { entry: horse() },
            TableEffect::SeatMoved {
                seat: seat(1),
                scene: scene(11),
            },
        ],
    )
    .expect("applies");
    let snapshot = table.snapshot();
    assert_eq!(
        snapshot.cast_entry(&element("horse")).unwrap().scene(),
        scene(11)
    );
    assert_eq!(
        snapshot.cast_entry(&element("pip")).unwrap().scene(),
        scene(11)
    );
}

#[test]
fn a_creature_a_seat_controls_must_stand_where_the_seat_stands() {
    let mut table = party_that_may_split();
    apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(1),
            scene: scene(11),
        }],
    )
    .expect("the seat goes alone");
    let astray = apply(
        &mut table,
        3,
        &[TableEffect::CastEntered { entry: horse() }],
    );
    assert_eq!(refusal(astray), EffectCause::ControlledOutsideSeatScene);
}

#[test]
fn each_new_effect_is_one_class_a_event_with_its_own_kind() {
    let mut table = table_in_scene_ten();
    let outcome = apply(
        &mut table,
        2,
        &[
            TableEffect::TimeAdvanced { advance_s: 6 },
            TableEffect::TableIdled,
            TableEffect::TableResumed,
        ],
    )
    .expect("applies");
    let ApplyOutcome::Applied(events) = outcome else {
        panic!("the commit was applied");
    };
    let lines: Vec<_> = events
        .iter()
        .map(|event| event.render_audit_line())
        .collect();
    assert_eq!(lines.len(), 3);
    for (line, kind) in lines
        .iter()
        .zip(["time_advanced", "table_idled", "table_resumed"])
    {
        assert!(line.starts_with("EFFECT applied "), "{line}");
        assert!(line.ends_with(&format!("effect={kind}")), "{line}");
    }
}

#[test]
fn the_table_exports_the_machines_import_names_from_effects_unchanged() {
    let table: crate::effects::TableState =
        crate::effects::TableState::new(2).expect("valid table");
    let same: TableState = table.clone();
    assert_eq!(table, same);
    assert_eq!(table.seats().len(), 0);
    assert!(table.seats().is_empty());
    assert_eq!(table.seat_capacity(), 2);
    assert_eq!(table.active_scene(), None);
    assert!(table.last_hand_off().is_none());
}

#[test]
fn a_seat_that_leaves_takes_every_creature_it_controlled_not_only_its_hero() {
    let mut table = table_in_scene_ten();
    let wolf = CastEntry::new(
        element("wolf"),
        CastKind::Summon,
        Controller::Seat { seat: seat(1) },
        scene(10),
        display_name("Wolf"),
        Vitals::new(11, 11).expect("valid vitals"),
    );
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered { entry: wolf },
            TableEffect::CastEntered {
                entry: hero("vex", "Vex", 2, 10, 50),
            },
        ],
    )
    .expect("the cast enters");
    apply(&mut table, 3, &[TableEffect::SeatLeft { seat: seat(1) }]).expect("leaves");
    let snapshot = table.snapshot();
    assert!(snapshot.cast_entry(&element("pip")).is_none());
    assert!(snapshot.cast_entry(&element("wolf")).is_none());
    assert!(
        snapshot.cast_entry(&element("vex")).is_some(),
        "the other seat's hero stays"
    );
}
