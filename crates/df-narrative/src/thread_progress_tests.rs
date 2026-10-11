use super::{
    AcceptedThreadConsequence, CheckpointProgressError, ProgressError, ProgressLimits,
    ThreadCheckpointRequest, ThreadConsequenceSelection, ThreadDisposition, ThreadProgressBasis,
    stage_checkpoint_thread_progress, stage_thread_progress,
};
use df_model::checkpoint::{
    AcceptedDecision, AudienceScope, Basis, CHECKPOINT_SCHEMA, Checkpoint, CheckpointLimits,
    CheckpointPins, ContentDigest, ContentPins, ContentReference, ContinuityState, ExecutionMode,
    FactId, FactValue, GameFact, GameState, LogicalTime, NarrativeState, RecoveryState,
    ReferenceInventory, RulesMode, RulesPins, TempoState,
};
use df_types::{
    BuildIdentity, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
};

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}

fn reference(entry: &str) -> ContentReference {
    ContentReference {
        package: label("package-1"),
        entry: label(entry),
    }
}

fn revision(sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(1).unwrap(), sequence)
}

fn fact_id(value: u8) -> FactId {
    FactId::from_bytes(&[value; 16]).unwrap()
}

fn limits() -> ProgressLimits {
    // Explicit fixture bounds, not calibrated production limits.
    ProgressLimits {
        records: 16,
        consequences: 4,
        work: 4096,
    }
}

struct Fixture {
    basis: Basis,
    content: ContentPins,
    policy: RevisionLabel,
    narrative: NarrativeState,
    admitted: Vec<ContentReference>,
    facts: Vec<GameFact>,
}

impl Fixture {
    fn new() -> Self {
        let definition = reference("campaign");
        let active = reference("active-beat");
        let completed = reference("completed-beat");
        let threads = vec![
            reference("promise"),
            reference("mystery"),
            reference("rival"),
        ];
        let event = reference("resolved-event");
        let mut admitted = vec![
            definition.clone(),
            active.clone(),
            completed.clone(),
            event.clone(),
        ];
        admitted.extend(threads.iter().cloned());
        let facts = (1_u8..=3)
            .map(|id| GameFact {
                id: fact_id(id),
                revision: revision(u64::from(id)),
                operation: OperationId::from_bytes(&[id; 16]).unwrap(),
                ordinal: 0,
                cause: if id == 1 { None } else { Some(fact_id(id - 1)) },
                audience: AudienceScope::Host,
                value: FactValue::ContentEvent {
                    definition: event.clone(),
                    subjects: vec![],
                },
            })
            .collect();
        Self {
            basis: Basis {
                session: SessionId::from_bytes(&[1; 16]).unwrap(),
                run: RunId::from_bytes(&[2; 16]).unwrap(),
                revision: revision(7),
            },
            content: ContentPins {
                content: label("content-1"),
                content_digest: ContentDigest([1; 32]),
                package: label("package-1"),
                package_digest: ContentDigest([2; 32]),
            },
            policy: label("thread-policy-1"),
            narrative: NarrativeState {
                definition,
                active_beats: vec![active],
                completed_beats: vec![completed],
                open_threads: threads,
                accepted_facts: vec![fact_id(1)],
                remaining_budget: 43,
            },
            admitted,
            facts,
        }
    }

    fn view(&self) -> ThreadProgressBasis<'_> {
        ThreadProgressBasis {
            current: self.basis,
            expected: self.basis,
            content: &self.content,
            expected_content: &self.content,
            policy: &self.policy,
            expected_policy: &self.policy,
            narrative: &self.narrative,
            admitted_content: &self.admitted,
            facts: &self.facts,
        }
    }

    fn consequence(
        &self,
        thread: usize,
        source: usize,
        disposition: ThreadDisposition,
    ) -> AcceptedThreadConsequence<'_> {
        AcceptedThreadConsequence {
            thread: &self.narrative.open_threads[thread],
            source: &self.facts[source],
            disposition,
            ending_policy: None,
        }
    }
}

#[test]
fn accepted_consequences_retain_unresolved_branches_and_immutable_source() {
    let fixture = Fixture::new();
    let before = fixture.narrative.clone();
    let facts_before = fixture.facts.clone();
    let consequences = [
        fixture.consequence(0, 1, ThreadDisposition::Continue),
        fixture.consequence(1, 2, ThreadDisposition::Resolve),
    ];
    let proposal = stage_thread_progress(fixture.view(), &consequences, limits()).unwrap();

    assert_eq!(
        proposal.narrative.open_threads,
        [reference("promise"), reference("rival")]
    );
    assert_eq!(
        proposal.narrative.accepted_facts,
        [fact_id(1), fact_id(2), fact_id(3)]
    );
    assert_eq!(proposal.narrative.active_beats, before.active_beats);
    assert_eq!(proposal.narrative.completed_beats, before.completed_beats);
    assert_eq!(proposal.narrative.definition, before.definition);
    assert_eq!(proposal.narrative.remaining_budget, before.remaining_budget);
    assert_eq!(fixture.narrative, before);
    assert_eq!(fixture.facts, facts_before);
    assert_eq!(proposal.basis, fixture.basis);
    assert_eq!(proposal.content, fixture.content);
    assert_eq!(proposal.policy, fixture.policy);
    assert_eq!(proposal.evidence[1].thread, reference("mystery"));
    assert_eq!(proposal.evidence[1].source, fact_id(3));
    assert_eq!(proposal.evidence[1].cause, Some(fact_id(2)));
    assert_eq!(proposal.evidence[1].operation, fixture.facts[2].operation);
    assert_eq!(proposal.evidence[1].revision, fixture.facts[2].revision);
    assert_eq!(proposal.evidence[1].ordinal, fixture.facts[2].ordinal);
}

#[test]
fn ignored_or_refused_opportunity_preserves_threads_without_consuming_a_fact() {
    let fixture = Fixture::new();
    let proposal = stage_thread_progress(fixture.view(), &[], limits()).unwrap();
    assert_eq!(proposal.narrative, fixture.narrative);
    assert!(proposal.evidence.is_empty());
}

#[test]
fn continue_never_infers_closure_from_a_content_event() {
    let fixture = Fixture::new();
    let consequence = fixture.consequence(0, 1, ThreadDisposition::Continue);
    let proposal = stage_thread_progress(fixture.view(), &[consequence], limits()).unwrap();
    assert_eq!(
        proposal.narrative.open_threads,
        fixture.narrative.open_threads
    );
    assert_eq!(proposal.narrative.accepted_facts, [fact_id(1), fact_id(2)]);
}

#[test]
fn same_immutable_input_replays_to_the_same_proposal_without_mutation() {
    let fixture = Fixture::new();
    let consequences = [fixture.consequence(1, 2, ThreadDisposition::Resolve)];
    let first = stage_thread_progress(fixture.view(), &consequences, limits());
    let second = stage_thread_progress(fixture.view(), &consequences, limits());
    assert_eq!(first, second);
    assert_eq!(fixture.narrative.open_threads.len(), 3);
}

#[test]
fn independently_admitted_consequences_keep_committed_order_in_either_input_order() {
    let fixture = Fixture::new();
    let forward = [
        fixture.consequence(0, 1, ThreadDisposition::Continue),
        fixture.consequence(1, 2, ThreadDisposition::Resolve),
    ];
    let reverse = [
        fixture.consequence(1, 2, ThreadDisposition::Resolve),
        fixture.consequence(0, 1, ThreadDisposition::Continue),
    ];
    assert_eq!(
        stage_thread_progress(fixture.view(), &forward, limits()),
        stage_thread_progress(fixture.view(), &reverse, limits()),
    );
}

#[test]
fn consumed_source_cannot_advance_a_thread_again_on_a_new_revision() {
    let fixture = Fixture::new();
    let mut narrative = fixture.narrative.clone();
    narrative.accepted_facts.push(fact_id(2));
    let mut view = fixture.view();
    view.narrative = &narrative;
    view.current.revision = revision(8);
    view.expected = view.current;
    let consequence = fixture.consequence(0, 1, ThreadDisposition::Continue);
    assert_eq!(
        stage_thread_progress(view, &[consequence], limits()),
        Err(ProgressError::RepeatedConsequence),
    );
}

#[test]
fn one_resolved_thread_does_not_resolve_another_with_the_same_event_definition() {
    let fixture = Fixture::new();
    let consequences = [fixture.consequence(0, 1, ThreadDisposition::Resolve)];
    let proposal = stage_thread_progress(fixture.view(), &consequences, limits()).unwrap();
    assert_eq!(
        proposal.narrative.open_threads,
        [reference("mystery"), reference("rival")]
    );
}

#[test]
fn invalid_later_consequence_refuses_the_entire_candidate() {
    let fixture = Fixture::new();
    let before = fixture.narrative.clone();
    let missing = reference("unadmitted-thread");
    let consequences = [
        fixture.consequence(0, 1, ThreadDisposition::Resolve),
        AcceptedThreadConsequence {
            thread: &missing,
            source: &fixture.facts[2],
            disposition: ThreadDisposition::Continue,
            ending_policy: None,
        },
    ];
    assert_eq!(
        stage_thread_progress(fixture.view(), &consequences, limits()),
        Err(ProgressError::UnadmittedContent)
    );
    assert_eq!(fixture.narrative, before);
}

#[test]
fn closed_thread_and_conflicting_batch_have_typed_refusals() {
    let fixture = Fixture::new();
    let closed = reference("completed-beat");
    let consequence = AcceptedThreadConsequence {
        thread: &closed,
        source: &fixture.facts[1],
        disposition: ThreadDisposition::Resolve,
        ending_policy: None,
    };
    assert_eq!(
        stage_thread_progress(fixture.view(), &[consequence], limits()),
        Err(ProgressError::ClosedThread)
    );
    let conflicts = [
        fixture.consequence(0, 1, ThreadDisposition::Continue),
        fixture.consequence(0, 2, ThreadDisposition::Resolve),
    ];
    assert_eq!(
        stage_thread_progress(fixture.view(), &conflicts, limits()),
        Err(ProgressError::ConflictingConsequences)
    );
    let repeated = [
        fixture.consequence(0, 1, ThreadDisposition::Continue),
        fixture.consequence(1, 1, ThreadDisposition::Resolve),
    ];
    assert_eq!(
        stage_thread_progress(fixture.view(), &repeated, limits()),
        Err(ProgressError::RepeatedConsequence)
    );
}

#[test]
fn missing_or_forged_source_record_cannot_supply_progress() {
    let fixture = Fixture::new();
    for change in 0..7 {
        let mut source = fixture.facts[1].clone();
        let expected = match change {
            0 => {
                source.id = fact_id(9);
                ProgressError::MissingFact
            }
            1 => {
                source.revision = revision(1);
                ProgressError::SourceFactMismatch
            }
            2 => {
                source.operation = OperationId::from_bytes(&[9; 16]).unwrap();
                ProgressError::SourceFactMismatch
            }
            3 => {
                source.ordinal = 3;
                ProgressError::SourceFactMismatch
            }
            4 => {
                source.cause = None;
                ProgressError::SourceFactMismatch
            }
            5 => {
                source.audience = AudienceScope::Shared;
                ProgressError::SourceFactMismatch
            }
            _ => {
                source.value = FactValue::ContentEvent {
                    definition: reference("promise"),
                    subjects: vec![],
                };
                ProgressError::SourceFactMismatch
            }
        };
        let consequence = AcceptedThreadConsequence {
            thread: &fixture.narrative.open_threads[0],
            source: &source,
            disposition: ThreadDisposition::Continue,
            ending_policy: None,
        };
        assert_eq!(
            stage_thread_progress(fixture.view(), &[consequence], limits()),
            Err(expected)
        );
    }
}

#[test]
fn stale_session_run_revision_recovery_policy_and_content_are_refused() {
    let fixture = Fixture::new();
    for case in 0..4 {
        let mut view = fixture.view();
        let expected = match case {
            0 => {
                view.expected.session = SessionId::from_bytes(&[9; 16]).unwrap();
                ProgressError::WrongSession
            }
            1 => {
                view.expected.run = RunId::from_bytes(&[9; 16]).unwrap();
                ProgressError::WrongRun
            }
            2 => {
                view.expected.revision = revision(6);
                ProgressError::StaleBasis
            }
            _ => {
                view.expected.revision = SessionRevision::new(RecoveryEpoch::new(2).unwrap(), 7);
                ProgressError::StaleBasis
            }
        };
        assert_eq!(stage_thread_progress(view, &[], limits()), Err(expected));
    }
    let changed_policy = label("thread-policy-2");
    let mut view = fixture.view();
    view.expected_policy = &changed_policy;
    assert_eq!(
        stage_thread_progress(view, &[], limits()),
        Err(ProgressError::StalePolicy)
    );
    for case in 0..4 {
        let mut changed = fixture.content.clone();
        match case {
            0 => changed.content = label("content-2"),
            1 => changed.content_digest = ContentDigest([9; 32]),
            2 => changed.package = label("package-2"),
            _ => changed.package_digest = ContentDigest([9; 32]),
        }
        let mut view = fixture.view();
        view.expected_content = &changed;
        assert_eq!(
            stage_thread_progress(view, &[], limits()),
            Err(ProgressError::StaleContent)
        );
    }
}

#[test]
fn causal_fact_inventory_refuses_missing_self_future_and_reversed_causes() {
    for case in 0..5 {
        let mut fixture = Fixture::new();
        let expected = match case {
            0 => {
                fixture.facts[1].cause = Some(fact_id(9));
                ProgressError::InvalidChronology
            }
            1 => {
                fixture.facts[1].cause = Some(fact_id(2));
                ProgressError::InvalidChronology
            }
            2 => {
                fixture.facts[1].cause = Some(fact_id(3));
                ProgressError::InvalidChronology
            }
            3 => {
                fixture.facts[2].revision = revision(8);
                ProgressError::FutureFact
            }
            _ => {
                fixture.facts[2].revision = revision(1);
                ProgressError::InvalidChronology
            }
        };
        assert_eq!(
            stage_thread_progress(fixture.view(), &[], limits()),
            Err(expected)
        );
    }
}

#[test]
fn duplicate_or_unordered_fact_records_and_dangling_accepted_facts_are_refused() {
    let mut fixture = Fixture::new();
    fixture.facts[2].id = fact_id(2);
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::DuplicateFact)
    );
    let mut fixture = Fixture::new();
    fixture.facts[2].operation = fixture.facts[1].operation;
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::InvalidChronology)
    );
    fixture.facts[2].ordinal = 2;
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::InvalidChronology)
    );
    let mut fixture = Fixture::new();
    fixture.facts[0].ordinal = 1;
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::InvalidChronology)
    );
    let mut fixture = Fixture::new();
    fixture.narrative.accepted_facts.push(fact_id(9));
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::MissingFact)
    );
    let mut fixture = Fixture::new();
    fixture.narrative.accepted_facts.push(fact_id(1));
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::DuplicateFact)
    );
}

#[test]
fn content_admission_covers_untouched_threads_and_causal_event_definitions() {
    let mut fixture = Fixture::new();
    fixture.narrative.open_threads[2].package = label("unpublished-package");
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::UnadmittedContent)
    );
    let mut fixture = Fixture::new();
    fixture
        .admitted
        .retain(|item| item.entry != label("resolved-event"));
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::UnadmittedContent)
    );
    let mut fixture = Fixture::new();
    fixture.narrative.open_threads.push(reference("promise"));
    assert_eq!(
        stage_thread_progress(fixture.view(), &[], limits()),
        Err(ProgressError::DuplicateThread)
    );
}

#[test]
fn capacity_covers_input_output_and_whole_pass_validation_work() {
    let fixture = Fixture::new();
    let consequences = [fixture.consequence(0, 1, ThreadDisposition::Continue)];
    for constrained in [
        ProgressLimits {
            records: 6,
            ..limits()
        },
        ProgressLimits {
            consequences: 0,
            ..limits()
        },
        ProgressLimits {
            work: 0,
            ..limits()
        },
        ProgressLimits {
            work: 30,
            ..limits()
        },
    ] {
        assert_eq!(
            stage_thread_progress(fixture.view(), &consequences, constrained),
            Err(ProgressError::Capacity)
        );
    }
    let mut fixture = Fixture::new();
    fixture.narrative.accepted_facts = vec![fact_id(1); 16];
    let consequences = [fixture.consequence(0, 1, ThreadDisposition::Continue)];
    assert_eq!(
        stage_thread_progress(fixture.view(), &consequences, limits()),
        Err(ProgressError::Capacity)
    );
}

#[test]
fn work_exhaustion_during_staging_exposes_no_partial_candidate() {
    let fixture = Fixture::new();
    let narrative_before = fixture.narrative.clone();
    let facts_before = fixture.facts.clone();
    // Inventory positions are campaign 1, active 2, completed 3, event 4,
    // promise 5, mystery 6, rival 7. Validation charges 27 narrative, 18 fact,
    // 1 accepted-source and 10 consequence comparisons: 56 before staging.
    let validation_work = 56;
    for (disposition, staging_work) in [
        (ThreadDisposition::Continue, 3),
        (ThreadDisposition::Resolve, 6),
    ] {
        let consequences = [fixture.consequence(0, 1, disposition)];
        // Both variants compare all three canonical facts. Resolve also compares
        // the three open threads. One less fails at the final fact, after the
        // detached candidate has consumed its source and staged its evidence.
        let required_work = validation_work + staging_work;
        let sufficient = ProgressLimits {
            work: required_work,
            ..limits()
        };
        let insufficient = ProgressLimits {
            work: required_work - 1,
            ..limits()
        };
        let proposed = stage_thread_progress(fixture.view(), &consequences, sufficient).unwrap();
        assert_eq!(proposed.narrative.accepted_facts, [fact_id(1), fact_id(2)]);
        assert_eq!(proposed.evidence.len(), 1);
        assert_eq!(proposed.evidence[0].source, fact_id(2));
        assert_eq!(proposed.evidence[0].cause, Some(fact_id(1)));
        assert_eq!(proposed.evidence[0].disposition, disposition);
        let expected_threads = match disposition {
            ThreadDisposition::Continue => fixture.narrative.open_threads.clone(),
            ThreadDisposition::Resolve => vec![reference("mystery"), reference("rival")],
        };
        assert_eq!(proposed.narrative.open_threads, expected_threads);
        assert_eq!(
            stage_thread_progress(fixture.view(), &consequences, insufficient),
            Err(ProgressError::Capacity)
        );
        assert_eq!(fixture.narrative, narrative_before);
        assert_eq!(fixture.facts, facts_before);
    }
}

fn checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 128,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 4096,
        maximum_retained_bytes: 1024 * 1024,
    }
}

fn inventory(fixture: &Fixture) -> ReferenceInventory<'_> {
    ReferenceInventory {
        rules: &[],
        content: &fixture.admitted,
        resources: &[],
        assets: &[],
    }
}

fn checkpoint(fixture: &Fixture) -> Checkpoint {
    // Public checkpoint selection consumes accepted events, not unowned fixture facts.
    let decisions = fixture
        .facts
        .iter()
        .map(|fact| AcceptedDecision {
            operation: fact.operation,
            revision: fact.revision,
            facts: vec![fact.id],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-accepted-event-policy"),
            semantic_output: None,
        })
        .collect();
    checkpoint_with_decisions(fixture, decisions)
}

fn checkpoint_with_decisions(fixture: &Fixture, decisions: Vec<AcceptedDecision>) -> Checkpoint {
    let pins = CheckpointPins {
        content: fixture.content.clone(),
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("rules-1"),
            catalog: label("catalog-1"),
            catalog_digest: ContentDigest([3; 32]),
            source_manifest: label("sources-1"),
            source_manifest_digest: ContentDigest([4; 32]),
            handler: label("handler-1"),
            handler_digest: ContentDigest([5; 32]),
        },
        build: BuildIdentity::new(
            Some("source-1"),
            Some("native-1"),
            Some("wasm-1"),
            Some("config-1"),
            Some("content-1"),
        )
        .unwrap(),
    };
    let state = GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 0,
            ticks_per_second: 10,
        },
        members: vec![],
        entities: vec![],
        characters: vec![],
        resources: vec![],
        inventory: vec![],
        facts: fixture.facts.clone(),
        draws: vec![],
        decisions,
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
        narrative: fixture.narrative.clone(),
        encounters: vec![],
        activity: vec![],
        tempo: TempoState {
            policy: fixture.narrative.definition.clone(),
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
    };
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        fixture.basis,
        pins,
        state,
        inventory(fixture),
        checkpoint_limits(),
    )
    .unwrap()
}

fn checkpoint_request<'a>(
    fixture: &'a Fixture,
    checkpoint: &'a Checkpoint,
    selections: &'a [ThreadConsequenceSelection<'a>],
) -> ThreadCheckpointRequest<'a> {
    ThreadCheckpointRequest {
        expected_basis: checkpoint.basis(),
        admitted_pins: checkpoint.pins(),
        policy: &fixture.policy,
        expected_policy: &fixture.policy,
        inventory: inventory(fixture),
        checkpoint_limits: checkpoint_limits(),
        selections,
    }
}

#[test]
fn checkpoint_producer_consumer_selects_current_consequences_and_applies_once() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let before = current.clone();
    let event = reference("resolved-event");
    let selections = [
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[0],
            event_definition: &event,
            source: fact_id(2),
            disposition: ThreadDisposition::Continue,
        },
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[1],
            event_definition: &event,
            source: fact_id(3),
            disposition: ThreadDisposition::Resolve,
        },
    ];
    let proposal = stage_checkpoint_thread_progress(
        &current,
        checkpoint_request(&fixture, &current, &selections),
        limits(),
    )
    .unwrap();
    assert_eq!(current, before);
    assert_eq!(proposal.checkpoint.basis(), current.basis());
    assert_eq!(proposal.checkpoint.pins(), current.pins());
    assert_eq!(proposal.policy, fixture.policy);
    let mut protected = proposal.checkpoint.state().clone();
    protected.narrative = current.state().narrative.clone();
    assert_eq!(protected, *current.state());
    assert_eq!(proposal.evidence[1].source, fact_id(3));
    assert_eq!(proposal.evidence[1].cause, Some(fact_id(2)));

    // The fixture owner consumes the canonical detached proposal. Durable session
    // authorization/commit is deliberately outside this pure producer/consumer.
    let selected = proposal.checkpoint;
    assert_eq!(
        selected.state().narrative.open_threads,
        [reference("promise"), reference("rival")]
    );
    assert_eq!(
        selected.state().narrative.accepted_facts,
        [fact_id(1), fact_id(2), fact_id(3)]
    );
    let selected_before_retry = selected.clone();
    assert_eq!(
        stage_checkpoint_thread_progress(
            &selected,
            checkpoint_request(&fixture, &selected, &selections),
            limits(),
        ),
        Err(CheckpointProgressError::Progress(
            ProgressError::RepeatedConsequence
        ))
    );
    assert_eq!(selected, selected_before_retry);
}

#[test]
fn checkpoint_selection_refuses_unrelated_definition_without_narrative_output() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let before = current.clone();
    let wrong_event = reference("promise");
    let selections = [ThreadConsequenceSelection {
        thread: &fixture.narrative.open_threads[0],
        event_definition: &wrong_event,
        source: fact_id(2),
        disposition: ThreadDisposition::Resolve,
    }];
    assert_eq!(
        stage_checkpoint_thread_progress(
            &current,
            checkpoint_request(&fixture, &current, &selections),
            limits(),
        ),
        Err(CheckpointProgressError::SourceDefinition)
    );
    assert_eq!(current, before);
}

#[test]
fn checkpoint_producer_rechecks_complete_source_pins_and_output_validation() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let mut changed_pins = current.pins().clone();
    changed_pins.rules.source_manifest_digest = ContentDigest([9; 32]);
    let mut request = checkpoint_request(&fixture, &current, &[]);
    request.admitted_pins = &changed_pins;
    assert_eq!(
        stage_checkpoint_thread_progress(&current, request, limits()),
        Err(CheckpointProgressError::Binding(
            df_model::checkpoint::CheckpointError::RulesMismatch
        ))
    );
    let mut request = checkpoint_request(&fixture, &current, &[]);
    request.checkpoint_limits.maximum_records = 1;
    assert_eq!(
        stage_checkpoint_thread_progress(&current, request, limits()),
        Err(CheckpointProgressError::InvalidCandidate(
            df_model::checkpoint::CheckpointError::Capacity
        ))
    );
}

#[test]
fn checkpoint_selection_requires_exact_accepted_decision_owner() {
    let fixture = Fixture::new();
    let canonical = checkpoint(&fixture);
    let event = reference("resolved-event");
    for case in 0..5 {
        let mut decisions = canonical.state().decisions.clone();
        match case {
            0 => decisions.clear(),
            // The other accepted decisions and canonical source still exist.
            1 => {
                decisions.remove(1);
            }
            2 => {
                decisions[1].operation = OperationId::from_bytes(&[9; 16]).unwrap();
                decisions[1].facts.clear();
            }
            3 => {
                decisions[1].revision = revision(4);
                decisions[1].facts.clear();
            }
            4 => decisions[1].facts.clear(),
            _ => unreachable!(),
        }
        let current = checkpoint_with_decisions(&fixture, decisions);
        let unchanged = current.clone();
        for disposition in [ThreadDisposition::Continue, ThreadDisposition::Resolve] {
            let selections = [ThreadConsequenceSelection {
                thread: &fixture.narrative.open_threads[0],
                event_definition: &event,
                source: fact_id(2),
                disposition,
            }];
            assert_eq!(
                stage_checkpoint_thread_progress(
                    &current,
                    checkpoint_request(&fixture, &current, &selections),
                    limits(),
                ),
                Err(CheckpointProgressError::Progress(
                    ProgressError::SourceFactMismatch
                )),
                "owner case {case}, disposition {disposition:?}",
            );
            assert_eq!(current, unchanged);
        }
        // This boundary qualifies selected events; it does not reconstruct historical causes.
        let retained = stage_checkpoint_thread_progress(
            &current,
            checkpoint_request(&fixture, &current, &[]),
            limits(),
        )
        .unwrap();
        assert_eq!(retained.checkpoint, unchanged);
        assert!(retained.evidence.is_empty());
    }
}

#[test]
fn checkpoint_selection_checks_ordinal_membership_and_accepts_nonterminal_historical_cause() {
    let mut fixture = Fixture::new();
    let canonical = checkpoint(&fixture);
    let mut decisions = canonical.state().decisions.clone();
    fixture.facts[1].revision = revision(3);
    fixture.facts[2].operation = fixture.facts[1].operation;
    fixture.facts[2].ordinal = 1;
    decisions[1].revision = revision(3);
    decisions[1].facts.push(fact_id(3));
    decisions.pop();
    // The selected event is ordinal zero, before its owner's terminal event, and
    // cites a historical fact from another operation. Neither changes its eligibility.
    let current = checkpoint_with_decisions(&fixture, decisions.clone());
    let unchanged = current.clone();
    let event = reference("resolved-event");
    for (source, ordinal, cause) in [(fact_id(2), 0, fact_id(1)), (fact_id(3), 1, fact_id(2))] {
        let selections = [ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[0],
            event_definition: &event,
            source,
            disposition: ThreadDisposition::Continue,
        }];
        let proposed = stage_checkpoint_thread_progress(
            &current,
            checkpoint_request(&fixture, &current, &selections),
            limits(),
        )
        .unwrap();
        assert_eq!(proposed.evidence[0].ordinal, ordinal);
        assert_eq!(proposed.evidence[0].cause, Some(cause));
        assert_eq!(proposed.evidence[0].revision, revision(3));
        assert_eq!(
            proposed.checkpoint.state().narrative.accepted_facts,
            [fact_id(1), source]
        );
        assert_eq!(
            proposed.checkpoint.state().narrative.open_threads,
            fixture.narrative.open_threads
        );
        let mut protected = proposed.checkpoint.state().clone();
        protected.narrative = current.state().narrative.clone();
        assert_eq!(protected, *current.state());
        assert_eq!(current, unchanged);
    }
    // Model preserves set membership but does not require accepted ordinal order.
    // A reversed owner list must not admit either selected event at the wrong slot.
    decisions[1].facts.reverse();
    let reversed = checkpoint_with_decisions(&fixture, decisions);
    let unchanged = reversed.clone();
    for source in [fact_id(2), fact_id(3)] {
        let selections = [ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[0],
            event_definition: &event,
            source,
            disposition: ThreadDisposition::Resolve,
        }];
        assert_eq!(
            stage_checkpoint_thread_progress(
                &reversed,
                checkpoint_request(&fixture, &reversed, &selections),
                limits(),
            ),
            Err(CheckpointProgressError::Progress(
                ProgressError::SourceFactMismatch
            )),
        );
        assert_eq!(reversed, unchanged);
    }
}

#[test]
fn checkpoint_selection_invalid_later_owner_discards_whole_candidate() {
    let fixture = Fixture::new();
    let canonical = checkpoint(&fixture);
    let mut decisions = canonical.state().decisions.clone();
    decisions.pop();
    let current = checkpoint_with_decisions(&fixture, decisions);
    let unchanged = current.clone();
    let event = reference("resolved-event");
    let selections = [
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[0],
            event_definition: &event,
            source: fact_id(2),
            disposition: ThreadDisposition::Resolve,
        },
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[1],
            event_definition: &event,
            source: fact_id(3),
            disposition: ThreadDisposition::Continue,
        },
    ];
    assert_eq!(
        stage_checkpoint_thread_progress(
            &current,
            checkpoint_request(&fixture, &current, &selections),
            limits(),
        ),
        Err(CheckpointProgressError::Progress(
            ProgressError::SourceFactMismatch
        )),
    );
    assert_eq!(current, unchanged);
}

#[test]
fn checkpoint_selection_bounds_decision_records_and_ownership_work() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let unchanged = current.clone();
    let event = reference("resolved-event");
    let selections = [ThreadConsequenceSelection {
        thread: &fixture.narrative.open_threads[0],
        event_definition: &event,
        source: fact_id(2),
        disposition: ThreadDisposition::Continue,
    }];
    // Twelve comparisons admit these references and find the source. The next
    // decision comparison exceeds the admitted budget before any narrative clone.
    let bounded = ProgressLimits {
        work: 12,
        ..limits()
    };
    assert_eq!(
        stage_checkpoint_thread_progress(
            &current,
            checkpoint_request(&fixture, &current, &selections),
            bounded,
        ),
        Err(CheckpointProgressError::Progress(ProgressError::Capacity)),
    );
    assert_eq!(current, unchanged);
    let mut decisions = current.state().decisions.clone();
    for value in 10..24 {
        decisions.push(AcceptedDecision {
            operation: OperationId::from_bytes(&[value; 16]).unwrap(),
            revision: revision(7),
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-unrelated-policy"),
            semantic_output: None,
        });
    }
    assert_eq!(decisions.len(), limits().records + 1);
    let excessive = checkpoint_with_decisions(&fixture, decisions);
    let unchanged = excessive.clone();
    assert_eq!(
        stage_checkpoint_thread_progress(
            &excessive,
            checkpoint_request(&fixture, &excessive, &selections),
            limits(),
        ),
        Err(CheckpointProgressError::Progress(ProgressError::Capacity)),
    );
    assert_eq!(excessive, unchanged);
}

struct ThreadSourceRecord {
    thread: ContentReference,
    source: GameFact,
    disposition: ThreadDisposition,
    ending_required: bool,
    ending_policy: Option<ContentReference>,
}

struct ThreadSourceFixture {
    basis: Basis,
    pins: CheckpointPins,
    policy: RevisionLabel,
    records: Vec<ThreadSourceRecord>,
}

impl ThreadSourceFixture {
    fn new(fixture: &Fixture, current: &Checkpoint, disposition: ThreadDisposition) -> Self {
        Self {
            basis: current.basis(),
            pins: current.pins().clone(),
            policy: fixture.policy.clone(),
            records: (0..2)
                .map(|index| ThreadSourceRecord {
                    thread: fixture.narrative.open_threads[index].clone(),
                    source: fixture.facts[index + 1].clone(),
                    disposition,
                    ending_required: false,
                    ending_policy: None,
                })
                .collect(),
        }
    }

    fn admit(
        &self,
        current: &Checkpoint,
        policy: &RevisionLabel,
        selection: &ThreadConsequenceSelection<'_>,
        source: &GameFact,
    ) -> Result<Option<ContentReference>, CheckpointProgressError> {
        if current.basis() != self.basis || current.pins() != &self.pins || policy != &self.policy {
            return Err(CheckpointProgressError::SourceAdmission);
        }
        let record = self.records.iter().find(|record| {
            record.thread == *selection.thread
                && record.source == *source
                && record.source.id == selection.source
                && record.disposition == selection.disposition
                && matches!(
                    &record.source.value,
                    FactValue::ContentEvent { definition, .. } if definition == selection.event_definition
                )
        }).ok_or(CheckpointProgressError::SourceAdmission)?;
        if record.ending_required && record.ending_policy.is_none() {
            return Err(CheckpointProgressError::UnsupportedEndingPolicy);
        }
        Ok(record.ending_policy.clone())
    }
}

#[test]
fn source_owned_ordinary_resolution_preserves_every_ongoing_sibling() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let before = current.clone();
    let owner = ThreadSourceFixture::new(&fixture, &current, ThreadDisposition::Resolve);
    let event = reference("resolved-event");
    let selections = [ThreadConsequenceSelection {
        thread: &fixture.narrative.open_threads[0],
        event_definition: &event,
        source: fact_id(2),
        disposition: ThreadDisposition::Resolve,
    }];
    let propose = || {
        checkpoint_request(&fixture, &current, &selections).stage_source_admitted(
            &current,
            limits(),
            |current, policy, selection, source| owner.admit(current, policy, selection, source),
        )
    };
    let result = propose().unwrap();
    assert_eq!(result, propose().unwrap());
    assert_eq!(current, before);
    assert_eq!(
        result.checkpoint.state().narrative.open_threads,
        [reference("mystery"), reference("rival")]
    );
    assert_eq!(result.checkpoint.state().narrative.remaining_budget, 43);
    assert!(
        result
            .evidence
            .iter()
            .all(|item| item.ending_policy.is_none())
    );
    let mut protected = result.checkpoint.state().clone();
    protected.narrative = before.state().narrative.clone();
    assert_eq!(protected, *before.state());
}

#[test]
fn source_owned_continue_retains_causal_progress_without_ending_eligibility() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let before = current.clone();
    let owner = ThreadSourceFixture::new(&fixture, &current, ThreadDisposition::Continue);
    let event = reference("resolved-event");
    let selections = [ThreadConsequenceSelection {
        thread: &fixture.narrative.open_threads[0],
        event_definition: &event,
        source: fact_id(2),
        disposition: ThreadDisposition::Continue,
    }];
    let result = checkpoint_request(&fixture, &current, &selections)
        .stage_source_admitted(&current, limits(), |current, policy, selection, source| {
            owner.admit(current, policy, selection, source)
        })
        .unwrap();
    assert_eq!(
        result.checkpoint.state().narrative.open_threads,
        fixture.narrative.open_threads
    );
    assert_eq!(
        result.checkpoint.state().narrative.accepted_facts,
        [fact_id(1), fact_id(2)]
    );
    assert_eq!(result.evidence[0].operation, fixture.facts[1].operation);
    assert!(result.evidence[0].ending_policy.is_none());
    assert_eq!(current, before);
    assert_eq!(
        checkpoint_request(&fixture, &current, &selections).stage_source_admitted(
            &current,
            limits(),
            |current, policy, selection, source| {
                owner.admit(current, policy, selection, source)?;
                Ok(Some(reference("campaign")))
            },
        ),
        Err(CheckpointProgressError::InvalidEndingDisposition)
    );
    assert_eq!(current, before);
}

#[test]
fn authored_ending_eligibility_is_exact_detached_causal_evidence() {
    let mut fixture = Fixture::new();
    let ending = reference("authored-ending-policy");
    fixture.admitted.push(ending.clone());
    let current = checkpoint(&fixture);
    let before = current.clone();
    let mut owner = ThreadSourceFixture::new(&fixture, &current, ThreadDisposition::Resolve);
    owner.records[1].ending_required = true;
    owner.records[1].ending_policy = Some(ending.clone());
    let event = reference("resolved-event");
    // Canonical committed order must survive reverse producer order.
    let selections = [
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[1],
            event_definition: &event,
            source: fact_id(3),
            disposition: ThreadDisposition::Resolve,
        },
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[0],
            event_definition: &event,
            source: fact_id(2),
            disposition: ThreadDisposition::Resolve,
        },
    ];
    let result = checkpoint_request(&fixture, &current, &selections)
        .stage_source_admitted(&current, limits(), |current, policy, selection, source| {
            owner.admit(current, policy, selection, source)
        })
        .unwrap();
    assert_eq!(result.evidence[0].source, fact_id(2));
    assert!(result.evidence[0].ending_policy.is_none());
    assert_eq!(result.evidence[1].source, fact_id(3));
    assert_eq!(result.evidence[1].ordinal, fixture.facts[2].ordinal);
    assert_eq!(result.evidence[1].cause, Some(fact_id(2)));
    assert_eq!(result.evidence[1].ending_policy, Some(ending));
    assert_eq!(
        result.checkpoint.state().narrative.open_threads,
        [reference("rival")]
    );
    assert_eq!(current, before);
    let mut protected = result.checkpoint.state().clone();
    protected.narrative = before.state().narrative.clone();
    assert_eq!(protected, *before.state());
}

#[test]
fn absent_withdrawn_or_unadmitted_ending_policy_refuses_without_closure() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let before = current.clone();
    let event = reference("resolved-event");
    let selections = [ThreadConsequenceSelection {
        thread: &fixture.narrative.open_threads[1],
        event_definition: &event,
        source: fact_id(3),
        disposition: ThreadDisposition::Resolve,
    }];
    for case in 0..3 {
        let mut owner = ThreadSourceFixture::new(&fixture, &current, ThreadDisposition::Resolve);
        owner.records[1].ending_required = true;
        let expected = match case {
            0 => CheckpointProgressError::UnsupportedEndingPolicy,
            1 => {
                owner.records.pop();
                CheckpointProgressError::SourceAdmission
            }
            _ => {
                owner.records[1].ending_policy = Some(reference("unsupported-ending-policy"));
                CheckpointProgressError::Progress(ProgressError::UnadmittedContent)
            }
        };
        assert_eq!(
            checkpoint_request(&fixture, &current, &selections).stage_source_admitted(
                &current,
                limits(),
                |current, policy, selection, source| owner
                    .admit(current, policy, selection, source),
            ),
            Err(expected)
        );
        assert_eq!(current, before);
    }
}

#[test]
fn source_owner_rejects_wrong_thread_disposition_and_current_binding() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let before = current.clone();
    let event = reference("resolved-event");
    let selections = [ThreadConsequenceSelection {
        thread: &fixture.narrative.open_threads[0],
        event_definition: &event,
        source: fact_id(2),
        disposition: ThreadDisposition::Resolve,
    }];
    for case in 0..7 {
        let mut owner = ThreadSourceFixture::new(&fixture, &current, ThreadDisposition::Resolve);
        match case {
            0 => owner.records[0].thread = reference("mystery"),
            1 => owner.records[0].disposition = ThreadDisposition::Continue,
            2 => owner.basis.revision = revision(6),
            3 => owner.pins.content.package_digest = ContentDigest([9; 32]),
            4 => owner.policy = label("withdrawn-policy"),
            5 => owner.records[0].source.operation = OperationId::from_bytes(&[9; 16]).unwrap(),
            _ => owner.records[0].source.ordinal = 1,
        }
        assert_eq!(
            checkpoint_request(&fixture, &current, &selections).stage_source_admitted(
                &current,
                limits(),
                |current, policy, selection, source| owner
                    .admit(current, policy, selection, source),
            ),
            Err(CheckpointProgressError::SourceAdmission)
        );
        assert_eq!(current, before);
    }
}

#[test]
fn source_entry_reuses_canonical_owner_and_conflict_rejection_atomically() {
    let fixture = Fixture::new();
    let canonical = checkpoint(&fixture);
    let event = reference("resolved-event");
    let selections = [
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[0],
            event_definition: &event,
            source: fact_id(2),
            disposition: ThreadDisposition::Resolve,
        },
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[1],
            event_definition: &event,
            source: fact_id(3),
            disposition: ThreadDisposition::Resolve,
        },
    ];
    let mut decisions = canonical.state().decisions.clone();
    decisions.pop();
    let current = checkpoint_with_decisions(&fixture, decisions);
    let before = current.clone();
    let owner = ThreadSourceFixture::new(&fixture, &current, ThreadDisposition::Resolve);
    assert_eq!(
        checkpoint_request(&fixture, &current, &selections).stage_source_admitted(
            &current,
            limits(),
            |current, policy, selection, source| owner.admit(current, policy, selection, source),
        ),
        Err(CheckpointProgressError::Progress(
            ProgressError::SourceFactMismatch
        ))
    );
    assert_eq!(current, before);

    let owner = ThreadSourceFixture::new(&fixture, &canonical, ThreadDisposition::Resolve);
    let proposed = checkpoint_request(&fixture, &canonical, &selections)
        .stage_source_admitted(
            &canonical,
            limits(),
            |current, policy, selection, source| owner.admit(current, policy, selection, source),
        )
        .unwrap();
    let repeated = proposed.checkpoint;
    let owner = ThreadSourceFixture::new(&fixture, &repeated, ThreadDisposition::Resolve);
    assert_eq!(
        checkpoint_request(&fixture, &repeated, &selections).stage_source_admitted(
            &repeated,
            limits(),
            |current, policy, selection, source| owner.admit(current, policy, selection, source),
        ),
        Err(CheckpointProgressError::Progress(
            ProgressError::ClosedThread
        ))
    );
}

#[test]
fn final_source_refusal_capacity_and_candidate_validation_publish_no_partial_state() {
    let fixture = Fixture::new();
    let current = checkpoint(&fixture);
    let before = current.clone();
    let event = reference("resolved-event");
    let selections = [
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[0],
            event_definition: &event,
            source: fact_id(2),
            disposition: ThreadDisposition::Resolve,
        },
        ThreadConsequenceSelection {
            thread: &fixture.narrative.open_threads[1],
            event_definition: &event,
            source: fact_id(3),
            disposition: ThreadDisposition::Resolve,
        },
    ];
    for case in 0..4 {
        let mut owner = ThreadSourceFixture::new(&fixture, &current, ThreadDisposition::Resolve);
        let mut request = checkpoint_request(&fixture, &current, &selections);
        let mut bounded = limits();
        let expected = match case {
            0 => {
                owner.records.pop();
                CheckpointProgressError::SourceAdmission
            }
            1 => {
                bounded.consequences = 1;
                CheckpointProgressError::Progress(ProgressError::Capacity)
            }
            2 => {
                bounded.work = 0;
                CheckpointProgressError::Progress(ProgressError::Capacity)
            }
            _ => {
                request.checkpoint_limits.maximum_records = 1;
                CheckpointProgressError::InvalidCandidate(
                    df_model::checkpoint::CheckpointError::Capacity,
                )
            }
        };
        assert_eq!(
            request.stage_source_admitted(
                &current,
                bounded,
                |current, policy, selection, source| owner
                    .admit(current, policy, selection, source),
            ),
            Err(expected)
        );
        assert_eq!(current, before);
    }
}

#[test]
fn resolving_last_thread_or_completing_beats_never_infers_campaign_ending() {
    let mut fixture = Fixture::new();
    fixture.narrative.open_threads.truncate(1);
    fixture
        .narrative
        .completed_beats
        .append(&mut fixture.narrative.active_beats);
    let current = checkpoint(&fixture);
    let before = current.clone();
    let mut owner = ThreadSourceFixture::new(&Fixture::new(), &current, ThreadDisposition::Resolve);
    owner.records.truncate(1);
    let event = reference("resolved-event");
    let selections = [ThreadConsequenceSelection {
        thread: &fixture.narrative.open_threads[0],
        event_definition: &event,
        source: fact_id(2),
        disposition: ThreadDisposition::Resolve,
    }];
    let result = checkpoint_request(&fixture, &current, &selections)
        .stage_source_admitted(&current, limits(), |current, policy, selection, source| {
            owner.admit(current, policy, selection, source)
        })
        .unwrap();
    assert!(result.checkpoint.state().narrative.open_threads.is_empty());
    assert!(
        result
            .evidence
            .iter()
            .all(|item| item.ending_policy.is_none())
    );
    let selected = result.checkpoint;
    let no_input = checkpoint_request(&fixture, &selected, &[])
        .stage_source_admitted(&selected, limits(), |_, _, _, _| {
            panic!("Empty thread list cannot imply a source consequence")
        })
        .unwrap();
    assert_eq!(no_input.checkpoint, selected);
    assert!(no_input.evidence.is_empty());
    assert_eq!(current, before);
}
