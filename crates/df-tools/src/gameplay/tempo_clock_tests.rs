use super::*;
use crate::gameplay::model;
use df_model::checkpoint::{
    AcceptedDecision, CHECKPOINT_SCHEMA, Checkpoint, ContentDigest, GameState, ReferenceInventory,
};
use df_tempo::elapsed::ElapsedDisposition;
use df_types::{OperationId, RecoveryEpoch, RunId, SessionId, SessionRevision};

fn policy(current: &Checkpoint, maximum: u64) -> TempoElapsedPolicy {
    TempoElapsedPolicy {
        definition: current.state().tempo.policy.clone(),
        ticks_per_second: NANOSECOND_TICKS_PER_SECOND,
        maximum_elapsed_ticks: maximum,
    }
}
fn limits() -> TempoElapsedLimits {
    TempoElapsedLimits {
        maximum_checkpoint_bytes: 1024 * 1024,
        maximum_fatigue_entries: 16,
    }
}
fn after(origin: Instant, elapsed: Duration) -> Instant {
    origin.checked_add(elapsed).unwrap()
}
fn snapshot(current: &Checkpoint, basis: Basis, state: GameState) -> Checkpoint {
    let rule = model::rule().unwrap();
    let contents = model::contents().unwrap();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        current.pins().clone(),
        state,
        ReferenceInventory {
            rules: std::slice::from_ref(&rule),
            content: &contents,
            resources: &[],
            assets: &[],
        },
        model::limits(),
    )
    .unwrap()
}

/// Controlled authoritative-checkpoint fixture, not a PostgreSQL acknowledgement.
fn acknowledged(current: &Checkpoint, anchor: u64) -> Checkpoint {
    let mut basis = current.basis();
    basis.revision = basis.revision.next_sequence().unwrap();
    let mut state = current.state().clone();
    state.tempo.presentation_ticks = anchor;
    state.decisions.push(AcceptedDecision {
        operation: OperationId::from_bytes(&[0xc1; 16]).unwrap(),
        revision: basis.revision,
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: model::label("clock-checkpoint-fixture").unwrap(),
        semantic_output: None,
    });
    snapshot(current, basis, state)
}

#[test]
fn supplied_native_instants_produce_exact_request_for_actual_pure_elapsed_boundary() {
    let current = model::initial().unwrap();
    let before = current.clone();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let observed = after(origin, Duration::from_nanos(1234));
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    let first = clock
        .sample(&current, current.pins(), observed, false)
        .unwrap();
    let repeated = clock
        .sample(&current, current.pins(), observed, false)
        .unwrap();
    assert_eq!(first, repeated);
    assert_eq!(first.expected_basis, current.basis());
    assert_eq!(first.observed_logical_time, current.state().logical_time);
    assert_eq!(first.from_presentation_ticks, 0);
    assert_eq!(first.elapsed, Duration::from_nanos(1234));
    let result =
        advance_elapsed(&current, current.pins(), first, Some(&admitted), limits()).unwrap();
    assert_eq!(result.elapsed_ticks, 1234);
    assert_eq!(result.state.presentation_ticks, 1234);
    assert_eq!(result.disposition, ElapsedDisposition::Advanced);
    assert_eq!(current, before);
}

#[test]
fn unacknowledged_proposal_does_not_consume_elapsed_but_later_current_checkpoint_does() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    let first = clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(100)),
            false,
        )
        .unwrap();
    assert_eq!(first.elapsed, Duration::from_nanos(100));
    let second = clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(200)),
            false,
        )
        .unwrap();
    assert_eq!(second.elapsed, Duration::from_nanos(200));
    let committed = acknowledged(&current, 200);
    let third = clock
        .sample(
            &committed,
            committed.pins(),
            after(origin, Duration::from_nanos(250)),
            false,
        )
        .unwrap();
    assert_eq!(third.expected_basis, committed.basis());
    assert_eq!(third.from_presentation_ticks, 200);
    assert_eq!(third.elapsed, Duration::from_nanos(50));
    let reduced = advance_elapsed(
        &committed,
        committed.pins(),
        third,
        Some(&admitted),
        limits(),
    )
    .unwrap();
    assert_eq!(reduced.state.presentation_ticks, 250);
    assert_eq!(reduced.state.fatigue, committed.state().tempo.fatigue);
}

#[test]
fn restore_origin_does_not_infer_process_downtime_or_advance_game_time() {
    let current = acknowledged(&model::initial().unwrap(), 9_000_000_000);
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    let supplied = clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(25)),
            false,
        )
        .unwrap();
    assert_eq!(supplied.from_presentation_ticks, 9_000_000_000);
    assert_eq!(supplied.elapsed, Duration::from_nanos(25));
    let result = advance_elapsed(
        &current,
        current.pins(),
        supplied,
        Some(&admitted),
        limits(),
    )
    .unwrap();
    assert_eq!(result.state.presentation_ticks, 9_000_000_025);
    assert_eq!(current.state().logical_time.ticks, 0);
    assert_eq!(current.state().timers, Vec::new());
}

#[test]
fn pause_and_resume_boundaries_never_accrue_paused_or_unaccepted_catchup_intervals() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(100)),
            false,
        )
        .unwrap();
    let paused = clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_secs(1)),
            true,
        )
        .unwrap();
    assert!(paused.paused);
    assert_eq!(paused.elapsed, Duration::ZERO);
    let result =
        advance_elapsed(&current, current.pins(), paused, Some(&admitted), limits()).unwrap();
    assert_eq!(result.disposition, ElapsedDisposition::Paused);
    assert_eq!(result.state, current.state().tempo);
    let still_paused = clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_secs(100)),
            true,
        )
        .unwrap();
    assert_eq!(still_paused.elapsed, Duration::ZERO);
    let resumed = clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_secs(200)),
            false,
        )
        .unwrap();
    assert!(!resumed.paused);
    assert_eq!(resumed.elapsed, Duration::ZERO);
    let active = clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_millis(200_250)),
            false,
        )
        .unwrap();
    assert_eq!(active.elapsed, Duration::from_millis(250));
}

#[test]
fn initially_paused_clock_waits_for_a_fresh_active_interval() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock =
        NativePresentationClock::start(&current, current.pins(), &admitted, limits(), origin, true)
            .unwrap();
    assert_eq!(
        clock
            .sample(
                &current,
                current.pins(),
                after(origin, Duration::from_secs(500)),
                true
            )
            .unwrap()
            .elapsed,
        Duration::ZERO
    );
    assert_eq!(
        clock
            .sample(
                &current,
                current.pins(),
                after(origin, Duration::from_secs(1000)),
                false
            )
            .unwrap()
            .elapsed,
        Duration::ZERO
    );
    assert_eq!(
        clock
            .sample(
                &current,
                current.pins(),
                after(origin, Duration::from_millis(1_000_001)),
                false
            )
            .unwrap()
            .elapsed,
        Duration::from_millis(1)
    );
}

#[test]
fn replay_exit_discards_replay_interval_and_preserves_recorded_anchor() {
    let initial = model::initial().unwrap();
    let mut replay_state = initial.state().clone();
    replay_state.mode = ExecutionMode::Replay;
    let replay = snapshot(&initial, initial.basis(), replay_state);
    let admitted = policy(&replay, 1_000_000_000);
    let origin = Instant::now();
    let mut clock =
        NativePresentationClock::start(&replay, replay.pins(), &admitted, limits(), origin, false)
            .unwrap();
    let recorded = clock
        .sample(
            &replay,
            replay.pins(),
            after(origin, Duration::from_secs(100)),
            false,
        )
        .unwrap();
    assert_eq!(recorded.elapsed, Duration::ZERO);
    let reduced =
        advance_elapsed(&replay, replay.pins(), recorded, Some(&admitted), limits()).unwrap();
    assert_eq!(reduced.state, replay.state().tempo);
    assert_eq!(reduced.disposition, ElapsedDisposition::Replay);
    let active = acknowledged(&initial, 0);
    let resumed = clock
        .sample(
            &active,
            active.pins(),
            after(origin, Duration::from_secs(200)),
            false,
        )
        .unwrap();
    assert_eq!(resumed.elapsed, Duration::ZERO);
    let next = clock
        .sample(
            &active,
            active.pins(),
            after(origin, Duration::from_secs(201)),
            false,
        )
        .unwrap();
    assert_eq!(next.elapsed, Duration::from_secs(1));
}

#[test]
fn backwards_instant_refuses_without_consuming_a_later_valid_interval() {
    // The Windows performance counter cannot order single nanoseconds: an
    // earlier sub-tick instant reads back as zero elapsed instead of refusing.
    // The same refusal property is exercised a million times larger there.
    #[cfg(windows)]
    let unit = Duration::from_nanos(1_000_000);
    #[cfg(unix)]
    let unit = Duration::from_nanos(1);
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    clock
        .sample(&current, current.pins(), after(origin, unit * 100), false)
        .unwrap();
    assert_eq!(
        clock.sample(&current, current.pins(), after(origin, unit * 99), false),
        Err(NativeClockError::BackwardsInstant)
    );
    assert_eq!(
        clock
            .sample(&current, current.pins(), after(origin, unit * 150), false)
            .unwrap()
            .elapsed,
        unit * 150
    );
    assert_eq!(
        clock.rebind(&current, current.pins(), after(origin, unit * 149), false),
        Err(NativeClockError::BackwardsInstant)
    );
}

#[test]
fn wrong_session_run_epoch_and_stale_revision_cannot_rebind_the_owner() {
    let initial = model::initial().unwrap();
    let current = acknowledged(&initial, 0);
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    let mut wrong_session = current.basis();
    wrong_session.session = SessionId::from_bytes(&[0xd1; 16]).unwrap();
    let mut wrong_run = current.basis();
    wrong_run.run = RunId::from_bytes(&[0xd2; 16]).unwrap();
    let mut wrong_epoch = current.basis();
    wrong_epoch.revision = SessionRevision::new(
        RecoveryEpoch::new(2).unwrap(),
        current.basis().revision.sequence(),
    );
    for (basis, expected) in [
        (wrong_session, NativeClockError::WrongSession),
        (wrong_run, NativeClockError::WrongRun),
        (wrong_epoch, NativeClockError::WrongEpoch),
    ] {
        let other = snapshot(&current, basis, current.state().clone());
        assert_eq!(
            clock.sample(&other, other.pins(), origin, false),
            Err(expected)
        );
        assert_eq!(
            clock.rebind(&other, other.pins(), origin, false),
            Err(expected)
        );
    }
    assert_eq!(
        clock.sample(&initial, initial.pins(), origin, false),
        Err(NativeClockError::StaleCheckpoint)
    );
    assert_eq!(
        clock.rebind(&initial, initial.pins(), origin, false),
        Err(NativeClockError::StaleCheckpoint)
    );
}

#[test]
fn full_pinned_source_change_refuses_without_modifying_native_clock() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    let mut stale = current.pins().clone();
    stale.content.package_digest = ContentDigest([0xff; 32]);
    assert_eq!(
        clock.sample(
            &current,
            &stale,
            after(origin, Duration::from_nanos(100)),
            false
        ),
        Err(NativeClockError::SourceChanged)
    );
    assert_eq!(
        clock.rebind(&current, &stale, origin, false),
        Err(NativeClockError::SourceChanged)
    );
    assert_eq!(
        clock
            .sample(
                &current,
                current.pins(),
                after(origin, Duration::from_nanos(125)),
                false
            )
            .unwrap()
            .elapsed,
        Duration::from_nanos(125)
    );
}

#[test]
fn unproposed_or_same_revision_anchor_change_refuses() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(100)),
            false,
        )
        .unwrap();
    let unproposed = acknowledged(&current, 101);
    assert_eq!(
        clock.sample(&unproposed, unproposed.pins(), origin, false),
        Err(NativeClockError::AnchorConflict)
    );
    let mut changed = current.state().clone();
    changed.tempo.presentation_ticks = 100;
    let same_revision = snapshot(&current, current.basis(), changed);
    assert_eq!(
        clock.sample(&same_revision, same_revision.pins(), origin, false),
        Err(NativeClockError::AnchorConflict)
    );
}

#[test]
fn anchor_overflow_refuses_without_any_partial_request_or_checkpoint_change() {
    let current = acknowledged(&model::initial().unwrap(), u64::MAX);
    let before = current.clone();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    assert_eq!(
        clock.sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(1)),
            false
        ),
        Err(NativeClockError::AnchorOverflow)
    );
    assert_eq!(
        clock
            .sample(&current, current.pins(), origin, false)
            .unwrap()
            .elapsed,
        Duration::ZERO
    );
    assert_eq!(current, before);
}

#[test]
fn catchup_limit_refuses_and_explicit_rebind_discards_only_unaccepted_cosmetic_time() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 100);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    assert_eq!(
        clock.sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(101)),
            false
        ),
        Err(NativeClockError::Elapsed(TempoAdvanceError::ElapsedLimit))
    );
    // Failure did not advance the observed timestamp or consume the interval.
    assert_eq!(
        clock
            .sample(
                &current,
                current.pins(),
                after(origin, Duration::from_nanos(100)),
                false
            )
            .unwrap()
            .elapsed,
        Duration::from_nanos(100)
    );
    clock
        .rebind(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(200)),
            false,
        )
        .unwrap();
    assert_eq!(
        clock
            .sample(
                &current,
                current.pins(),
                after(origin, Duration::from_nanos(225)),
                false
            )
            .unwrap()
            .elapsed,
        Duration::from_nanos(25)
    );
    assert_eq!(current.state().tempo.presentation_ticks, 0);
    assert_eq!(current.state().logical_time.ticks, 0);
}

#[test]
fn unsupported_units_zero_catchup_and_missing_capacity_do_not_select_defaults() {
    let current = model::initial().unwrap();
    let origin = Instant::now();
    let mut wrong = policy(&current, 100);
    wrong.ticks_per_second = 10;
    assert!(matches!(
        NativePresentationClock::start(&current, current.pins(), &wrong, limits(), origin, false),
        Err(NativeClockError::UnsupportedTimebase)
    ));
    let zero = policy(&current, 0);
    assert!(matches!(
        NativePresentationClock::start(&current, current.pins(), &zero, limits(), origin, false),
        Err(NativeClockError::Elapsed(TempoAdvanceError::InvalidPolicy))
    ));
    let mut bounded = limits();
    bounded.maximum_checkpoint_bytes = 0;
    assert!(matches!(
        NativePresentationClock::start(
            &current,
            current.pins(),
            &policy(&current, 100),
            bounded,
            origin,
            false
        ),
        Err(NativeClockError::Elapsed(TempoAdvanceError::InvalidLimits))
    ));
    bounded.maximum_checkpoint_bytes = 1;
    assert!(matches!(
        NativePresentationClock::start(
            &current,
            current.pins(),
            &policy(&current, 100),
            bounded,
            origin,
            false
        ),
        Err(NativeClockError::Elapsed(TempoAdvanceError::Capacity))
    ));
}

#[test]
fn disposal_is_terminal_and_idempotent_for_sampling_and_rebinding() {
    let current = model::initial().unwrap();
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &policy(&current, 100),
        limits(),
        origin,
        false,
    )
    .unwrap();
    clock.dispose();
    clock.dispose();
    assert_eq!(
        clock.sample(&current, current.pins(), origin, false),
        Err(NativeClockError::Disposed)
    );
    assert_eq!(
        clock.rebind(&current, current.pins(), origin, false),
        Err(NativeClockError::Disposed)
    );
}

#[test]
fn current_game_time_observation_never_supplies_presentation_elapsed() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(100)),
            false,
        )
        .unwrap();
    let accepted = acknowledged(&current, 100);
    let mut state = accepted.state().clone();
    state.logical_time.ticks = 3600;
    let rested = snapshot(&accepted, accepted.basis(), state);
    let supplied = clock
        .sample(
            &rested,
            rested.pins(),
            after(origin, Duration::from_nanos(150)),
            false,
        )
        .unwrap();
    assert_eq!(supplied.observed_logical_time.ticks, 3600);
    assert_eq!(supplied.elapsed, Duration::from_nanos(50));
    let staged =
        advance_elapsed(&rested, rested.pins(), supplied, Some(&admitted), limits()).unwrap();
    assert_eq!(staged.state.presentation_ticks, 150);
    assert_eq!(rested.state().logical_time.ticks, 3600);
}

#[test]
fn changed_tempo_policy_refuses_without_consuming_a_sample() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    let mut state = current.state().clone();
    state.tempo.policy = model::content("inspect-seal").unwrap();
    let changed = snapshot(&current, current.basis(), state);
    assert_eq!(
        clock.sample(
            &changed,
            changed.pins(),
            after(origin, Duration::from_nanos(100)),
            false
        ),
        Err(NativeClockError::Elapsed(TempoAdvanceError::PolicyMismatch))
    );
    assert_eq!(
        clock
            .sample(
                &current,
                current.pins(),
                after(origin, Duration::from_nanos(125)),
                false
            )
            .unwrap()
            .elapsed,
        Duration::from_nanos(125)
    );
}

#[test]
fn rebind_to_acknowledged_anchor_starts_a_new_interval_without_changing_durable_state() {
    let current = model::initial().unwrap();
    let admitted = policy(&current, 1_000_000_000);
    let origin = Instant::now();
    let mut clock = NativePresentationClock::start(
        &current,
        current.pins(),
        &admitted,
        limits(),
        origin,
        false,
    )
    .unwrap();
    clock
        .sample(
            &current,
            current.pins(),
            after(origin, Duration::from_nanos(100)),
            false,
        )
        .unwrap();
    let accepted = acknowledged(&current, 100);
    let before = accepted.clone();
    clock
        .rebind(
            &accepted,
            accepted.pins(),
            after(origin, Duration::from_nanos(500)),
            false,
        )
        .unwrap();
    let supplied = clock
        .sample(
            &accepted,
            accepted.pins(),
            after(origin, Duration::from_nanos(525)),
            false,
        )
        .unwrap();
    assert_eq!(supplied.from_presentation_ticks, 100);
    assert_eq!(supplied.elapsed, Duration::from_nanos(25));
    assert_eq!(accepted, before);
}
