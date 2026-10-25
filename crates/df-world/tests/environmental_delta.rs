use df_model::checkpoint::*;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use df_world::{
    AdmittedEnvironmentalChange, EnvironmentalDelta, EnvironmentalDeltaError,
    EnvironmentalDeltaLimits, stage_environmental_delta,
};

fn label(label_text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(label_text)).unwrap()
}
fn entity(repeated_byte: u8) -> EntityId {
    EntityId::from_bytes(&[repeated_byte; 16]).unwrap()
}
fn member(repeated_byte: u8) -> MemberId {
    MemberId::from_bytes(&[repeated_byte; 16]).unwrap()
}
fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}
fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: revision(2, 8),
    }
}
fn content() -> ContentReference {
    ContentReference {
        package: label("fixture-package-1"),
        entry: label("fixture-entry-1"),
    }
}
fn rule() -> RuleReference {
    RuleReference {
        catalog: label("fixture-catalog-1"),
        source: label("fixture-source-1"),
        entry: label("fixture-entry-1"),
        clause: label("fixture-clause-1"),
    }
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
fn resource_constraints() -> Vec<ResourceConstraint> {
    vec![ResourceConstraint {
        owner: entity(4),
        resource: label("fixture-resource-1"),
        minimum: 0,
        maximum: 8,
        source: rule(),
    }]
}
fn checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 512,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 8192,
        maximum_retained_bytes: 1024 * 1024,
    }
}
fn state() -> GameState {
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 9,
            ticks_per_second: 10,
        },
        members: vec![MembershipLink {
            member: member(3),
            character: Some(entity(4)),
        }],
        entities: vec![WorldEntity {
            id: entity(4),
            definition: content(),
            location: None,
            position: Some(Position { x: 0, y: 0, z: 0 }),
            identity_revision: label("fixture-entity-1"),
        }],
        characters: vec![CharacterState {
            entity: entity(4),
            build: content(),
            owner: member(3),
            choices: vec![],
        }],
        resources: vec![ResourceState {
            owner: entity(4),
            resource: label("fixture-resource-1"),
            value: 4,
            minimum: 0,
            maximum: 8,
            source: rule(),
        }],
        inventory: vec![],
        facts: vec![],
        draws: vec![],
        decisions: vec![],
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
        activity: vec![],
        tempo: TempoState {
            policy: content(),
            presentation_ticks: 0,
            intensity: 0,
            inertia: 0,
            fatigue: vec![],
        },
        presentation: vec![],
        continuity: continuity(),
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

fn fact_id(repeated_byte: u8) -> FactId {
    FactId::from_bytes(&[repeated_byte; 16]).unwrap()
}
fn storm() -> ContentReference {
    ContentReference {
        entry: label("fixture-storm-1"),
        ..content()
    }
}
fn environmental_state() -> GameState {
    let mut game_state = state();
    let mut second_entity = game_state.entities[0].clone();
    second_entity.id = entity(5);
    game_state.entities.push(second_entity);
    game_state.facts = vec![
        GameFact {
            id: fact_id(1),
            revision: revision(2, 7),
            operation: OperationId::from_bytes(&[1; 16]).unwrap(),
            ordinal: 0,
            cause: None,
            audience: AudienceScope::Host,
            value: FactValue::ContentEvent {
                definition: content(),
                subjects: vec![entity(4), entity(5)],
            },
        },
        GameFact {
            id: fact_id(2),
            revision: basis().revision,
            operation: OperationId::from_bytes(&[2; 16]).unwrap(),
            ordinal: 0,
            cause: Some(fact_id(1)),
            audience: AudienceScope::Host,
            value: FactValue::ContentEvent {
                definition: storm(),
                subjects: vec![entity(4), entity(5)],
            },
        },
        GameFact {
            id: fact_id(3),
            revision: basis().revision,
            operation: OperationId::from_bytes(&[2; 16]).unwrap(),
            ordinal: 1,
            cause: Some(fact_id(2)),
            audience: AudienceScope::Host,
            value: FactValue::ContentEvent {
                definition: storm(),
                subjects: vec![entity(4), entity(5)],
            },
        },
    ];
    game_state.decisions = vec![
        AcceptedDecision {
            operation: game_state.facts[0].operation,
            revision: game_state.facts[0].revision,
            facts: vec![fact_id(1)],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-environment-policy-1"),
            semantic_output: None,
        },
        AcceptedDecision {
            operation: game_state.facts[1].operation,
            revision: basis().revision,
            facts: vec![fact_id(2), fact_id(3)],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-environment-policy-1"),
            semantic_output: None,
        },
    ];
    game_state.continuity.environment = vec![EnvironmentalState {
        location: entity(4),
        definition: content(),
        change_facts: vec![fact_id(1)],
    }];
    game_state.continuity.scenes = vec![SceneIdentityRevision {
        scene: entity(4),
        revision: label("fixture-scene-1"),
        source_facts: vec![fact_id(1)],
        geometry: AssetReference {
            key: label("fixture-geometry-1"),
            digest: ContentDigest([6; 32]),
            byte_length: 128,
            kind: AssetKind::TacticalGeometry,
        },
        canonical_pack: label("fixture-pack-1"),
    }];
    game_state.continuity.canonical_packs = vec![CanonicalPack {
        revision: label("fixture-pack-1"),
        digest: ContentDigest([7; 32]),
        bible: VisualBible {
            revision: label("fixture-bible-1"),
            definition: content(),
            palette: vec![],
            style: String::new(),
            references: vec![],
        },
        identities: vec![],
    }];
    game_state
}
fn admitted_checkpoint(game_state: GameState) -> Result<Checkpoint, CheckpointError> {
    let assets: Vec<_> = game_state
        .continuity
        .scenes
        .iter()
        .map(|scene| scene.geometry.clone())
        .collect();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        game_state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content(), storm()],
            resources: &resource_constraints(),
            assets: &assets,
        },
        checkpoint_limits(),
    )
}
fn checkpoint(game_state: GameState) -> Checkpoint {
    admitted_checkpoint(game_state).unwrap()
}
fn replacement() -> EnvironmentalState {
    EnvironmentalState {
        location: entity(4),
        definition: storm(),
        change_facts: vec![fact_id(1), fact_id(2), fact_id(3)],
    }
}
fn limits() -> EnvironmentalDeltaLimits {
    EnvironmentalDeltaLimits {
        input_records: 256,
        changes: 4,
        output_bytes: 16 * 1024,
    }
}
fn change<'a>(
    checkpoint: &'a Checkpoint,
    replacement: &'a EnvironmentalState,
) -> AdmittedEnvironmentalChange<'a> {
    AdmittedEnvironmentalChange {
        expected: &checkpoint.state().continuity.environment[0],
        replacement,
        geometry: &checkpoint.state().continuity.scenes[0],
    }
}
fn stage<'a>(
    checkpoint: &'a Checkpoint,
    changes: &[AdmittedEnvironmentalChange<'_>],
) -> Result<EnvironmentalDelta<'a>, EnvironmentalDeltaError> {
    stage_environmental_delta(checkpoint, &pins(), basis(), Some(changes), limits())
}

#[test]
fn admitted_source_linked_change_is_owned_deterministic_and_preserves_checkpoint_time_and_facts() {
    let checkpoint = checkpoint(environmental_state());
    let before = checkpoint.clone();
    let replacement = replacement();
    let first = stage(&checkpoint, &[change(&checkpoint, &replacement)]).unwrap();
    assert_eq!(first.replacements, vec![replacement.clone()]);
    assert_eq!(first.basis, basis());
    assert_eq!(first.pins, checkpoint.pins());
    assert_eq!(
        stage(&checkpoint, &[change(&checkpoint, &replacement)]).unwrap(),
        first
    );
    assert_eq!(checkpoint, before);
}

#[test]
fn missing_admission_is_refused_and_empty_or_identical_admission_is_no_change() {
    let checkpoint = checkpoint(environmental_state());
    assert_eq!(
        stage_environmental_delta(&checkpoint, &pins(), basis(), None, limits()),
        Err(EnvironmentalDeltaError::NotAdmitted)
    );
    assert!(stage(&checkpoint, &[]).unwrap().replacements.is_empty());
    assert!(
        stage(
            &checkpoint,
            &[change(
                &checkpoint,
                &checkpoint.state().continuity.environment[0]
            )]
        )
        .unwrap()
        .replacements
        .is_empty()
    );
}

#[test]
fn full_current_basis_and_all_pin_groups_are_required() {
    let checkpoint = checkpoint(environmental_state());
    let replacement = replacement();
    let changes = [change(&checkpoint, &replacement)];
    for (expected, error) in [
        (
            Basis {
                session: SessionId::from_bytes(&[8; 16]).unwrap(),
                ..basis()
            },
            CheckpointError::WrongSession,
        ),
        (
            Basis {
                run: RunId::from_bytes(&[8; 16]).unwrap(),
                ..basis()
            },
            CheckpointError::WrongRun,
        ),
        (
            Basis {
                revision: revision(2, 7),
                ..basis()
            },
            CheckpointError::StaleBasis,
        ),
        (
            Basis {
                revision: revision(3, 8),
                ..basis()
            },
            CheckpointError::StaleBasis,
        ),
    ] {
        assert_eq!(
            stage_environmental_delta(&checkpoint, &pins(), expected, Some(&changes), limits()),
            Err(EnvironmentalDeltaError::Checkpoint(error))
        );
    }
    let mut rules = pins();
    rules.rules.handler_digest = ContentDigest([9; 32]);
    let mut content = pins();
    content.content.package_digest = ContentDigest([9; 32]);
    let mut build = pins();
    build.build = BuildIdentity::new(
        Some("other-source"),
        Some("native"),
        Some("wasm"),
        Some("config"),
        Some("content"),
    )
    .unwrap();
    for (admitted, error) in [
        (rules, CheckpointError::RulesMismatch),
        (content, CheckpointError::ContentMismatch),
        (build, CheckpointError::BuildMismatch),
    ] {
        assert_eq!(
            stage_environmental_delta(&checkpoint, &admitted, basis(), Some(&changes), limits()),
            Err(EnvironmentalDeltaError::Checkpoint(error))
        );
    }
}

#[test]
fn replacement_requires_current_location_environment_and_history_without_rewriting() {
    let checkpoint = checkpoint(environmental_state());
    let mut proposed = replacement();
    proposed.location = entity(5);
    assert_eq!(
        stage(&checkpoint, &[change(&checkpoint, &proposed)]),
        Err(EnvironmentalDeltaError::UnknownLocation)
    );
    let mut proposed = replacement();
    proposed.change_facts.remove(0);
    assert_eq!(
        stage(&checkpoint, &[change(&checkpoint, &proposed)]),
        Err(EnvironmentalDeltaError::StaleEnvironment)
    );
    let proposed = replacement();
    let mut old = checkpoint.state().continuity.environment[0].clone();
    old.definition = storm();
    assert_eq!(
        stage(
            &checkpoint,
            &[AdmittedEnvironmentalChange {
                expected: &old,
                ..change(&checkpoint, &proposed)
            }]
        ),
        Err(EnvironmentalDeltaError::StaleEnvironment)
    );
    old.location = entity(5);
    let mut proposed = replacement();
    proposed.location = entity(5);
    assert_eq!(
        stage(
            &checkpoint,
            &[AdmittedEnvironmentalChange {
                expected: &old,
                ..change(&checkpoint, &proposed)
            }]
        ),
        Err(EnvironmentalDeltaError::UnknownEnvironment)
    );
    old.location = entity(8);
    proposed.location = entity(8);
    assert_eq!(
        stage(
            &checkpoint,
            &[AdmittedEnvironmentalChange {
                expected: &old,
                ..change(&checkpoint, &proposed)
            }]
        ),
        Err(EnvironmentalDeltaError::UnknownLocation)
    );
}

#[test]
fn duplicate_or_ambiguous_location_records_are_refused_atomically() {
    let mut supplied = environmental_state();
    supplied
        .continuity
        .environment
        .push(supplied.continuity.environment[0].clone());
    let ambiguous = checkpoint(supplied);
    let proposed = replacement();
    assert_eq!(
        stage(&ambiguous, &[change(&ambiguous, &proposed)]),
        Err(EnvironmentalDeltaError::AmbiguousEnvironment)
    );
    let checkpoint = checkpoint(environmental_state());
    assert_eq!(
        stage(
            &checkpoint,
            &[
                change(&checkpoint, &proposed),
                change(&checkpoint, &proposed)
            ]
        ),
        Err(EnvironmentalDeltaError::DuplicateLocation)
    );
    assert_eq!(
        checkpoint.state().continuity.environment[0].definition,
        content()
    );
}

#[test]
fn exact_committed_scene_revision_geometry_asset_and_source_facts_are_required() {
    let checkpoint = checkpoint(environmental_state());
    let proposed = replacement();
    let original = &checkpoint.state().continuity.scenes[0];
    let mut variations = Vec::new();
    let mut scene = original.clone();
    scene.scene = entity(5);
    variations.push(scene);
    let mut scene = original.clone();
    scene.revision = label("other");
    variations.push(scene);
    let mut scene = original.clone();
    scene.geometry.digest = ContentDigest([9; 32]);
    variations.push(scene);
    let mut scene = original.clone();
    scene.geometry.key = label("other");
    variations.push(scene);
    let mut scene = original.clone();
    scene.geometry.byte_length += 1;
    variations.push(scene);
    let mut scene = original.clone();
    scene.geometry.kind = AssetKind::Image;
    variations.push(scene);
    let mut scene = original.clone();
    scene.source_facts = vec![fact_id(2)];
    variations.push(scene);
    let mut scene = original.clone();
    scene.canonical_pack = label("other");
    variations.push(scene);
    for scene in variations {
        assert_eq!(
            stage(
                &checkpoint,
                &[AdmittedEnvironmentalChange {
                    geometry: &scene,
                    ..change(&checkpoint, &proposed)
                }]
            ),
            Err(EnvironmentalDeltaError::StaleGeometry)
        );
    }
}

#[test]
fn missing_ambiguous_and_non_tactical_geometry_are_refused() {
    let proposed = replacement();
    let mut supplied = environmental_state();
    supplied.continuity.scenes.clear();
    let missing = checkpoint(supplied);
    let scene = environmental_state().continuity.scenes.remove(0);
    let admission = AdmittedEnvironmentalChange {
        expected: &missing.state().continuity.environment[0],
        replacement: &proposed,
        geometry: &scene,
    };
    assert_eq!(
        stage(&missing, &[admission]),
        Err(EnvironmentalDeltaError::UnknownGeometry)
    );
    let mut supplied = environmental_state();
    supplied.continuity.scenes.push(scene.clone());
    let ambiguous = checkpoint(supplied);
    assert_eq!(
        stage(&ambiguous, &[change(&ambiguous, &proposed)]),
        Err(EnvironmentalDeltaError::AmbiguousGeometry)
    );
    let mut supplied = environmental_state();
    supplied.continuity.scenes[0].geometry.kind = AssetKind::Image;
    let unsupported = checkpoint(supplied);
    assert_eq!(
        stage(&unsupported, &[change(&unsupported, &proposed)]),
        Err(EnvironmentalDeltaError::UnsupportedGeometry)
    );
    let mut supplied = environmental_state();
    supplied.continuity.scenes[0].geometry.byte_length = 0;
    assert_eq!(
        admitted_checkpoint(supplied),
        Err(CheckpointError::InvalidReference)
    );
}

#[test]
fn stale_definition_unknown_duplicate_reordered_and_absent_fact_causes_are_refused() {
    let checkpoint = checkpoint(environmental_state());
    let mut stale = replacement();
    stale.definition.package = label("other-package");
    let mut unknown = replacement();
    unknown.change_facts.push(fact_id(8));
    let mut duplicate = replacement();
    duplicate.change_facts.push(fact_id(2));
    let mut reversed = replacement();
    reversed.change_facts.swap(1, 2);
    let mut missing = replacement();
    missing.change_facts.truncate(1);
    for (proposed, error) in [
        (stale, EnvironmentalDeltaError::StaleSource),
        (unknown, EnvironmentalDeltaError::UnknownFact),
        (duplicate, EnvironmentalDeltaError::DuplicateFact),
        (reversed, EnvironmentalDeltaError::InvalidFactOrder),
        (missing, EnvironmentalDeltaError::MissingCause),
    ] {
        assert_eq!(
            stage(&checkpoint, &[change(&checkpoint, &proposed)]),
            Err(error)
        );
    }
}

#[test]
fn source_fact_requires_exact_content_event_definition_and_affected_location() {
    let proposed = replacement();
    let mut wrong_definition = environmental_state();
    wrong_definition.facts[1].value = FactValue::ContentEvent {
        definition: content(),
        subjects: vec![entity(4)],
    };
    let mut wrong_location = environmental_state();
    wrong_location.facts[1].value = FactValue::ContentEvent {
        definition: storm(),
        subjects: vec![entity(5)],
    };
    let mut wrong_kind = environmental_state();
    wrong_kind.facts[1].value = FactValue::EntityCreated {
        entity: entity(4),
        definition: storm(),
    };
    for supplied in [wrong_definition, wrong_location, wrong_kind] {
        let checkpoint = checkpoint(supplied);
        assert_eq!(
            stage(&checkpoint, &[change(&checkpoint, &proposed)]),
            Err(EnvironmentalDeltaError::InvalidFactSource)
        );
    }
}

#[test]
fn historical_or_unaccepted_fact_cannot_authorize_current_environment_change() {
    let proposed = replacement();
    let mut supplied = environmental_state();
    supplied.facts[1].revision = revision(2, 7);
    supplied.facts[2].revision = revision(2, 7);
    supplied.decisions[1].revision = revision(2, 7);
    let historical = checkpoint(supplied);
    assert_eq!(
        stage(&historical, &[change(&historical, &proposed)]),
        Err(EnvironmentalDeltaError::StaleFact)
    );
    let mut supplied = environmental_state();
    supplied.decisions[1].facts.clear();
    let unaccepted = checkpoint(supplied);
    assert_eq!(
        stage(&unaccepted, &[change(&unaccepted, &proposed)]),
        Err(EnvironmentalDeltaError::FactNotAccepted)
    );
}

#[test]
fn invalid_causal_reference_is_rejected_at_canonical_checkpoint_admission() {
    let mut supplied = environmental_state();
    supplied.facts[1].cause = Some(fact_id(8));
    assert_eq!(
        admitted_checkpoint(supplied),
        Err(CheckpointError::InvalidReference)
    );
    let mut supplied = environmental_state();
    supplied.facts[1].cause = Some(fact_id(3));
    assert_eq!(
        admitted_checkpoint(supplied),
        Err(CheckpointError::InvalidReference)
    );
}

#[test]
fn canonical_output_order_is_independent_of_admission_order_and_last_refusal_discards_batch() {
    let mut supplied = environmental_state();
    let mut second = supplied.continuity.environment[0].clone();
    second.location = entity(5);
    supplied.continuity.environment.push(second);
    let mut scene = supplied.continuity.scenes[0].clone();
    scene.scene = entity(5);
    supplied.continuity.scenes.push(scene);
    let checkpoint = checkpoint(supplied);
    let first = replacement();
    let mut second = replacement();
    second.location = entity(5);
    let second_change = || AdmittedEnvironmentalChange {
        expected: &checkpoint.state().continuity.environment[1],
        replacement: &second,
        geometry: &checkpoint.state().continuity.scenes[1],
    };
    let ordered = stage(&checkpoint, &[change(&checkpoint, &first), second_change()]).unwrap();
    let shuffled = stage(&checkpoint, &[second_change(), change(&checkpoint, &first)]).unwrap();
    assert_eq!(ordered, shuffled);
    assert_eq!(ordered.replacements, vec![first.clone(), second.clone()]);
    second.change_facts.push(fact_id(8));
    assert_eq!(
        stage(
            &checkpoint,
            &[
                change(&checkpoint, &first),
                AdmittedEnvironmentalChange {
                    expected: &checkpoint.state().continuity.environment[1],
                    replacement: &second,
                    geometry: &checkpoint.state().continuity.scenes[1],
                }
            ]
        ),
        Err(EnvironmentalDeltaError::UnknownFact)
    );
    assert!(
        checkpoint
            .state()
            .continuity
            .environment
            .iter()
            .all(|environment| environment.definition == content())
    );
}

#[test]
fn committed_replacement_is_no_change_and_duplicate_fact_delivery_is_refused() {
    let proposed = replacement();
    let mut supplied = environmental_state();
    supplied.continuity.environment[0] = proposed.clone();
    let checkpoint = checkpoint(supplied);
    assert!(
        stage(&checkpoint, &[change(&checkpoint, &proposed)])
            .unwrap()
            .replacements
            .is_empty()
    );
    let mut duplicate = proposed.clone();
    duplicate.change_facts.push(fact_id(3));
    assert_eq!(
        stage(&checkpoint, &[change(&checkpoint, &duplicate)]),
        Err(EnvironmentalDeltaError::DuplicateFact)
    );
}

#[test]
fn input_work_and_owned_output_capacity_limits_are_explicit_and_exact() {
    let checkpoint = checkpoint(environmental_state());
    let proposed = replacement();
    let changes = [change(&checkpoint, &proposed)];
    for bounded in [
        EnvironmentalDeltaLimits {
            input_records: 0,
            ..limits()
        },
        EnvironmentalDeltaLimits {
            input_records: 4097,
            ..limits()
        },
        EnvironmentalDeltaLimits {
            changes: 0,
            ..limits()
        },
        EnvironmentalDeltaLimits {
            changes: 257,
            ..limits()
        },
        EnvironmentalDeltaLimits {
            output_bytes: 0,
            ..limits()
        },
    ] {
        assert_eq!(
            stage_environmental_delta(&checkpoint, &pins(), basis(), Some(&changes), bounded),
            Err(EnvironmentalDeltaError::InvalidLimits)
        );
    }
    assert_eq!(
        stage_environmental_delta(
            &checkpoint,
            &pins(),
            basis(),
            Some(&changes),
            EnvironmentalDeltaLimits {
                input_records: 4,
                changes: 1,
                ..limits()
            }
        ),
        Err(EnvironmentalDeltaError::InputCapacity)
    );
    let measured = stage(&checkpoint, &changes).unwrap().accounted_output_bytes;
    assert_eq!(
        stage_environmental_delta(
            &checkpoint,
            &pins(),
            basis(),
            Some(&changes),
            EnvironmentalDeltaLimits {
                output_bytes: measured - 1,
                ..limits()
            }
        ),
        Err(EnvironmentalDeltaError::OutputCapacity)
    );
    let exact = stage_environmental_delta(
        &checkpoint,
        &pins(),
        basis(),
        Some(&changes),
        EnvironmentalDeltaLimits {
            output_bytes: measured,
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(exact.accounted_output_bytes, measured);
    let mut large = proposed.clone();
    large.change_facts = vec![fact_id(1); 257];
    assert_eq!(
        stage(&checkpoint, &[change(&checkpoint, &large)]),
        Err(EnvironmentalDeltaError::InputCapacity)
    );
}
