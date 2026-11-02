//! The group commit of the S1 decision: batches, atomicity, the commit deadline, the backoff and
//! the order inputs are released in.

use std::sync::Arc;

use df_table::outcome::{DeclineCode, OutcomeKind};

use super::commit::{CommitError, CommitPolicy, ScriptedCommit};
use super::handle::EngineHandle;
use super::report::{CommitDisposition, TurnReport};
use super::request::QueueControl;
use super::tests_support::{
    RigOptions, TickingClock, adapters, config, count_lines, lines, rig_with, tap,
};
use crate::event_log::EventLog;

async fn join_and_release(
    rig: &super::tests_support::Rig,
    seats: std::ops::RangeInclusive<u64>,
) -> Vec<TurnReport> {
    for seat in seats {
        let reply = rig
            .handle
            .submit(tap(seat, "join"))
            .await
            .expect("answered");
        assert!(reply.turns.is_empty(), "a held owner serves nothing");
    }
    rig.handle
        .queue_control(QueueControl::Release)
        .await
        .expect("answered")
        .turns
}

fn held() -> RigOptions {
    RigOptions {
        held: true,
        ..RigOptions::default()
    }
}

#[tokio::test]
async fn five_inputs_that_arrive_together_are_served_in_one_batch_and_saved_once() {
    let rig = rig_with(held());

    let turns = join_and_release(&rig, 1..=5).await;

    assert_eq!(turns.len(), 5);
    assert!(
        turns
            .iter()
            .all(|turn| turn.commit == CommitDisposition::Committed { id: 1 })
    );
    let records = rig.store.records();
    assert_eq!((records.len(), records[0].inputs), (1, 5));
    assert_eq!(count_lines(&rig.log, "COMMIT applied"), 1);
    assert_eq!(
        rig.handle.dump().await.expect("answered").table.seats,
        [1, 2, 3, 4, 5]
    );
}

#[tokio::test]
async fn the_per_input_policy_commits_each_input_on_its_own() {
    let rig = rig_with(RigOptions {
        policy: CommitPolicy::PerInput,
        ..held()
    });

    let turns = join_and_release(&rig, 1..=5).await;

    let ids: Vec<CommitDisposition> = turns.iter().map(|turn| turn.commit.clone()).collect();
    let expected: Vec<CommitDisposition> = (1..=5)
        .map(|id| CommitDisposition::Committed { id })
        .collect();
    assert_eq!(ids, expected);
    assert_eq!(rig.store.records().len(), 5);
}

#[tokio::test]
async fn inputs_are_served_in_order_and_each_sees_what_the_ones_before_it_did_in_the_batch() {
    let rig = rig_with(held());
    for action in ["join", "ready", "start"] {
        rig.handle.submit(tap(1, action)).await.expect("answered");
    }

    let turns = rig
        .handle
        .queue_control(QueueControl::Release)
        .await
        .expect("answered")
        .turns;

    let sequence: Vec<u64> = turns.iter().map(|turn| turn.turn_seq).collect();
    assert_eq!(
        sequence,
        [1, 2, 3, 4],
        "the hand-off is served after the start that asked for it"
    );
    let outcomes: Vec<OutcomeKind> = turns.iter().map(|turn| turn.outcome).collect();
    assert_eq!(outcomes, [OutcomeKind::Applied; 4]);
    assert_eq!(turns[3].machine_after, "explore");
    assert_eq!(rig.store.records().len(), 1);
}

#[tokio::test]
async fn a_failed_commit_fails_the_whole_batch_and_nothing_of_it_is_kept_or_reported_applied() {
    let rig = rig_with(RigOptions {
        script: vec![ScriptedCommit::Fails(CommitError::Unavailable)],
        ..held()
    });

    let turns = join_and_release(&rig, 1..=2).await;

    assert_eq!(turns.len(), 2);
    for turn in &turns {
        assert_eq!(turn.outcome, OutcomeKind::Declined);
        assert_eq!(turn.decline, Some(DeclineCode::TableBusy));
        assert_eq!(
            turn.commit,
            CommitDisposition::Failed {
                cause: "unavailable".to_owned()
            }
        );
        assert_eq!(turn.effects, 0);
        assert_eq!(turn.state_after, turn.state_before);
    }
    let dump = rig.handle.dump().await.expect("answered");
    assert_eq!((dump.revision, dump.table.seats.len()), (0, 0));
    assert!(rig.store.records().is_empty());
    assert_eq!(
        count_lines(&rig.log, "COMMIT failed inputs=2 cause=unavailable"),
        1
    );
    assert_eq!(
        count_lines(&rig.log, "EFFECT applied"),
        0,
        "the events of work that was rolled back never reach the log"
    );
    assert_eq!(count_lines(&rig.log, "code=table_busy"), 2);
    let queue = rig.handle.queue_show().await.expect("answered");
    assert_eq!(queue.commit.failures, 1);
    assert_eq!(queue.commit.commits, 0);
}

#[tokio::test]
async fn a_commit_that_misses_its_deadline_refuses_the_batch_table_busy_and_keeps_nothing() {
    let rig = rig_with(RigOptions {
        commit_deadline_ms: 1_000,
        script: vec![ScriptedCommit::Succeeds { latency_ms: 1_001 }],
        ..RigOptions::default()
    });

    let reply = rig.handle.submit(tap(1, "join")).await.expect("answered");

    let turn = &reply.turns[0];
    assert_eq!(turn.decline, Some(DeclineCode::TableBusy));
    assert_eq!(
        turn.commit,
        CommitDisposition::Failed {
            cause: "deadline_exceeded".to_owned()
        }
    );
    assert!(rig.store.records().is_empty());
    assert_eq!(rig.store.attempts(), 1);
    assert_eq!(
        count_lines(
            &rig.log,
            "COMMIT failed inputs=1 cause=deadline_exceeded discarded_events=",
        ),
        1
    );
    assert_eq!(count_lines(&rig.log, "deadline_ms=1000"), 1);
}

#[tokio::test]
async fn after_a_failed_commit_the_owner_refuses_new_input_until_the_backoff_ends() {
    let rig = rig_with(RigOptions {
        commit_backoff_ms: 5_000,
        script: vec![ScriptedCommit::Fails(CommitError::Refused)],
        ..RigOptions::default()
    });
    rig.handle.submit(tap(1, "join")).await.expect("answered");

    let refused = rig.handle.submit(tap(2, "join")).await.expect("answered");

    let refusal = refused
        .submission
        .refusal
        .expect("refused while backing off");
    assert_eq!(refusal.cause, "table_busy");
    assert_eq!(refusal.retry_after_ms, Some(5_000));
    assert!(refused.turns.is_empty());
    let queue = rig.handle.queue_show().await.expect("answered");
    assert_eq!(queue.commit.backing_off_for_ms, Some(5_000));
    assert!(
        queue.lanes.iter().all(|lane| lane.depth == 0),
        "nothing piled up"
    );
    assert_eq!(count_lines(&rig.log, "COMMIT refused operation_id="), 1);

    rig.clock.advance_ms(5_000);
    let served = rig.handle.submit(tap(2, "join")).await.expect("answered");

    assert!(served.submission.refusal.is_none());
    assert_eq!(
        served.turns[0].commit,
        CommitDisposition::Committed { id: 2 },
        "the failed attempt's id is burned, so the next write is the second"
    );
    let queue = rig.handle.queue_show().await.expect("answered");
    assert_eq!(queue.commit.backing_off_for_ms, None);
}

#[tokio::test]
async fn an_operation_id_of_a_failed_batch_is_released_so_the_same_id_can_be_retried() {
    let rig = rig_with(RigOptions {
        script: vec![ScriptedCommit::Fails(CommitError::Unavailable)],
        ..RigOptions::default()
    });
    let failed = rig
        .handle
        .submit(super::tests_support::with_operation(tap(1, "join"), 9))
        .await
        .expect("answered");
    assert_eq!(failed.turns[0].decline, Some(DeclineCode::TableBusy));
    rig.clock.advance_ms(5_000);

    let again = rig
        .handle
        .submit(super::tests_support::with_operation(tap(1, "join"), 9))
        .await
        .expect("answered");

    assert!(
        again.submission.refusal.is_none(),
        "nothing was kept of the first, so the id is free: {:?}",
        again.submission.refusal
    );
    assert_eq!(again.turns[0].outcome, OutcomeKind::Applied);
    assert!(matches!(
        again.turns[0].commit,
        CommitDisposition::Committed { .. }
    ));
    let third = rig
        .handle
        .submit(super::tests_support::with_operation(tap(1, "join"), 9))
        .await
        .expect("answered");
    assert_eq!(
        third
            .submission
            .refusal
            .expect("now it is remembered")
            .cause,
        "duplicate"
    );
    assert_eq!(
        rig.handle.dump().await.expect("answered").table.seats,
        [1],
        "the seat joined once"
    );
}

#[tokio::test]
async fn an_attempt_that_failed_after_its_write_landed_still_burns_its_id() {
    let rig = rig_with(RigOptions {
        script: vec![ScriptedCommit::FailsButLands(CommitError::Unavailable)],
        ..RigOptions::default()
    });
    rig.handle.submit(tap(1, "join")).await.expect("answered");
    rig.clock.advance_ms(5_000);

    let next = rig.handle.submit(tap(2, "join")).await.expect("answered");

    assert_eq!(next.turns[0].commit, CommitDisposition::Committed { id: 2 });
    let ids: Vec<u64> = rig
        .store
        .records()
        .iter()
        .map(|record| record.commit.get())
        .collect();
    assert_eq!(
        ids,
        [1, 2],
        "the write that landed and the next one never share an id"
    );
}

#[tokio::test]
async fn a_view_is_released_with_its_commit_and_never_after_a_failed_one() {
    let rig = rig_with(RigOptions {
        script: vec![ScriptedCommit::Fails(CommitError::Refused)],
        ..RigOptions::default()
    });
    let failed = rig.handle.submit(tap(1, "join")).await.expect("answered");
    rig.clock.advance_ms(5_000);
    let saved = rig.handle.submit(tap(1, "join")).await.expect("answered");

    assert!(
        failed.turns[0].view.is_none(),
        "nothing to show of a rolled-back turn"
    );
    assert!(
        saved.turns[0].view.is_some(),
        "the saved turn carries its sender's view"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn inputs_that_pile_up_during_a_commit_are_served_together_so_n_submits_make_fewer_commits() {
    let rig = super::tests_support::blocking_rig(&RigOptions::default());
    let first = tokio::spawn({
        let handle = Arc::clone(&rig.handle);
        async move { handle.submit(tap(1, "join")).await }
    });
    tokio::task::yield_now().await;
    rig.entered
        .recv()
        .expect("the owner began its first commit");
    let mut others = Vec::new();
    for seat in 2..=6 {
        others.push(tokio::spawn({
            let handle = Arc::clone(&rig.handle);
            async move { handle.submit(tap(seat, "join")).await }
        }));
    }
    tokio::task::yield_now().await;

    rig.release.send(()).expect("the store is waiting");
    rig.entered
        .recv()
        .expect("the owner began its second commit");
    rig.release.send(()).expect("the store is waiting");

    assert!(first.await.expect("joined").is_ok());
    for other in others {
        let reply = other.await.expect("joined").expect("answered");
        assert_eq!(reply.turns.len(), 1, "{:?}", reply.submission);
        assert!(matches!(
            reply.turns[0].commit,
            CommitDisposition::Committed { id: 2 }
        ));
    }
    assert_eq!(
        rig.store.attempts(),
        [(1, 1), (2, 5)],
        "six submits, two commits: the five that waited were one batch"
    );
    assert_eq!(
        rig.handle.dump().await.expect("answered").table.seats.len(),
        6
    );
}

#[tokio::test(flavor = "current_thread")]
async fn the_owner_gives_up_on_a_write_that_outlasts_its_deadline_and_takes_no_second_one_meanwhile()
 {
    let rig = super::tests_support::blocking_rig(&RigOptions {
        commit_deadline_ms: 50,
        commit_backoff_ms: 1_000,
        ..RigOptions::default()
    });

    let first = rig.handle.submit(tap(1, "join")).await.expect("answered");

    assert_eq!(
        first.turns[0].commit,
        CommitDisposition::Failed {
            cause: "deadline_exceeded".to_owned()
        },
        "the owner enforces the deadline itself; the store never returned"
    );
    assert_eq!(first.turns[0].decline, Some(DeclineCode::TableBusy));
    rig.clock.advance_ms(1_000);
    let second = rig.handle.submit(tap(1, "join")).await.expect("answered");
    assert_eq!(
        second.turns[0].commit,
        CommitDisposition::Failed {
            cause: "unavailable".to_owned()
        },
        "the first write is still stuck, so no second one is started"
    );
    assert_eq!(rig.store.attempts().len(), 1);

    // One permit lets the stuck write return; the rest let every later write return at once.
    for _ in 0..64 {
        rig.release.send(()).expect("the store is waiting");
    }
    let mut committed = None;
    for _ in 0..200 {
        rig.clock.advance_ms(1_000);
        let reply = rig.handle.submit(tap(1, "join")).await.expect("answered");
        if let CommitDisposition::Committed { id } = reply.turns[0].commit {
            committed = Some(id);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let id = committed.expect("the owner recovers once the stuck write returns");
    assert!(id >= 3, "the ids of the refused attempts are burned: {id}");
}

#[tokio::test]
async fn a_batch_that_changed_nothing_worth_saving_writes_no_checkpoint() {
    let rig = rig_with(held());
    for _ in 0..3 {
        rig.handle.submit(tap(1, "ready")).await.expect("answered");
    }

    let turns = rig
        .handle
        .queue_control(QueueControl::Release)
        .await
        .expect("answered")
        .turns;

    assert_eq!(turns.len(), 3);
    assert!(
        turns
            .iter()
            .all(|turn| turn.commit == CommitDisposition::NothingToCommit)
    );
    assert_eq!(rig.store.attempts(), 0);
    assert_eq!(count_lines(&rig.log, "COMMIT none inputs=3"), 1);
}

#[tokio::test]
async fn the_time_slice_cuts_a_burst_into_several_batches_and_every_input_is_still_saved() {
    let log = Arc::new(EventLog::default());
    let store = Arc::new(super::commit::ScriptedCommitStore::default());
    let options = RigOptions {
        held: true,
        policy: CommitPolicy::PerBatch {
            time_slice_ms: 50,
            maximum_inputs: std::num::NonZeroUsize::MAX,
        },
        ..RigOptions::default()
    };
    let handle = EngineHandle::start(
        config(&options),
        adapters(Arc::new(TickingClock::stepping(10)), &log),
        store.clone(),
        Arc::clone(&log),
    )
    .expect("starts");
    for seat in 1..=6 {
        handle.submit(tap(seat, "join")).await.expect("answered");
    }

    let served = handle
        .queue_control(QueueControl::Release)
        .await
        .expect("answered");

    assert_eq!(served.turns.len(), 6);
    let records = store.records();
    assert!(records.len() >= 2, "{} commits", records.len());
    assert_eq!(records.iter().map(|record| record.inputs).sum::<usize>(), 6);
    assert_eq!(handle.dump().await.expect("answered").table.seats.len(), 6);
    assert!(
        lines(&log)
            .iter()
            .any(|line| line.starts_with("COMMIT applied commit=2"))
    );
}

#[tokio::test]
async fn step_serves_one_batch_of_at_most_the_count_and_a_zero_step_is_refused() {
    let rig = rig_with(held());
    for seat in 1..=4 {
        rig.handle
            .submit(tap(seat, "join"))
            .await
            .expect("answered");
    }

    let first = rig
        .handle
        .queue_control(QueueControl::Step { count: 3 })
        .await
        .expect("answered");
    let rest = rig
        .handle
        .queue_control(QueueControl::Step { count: 3 })
        .await
        .expect("answered");

    assert_eq!((first.turns.len(), rest.turns.len()), (3, 1));
    assert!(matches!(
        rig.handle
            .queue_control(QueueControl::Step { count: 0 })
            .await,
        Err(super::report::EngineError::BadRequest { .. })
    ));
    assert_eq!(rig.store.records().len(), 2);
}
