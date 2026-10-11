//! B-F43-D01: real Engine caller for the canonical bounded scalar Tempo reducer.
//! Finite fixture coefficients are source-admitted examples, not production/device calibration.
use df_engine::director_staging::*;
use df_engine::tempo_staging::{TempoStageError, stage_tempo_candidate};
use df_model::checkpoint::*;
use df_tempo::elapsed::*;
use df_tempo::impulse::*;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use df_world::{DueSelectionLimits, DueSelectionRequest};
use std::time::Duration;

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}
fn content() -> ContentReference {
    ContentReference {
        package: label("fixture-package-1"),
        entry: label("fixture-entry-1"),
    }
}
fn revision(sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(2).unwrap(), sequence)
}
fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: revision(8),
    }
}
fn fact_id() -> FactId {
    FactId::from_bytes(&[5; 16]).unwrap()
}
fn pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: label("fixture-rules-1"),
            catalog: label("fixture-catalog-1"),
            catalog_digest: ContentDigest([1; 32]),
            source_manifest: label("fixture-sources-1"),
            source_manifest_digest: ContentDigest([2; 32]),
            handler: label("fixture-handler-1"),
            handler_digest: ContentDigest([3; 32]),
        },
        content: ContentPins {
            content: label("fixture-content-1"),
            content_digest: ContentDigest([4; 32]),
            package: label("fixture-package-1"),
            package_digest: ContentDigest([5; 32]),
        },
        build: BuildIdentity::new(
            Some("fixture-source-1"),
            Some("fixture-native-1"),
            Some("fixture-wasm-1"),
            Some("fixture-config-1"),
            Some("fixture-content-1"),
        )
        .unwrap(),
    }
}
fn continuity() -> ContinuityState {
    ContinuityState {
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
    }
}

fn checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 1024,
        maximum_retained_bytes: 1024 * 1024,
    }
}
fn checkpoint_for(
    mode: ExecutionMode,
    fatigue: Vec<(ContentReference, u64)>,
    presentation_ticks: u64,
) -> Checkpoint {
    let member = MemberId::from_bytes(&[3; 16]).unwrap();
    let operation = OperationId::from_bytes(&[4; 16]).unwrap();
    let state = GameState {
        mode,
        logical_time: LogicalTime {
            ticks: 9,
            ticks_per_second: 10,
        },
        members: vec![MembershipLink {
            member,
            character: None,
        }],
        entities: vec![],
        characters: vec![],
        resources: vec![],
        inventory: vec![],
        facts: vec![GameFact {
            id: fact_id(),
            revision: revision(8),
            operation,
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: content(),
                subjects: vec![],
            },
        }],
        draws: vec![],
        decisions: vec![AcceptedDecision {
            operation,
            revision: revision(8),
            facts: vec![fact_id()],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-policy-1"),
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
            definition: content(),
            active_beats: vec![],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 0,
        },
        encounters: vec![],
        activity: vec![ActivityWindow {
            member,
            started: LogicalTime {
                ticks: 1,
                ticks_per_second: 10,
            },
            ends: LogicalTime {
                ticks: 9,
                ticks_per_second: 10,
            },
            observed_facts: vec![fact_id()],
            spotlight_opt_in: false,
        }],
        tempo: TempoState {
            policy: content(),
            presentation_ticks,
            intensity: 31,
            inertia: -2,
            fatigue,
        },
        presentation: vec![],
        continuity: continuity(),
    };
    checkpoint_from(state)
}
fn checkpoint_from(state: GameState) -> Checkpoint {
    let mut contents = vec![content()];
    for (definition, _) in &state.tempo.fatigue {
        if !contents.contains(definition) {
            contents.push(definition.clone());
        }
    }
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        state,
        ReferenceInventory {
            rules: &[],
            content: &contents,
            resources: &[],
            assets: &[],
        },
        checkpoint_limits(),
    )
    .unwrap()
}
fn request(checkpoint: &Checkpoint, elapsed: Duration, paused: bool) -> TempoAdvanceRequest {
    TempoAdvanceRequest {
        expected_basis: checkpoint.basis(),
        observed_logical_time: checkpoint.state().logical_time,
        from_presentation_ticks: checkpoint.state().tempo.presentation_ticks,
        elapsed,
        paused,
    }
}
fn policy(checkpoint: &Checkpoint) -> TempoElapsedPolicy {
    TempoElapsedPolicy {
        definition: checkpoint.state().tempo.policy.clone(),
        ticks_per_second: 10,
        maximum_elapsed_ticks: 100,
    }
}
fn limits() -> TempoElapsedLimits {
    TempoElapsedLimits {
        maximum_checkpoint_bytes: 1024 * 1024,
        maximum_fatigue_entries: 10,
    }
}

fn impulse_policy(current: &Checkpoint) -> ImpulsePolicy {
    ImpulsePolicy {
        definition: current.state().tempo.policy.clone(),
        minimum_intensity: 0,
        maximum_intensity: 50,
        maximum_absolute_inertia: 20,
        recovery_units_per_tick: 1,
        rise_units_per_tick: 2,
        fall_units_per_tick: 3,
        deadband_units: 2,
        named_impulses: vec![NamedImpulseRule {
            event: content(),
            inertia_units: 40,
            rise_units_per_tick: 10,
            fall_units_per_tick: 10,
        }],
    }
}
fn input(current: &Checkpoint, target: i64, shock: bool) -> PermittedTempoInput {
    PermittedTempoInput {
        expected_basis: current.basis(),
        target_intensity: target,
        impulse: shock.then_some(ImpulseEvent {
            fact: fact_id(),
            at_presentation_ticks: current.state().tempo.presentation_ticks,
        }),
    }
}
fn impulse_limits() -> ImpulseLimits {
    ImpulseLimits {
        elapsed: limits(),
        maximum_named_impulses: 4,
    }
}
fn stage(
    current: &Checkpoint,
    target: i64,
    shock: bool,
    ticks: u64,
) -> (Checkpoint, ImpulseAdvance) {
    stage_tempo_candidate(
        current,
        current.pins(),
        request(current, Duration::from_millis(ticks * 100), false),
        (Some(&policy(current)), Some(&impulse_policy(current))),
        input(current, target, shock),
        (impulse_limits(), checkpoint_limits()),
        ReferenceInventory {
            rules: &[],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
    )
    .unwrap()
}
fn assert_only_tempo_changed(current: &Checkpoint, candidate: &Checkpoint) {
    assert_eq!(candidate.basis(), current.basis());
    assert_eq!(candidate.pins(), current.pins());
    assert_eq!(candidate.schema(), current.schema());
    let mut protected = candidate.state().clone();
    protected.tempo = current.state().tempo.clone();
    assert_eq!(protected, *current.state());
}

#[test]
fn actual_director_candidate_reaches_tempo_and_preserves_all_other_canonical_fields() {
    let current = checkpoint_for(ExecutionMode::Live, vec![(content(), 7)], 40);
    let original = current.clone();
    let directors = compose_director_candidates(
        &current,
        current.pins(),
        DueSelectionRequest {
            expected_basis: current.basis(),
            target_time: current.state().logical_time,
            paused: false,
            deadline_remaining: Duration::from_secs(1),
            policy: &content(),
        },
        DirectorCandidates {
            interaction: None,
            narrative: None,
        },
        DirectorLimits {
            maximum_checkpoint_bytes: 1024 * 1024,
            maximum_pass_bytes: 5 * 1024 * 1024,
            maximum_relationships: 10,
            world: DueSelectionLimits {
                queue_events: 10,
                selected_events: 10,
                output_bytes: 1024 * 1024,
            },
        },
    )
    .unwrap();
    let DirectorStaging::Staged(directors) = directors else {
        panic!("empty world reaches Tempo");
    };
    let selected = directors.candidate();
    let (candidate, advance) = stage(selected, 31, true, 10);
    assert_eq!(advance.basis, selected.basis());
    assert_eq!(advance.state, candidate.state().tempo);
    assert_eq!(advance.disposition, ElapsedDisposition::Advanced);
    assert_eq!(candidate.state().tempo.intensity, 50);
    assert_eq!(candidate.state().tempo.inertia, 10);
    assert_eq!(candidate.state().tempo.presentation_ticks, 50);
    assert_only_tempo_changed(selected, &candidate);
    assert_eq!(current, original);
}

#[test]
fn named_positive_negative_shocks_saturation_and_successive_curve_samples_remain_bounded() {
    let mut current = checkpoint_for(ExecutionMode::Live, vec![(content(), 7)], 40);
    for _ in 0..8 {
        let (candidate, advance) = stage(&current, 31, true, 10);
        assert_only_tempo_changed(&current, &candidate);
        assert!((-20..=20).contains(&advance.state.inertia));
        let curve = advance.curve.unwrap();
        assert_eq!(curve.ticks_per_second(), 10);
        for tick in curve.from_ticks()..=curve.through_ticks() {
            assert!((0..=50).contains(&curve.intensity_at(tick).unwrap()));
        }
        assert_eq!(curve.intensity_at(curve.from_ticks() - 1), None);
        assert_eq!(curve.intensity_at(curve.through_ticks() + 1), None);
        current = candidate;
    }
    let mut negative = impulse_policy(&current);
    negative.named_impulses[0].inertia_units = i64::MIN;
    let (candidate, advance) = stage_tempo_candidate(
        &current,
        current.pins(),
        request(&current, Duration::from_secs(1), false),
        (Some(&policy(&current)), Some(&negative)),
        input(&current, 0, true),
        (impulse_limits(), checkpoint_limits()),
        ReferenceInventory {
            rules: &[],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
    )
    .unwrap();
    assert_eq!(advance.state.intensity, 0);
    assert_eq!(advance.state.inertia, -10);
    assert_only_tempo_changed(&current, &candidate);
}

#[test]
fn controlled_slew_deadband_decay_and_old_request_reuse_follow_existing_policy() {
    let current = checkpoint_for(ExecutionMode::Live, vec![], 40);
    assert_eq!(stage(&current, 31, false, 10).1.state.intensity, 31);
    assert_eq!(stage(&current, 50, false, 1).1.state.intensity, 33);
    let (shocked, shock) = stage(&current, 31, true, 1);
    assert_eq!((shock.state.intensity, shock.state.inertia), (41, 19));
    assert_eq!(stage(&current, 31, true, 1), (shocked.clone(), shock));
    let (released, release) = stage(&shocked, 0, false, 10);
    assert_eq!((release.state.intensity, release.state.inertia), (19, 9));
    let (recovered, recovery) = stage(&released, 0, false, 10);
    assert_eq!((recovery.state.intensity, recovery.state.inertia), (9, 0));
    assert_eq!(stage(&recovered, 0, false, 10).1.state.intensity, 0);
    let repeat = stage_tempo_candidate(
        &shocked,
        shocked.pins(),
        request(&current, Duration::from_millis(100), false),
        (Some(&policy(&shocked)), Some(&impulse_policy(&shocked))),
        input(&current, 31, true),
        (impulse_limits(), checkpoint_limits()),
        ReferenceInventory {
            rules: &[],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
    );
    assert_eq!(
        repeat,
        Err(TempoStageError::Impulse(ImpulseError::Elapsed(
            TempoAdvanceError::StalePresentationAnchor
        )))
    );
}

#[test]
fn pause_replay_prepared_and_zero_duration_preserve_their_explicit_outcomes() {
    for (mode, paused, elapsed, disposition) in [
        (
            ExecutionMode::Live,
            true,
            Duration::MAX,
            ElapsedDisposition::Paused,
        ),
        (
            ExecutionMode::Replay,
            false,
            Duration::MAX,
            ElapsedDisposition::Replay,
        ),
        (
            ExecutionMode::PreparedOnly,
            false,
            Duration::from_secs(1),
            ElapsedDisposition::Advanced,
        ),
        (
            ExecutionMode::Live,
            false,
            Duration::ZERO,
            ElapsedDisposition::Unchanged,
        ),
    ] {
        let current = checkpoint_for(mode, vec![(content(), 7)], 40);
        let original = current.clone();
        let (candidate, advance) = stage_tempo_candidate(
            &current,
            current.pins(),
            request(&current, elapsed, paused),
            (Some(&policy(&current)), Some(&impulse_policy(&current))),
            input(&current, 31, disposition != ElapsedDisposition::Unchanged),
            (impulse_limits(), checkpoint_limits()),
            ReferenceInventory {
                rules: &[],
                content: &[content()],
                resources: &[],
                assets: &[],
            },
        )
        .unwrap();
        assert_eq!(advance.disposition, disposition);
        assert_only_tempo_changed(&current, &candidate);
        if disposition != ElapsedDisposition::Advanced {
            assert_eq!(candidate, current);
            assert_eq!(advance.curve, None);
        }
        assert_eq!(current, original);
    }
}

#[test]
fn stale_bindings_anchors_time_and_unavailable_policy_return_no_candidate() {
    let current = checkpoint_for(ExecutionMode::Live, vec![], 40);
    let original = current.clone();
    let elapsed = policy(&current);
    let impulse = impulse_policy(&current);
    let normal = request(&current, Duration::from_secs(1), false);
    let call = |pins: &CheckpointPins, request, policies, input| {
        stage_tempo_candidate(
            &current,
            pins,
            request,
            policies,
            input,
            (impulse_limits(), checkpoint_limits()),
            ReferenceInventory {
                rules: &[],
                content: &[content()],
                resources: &[],
                assets: &[],
            },
        )
    };
    let mut stale = normal;
    stale.expected_basis.revision = revision(7);
    assert!(matches!(
        call(
            current.pins(),
            stale,
            (Some(&elapsed), Some(&impulse)),
            input(&current, 31, true)
        ),
        Err(TempoStageError::Impulse(ImpulseError::Elapsed(
            TempoAdvanceError::Binding(_)
        )))
    ));
    let mut pins = current.pins().clone();
    pins.content.package = label("wrong-package");
    assert!(matches!(
        call(
            &pins,
            normal,
            (Some(&elapsed), Some(&impulse)),
            input(&current, 31, true)
        ),
        Err(TempoStageError::Impulse(ImpulseError::Elapsed(
            TempoAdvanceError::Binding(_)
        )))
    ));
    for (duration, expected) in [
        (Duration::from_nanos(1), TempoAdvanceError::FractionalTicks),
        (Duration::from_secs(11), TempoAdvanceError::ElapsedLimit),
    ] {
        assert_eq!(
            call(
                current.pins(),
                request(&current, duration, false),
                (Some(&elapsed), Some(&impulse)),
                input(&current, 31, true)
            ),
            Err(TempoStageError::Impulse(ImpulseError::Elapsed(expected)))
        );
    }
    stale = normal;
    stale.from_presentation_ticks -= 1;
    assert_eq!(
        call(
            current.pins(),
            stale,
            (Some(&elapsed), Some(&impulse)),
            input(&current, 31, true)
        ),
        Err(TempoStageError::Impulse(ImpulseError::Elapsed(
            TempoAdvanceError::StalePresentationAnchor
        )))
    );
    assert_eq!(
        call(
            current.pins(),
            normal,
            (None, Some(&impulse)),
            input(&current, 31, true)
        ),
        Err(TempoStageError::Impulse(ImpulseError::Elapsed(
            TempoAdvanceError::PolicyUnavailable
        )))
    );
    assert_eq!(
        call(
            current.pins(),
            normal,
            (Some(&elapsed), None),
            input(&current, 31, true)
        ),
        Err(TempoStageError::Impulse(ImpulseError::PolicyUnavailable))
    );
    assert_eq!(current, original);
}

#[test]
fn hidden_unapproved_stale_facts_and_invalid_targets_or_policies_refuse_atomically() {
    let current = checkpoint_for(ExecutionMode::Live, vec![], 40);
    let original = current.clone();
    let normal = request(&current, Duration::from_secs(1), false);
    let call = |checkpoint: &Checkpoint, admitted: &ImpulsePolicy, input| {
        stage_tempo_candidate(
            checkpoint,
            checkpoint.pins(),
            normal,
            (Some(&policy(checkpoint)), Some(admitted)),
            input,
            (impulse_limits(), checkpoint_limits()),
            ReferenceInventory {
                rules: &[],
                content: &[content()],
                resources: &[],
                assets: &[],
            },
        )
    };
    let mut invalid = impulse_policy(&current);
    invalid.maximum_absolute_inertia = -1;
    assert_eq!(
        call(&current, &invalid, input(&current, 31, true)),
        Err(TempoStageError::Impulse(ImpulseError::InvalidPolicy))
    );
    invalid = impulse_policy(&current);
    invalid.definition.entry = label("wrong-policy");
    assert_eq!(
        call(&current, &invalid, input(&current, 31, true)),
        Err(TempoStageError::Impulse(ImpulseError::PolicyMismatch))
    );
    invalid = impulse_policy(&current);
    invalid.named_impulses[0].event.entry = label("unapproved");
    assert_eq!(
        call(&current, &invalid, input(&current, 31, true)),
        Err(TempoStageError::Impulse(ImpulseError::UnapprovedImpulse))
    );
    assert_eq!(
        call(
            &current,
            &impulse_policy(&current),
            input(&current, 51, true)
        ),
        Err(TempoStageError::Impulse(ImpulseError::InvalidTarget))
    );
    for (audience, stale, expected) in [
        (AudienceScope::Host, false, ImpulseError::HiddenImpulse),
        (AudienceScope::Shared, true, ImpulseError::StaleFact),
    ] {
        let mut state = current.state().clone();
        state.facts[0].audience = audience;
        if stale {
            state.facts[0].revision = revision(7);
            state.decisions[0].revision = revision(7);
        }
        let checkpoint = checkpoint_from(state);
        assert_eq!(
            call(
                &checkpoint,
                &impulse_policy(&checkpoint),
                input(&checkpoint, 31, true)
            ),
            Err(TempoStageError::Impulse(expected))
        );
    }
    assert_eq!(current, original);
}

#[test]
fn capacity_and_final_checkpoint_inventory_rejection_discard_the_whole_tempo_candidate() {
    let current = checkpoint_for(ExecutionMode::Live, vec![], 40);
    let original = current.clone();
    let mut bounds = impulse_limits();
    bounds.elapsed.maximum_checkpoint_bytes = 1;
    let call = |limits, inventory| {
        stage_tempo_candidate(
            &current,
            current.pins(),
            request(&current, Duration::from_secs(1), false),
            (Some(&policy(&current)), Some(&impulse_policy(&current))),
            input(&current, 31, true),
            limits,
            inventory,
        )
    };
    let content = [content()];
    assert_eq!(
        call(
            (bounds, checkpoint_limits()),
            ReferenceInventory {
                rules: &[],
                content: &content,
                resources: &[],
                assets: &[]
            }
        ),
        Err(TempoStageError::Impulse(ImpulseError::Elapsed(
            TempoAdvanceError::Capacity
        )))
    );
    bounds = impulse_limits();
    bounds.maximum_named_impulses = 0;
    assert_eq!(
        call(
            (bounds, checkpoint_limits()),
            ReferenceInventory {
                rules: &[],
                content: &content,
                resources: &[],
                assets: &[]
            }
        ),
        Err(TempoStageError::Impulse(ImpulseError::Capacity))
    );
    assert!(matches!(
        call(
            (impulse_limits(), checkpoint_limits()),
            ReferenceInventory {
                rules: &[],
                content: &[],
                resources: &[],
                assets: &[]
            }
        ),
        Err(TempoStageError::InvalidCandidate(_))
    ));
    let mut checkpoint_bound = checkpoint_limits();
    checkpoint_bound.maximum_retained_bytes = 1;
    assert!(matches!(
        call(
            (impulse_limits(), checkpoint_bound),
            ReferenceInventory {
                rules: &[],
                content: &content,
                resources: &[],
                assets: &[]
            }
        ),
        Err(TempoStageError::InvalidCandidate(CheckpointError::Capacity))
    ));
    assert_eq!(current, original);
}

fn mechanical_checkpoint(mode: ExecutionMode) -> Checkpoint {
    let mut state = checkpoint_for(mode, vec![(content(), 7)], 40)
        .state()
        .clone();
    let entity = EntityId::from_bytes(&[16; 16]).unwrap();
    let member = MemberId::from_bytes(&[3; 16]).unwrap();
    let rule = RuleReference {
        catalog: pins().rules.catalog,
        source: label("fixture-source-1"),
        entry: label("fixture-rule-1"),
        clause: label("fixture-clause-1"),
    };
    state.entities.push(WorldEntity {
        id: entity,
        definition: content(),
        location: None,
        position: None,
        identity_revision: label("fixture-entity-1"),
    });
    state.resources.push(ResourceState {
        owner: entity,
        resource: label("fixture-resource-1"),
        value: 4,
        minimum: 0,
        maximum: 8,
        source: rule.clone(),
    });
    state.timers.push(OwnedTimer {
        id: TimerId::from_bytes(&[17; 16]).unwrap(),
        basis: basis(),
        generation: 1,
        due: LogicalTime {
            ticks: 10,
            ticks_per_second: 10,
        },
        source: rule.clone(),
        status: DurableStatus::Pending,
    });
    state.pending.push(PendingResolution {
        id: ResolutionId::from_bytes(&[18; 16]).unwrap(),
        basis: basis(),
        continuation: label("fixture-continuation-1"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[19; 16]).unwrap(),
            phase: TriggerPhase::BeforeDraw,
            causal_fact: fact_id(),
            source: rule.clone(),
            timer: Some(state.timers[0].id),
        },
        next: PendingInput::Reaction {
            remaining: vec![OfferedResponse {
                participant: member,
                offer: label("fixture-offer-1"),
                options: vec![label("fixture-option-1")],
                source: rule.clone(),
            }],
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![ResourceSpend {
            owner: entity,
            resource: label("fixture-resource-1"),
            amount: 1,
            source: rule.clone(),
        }],
        rulings: vec![],
    });
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        state,
        ReferenceInventory {
            rules: std::slice::from_ref(&rule),
            content: &[content()],
            resources: &[ResourceConstraint {
                owner: entity,
                resource: label("fixture-resource-1"),
                minimum: 0,
                maximum: 8,
                source: rule.clone(),
            }],
            assets: &[],
        },
        checkpoint_limits(),
    )
    .unwrap()
}

#[test]
fn existing_pending_reaction_legal_options_timers_resources_and_mode_survive_tempo_staging() {
    let current = mechanical_checkpoint(ExecutionMode::Live);
    let original = current.clone();
    let rule = current.state().resources[0].source.clone();
    let resource = ResourceConstraint {
        owner: current.state().resources[0].owner,
        resource: label("fixture-resource-1"),
        minimum: 0,
        maximum: 8,
        source: rule.clone(),
    };
    let (candidate, advance) = stage_tempo_candidate(
        &current,
        current.pins(),
        request(&current, Duration::from_secs(1), false),
        (Some(&policy(&current)), Some(&impulse_policy(&current))),
        input(&current, 31, true),
        (impulse_limits(), checkpoint_limits()),
        ReferenceInventory {
            rules: &[rule],
            content: &[content()],
            resources: &[resource],
            assets: &[],
        },
    )
    .unwrap();
    assert_eq!(advance.state.intensity, 50);
    assert_only_tempo_changed(&current, &candidate);
    assert_eq!(candidate.state().pending, current.state().pending);
    assert_eq!(candidate.state().timers, current.state().timers);
    assert_eq!(candidate.state().resources, current.state().resources);
    assert_eq!(current, original);
}
