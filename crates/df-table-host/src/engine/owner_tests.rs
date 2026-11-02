use df_table::outcome::{DeclineCode, OutcomeKind};
use df_table::owner_queue::Lane;
use df_table::private_data::LengthClass;

use super::report::{CommitDisposition, EngineError};
use super::request::{AudienceSpec, ForceRequest, InputBody, QueueControl};
use super::tests_support::{
    FailingInterpreter, RigOptions, count_lines, lines, proposal, request, rig, rig_customised,
    rig_with, tap, text, timer, with_operation,
};

#[tokio::test]
async fn an_input_goes_through_the_queue_runs_the_lobby_and_the_projection_shows_the_result() {
    let rig = rig();

    let reply = rig.handle.submit(tap(1, "join")).await.expect("answered");

    assert_eq!(reply.submission.lane, Some(Lane::Player));
    assert!(reply.submission.refusal.is_none());
    let [turn] = reply.turns.as_slice() else {
        panic!("one turn ran: {:?}", reply.turns);
    };
    assert_eq!(turn.outcome, OutcomeKind::Applied);
    assert_eq!(turn.action.as_deref(), Some("join"));
    assert_eq!(
        (turn.machine_after.as_str(), turn.state_after.as_str()),
        ("lobby", "gathering")
    );
    assert_eq!(turn.effects, 1);
    assert_eq!(turn.commit, CommitDisposition::Committed { id: 1 });
    assert_eq!(turn.narration, Some(LengthClass::Short));

    let dump = rig.handle.dump().await.expect("answered");
    assert_eq!((dump.revision, dump.commit), (1, 1));
    assert_eq!(dump.table.seats, [1]);

    let tv = rig
        .handle
        .projection(AudienceSpec::Tv)
        .await
        .expect("answered");
    assert_eq!(tv["viewer"]["kind"], "shared");
    assert_eq!(tv["readout"], "Seat 1: join done.");
    let seat = rig
        .handle
        .projection(AudienceSpec::Seat { seat: 1 })
        .await
        .expect("answered");
    assert_eq!(seat["viewer"]["seat"], 1);
    assert_eq!(seat["view"]["hero"], "Seat 1");
    assert_eq!(turn.readout, "Seat 1: join done.");
    let released = turn.view.as_ref().expect("the sender's view is released");
    assert_eq!(released["viewer"]["seat"], 1);
    for needle in [
        "INPUT received",
        "lane=player",
        "EFFECT applied",
        "effect=seat_joined",
        "TURN applied",
        "COMMIT applied commit=1",
        "ENGINE input sender=seat:1 kind=tap",
    ] {
        assert!(
            count_lines(&rig.log, needle) >= 1,
            "no log line has '{needle}'"
        );
    }
}

#[tokio::test]
async fn a_finished_lobby_hands_the_table_to_explore_and_explore_hands_it_back_in_the_same_commit()
{
    let rig = rig();
    rig.handle.submit(tap(1, "join")).await.expect("answered");
    rig.handle.submit(tap(2, "join")).await.expect("answered");
    rig.handle.submit(tap(1, "ready")).await.expect("answered");
    rig.handle.submit(tap(2, "ready")).await.expect("answered");

    let started = rig.handle.submit(tap(1, "start")).await.expect("answered");

    let [start, hand_off] = started.turns.as_slice() else {
        panic!("start and its hand-off ran: {:?}", started.turns);
    };
    assert_eq!(
        (start.state_before.as_str(), start.state_after.as_str()),
        ("gathering", "started")
    );
    assert_eq!(
        (hand_off.sender.as_str(), hand_off.lane),
        ("engine", Lane::Engine)
    );
    assert_eq!(hand_off.kind, "engine_proposal");
    assert_eq!(
        (
            hand_off.machine_before.as_str(),
            hand_off.machine_after.as_str(),
            hand_off.state_after.as_str()
        ),
        ("lobby", "explore", "presenting")
    );
    assert_eq!(
        start.commit, hand_off.commit,
        "one commit covers the input and its hand-off"
    );
    let dump = rig.handle.dump().await.expect("answered");
    assert_eq!(
        (dump.machine.as_str(), dump.state.as_str()),
        ("explore", "presenting")
    );
    assert_eq!(dump.table.active_scene, Some(1));
    let moved = dump.table.last_hand_off.expect("a hand-off was recorded");
    assert_eq!(
        (moved.from.as_str(), moved.to.as_str(), moved.carried),
        ("lobby", "explore", 3)
    );
    let viable = rig.handle.viable(1).await.expect("answered");
    assert_eq!(viable.actions, ["pick_lock"]);

    let checked = rig
        .handle
        .submit(tap(1, "pick_lock"))
        .await
        .expect("answered");

    let [check, back] = checked.turns.as_slice() else {
        panic!("the check and its hand-off ran: {:?}", checked.turns);
    };
    assert_eq!(check.outcome, OutcomeKind::Applied);
    assert_eq!(
        (check.state_before.as_str(), check.state_after.as_str()),
        ("presenting", "resolved")
    );
    assert_eq!(back.outcome, OutcomeKind::Applied);
    assert_eq!(check.commit, back.commit);
    let dump = rig.handle.dump().await.expect("answered");
    assert_eq!(dump.table.active_beat.as_deref(), Some("locked_gate"));
    let returned = dump.table.last_hand_off.expect("a hand-off was recorded");
    assert_eq!(
        (returned.from.as_str(), returned.to.as_str()),
        ("explore", "lobby")
    );
    assert!(count_lines(&rig.log, "MACHINE exited") >= 2);
    assert!(count_lines(&rig.log, "STEP finished") >= 1);
}

#[tokio::test]
async fn a_typed_line_that_is_one_of_the_rules_choices_is_interpreted_and_any_other_needs_clarifying()
 {
    let rig = rig();
    for seat in [1, 2] {
        rig.handle
            .submit(tap(seat, "join"))
            .await
            .expect("answered");
    }
    for seat in [1, 2] {
        rig.handle
            .submit(tap(seat, "ready"))
            .await
            .expect("answered");
    }
    rig.handle.submit(tap(1, "start")).await.expect("answered");

    let unclear = rig
        .handle
        .submit(text(1, "please open the gate for me"))
        .await
        .expect("answered");
    let repeated = rig
        .handle
        .submit(text(1, "please open the gate for me"))
        .await
        .expect("answered");
    let clear = rig
        .handle
        .submit(text(1, "pick_lock"))
        .await
        .expect("answered");

    assert_eq!(unclear.turns[0].outcome, OutcomeKind::NeedsClarification);
    assert_eq!(unclear.turns[0].readout, "I could not read that. Pick one:");
    assert!(
        matches!(unclear.turns[0].commit, CommitDisposition::Committed { .. }),
        "the seat's open question is table state, so it is saved"
    );
    assert_eq!(
        repeated.turns[0].commit,
        CommitDisposition::NothingToCommit,
        "the same question again changes nothing"
    );
    assert_eq!(clear.turns[0].outcome, OutcomeKind::Applied);
    assert_eq!(clear.turns[0].action.as_deref(), Some("pick_lock"));
}

#[tokio::test]
async fn an_interpreter_that_is_down_is_recorded_once_and_the_seat_is_asked_to_pick() {
    let rig = rig_customised(RigOptions::default(), |adapters| {
        adapters.interpreter = std::sync::Arc::new(FailingInterpreter);
    });
    rig.handle.submit(tap(1, "join")).await.expect("answered");

    let reply = rig.handle.submit(text(1, "ready")).await.expect("answered");

    assert_eq!(reply.turns[0].outcome, OutcomeKind::NeedsClarification);
    assert_eq!(count_lines(&rig.log, "BOUNDARY failed"), 1);
    assert_eq!(
        count_lines(&rig.log, "port=interpreter cause=unavailable"),
        1
    );
}

#[tokio::test]
async fn a_duplicate_operation_runs_once_and_learns_where_the_first_stands() {
    let rig = rig();
    let first = rig
        .handle
        .submit(with_operation(tap(1, "join"), 42))
        .await
        .expect("answered");
    let second = rig
        .handle
        .submit(with_operation(tap(1, "join"), 42))
        .await
        .expect("answered");

    let refusal = second.submission.refusal.expect("the duplicate is refused");
    assert_eq!(refusal.cause, "duplicate");
    let first_status = refusal
        .first_status
        .expect("it says where the first stands");
    assert!(first_status.contains("finished") && first_status.contains("outcome=applied"));
    assert!(second.turns.is_empty());
    assert_eq!(first.turns.len(), 1);
    assert_eq!(rig.handle.dump().await.expect("answered").table.seats, [1]);
    let queue = rig.handle.queue_show().await.expect("answered");
    assert_eq!((queue.submissions.len(), queue.turns.len()), (2, 1));
    assert_eq!(count_lines(&rig.log, "cause=duplicate"), 1);
}

#[tokio::test]
async fn a_seat_that_sends_faster_than_its_bucket_refills_is_rate_limited_with_a_retry_time() {
    let rig = rig_with(RigOptions {
        default_seat_limits: true,
        ..RigOptions::default()
    });
    for _ in 0..3 {
        let reply = rig.handle.submit(text(1, "x")).await.expect("answered");
        assert!(reply.submission.refusal.is_none());
    }

    let fourth = rig.handle.submit(text(1, "x")).await.expect("answered");

    let refusal = fourth.submission.refusal.expect("refused");
    assert_eq!(refusal.cause, "rate_limited");
    assert!(refusal.retry_after_ms.is_some_and(|retry| retry > 0));
    assert_eq!(count_lines(&rig.log, "code=rate_limited"), 1);
}

#[tokio::test]
async fn a_seat_with_three_inputs_waiting_is_told_the_table_is_busy_and_others_are_not() {
    let rig = rig_with(RigOptions {
        held: true,
        default_seat_limits: true,
        ..RigOptions::default()
    });
    for _ in 0..3 {
        let reply = rig.handle.submit(tap(1, "join")).await.expect("answered");
        assert!(reply.submission.refusal.is_none());
    }

    let fourth = rig.handle.submit(tap(1, "join")).await.expect("answered");
    let other = rig.handle.submit(tap(2, "join")).await.expect("answered");

    assert_eq!(
        fourth.submission.refusal.expect("refused").cause,
        "table_busy"
    );
    assert!(other.submission.refusal.is_none());
    let queue = rig.handle.queue_show().await.expect("answered");
    let pending: Vec<(u64, usize)> = queue
        .seats
        .iter()
        .filter(|seat| seat.pending > 0)
        .map(|seat| (seat.seat, seat.pending))
        .collect();
    assert_eq!(pending, [(1, 3), (2, 1)]);
}

#[tokio::test]
async fn a_seat_that_is_not_at_the_session_is_refused_and_the_refusal_is_listed() {
    let rig = rig();

    let reply = rig.handle.submit(tap(99, "join")).await.expect("answered");

    assert_eq!(
        reply.submission.refusal.expect("refused").cause,
        "unknown_seat"
    );
    assert_eq!(
        rig.handle.viable(99).await,
        Err(EngineError::UnknownSeat { seat: 99 })
    );
}

#[tokio::test]
async fn lanes_are_served_controls_then_engine_then_timers_then_player_and_queue_show_says_so() {
    let rig = rig_with(RigOptions {
        held: true,
        ..RigOptions::default()
    });
    rig.handle.submit(tap(2, "join")).await.expect("answered");
    rig.handle
        .submit(timer("idle_nudge"))
        .await
        .expect("answered");
    rig.handle
        .submit(proposal("npc", "guard"))
        .await
        .expect("answered");
    rig.handle
        .submit(request(InputBody::Pause { seat: 3 }))
        .await
        .expect("answered");

    let waiting = rig.handle.queue_show().await.expect("answered");
    let depths: Vec<(Lane, usize)> = waiting
        .lanes
        .iter()
        .map(|lane| (lane.lane, lane.depth))
        .collect();
    assert_eq!(
        depths,
        [
            (Lane::SeatControls, 1),
            (Lane::Engine, 1),
            (Lane::TimersAndResults, 1),
            (Lane::Player, 1)
        ]
    );
    assert_eq!(waiting.serving, "held");

    let served = rig
        .handle
        .queue_control(QueueControl::Step { count: 4 })
        .await
        .expect("answered");

    let order: Vec<(u64, Lane, OutcomeKind)> = served
        .turns
        .iter()
        .map(|turn| (turn.turn_seq, turn.lane, turn.outcome))
        .collect();
    assert_eq!(
        order,
        [
            (1, Lane::SeatControls, OutcomeKind::Applied),
            (2, Lane::Engine, OutcomeKind::Declined),
            (3, Lane::TimersAndResults, OutcomeKind::Declined),
            (4, Lane::Player, OutcomeKind::Declined)
        ]
    );
    assert_eq!(served.turns[1].decline, Some(DeclineCode::NotViable));
    assert_eq!(
        served.turns[3].decline,
        Some(DeclineCode::TimeLocked),
        "the pause card paused the table, so the tap behind it finds it paused"
    );
}

#[tokio::test]
async fn a_machine_that_is_not_addressed_declines_what_it_does_not_handle_with_its_own_reason() {
    let rig = rig();

    let reply = rig.handle.submit(tap(1, "ready")).await.expect("answered");

    let turn = &reply.turns[0];
    assert_eq!(
        (turn.outcome, turn.decline),
        (OutcomeKind::Declined, Some(DeclineCode::NotViable))
    );
    assert_eq!(turn.commit, CommitDisposition::NothingToCommit);
    assert_eq!(rig.store.attempts(), 0, "a declined input saves nothing");
    assert_eq!(count_lines(&rig.log, "INPUT declined"), 1);
    assert_eq!(count_lines(&rig.log, "code=not_viable"), 1);
}

#[tokio::test]
async fn force_puts_the_table_in_a_machine_state_through_the_machines_own_entry_and_saves_it() {
    let rig = rig();
    assert!(matches!(
        rig.handle
            .force(ForceRequest {
                machine: "explore".to_owned(),
                state: "presenting".to_owned(),
                payload: None
            })
            .await,
        Err(EngineError::ForceRefused { .. })
    ));
    assert!(matches!(
        rig.handle
            .force(ForceRequest {
                machine: "nobody".to_owned(),
                state: "x".to_owned(),
                payload: None
            })
            .await,
        Err(EngineError::UnknownMachine { .. })
    ));
    assert!(matches!(
        rig.handle
            .force(ForceRequest {
                machine: "lobby".to_owned(),
                state: "elsewhere".to_owned(),
                payload: None
            })
            .await,
        Err(EngineError::UnknownState { .. })
    ));
    assert!(matches!(
        rig.handle
            .force(ForceRequest {
                machine: "lobby".to_owned(),
                state: "started".to_owned(),
                payload: None
            })
            .await,
        Err(EngineError::BadPayload { .. })
    ));
    assert!(matches!(
        rig.handle
            .force(ForceRequest {
                machine: "lobby".to_owned(),
                state: "started".to_owned(),
                payload: Some(serde_json::json!({"phase": "nope"}))
            })
            .await,
        Err(EngineError::BadPayload { .. })
    ));
    assert_eq!(rig.handle.dump().await.expect("answered").commit, 0);

    let forced = rig
        .handle
        .force(ForceRequest {
            machine: "lobby".to_owned(),
            state: "started".to_owned(),
            payload: Some(serde_json::json!({"phase": "started", "scene": 1})),
        })
        .await
        .expect("forced");

    assert_eq!(forced.commit, CommitDisposition::Committed { id: 1 });
    let dump = rig.handle.dump().await.expect("answered");
    assert_eq!(
        (dump.machine.as_str(), dump.state.as_str(), dump.commit),
        ("lobby", "started", 1)
    );
    assert_eq!(rig.store.records()[0].inputs, 0);
    assert_eq!(
        count_lines(
            &rig.log,
            "ENGINE force machine=lobby state=started driven=true revision=1"
        ),
        1
    );
    assert_eq!(count_lines(&rig.log, "MACHINE entered"), 1);
    assert_eq!(count_lines(&rig.log, "ENGINE force_refused"), 5);
}

#[tokio::test]
async fn a_hand_off_the_table_refuses_is_recorded_and_the_machine_stays_where_it_is() {
    let rig = rig();
    rig.handle
        .force(ForceRequest {
            machine: "lobby".to_owned(),
            state: "started".to_owned(),
            payload: Some(serde_json::json!({"phase": "started", "scene": 1})),
        })
        .await
        .expect("forced");

    let reply = rig
        .handle
        .submit(proposal("machine", "lobby"))
        .await
        .expect("answered");

    assert_eq!(reply.turns[0].outcome, OutcomeKind::Declined);
    assert_eq!(count_lines(&rig.log, "MACHINE failed"), 1);
    assert_eq!(count_lines(&rig.log, "cause=entry_refused"), 1);
    assert_eq!(rig.handle.dump().await.expect("answered").machine, "lobby");
}

#[tokio::test]
async fn describe_is_generated_from_the_registry_and_dump_shows_the_saved_state() {
    let rig = rig();

    let description = rig.handle.describe().await.expect("answered");

    let names: Vec<&str> = description
        .machines
        .iter()
        .map(|m| m.machine.as_str())
        .collect();
    assert_eq!(names, ["explore", "lobby"]);
    let lobby = description
        .machines
        .iter()
        .find(|m| m.machine == "lobby")
        .expect("listed");
    assert!(lobby.active && lobby.enabled);
    assert_eq!(lobby.states, ["gathering", "started"]);
    assert_eq!(lobby.hand_off_targets, ["explore"]);
    assert!(lobby.actions.contains(&"start".to_owned()));
    assert_eq!(description.lanes[0], Lane::SeatControls);
    assert!(
        description
            .verbs
            .iter()
            .any(|verb| verb.starts_with("projection show"))
    );
    let dump = rig.handle.dump().await.expect("answered");
    assert_eq!(
        dump.slot,
        serde_json::json!({"phase": "gathering", "ready": []})
    );
    assert_eq!(dump.schema_version, 1);
}

#[tokio::test]
async fn a_request_to_the_broker_settles_off_the_owner_and_arrives_as_an_engine_input() {
    let rig = rig_with(RigOptions {
        held: true,
        ..RigOptions::default()
    });

    let requested = rig
        .handle
        .broker_request("plate".to_owned())
        .await
        .expect("answered");
    assert_eq!(
        (requested.handle, requested.readiness.as_str()),
        (1, "pending")
    );
    let queues = match rig.handle.broker_queues().await.expect("answered") {
        super::report::BrokerView::Queues(queues) => queues,
        other => panic!("expected the queues, got {other:?}"),
    };
    assert_eq!(queues.running.len(), 1);
    assert!(queues.running[0].sent);

    rig.clock.advance_ms(super::limits::DEV_IMAGE_LATENCY_MS);
    let waiting = rig.handle.queue_show().await.expect("answered");
    assert_eq!(
        waiting
            .lanes
            .iter()
            .find(|lane| lane.lane == Lane::TimersAndResults)
            .map(|lane| lane.depth),
        Some(1)
    );
    let served = rig
        .handle
        .queue_control(QueueControl::Step { count: 1 })
        .await
        .expect("answered");

    assert_eq!(served.turns[0].kind, "resource_result");
    assert_eq!(served.turns[0].lane, Lane::TimersAndResults);
    let ledger = match rig.handle.broker_ledger().await.expect("answered") {
        super::report::BrokerView::Ledger(ledger) => ledger,
        other => panic!("expected the ledger, got {other:?}"),
    };
    assert_eq!(ledger.totals[0].totals.calls_sent, 1);
    assert_eq!(ledger.entries[0].handle, 1);
    assert!(count_lines(&rig.log, "RESOURCE request purpose=plate") >= 1);
    assert!(count_lines(&rig.log, "SPEND commit purpose=plate") >= 1);
    assert!(matches!(
        rig.handle
            .broker_request("no_such_purpose".to_owned())
            .await,
        Err(EngineError::UnknownPurpose { .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn a_full_mailbox_refuses_at_once_and_the_owner_answers_what_it_took() {
    let (store, entered, release) = super::tests_support::BlockingStore::new();
    let clock = std::sync::Arc::new(df_table::testkit::ManualClock::default());
    let log = std::sync::Arc::new(crate::event_log::EventLog::default());
    let options = RigOptions::default();
    let handle = std::sync::Arc::new(
        super::handle::EngineHandle::start_with_mailbox(
            super::tests_support::config(&options),
            super::tests_support::adapters(clock, &log),
            store,
            std::sync::Arc::clone(&log),
            1,
        )
        .expect("starts"),
    );
    let first = tokio::spawn({
        let handle = std::sync::Arc::clone(&handle);
        async move { handle.submit(tap(1, "join")).await }
    });
    tokio::task::yield_now().await;
    entered.recv().expect("the owner began its commit");
    let second = tokio::spawn({
        let handle = std::sync::Arc::clone(&handle);
        async move { handle.submit(tap(2, "join")).await }
    });
    tokio::task::yield_now().await;

    let refused = handle.submit(tap(3, "join")).await;

    assert_eq!(refused, Err(EngineError::MailboxFull));
    release.send(()).expect("the store is waiting");
    release.send(()).expect("the store is waiting");
    assert!(first.await.expect("joined").is_ok());
    assert!(second.await.expect("joined").is_ok());
}

#[tokio::test]
async fn a_forced_name_with_a_newline_cannot_forge_a_log_line() {
    let rig = rig();
    let before = lines(&rig.log).len();

    for machine in [
        "lobby\nCOMMIT applied commit=77 inputs=1",
        "COMMIT applied commit=77\r\nENGINE x",
    ] {
        let refused = rig
            .handle
            .force(ForceRequest {
                machine: machine.to_owned(),
                state: "started".to_owned(),
                payload: None,
            })
            .await;
        assert!(refused.is_err(), "{machine:?} is not a machine");
    }

    let after = lines(&rig.log);
    assert_eq!(
        after.len() - before,
        2,
        "each refused force is one line and no more"
    );
    for line in &after[before..] {
        assert!(line.starts_with("ENGINE force_refused"), "{line}");
        assert!(!line.contains('\n') && !line.contains('\r'), "{line:?}");
        assert!(!line.contains("commit=77"), "{line}");
    }
    assert_eq!(count_lines(&rig.log, "COMMIT applied"), 0);
}

#[tokio::test]
async fn an_owner_thread_that_panics_says_so_once_and_every_call_answers_with_a_typed_error() {
    let rig = rig_customised(RigOptions::default(), |adapters| {
        adapters.interpreter = std::sync::Arc::new(super::tests_support::PanickingInterpreter);
    });
    rig.handle.submit(tap(1, "join")).await.expect("answered");

    let lost = rig.handle.submit(text(1, "anything")).await;

    assert_eq!(lost, Err(EngineError::OwnerPanicked));
    assert_eq!(rig.handle.dump().await, Err(EngineError::OwnerPanicked));
    assert_eq!(rig.handle.health().await, Err(EngineError::OwnerPanicked));
    assert_eq!(count_lines(&rig.log, "ENGINE stopped cause=panic"), 1);
    assert_eq!(
        count_lines(&rig.log, "the interpreter broke"),
        0,
        "the panic's own message is never recorded"
    );
}

#[tokio::test]
async fn dropping_the_handle_while_a_commit_is_stuck_does_not_block_the_runtime() {
    let rig = super::tests_support::blocking_rig(&RigOptions {
        commit_deadline_ms: 60_000,
        ..RigOptions::default()
    });
    let stuck = tokio::spawn({
        let handle = std::sync::Arc::clone(&rig.handle);
        async move { handle.submit(tap(1, "join")).await }
    });
    tokio::task::yield_now().await;
    rig.entered.recv().expect("the owner began its commit");

    let started = std::time::Instant::now();
    let super::tests_support::BlockingRig {
        handle, release, ..
    } = rig;
    stuck.abort();
    drop(handle);

    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the drop returned at once"
    );
    // Let the stuck write end so the owner thread and the committer finish.
    let _ = release.send(());
}

#[tokio::test]
async fn shutting_down_records_how_many_waiting_inputs_it_dropped() {
    let rig = rig_with(RigOptions {
        held: true,
        ..RigOptions::default()
    });
    for seat in 1..=3 {
        rig.handle
            .submit(tap(seat, "join"))
            .await
            .expect("answered");
    }
    let log = std::sync::Arc::clone(&rig.log);

    drop(rig.handle);

    for _ in 0..200 {
        if count_lines(&log, "ENGINE stopped") > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        count_lines(&log, "ENGINE stopped cause=closed dropped_inputs=3"),
        1
    );
}
