use std::time::Duration;

use df_experience::window::{
    AcceptedActivity, ActivityWindow, ActivityWindowLimits, Observation, WindowError,
};
use df_types::{OperationId, RecoveryEpoch, RunId, SessionId, SessionRevision};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Participant(u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Event(u16);

fn session(value: u8) -> SessionId {
    SessionId::from_bytes(&[value; 16]).unwrap()
}

fn run(value: u8) -> RunId {
    RunId::from_bytes(&[value; 16]).unwrap()
}

fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}

fn time(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

fn window(max_events: usize) -> ActivityWindow<Participant, Event> {
    ActivityWindow::new(
        session(1),
        run(2),
        ActivityWindowLimits {
            max_events,
            horizon: time(10),
        },
    )
    .unwrap()
}

fn activity(participant: u16, event: u16, at: u64) -> AcceptedActivity<Participant, Event> {
    AcceptedActivity {
        session: session(1),
        run: run(2),
        operation: OperationId::from_bytes(&[3; 16]).unwrap(),
        revision: revision(1, 7),
        participant: Participant(participant),
        event: Event(event),
        at: time(at),
    }
}

#[test]
fn accepted_events_dedupe_without_coalescing_distinct_events_in_one_operation() {
    let mut window = window(4);
    let first = activity(1, 10, 0);
    assert!(window.is_empty());
    assert_eq!(
        window.observe(first, revision(1, 9), time(5)),
        Ok(Observation::Inserted { removed_expired: 0 })
    );
    assert_eq!(
        window.observe(first, revision(1, 9), time(5)),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
    window
        .observe(activity(1, 11, 3), revision(1, 9), time(5))
        .unwrap();
    window
        .observe(activity(2, 12, 4), revision(1, 9), time(5))
        .unwrap();
    assert_eq!(window.activity_count(&Participant(1)), 2);
    assert_eq!(window.activity_count(&Participant(2)), 1);
    assert_eq!(window.activity_count(&Participant(99)), 0);
    assert_eq!(window.len(), 3);
}

#[test]
fn conflicting_retained_identity_refuses_without_changing_counts() {
    let mut window = window(4);
    let event = activity(1, 10, 3);
    window.observe(event, revision(1, 9), time(5)).unwrap();
    let mut conflicts = [event; 4];
    conflicts[0].participant = Participant(2);
    conflicts[1].operation = OperationId::from_bytes(&[4; 16]).unwrap();
    conflicts[2].revision = revision(1, 8);
    conflicts[3].at = time(4);
    for conflict in conflicts {
        assert_eq!(
            window.observe(conflict, revision(1, 9), time(6)),
            Err(WindowError::EventIdConflict)
        );
        assert_eq!(window.len(), 1);
        assert_eq!(window.activity_count(&Participant(1)), 1);
        assert_eq!(window.activity_count(&Participant(2)), 0);
    }
    // Failed observations did not advance the owned clock.
    assert_eq!(
        window.observe(event, revision(1, 9), time(5)),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
}

#[test]
fn out_of_order_committed_events_within_the_current_window_are_counted() {
    let mut window = window(4);
    window
        .observe(activity(1, 10, 8), revision(1, 9), time(9))
        .unwrap();
    let mut earlier = activity(1, 11, 2);
    earlier.revision = revision(1, 3);
    window.observe(earlier, revision(1, 9), time(9)).unwrap();
    assert_eq!(window.activity_count(&Participant(1)), 2);
    assert_eq!(window.advance(time(12)), Ok(1));
    assert_eq!(window.activity_count(&Participant(1)), 1);
}

#[test]
fn expired_replay_is_not_reinserted_after_dedupe_records_are_released() {
    let mut window = window(2);
    let old = activity(1, 10, 0);
    window.observe(old, revision(1, 9), time(0)).unwrap();
    assert_eq!(window.advance(time(9)), Ok(0));
    assert_eq!(window.advance(time(10)), Ok(1));
    assert!(window.is_empty());
    assert_eq!(
        window.observe(old, revision(1, 9), time(10)),
        Err(WindowError::ExpiredActivity)
    );
    assert_eq!(
        window.observe(old, revision(1, 9), time(100)),
        Err(WindowError::ExpiredActivity)
    );
    window
        .observe(activity(1, 11, 11), revision(1, 9), time(11))
        .unwrap();
    assert_eq!(window.len(), 1);
}

#[test]
fn overflow_refuses_without_evicting_or_advancing_time_then_expiry_frees_capacity() {
    let mut window = window(2);
    let first = activity(1, 10, 1);
    window.observe(first, revision(1, 9), time(5)).unwrap();
    window
        .observe(activity(2, 11, 2), revision(1, 9), time(5))
        .unwrap();
    let refused = activity(3, 12, 5);
    assert_eq!(
        window.observe(refused, revision(1, 9), time(6)),
        Err(WindowError::Capacity)
    );
    assert_eq!(window.len(), 2);
    assert_eq!(window.activity_count(&Participant(3)), 0);
    assert_eq!(
        window.observe(first, revision(1, 9), time(5)),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
    assert_eq!(window.advance(time(11)), Ok(1));
    assert_eq!(
        window.observe(refused, revision(1, 9), time(11)),
        Ok(Observation::Inserted { removed_expired: 0 })
    );
    assert_eq!(window.activity_count(&Participant(1)), 0);
    assert_eq!(window.activity_count(&Participant(2)), 1);
    assert_eq!(window.activity_count(&Participant(3)), 1);
    // An ordinary admission also expires old records before checking live capacity.
    assert_eq!(
        window.observe(activity(4, 13, 12), revision(1, 9), time(12)),
        Ok(Observation::Inserted { removed_expired: 1 })
    );
    assert_eq!(window.len(), 2);
    assert_eq!(window.activity_count(&Participant(2)), 0);
    assert_eq!(window.activity_count(&Participant(3)), 1);
    assert_eq!(window.activity_count(&Participant(4)), 1);
}

#[test]
fn provenance_and_time_refusals_preserve_the_whole_window() {
    let mut window = window(2);
    window
        .observe(activity(1, 1, 1), revision(1, 9), time(5))
        .unwrap();
    let candidate = activity(2, 2, 4);
    let mut variants = [candidate; 5];
    variants[0].session = session(9);
    variants[1].run = run(9);
    variants[2].revision = revision(2, 1);
    variants[3].revision = revision(1, 10);
    variants[4].at = time(7);
    let expected = [
        WindowError::WrongSession,
        WindowError::WrongRun,
        WindowError::WrongEpoch,
        WindowError::FutureRevision,
        WindowError::FutureActivity,
    ];
    for (variant, expected) in variants.into_iter().zip(expected) {
        assert_eq!(
            window.observe(variant, revision(1, 9), time(6)),
            Err(expected)
        );
        assert_eq!(window.len(), 1);
        assert_eq!(window.activity_count(&Participant(1)), 1);
        assert_eq!(window.activity_count(&Participant(2)), 0);
    }
    assert_eq!(
        window.observe(candidate, revision(1, 9), time(4)),
        Err(WindowError::TimeRegression)
    );
    assert_eq!(window.advance(time(4)), Err(WindowError::TimeRegression));
    window.observe(candidate, revision(1, 9), time(5)).unwrap();
    assert_eq!(window.len(), 2);
}

#[test]
fn a_duplicate_can_expire_other_records_and_reports_the_removed_count() {
    let mut window = window(2);
    window
        .observe(activity(1, 1, 0), revision(1, 9), time(9))
        .unwrap();
    let retained = activity(2, 2, 8);
    window.observe(retained, revision(1, 9), time(9)).unwrap();
    assert_eq!(
        window.observe(retained, revision(1, 9), time(10)),
        Ok(Observation::Duplicate { removed_expired: 1 })
    );
    assert_eq!(window.len(), 1);
    assert_eq!(window.activity_count(&Participant(1)), 0);
    assert_eq!(window.activity_count(&Participant(2)), 1);
}

#[test]
fn a_window_cannot_mix_successful_observation_epochs_even_after_expiry() {
    let mut window = window(2);
    let first = activity(1, 1, 1);
    window.observe(first, revision(1, 9), time(5)).unwrap();
    let mut next_epoch = activity(2, 2, 4);
    next_epoch.revision = revision(2, 1);

    assert_eq!(
        window.observe(next_epoch, revision(2, 1), time(6)),
        Err(WindowError::WrongEpoch)
    );
    assert_eq!(window.activity_count(&Participant(1)), 1);
    assert_eq!(window.activity_count(&Participant(2)), 0);
    assert_eq!(
        window.observe(first, revision(1, 9), time(5)),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );

    assert_eq!(window.advance(time(11)), Ok(1));
    next_epoch.at = time(11);
    assert_eq!(
        window.observe(next_epoch, revision(2, 1), time(11)),
        Err(WindowError::WrongEpoch)
    );
    assert!(window.is_empty());
}

#[test]
fn accepted_owner_basis_cannot_regress_but_historical_activity_is_valid() {
    let mut window = window(4);
    let first = activity(1, 1, 1);
    window.observe(first, revision(1, 9), time(5)).unwrap();
    let mut historical = activity(2, 2, 4);
    historical.revision = revision(1, 3);
    assert_eq!(
        window.observe(historical, revision(1, 6), time(6)),
        Err(WindowError::StaleBasis)
    );
    assert_eq!(window.len(), 1);
    assert_eq!(window.activity_count(&Participant(2)), 0);
    assert_eq!(
        window.observe(historical, revision(1, 10), time(5)),
        Ok(Observation::Inserted { removed_expired: 0 })
    );
    assert_eq!(
        window.observe(historical, revision(1, 11), time(6)),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
    assert_eq!(window.activity_count(&Participant(1)), 1);
    assert_eq!(window.activity_count(&Participant(2)), 1);
    assert_eq!(
        window.observe(historical, revision(1, 10), time(6)),
        Err(WindowError::StaleBasis)
    );

    assert_eq!(window.advance(time(20)), Ok(2));
    historical.at = time(20);
    assert_eq!(
        window.observe(historical, revision(1, 10), time(20)),
        Err(WindowError::StaleBasis)
    );
    assert!(window.is_empty());
}

#[test]
fn first_refused_observation_does_not_bind_recovery_epoch_or_clock() {
    let mut window = window(1);
    let mut future = activity(1, 1, 7);
    future.revision = revision(2, 1);
    assert_eq!(
        window.observe(future, revision(2, 1), time(6)),
        Err(WindowError::FutureActivity)
    );
    assert!(window.is_empty());
    assert_eq!(
        window.observe(activity(1, 1, 4), revision(1, 9), time(5)),
        Ok(Observation::Inserted { removed_expired: 0 })
    );
}

#[test]
fn refused_capacity_does_not_advance_the_accepted_owner_basis() {
    let mut window = window(1);
    let first = activity(1, 1, 1);
    window.observe(first, revision(1, 9), time(5)).unwrap();
    assert_eq!(
        window.observe(activity(2, 2, 4), revision(1, 10), time(6)),
        Err(WindowError::Capacity)
    );
    assert_eq!(window.len(), 1);
    assert_eq!(window.activity_count(&Participant(2)), 0);
    assert_eq!(
        window.observe(first, revision(1, 9), time(5)),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
}

#[test]
fn an_expired_refusal_does_not_silently_prune_unrelated_records() {
    let mut window = window(2);
    let old = activity(1, 1, 0);
    window.observe(old, revision(1, 9), time(9)).unwrap();
    assert_eq!(
        window.observe(old, revision(1, 9), time(10)),
        Err(WindowError::ExpiredActivity)
    );
    assert_eq!(window.len(), 1);
    assert_eq!(window.advance(time(10)), Ok(1));
}

#[test]
fn explicit_count_time_and_inline_byte_bounds_are_required_before_allocation() {
    for limits in [
        ActivityWindowLimits {
            max_events: 0,
            horizon: time(1),
        },
        ActivityWindowLimits {
            max_events: 4097,
            horizon: time(1),
        },
        ActivityWindowLimits {
            max_events: 1,
            horizon: Duration::ZERO,
        },
    ] {
        assert!(matches!(
            ActivityWindow::<Participant, Event>::new(session(1), run(2), limits),
            Err(WindowError::InvalidLimits)
        ));
    }
    assert!(matches!(
        ActivityWindow::<[u8; 4096], Event>::new(
            session(1),
            run(2),
            ActivityWindowLimits {
                max_events: 4096,
                horizon: time(1)
            }
        ),
        Err(WindowError::RecordBudget)
    ));
    let mut maximum = ActivityWindow::<Participant, Event>::new(
        session(1),
        run(2),
        ActivityWindowLimits {
            max_events: 4096,
            horizon: Duration::MAX,
        },
    )
    .unwrap();
    assert_eq!(
        maximum.observe(activity(1, 1, 0), revision(1, 9), Duration::MAX),
        Err(WindowError::ExpiredActivity)
    );
    assert!(maximum.is_empty());
}

fn checkpoint_member(value: u8) -> df_types::MemberId {
    df_types::MemberId::from_bytes(&[value; 16]).unwrap()
}

fn checkpoint_fact(value: u8) -> df_model::checkpoint::FactId {
    df_model::checkpoint::FactId::from_bytes(&[value; 16]).unwrap()
}

fn checkpoint_source() -> df_model::checkpoint::ContentReference {
    df_model::checkpoint::ContentReference {
        package: checkpoint_label("fixture-package"),
        entry: checkpoint_label("fixture-event"),
    }
}

fn checkpoint_label(value: &str) -> df_types::RevisionLabel {
    df_types::RevisionLabel::new(Some(value)).unwrap()
}

fn checkpoint_basis() -> df_model::checkpoint::Basis {
    df_model::checkpoint::Basis {
        session: session(1),
        run: run(2),
        revision: revision(1, 9),
    }
}

fn checkpoint_pins() -> df_model::checkpoint::CheckpointPins {
    use df_model::checkpoint::{CheckpointPins, ContentDigest, ContentPins, RulesMode, RulesPins};
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: checkpoint_label("fixture-rules"),
            catalog: checkpoint_label("fixture-catalog"),
            catalog_digest: ContentDigest([1; 32]),
            source_manifest: checkpoint_label("fixture-source"),
            source_manifest_digest: ContentDigest([2; 32]),
            handler: checkpoint_label("fixture-handler"),
            handler_digest: ContentDigest([3; 32]),
        },
        content: ContentPins {
            content: checkpoint_label("fixture-content"),
            content_digest: ContentDigest([4; 32]),
            package: checkpoint_label("fixture-package"),
            package_digest: ContentDigest([5; 32]),
        },
        build: df_types::BuildIdentity::new(
            Some("fixture-source"),
            Some("fixture-native"),
            Some("fixture-wasm"),
            Some("fixture-config"),
            Some("fixture-content"),
        )
        .unwrap(),
    }
}

fn checkpoint_limits() -> df_model::checkpoint::CheckpointLimits {
    df_model::checkpoint::CheckpointLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 4096,
        maximum_retained_bytes: 1024 * 1024,
    }
}

fn checkpoint_state() -> df_model::checkpoint::GameState {
    use df_model::checkpoint::*;
    let operation = OperationId::from_bytes(&[3; 16]).unwrap();
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 5,
            ticks_per_second: 1,
        },
        members: vec![
            MembershipLink {
                member: checkpoint_member(3),
                character: None,
            },
            MembershipLink {
                member: checkpoint_member(4),
                character: None,
            },
        ],
        entities: vec![],
        characters: vec![],
        resources: vec![],
        inventory: vec![],
        facts: vec![GameFact {
            id: checkpoint_fact(10),
            revision: revision(1, 7),
            operation,
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: checkpoint_source(),
                subjects: vec![],
            },
        }],
        draws: vec![],
        decisions: vec![AcceptedDecision {
            operation,
            revision: revision(1, 7),
            facts: vec![checkpoint_fact(10)],
            draws: vec![],
            effects: vec![],
            source_policy: checkpoint_label("fixture-participation"),
            semantic_output: None,
        }],
        pending: vec![],
        intents: vec![],
        timers: vec![],
        active_effects: vec![],
        knowledge: vec![],
        beliefs: vec![],
        memories: vec![],
        schedules: vec![],
        threats: vec![],
        relationships: vec![],
        conversations: vec![],
        obligations: vec![],
        narrative: NarrativeState {
            definition: checkpoint_source(),
            active_beats: vec![],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 0,
        },
        encounters: vec![],
        activity: vec![
            ActivityWindow {
                member: checkpoint_member(3),
                started: LogicalTime {
                    ticks: 0,
                    ticks_per_second: 1,
                },
                ends: LogicalTime {
                    ticks: 10,
                    ticks_per_second: 1,
                },
                observed_facts: vec![],
                spotlight_opt_in: true,
            },
            ActivityWindow {
                member: checkpoint_member(4),
                started: LogicalTime {
                    ticks: 0,
                    ticks_per_second: 1,
                },
                ends: LogicalTime {
                    ticks: 10,
                    ticks_per_second: 1,
                },
                observed_facts: vec![],
                spotlight_opt_in: true,
            },
        ],
        tempo: TempoState {
            policy: checkpoint_source(),
            presentation_ticks: 0,
            intensity: 0,
            inertia: 0,
            fatigue: vec![],
        },
        presentation: vec![],
        continuity: ContinuityState {
            creation: vec![],
            simulation: vec![],
            catch_up: None,
            environment: vec![],
            travel: vec![],
            witnesses: vec![],
            rumors: vec![],
            journal: vec![],
            summaries: vec![],
            retrieval: vec![],
            retrieved: vec![],
            consolidation: vec![],
            npcs: vec![],
            hooks: vec![],
            arcs: vec![],
            remote: None,
            presence: vec![],
            audio: None,
            private_offers: vec![],
            knowledge_cues: vec![],
            moments: vec![],
            demands: vec![],
            asset_jobs: vec![],
            asset_dependencies: vec![],
            canonical_packs: vec![],
            shots: vec![],
            prefetch: None,
            scenes: vec![],
            item_origins: vec![],
            bookends: vec![],
            exports: vec![],
            critical_cues: vec![],
            content_candidates: vec![],
            content_admissions: vec![],
            recovery: RecoveryState {
                origin: None,
                retired_epochs: vec![],
                lost_ranges: vec![],
                suppression_generation: 0,
                redacted_records: vec![],
                unavailable_sources: vec![],
            },
        },
    }
}

fn checkpoint_from(state: df_model::checkpoint::GameState) -> df_model::checkpoint::Checkpoint {
    let content = [checkpoint_source()];
    df_model::checkpoint::Checkpoint::new(
        df_model::checkpoint::CHECKPOINT_SCHEMA,
        checkpoint_basis(),
        checkpoint_pins(),
        state,
        df_model::checkpoint::ReferenceInventory {
            rules: &[],
            content: &content,
            resources: &[],
            assets: &[],
        },
        checkpoint_limits(),
    )
    .unwrap()
}

fn checkpoint_activity() -> AcceptedActivity<df_types::MemberId, df_model::checkpoint::FactId> {
    AcceptedActivity {
        session: session(1),
        run: run(2),
        operation: OperationId::from_bytes(&[3; 16]).unwrap(),
        revision: revision(1, 7),
        participant: checkpoint_member(3),
        event: checkpoint_fact(10),
        at: time(3),
    }
}

fn checkpoint_window(
    max_events: usize,
) -> ActivityWindow<df_types::MemberId, df_model::checkpoint::FactId> {
    ActivityWindow::new(
        session(1),
        run(2),
        ActivityWindowLimits {
            max_events,
            horizon: time(10),
        },
    )
    .unwrap()
}

struct CheckpointOwner {
    member: df_types::MemberId,
    at: Duration,
    permitted: bool,
    calls: std::cell::Cell<usize>,
}

impl CheckpointOwner {
    fn new() -> Self {
        Self {
            member: checkpoint_member(3),
            at: time(3),
            permitted: true,
            calls: std::cell::Cell::new(0),
        }
    }
}

impl df_experience::window::ActivitySourceOwner for CheckpointOwner {
    fn admits_activity(
        &self,
        checkpoint: &df_model::checkpoint::Checkpoint,
        fact: &df_model::checkpoint::GameFact,
        decision: &df_model::checkpoint::AcceptedDecision,
        activity: &AcceptedActivity<df_types::MemberId, df_model::checkpoint::FactId>,
        _now: Duration,
    ) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.permitted
            && checkpoint.basis() == checkpoint_basis()
            && fact.id == checkpoint_fact(10)
            && decision.source_policy == checkpoint_label("fixture-participation")
            && activity.participant == self.member
            && activity.at == self.at
    }
}

fn observe_checkpoint(
    window: &mut ActivityWindow<df_types::MemberId, df_model::checkpoint::FactId>,
    checkpoint: &df_model::checkpoint::Checkpoint,
    activity: AcceptedActivity<df_types::MemberId, df_model::checkpoint::FactId>,
    ordinal: u32,
    now: Duration,
    owner: &CheckpointOwner,
) -> Result<Observation, df_experience::window::CheckpointActivityError> {
    let content = [checkpoint_source()];
    window.observe_checkpoint(
        df_experience::window::CheckpointActivityRequest {
            checkpoint,
            basis: checkpoint_basis(),
            pins: &checkpoint_pins(),
            inventory: df_model::checkpoint::ReferenceInventory {
                rules: &[],
                content: &content,
                resources: &[],
                assets: &[],
            },
            checkpoint_limits: checkpoint_limits(),
            activity,
            ordinal,
            now,
        },
        owner,
    )
}

#[test]
fn checkpoint_activity_requires_current_consent_and_exact_accepted_retry() {
    let checkpoint = checkpoint_from(checkpoint_state());
    let unchanged = checkpoint.clone();
    let mut window = checkpoint_window(2);
    let owner = CheckpointOwner::new();
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &checkpoint,
            checkpoint_activity(),
            0,
            time(5),
            &owner
        ),
        Ok(Observation::Inserted { removed_expired: 0 })
    );
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &checkpoint,
            checkpoint_activity(),
            0,
            time(6),
            &owner
        ),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
    assert_eq!(window.activity_count(&checkpoint_member(3)), 1);
    assert_eq!(checkpoint, unchanged);
    assert_eq!(owner.calls.get(), 2);
}

#[test]
fn checkpoint_revocation_absent_member_and_ambiguous_consent_do_not_count() {
    use df_experience::window::CheckpointActivityError;
    let mut window = checkpoint_window(2);
    let owner = CheckpointOwner::new();
    let checkpoint = checkpoint_from(checkpoint_state());
    observe_checkpoint(
        &mut window,
        &checkpoint,
        checkpoint_activity(),
        0,
        time(5),
        &owner,
    )
    .unwrap();
    for case in 0..4 {
        let mut state = checkpoint_state();
        let mut activity = checkpoint_activity();
        let expected = match case {
            0 => {
                state.activity[0].spotlight_opt_in = false;
                CheckpointActivityError::NotOptedIn
            }
            1 => {
                state.activity.remove(0);
                CheckpointActivityError::NotOptedIn
            }
            2 => {
                activity.participant = checkpoint_member(99);
                CheckpointActivityError::MissingMember
            }
            3 => {
                state.activity.push(state.activity[0].clone());
                CheckpointActivityError::AmbiguousConsent
            }
            _ => unreachable!(),
        };
        let current = checkpoint_from(state);
        assert_eq!(
            observe_checkpoint(&mut window, &current, activity, 0, time(6), &owner),
            Err(expected)
        );
        assert_eq!(window.len(), 1);
        assert_eq!(window.activity_count(&checkpoint_member(3)), 1);
        assert_eq!(owner.calls.get(), 1);
    }
    // Refusals did not advance the window clock.
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &checkpoint,
            checkpoint_activity(),
            0,
            time(5),
            &owner
        ),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
}

#[test]
fn checkpoint_activity_refuses_orphans_and_mismatched_fact_lineage_before_owner() {
    use df_experience::window::CheckpointActivityError;
    let owner = CheckpointOwner::new();
    let mut window = checkpoint_window(2);
    let mut state = checkpoint_state();
    state.decisions.clear();
    let orphan = checkpoint_from(state);
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &orphan,
            checkpoint_activity(),
            0,
            time(5),
            &owner
        ),
        Err(CheckpointActivityError::UnacceptedFact)
    );
    let checkpoint = checkpoint_from(checkpoint_state());
    for case in 0..7 {
        let mut activity = checkpoint_activity();
        let mut ordinal = 0;
        let expected = match case {
            0 => {
                activity.operation = OperationId::from_bytes(&[8; 16]).unwrap();
                CheckpointActivityError::FactMismatch
            }
            1 => {
                activity.revision = revision(1, 8);
                CheckpointActivityError::FactMismatch
            }
            2 => {
                ordinal = 1;
                CheckpointActivityError::FactMismatch
            }
            3 => {
                activity.event = checkpoint_fact(99);
                CheckpointActivityError::UnacceptedFact
            }
            4 => {
                activity.session = session(9);
                CheckpointActivityError::FactMismatch
            }
            5 => {
                activity.run = run(9);
                CheckpointActivityError::FactMismatch
            }
            6 => {
                activity.revision = revision(2, 7);
                CheckpointActivityError::FactMismatch
            }
            _ => unreachable!(),
        };
        assert_eq!(
            observe_checkpoint(&mut window, &checkpoint, activity, ordinal, time(5), &owner),
            Err(expected)
        );
        assert!(window.is_empty());
        assert_eq!(owner.calls.get(), 0);
    }
}

#[test]
fn checkpoint_mapping_and_stable_event_time_need_current_native_source_admission() {
    use df_experience::window::CheckpointActivityError;
    let checkpoint = checkpoint_from(checkpoint_state());
    let owner = CheckpointOwner::new();
    let mut window = checkpoint_window(2);
    let mut activity = checkpoint_activity();
    activity.participant = checkpoint_member(4);
    assert_eq!(
        observe_checkpoint(&mut window, &checkpoint, activity, 0, time(5), &owner),
        Err(CheckpointActivityError::SourceRefused)
    );
    activity = checkpoint_activity();
    activity.at = time(4);
    assert_eq!(
        observe_checkpoint(&mut window, &checkpoint, activity, 0, time(5), &owner),
        Err(CheckpointActivityError::SourceRefused)
    );
    assert!(window.is_empty());
    observe_checkpoint(
        &mut window,
        &checkpoint,
        checkpoint_activity(),
        0,
        time(5),
        &owner,
    )
    .unwrap();
    let refused = CheckpointOwner {
        permitted: false,
        ..CheckpointOwner::new()
    };
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &checkpoint,
            checkpoint_activity(),
            0,
            time(6),
            &refused
        ),
        Err(CheckpointActivityError::SourceRefused)
    );
    assert_eq!(window.len(), 1);
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &checkpoint,
            checkpoint_activity(),
            0,
            time(5),
            &owner
        ),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
}

#[test]
fn checkpoint_activity_keeps_future_expired_and_capacity_refusals_atomic() {
    use df_experience::window::CheckpointActivityError;
    let checkpoint = checkpoint_from(checkpoint_state());
    let mut window = checkpoint_window(1);
    let mut future = checkpoint_activity();
    future.at = time(6);
    let future_owner = CheckpointOwner {
        at: time(6),
        ..CheckpointOwner::new()
    };
    assert_eq!(
        observe_checkpoint(&mut window, &checkpoint, future, 0, time(5), &future_owner),
        Err(CheckpointActivityError::Window(WindowError::FutureActivity))
    );
    assert!(window.is_empty());
    let owner = CheckpointOwner::new();
    observe_checkpoint(
        &mut window,
        &checkpoint,
        checkpoint_activity(),
        0,
        time(5),
        &owner,
    )
    .unwrap();
    let mut second_state = checkpoint_state();
    second_state.facts[0].id = checkpoint_fact(11);
    second_state.decisions[0].facts[0] = checkpoint_fact(11);
    let second = checkpoint_from(second_state);
    let mut second_activity = checkpoint_activity();
    second_activity.event = checkpoint_fact(11);
    // A different exact source-admitted event cannot evict retained activity.
    struct SecondOwner;
    impl df_experience::window::ActivitySourceOwner for SecondOwner {
        fn admits_activity(
            &self,
            _checkpoint: &df_model::checkpoint::Checkpoint,
            fact: &df_model::checkpoint::GameFact,
            decision: &df_model::checkpoint::AcceptedDecision,
            activity: &AcceptedActivity<df_types::MemberId, df_model::checkpoint::FactId>,
            _now: Duration,
        ) -> bool {
            fact.id == checkpoint_fact(11)
                && decision.facts == [checkpoint_fact(11)]
                && activity.participant == checkpoint_member(3)
                && activity.at == time(3)
        }
    }
    let content = [checkpoint_source()];
    assert_eq!(
        window.observe_checkpoint(
            df_experience::window::CheckpointActivityRequest {
                checkpoint: &second,
                basis: checkpoint_basis(),
                pins: &checkpoint_pins(),
                inventory: df_model::checkpoint::ReferenceInventory {
                    rules: &[],
                    content: &content,
                    resources: &[],
                    assets: &[],
                },
                checkpoint_limits: checkpoint_limits(),
                activity: second_activity,
                ordinal: 0,
                now: time(6),
            },
            &SecondOwner
        ),
        Err(CheckpointActivityError::Window(WindowError::Capacity))
    );
    assert_eq!(window.len(), 1);
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &checkpoint,
            checkpoint_activity(),
            0,
            time(13),
            &owner
        ),
        Err(CheckpointActivityError::Window(
            WindowError::ExpiredActivity
        ))
    );
    assert_eq!(window.len(), 1);
    assert_eq!(
        observe_checkpoint(
            &mut window,
            &checkpoint,
            checkpoint_activity(),
            0,
            time(5),
            &owner
        ),
        Ok(Observation::Duplicate { removed_expired: 0 })
    );
}

#[test]
fn checkpoint_activity_revalidates_current_basis_pins_and_existing_work_byte_bounds() {
    use df_experience::window::{CheckpointActivityError, CheckpointActivityRequest};
    use df_model::checkpoint::{CheckpointError, ReferenceInventory};
    let checkpoint = checkpoint_from(checkpoint_state());
    let unchanged = checkpoint.clone();
    let owner = CheckpointOwner::new();
    let mut window = checkpoint_window(2);
    let content = [checkpoint_source()];
    for case in 0..6 {
        let mut basis = checkpoint_basis();
        let mut pins = checkpoint_pins();
        let mut limits = checkpoint_limits();
        let expected = match case {
            0 => {
                basis.revision = revision(1, 8);
                CheckpointError::StaleBasis
            }
            1 => {
                basis.run = run(9);
                CheckpointError::WrongRun
            }
            2 => {
                basis.session = session(9);
                CheckpointError::WrongSession
            }
            3 => {
                pins.rules.handler_digest.0 = [9; 32];
                CheckpointError::RulesMismatch
            }
            4 => {
                limits.maximum_records = 1;
                CheckpointError::Capacity
            }
            5 => {
                limits.maximum_retained_bytes = 1;
                CheckpointError::Capacity
            }
            _ => unreachable!(),
        };
        assert_eq!(
            window.observe_checkpoint(
                CheckpointActivityRequest {
                    checkpoint: &checkpoint,
                    basis,
                    pins: &pins,
                    inventory: ReferenceInventory {
                        rules: &[],
                        content: &content,
                        resources: &[],
                        assets: &[],
                    },
                    checkpoint_limits: limits,
                    activity: checkpoint_activity(),
                    ordinal: 0,
                    now: time(5),
                },
                &owner
            ),
            Err(CheckpointActivityError::Checkpoint(expected))
        );
        assert!(window.is_empty());
        assert_eq!(owner.calls.get(), 0);
        assert_eq!(checkpoint, unchanged);
    }
}
