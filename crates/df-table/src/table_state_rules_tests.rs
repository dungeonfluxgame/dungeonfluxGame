use crate::effects::TableEffect;
use crate::identity::{RecoveryEpoch, SessionRevision, TableId, TableScope};
use crate::outcome::DeclineCode;
use crate::table_state::{
    IdleClock, IdleVerdict, InFlight, PlayInput, ResultStamp, SceneGeneration, SplitPolicy,
    StaleCause, TableState, TableStatus, split_party_decline,
};
use crate::table_state_tests::{apply, scene, seat, table_in_scene_ten};
use crate::testkit::correlation;

fn generation_of(table: &TableState, scene_number: u64) -> SceneGeneration {
    table
        .snapshot()
        .scene(scene(scene_number))
        .expect("the scene is open")
        .generation()
}

fn stamp(scene_number: u64, generation: SceneGeneration, turn: u64) -> ResultStamp {
    ResultStamp {
        scene: scene(scene_number),
        generation,
        revision: correlation(turn).revision,
    }
}

// The split-party decline (RES25, K.10).

#[test]
fn a_split_is_declined_not_supported_yet_with_the_two_things_to_do_instead() {
    let table = table_in_scene_ten();
    let decline = table.snapshot().judge_split(seat(1)).expect_err("declined");
    assert_eq!(decline.code(), DeclineCode::NotSupportedYet);
    assert!(
        decline
            .reason()
            .contains("move to another area of this place")
    );
    assert!(decline.reason().contains("ask the host to pause"));
    assert_eq!(
        decline.alternatives(),
        [
            "Move to another area of this place",
            "Ask the host to pause"
        ]
    );
    assert_eq!(decline, split_party_decline());
}

#[test]
fn the_split_decline_reads_in_plain_words_and_holds_no_code() {
    let decline = split_party_decline();
    for text in
        std::iter::once(decline.reason()).chain(decline.alternatives().iter().map(String::as_str))
    {
        assert!(!text.contains("not_supported_yet"), "{text}");
        assert!(!text.is_empty());
    }
}

#[test]
fn a_split_is_declined_while_the_party_stays_together_and_a_lone_seat_has_nothing_to_split_from() {
    let lone = TableState::seated(2, &[seat(1)]).expect("valid table");
    assert_eq!(lone.snapshot().split_policy(), SplitPolicy::Declined);
    let alone = lone.snapshot().judge_split(seat(1)).expect_err("declined");
    assert_eq!(alone.code(), DeclineCode::NotViable);
    let company = table_in_scene_ten();
    let decline = company
        .snapshot()
        .judge_split(seat(1))
        .expect_err("declined");
    assert_eq!(decline.code(), DeclineCode::NotSupportedYet);
}

#[test]
fn a_stranger_cannot_split_whatever_the_policy_says() {
    for policy in [SplitPolicy::Declined, SplitPolicy::Allowed] {
        let table = table_in_scene_ten().with_split_policy(policy);
        let stranger = table.snapshot().judge_split(seat(9)).expect_err("declined");
        assert_eq!(stranger.code(), DeclineCode::NotViable, "{policy:?}");
    }
}

#[test]
fn a_table_that_allows_a_split_lets_a_seated_seat_ask_and_still_refuses_a_stranger() {
    let table = table_in_scene_ten().with_split_policy(SplitPolicy::Allowed);
    assert_eq!(table.snapshot().judge_split(seat(1)), Ok(()));
    let stranger = table.snapshot().judge_split(seat(9)).expect_err("declined");
    assert_eq!(stranger.code(), DeclineCode::NotViable);
}

#[test]
fn the_applier_and_the_judge_agree_a_declined_split_is_also_a_refused_effect() {
    let mut table = table_in_scene_ten();
    assert!(table.snapshot().judge_split(seat(1)).is_err());
    let moved = apply(
        &mut table,
        2,
        &[TableEffect::SeatMoved {
            seat: seat(1),
            scene: scene(11),
        }],
    );
    assert!(moved.is_err());
}

// The pause and idle rules (D.3, RES29).

#[test]
fn a_paused_table_declines_play_input_as_time_locked_and_the_other_states_take_it() {
    let mut table = table_in_scene_ten();
    assert_eq!(table.snapshot().judge_play_input(), Ok(PlayInput::Proceed));
    apply(&mut table, 2, &[TableEffect::TableIdled]).expect("idles");
    assert_eq!(
        table.snapshot().judge_play_input(),
        Ok(PlayInput::WakeIdleTable)
    );
    apply(&mut table, 3, &[TableEffect::TablePaused]).expect("pauses");
    let decline = table.snapshot().judge_play_input().expect_err("declined");
    assert_eq!(decline.code(), DeclineCode::TimeLocked);
    assert!(decline.reason().contains("paused"));
}

fn clock(last_input_ms: u64, now_ms: u64) -> IdleClock {
    IdleClock {
        last_input_ms,
        now_ms,
        idle_after_ms: 600_000,
    }
}

#[test]
fn a_quiet_table_goes_idle_exactly_when_the_idle_time_has_passed() {
    let table = table_in_scene_ten();
    let snapshot = table.snapshot();
    let clear = InFlight::default();
    assert_eq!(
        snapshot.judge_idle(clock(0, 599_999), clear),
        IdleVerdict::StayActive { idle_in_ms: 1 }
    );
    assert_eq!(
        snapshot.judge_idle(clock(0, 600_000), clear),
        IdleVerdict::GoIdle
    );
    assert_eq!(
        snapshot.judge_idle(clock(0, 9_000_000), clear),
        IdleVerdict::GoIdle
    );
    assert_eq!(
        snapshot.judge_idle(clock(1_000, 1_000), clear),
        IdleVerdict::StayActive {
            idle_in_ms: 600_000
        }
    );
}

#[test]
fn a_clock_that_ran_backwards_counts_as_no_time_and_a_zero_idle_time_idles_at_once() {
    let table = table_in_scene_ten();
    let snapshot = table.snapshot();
    let clear = InFlight::default();
    assert_eq!(
        snapshot.judge_idle(clock(5_000, 1_000), clear),
        IdleVerdict::StayActive {
            idle_in_ms: 600_000
        }
    );
    let at_once = IdleClock {
        last_input_ms: 7,
        now_ms: 7,
        idle_after_ms: 0,
    };
    assert_eq!(snapshot.judge_idle(at_once, clear), IdleVerdict::GoIdle);
}

#[test]
fn a_table_cannot_go_idle_while_a_pending_or_a_held_turn_waits() {
    let table = table_in_scene_ten();
    let snapshot = table.snapshot();
    for blockers in [
        InFlight {
            open_pendings: 1,
            held_turns: 0,
        },
        InFlight {
            open_pendings: 0,
            held_turns: 2,
        },
        InFlight {
            open_pendings: 3,
            held_turns: 3,
        },
    ] {
        assert_eq!(
            snapshot.judge_idle(clock(0, 9_000_000), blockers),
            IdleVerdict::Blocked(blockers)
        );
    }
}

#[test]
fn an_idle_or_paused_table_has_nothing_to_idle() {
    let mut table = table_in_scene_ten();
    apply(&mut table, 2, &[TableEffect::TableIdled]).expect("idles");
    assert_eq!(
        table
            .snapshot()
            .judge_idle(clock(0, 9_000_000), InFlight::default()),
        IdleVerdict::NotActive(TableStatus::Idle)
    );
    apply(&mut table, 3, &[TableEffect::TablePaused]).expect("pauses");
    assert_eq!(
        table
            .snapshot()
            .judge_idle(clock(0, 9_000_000), InFlight::default()),
        IdleVerdict::NotActive(TableStatus::Paused)
    );
}

#[test]
fn a_verdict_changes_nothing_and_going_idle_is_a_commit_the_caller_makes() {
    let mut table = table_in_scene_ten();
    let before = table.clone();
    let verdict = table
        .snapshot()
        .judge_idle(clock(0, 600_000), InFlight::default());
    assert_eq!(verdict, IdleVerdict::GoIdle);
    assert_eq!(table, before, "judging idle is a question");
    apply(&mut table, 2, &[TableEffect::TableIdled]).expect("the caller commits it");
    assert_eq!(table.snapshot().status(), TableStatus::Idle);
}

// Stale-result rejection by scene generation (D.2, RES42).

#[test]
fn a_result_for_the_scene_and_generation_it_was_started_under_is_accepted() {
    let table = table_in_scene_ten();
    let generation = generation_of(&table, 10);
    assert_eq!(
        table.snapshot().judge_result(&stamp(10, generation, 5)),
        Ok(())
    );
}

#[test]
fn a_result_started_before_the_party_changed_scene_is_rejected_as_late() {
    let mut table = table_in_scene_ten();
    let old_generation = generation_of(&table, 10);
    apply(
        &mut table,
        2,
        &[TableEffect::SceneEntered { scene: scene(11) }],
    )
    .expect("enters");
    let stale = table
        .snapshot()
        .judge_result(&stamp(10, old_generation, 5))
        .expect_err("rejected");
    assert_eq!(stale.cause, StaleCause::SceneClosed);
    assert_eq!(stale.current_generation, None);
}

#[test]
fn a_result_for_a_scene_left_and_entered_again_is_rejected_by_its_generation() {
    let mut table = table_in_scene_ten();
    let first = generation_of(&table, 10);
    apply(
        &mut table,
        2,
        &[TableEffect::SceneEntered { scene: scene(11) }],
    )
    .expect("enters");
    apply(
        &mut table,
        3,
        &[TableEffect::SceneEntered { scene: scene(10) }],
    )
    .expect("returns");
    let again = generation_of(&table, 10);
    assert_ne!(first, again);
    let stale = table
        .snapshot()
        .judge_result(&stamp(10, first, 5))
        .expect_err("rejected");
    assert_eq!(stale.cause, StaleCause::GenerationMismatch);
    assert_eq!(stale.current_generation, Some(again));
    assert_eq!(table.snapshot().judge_result(&stamp(10, again, 5)), Ok(()));
}

#[test]
fn a_result_carrying_a_generation_the_scene_never_had_is_rejected_too() {
    let table = table_in_scene_ten();
    let forged = SceneGeneration::new(u64::MAX);
    let stale = table
        .snapshot()
        .judge_result(&stamp(10, forged, 5))
        .expect_err("rejected");
    assert_eq!(stale.cause, StaleCause::GenerationMismatch);
}

#[test]
fn a_result_from_an_older_recovery_epoch_is_rejected_first() {
    let mut table = table_in_scene_ten();
    let generation = generation_of(&table, 10);
    let newer = SessionRevision::new(RecoveryEpoch::new(2).expect("valid epoch"), 1);
    let mut commit = correlation(5);
    commit.revision = newer;
    table
        .apply(
            &TableScope::of_table(TableId::new(1)),
            commit,
            &[TableEffect::TimeAdvanced { advance_s: 6 }],
        )
        .expect("a commit of the second epoch");
    let stale = table
        .snapshot()
        .judge_result(&stamp(10, generation, 9))
        .expect_err("rejected");
    assert_eq!(stale.cause, StaleCause::OlderRecoveryEpoch);
}

#[test]
fn a_paused_table_holds_results_back_and_a_result_from_before_the_pause_stays_stale_after_it() {
    let mut table = table_in_scene_ten();
    let generation = generation_of(&table, 10);
    apply(&mut table, 2, &[TableEffect::TimeAdvanced { advance_s: 6 }]).expect("advances");
    apply(&mut table, 10, &[TableEffect::TablePaused]).expect("pauses");
    let held = table
        .snapshot()
        .judge_result(&stamp(10, generation, 12))
        .expect_err("held back");
    assert_eq!(held.cause, StaleCause::TablePaused);
    apply(&mut table, 11, &[TableEffect::TableResumed]).expect("resumes");
    let before = table
        .snapshot()
        .judge_result(&stamp(10, generation, 9))
        .expect_err("rejected");
    assert_eq!(before.cause, StaleCause::BeforePause);
    let at_the_pause = table
        .snapshot()
        .judge_result(&stamp(10, generation, 10))
        .expect_err("the pause commit's own snapshot is not after it");
    assert_eq!(at_the_pause.cause, StaleCause::BeforePause);
    assert_eq!(
        table.snapshot().judge_result(&stamp(10, generation, 11)),
        Ok(()),
        "a job started from the resumed table is current"
    );
}

#[test]
fn a_rejected_result_renders_one_class_a_line_with_the_pinned_context_and_both_generations() {
    let mut table = table_in_scene_ten();
    let old_generation = generation_of(&table, 10);
    apply(
        &mut table,
        2,
        &[TableEffect::SceneEntered { scene: scene(10) }],
    )
    .expect("re-enters");
    let stale = table
        .snapshot()
        .judge_result(&stamp(10, old_generation, 5))
        .expect_err("rejected");
    let scope = TableScope::of_table(TableId::new(1)).in_scene(scene(10));
    let event = stale.event(&scope, correlation(5));
    assert_eq!(event.class(), crate::event::EventClass::Always);
    assert_eq!(event.area(), "RESULT");
    let line = event.render_audit_line();
    assert!(
        line.starts_with("RESULT rejected table=1 scene=10 operation_id="),
        "{line}"
    );
    assert!(
        line.ends_with(
            "turn_seq=5 epoch=1 revision=5 cause=generation_mismatch generation=1 current=2"
        ),
        "{line}"
    );
    let round_trip: crate::event::TableEvent =
        serde_json::from_str(&serde_json::to_string(&event).expect("serializes")).expect("parses");
    assert_eq!(round_trip, event);
}

#[test]
fn a_rejection_of_a_closed_scene_has_no_current_generation_in_its_line() {
    let table = table_in_scene_ten();
    let stale = table
        .snapshot()
        .judge_result(&stamp(99, SceneGeneration::new(1), 5))
        .expect_err("rejected");
    let line = stale
        .event(&TableScope::of_table(TableId::new(1)), correlation(5))
        .render_audit_line();
    assert!(line.ends_with("cause=scene_closed generation=1"), "{line}");
}

#[test]
fn every_stale_cause_has_a_distinct_key() {
    let mut keys: Vec<_> = StaleCause::ALL.iter().map(|cause| cause.key()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), StaleCause::ALL.len());
}
