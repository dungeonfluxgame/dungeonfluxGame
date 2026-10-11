// Private bounded contract fixture; generic entity movement does not define faction membership.
use df_model::checkpoint::*;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use df_world::{
    AdmittedEnvironmentalChange, AdmittedScheduleDestination, DueSelectionLimits,
    DueSelectionRequest, EnvironmentalDelta, EnvironmentalDeltaError, EnvironmentalDeltaLimits,
    ScheduleAdvancement, ScheduleAdvancementError, ScheduleAdvancementLimits,
    stage_environmental_delta, stage_schedule_advancement,
};

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}
fn entity(value: u8) -> EntityId {
    EntityId::from_bytes(&[value; 16]).unwrap()
}
fn member(value: u8) -> MemberId {
    MemberId::from_bytes(&[value; 16]).unwrap()
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

fn fact_id(value: u8) -> FactId {
    FactId::from_bytes(&[value; 16]).unwrap()
}
fn storm() -> ContentReference {
    ContentReference {
        entry: label("fixture-storm-1"),
        ..content()
    }
}
fn environmental_state() -> GameState {
    let mut supplied = state();
    let mut second = supplied.entities[0].clone();
    second.id = entity(5);
    supplied.entities.push(second);
    supplied.facts = vec![
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
    supplied.decisions = vec![
        AcceptedDecision {
            operation: supplied.facts[0].operation,
            revision: supplied.facts[0].revision,
            facts: vec![fact_id(1)],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-environment-policy-1"),
            semantic_output: None,
        },
        AcceptedDecision {
            operation: supplied.facts[1].operation,
            revision: basis().revision,
            facts: vec![fact_id(2), fact_id(3)],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-environment-policy-1"),
            semantic_output: None,
        },
    ];
    supplied.continuity.environment = vec![EnvironmentalState {
        location: entity(4),
        definition: content(),
        change_facts: vec![fact_id(1)],
    }];
    supplied.continuity.scenes = vec![SceneIdentityRevision {
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
    supplied.continuity.canonical_packs = vec![CanonicalPack {
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
    supplied
}
fn admitted_checkpoint(supplied: GameState) -> Result<Checkpoint, CheckpointError> {
    let assets: Vec<_> = supplied
        .continuity
        .scenes
        .iter()
        .map(|scene| scene.geometry.clone())
        .collect();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        supplied,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content(), storm()],
            resources: &resource_constraints(),
            assets: &assets,
        },
        checkpoint_limits(),
    )
}
fn checkpoint(supplied: GameState) -> Checkpoint {
    admitted_checkpoint(supplied).unwrap()
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
        changes: 2,
        output_bytes: 64 * 1024,
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

fn record(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}

fn time(ticks: u64) -> LogicalTime {
    LogicalTime {
        ticks,
        ticks_per_second: 10,
    }
}

fn causal_state() -> GameState {
    let mut state = environmental_state();
    let mut faction = state.entities[0].clone();
    faction.id = entity(6);
    faction.location = Some(entity(4));
    state.entities.push(faction);
    state.entities[0].location = Some(entity(4));
    state.schedules = vec![
        ScheduledEvent {
            id: record(10),
            entity: entity(6),
            due: time(10),
            definition: content(),
        },
        ScheduledEvent {
            id: record(11),
            entity: entity(4),
            due: time(10),
            definition: content(),
        },
    ];
    state.continuity.travel.push(TravelLeg {
        id: record(12),
        travelers: vec![entity(4)],
        from: entity(4),
        destination: entity(5),
        route: content(),
        starts: time(9),
        arrives: time(10),
        source: rule(),
    });
    state
}

fn destinations(checkpoint: &Checkpoint) -> Vec<AdmittedScheduleDestination<'_>> {
    checkpoint
        .state()
        .schedules
        .iter()
        .map(|event| AdmittedScheduleDestination {
            event: event.id,
            definition: &event.definition,
            entity: event.entity,
            expected_location: Some(entity(4)),
            destination: entity(5),
        })
        .collect()
}

fn schedule_limits() -> ScheduleAdvancementLimits {
    ScheduleAdvancementLimits {
        selection: DueSelectionLimits {
            queue_events: 4,
            selected_events: 2,
            output_bytes: 64 * 1024,
        },
        entities: 4,
        destinations: 2,
        movements: 2,
        output_bytes: 64 * 1024,
    }
}

fn schedule<'a>(
    checkpoint: &'a Checkpoint,
    mappings: &[AdmittedScheduleDestination<'_>],
    paused: bool,
) -> Result<ScheduleAdvancement<'a>, ScheduleAdvancementError> {
    stage_schedule_advancement(
        checkpoint,
        checkpoint.pins(),
        Some(DueSelectionRequest {
            expected_basis: checkpoint.basis(),
            target_time: if paused {
                checkpoint.state().logical_time
            } else {
                time(10)
            },
            paused,
            deadline_remaining: std::time::Duration::from_secs(1),
            policy: &content(),
        }),
        mappings,
        schedule_limits(),
    )
}

#[test]
fn source_causal_environment_replacement_preserves_canonical_weather_fact_prefix() {
    let current = checkpoint(causal_state());
    let before = current.clone();
    let proposed = replacement();
    let delta = stage(&current, &[change(&current, &proposed)]).unwrap();
    assert_eq!(delta.replacements, vec![proposed]);
    assert!(
        delta.replacements[0]
            .change_facts
            .starts_with(&current.state().continuity.environment[0].change_facts)
    );
    assert_eq!(current, before);
}

#[test]
fn unaccepted_or_wrong_definition_weather_fact_refuses_whole_batch() {
    let proposed = replacement();
    let mut unaccepted = causal_state();
    unaccepted.decisions[1].facts.clear();
    let mut wrong = causal_state();
    wrong.facts[1].value = FactValue::ContentEvent {
        definition: content(),
        subjects: vec![entity(4)],
    };
    for (state, refusal) in [
        (unaccepted, EnvironmentalDeltaError::FactNotAccepted),
        (wrong, EnvironmentalDeltaError::InvalidFactSource),
    ] {
        let current = checkpoint(state);
        let before = current.clone();
        assert_eq!(
            stage(&current, &[change(&current, &proposed)]),
            Err(refusal)
        );
        assert_eq!(current, before);
    }
}

#[test]
fn stale_environment_geometry_basis_or_pins_refuses_without_mutation() {
    let current = checkpoint(causal_state());
    let before = current.clone();
    let proposed = replacement();
    let mut expected = current.state().continuity.environment[0].clone();
    expected.definition = storm();
    assert_eq!(
        stage(
            &current,
            &[AdmittedEnvironmentalChange {
                expected: &expected,
                ..change(&current, &proposed)
            }]
        ),
        Err(EnvironmentalDeltaError::StaleEnvironment)
    );
    let mut geometry = current.state().continuity.scenes[0].clone();
    geometry.revision = label("obsolete-scene");
    assert_eq!(
        stage(
            &current,
            &[AdmittedEnvironmentalChange {
                geometry: &geometry,
                ..change(&current, &proposed)
            }]
        ),
        Err(EnvironmentalDeltaError::StaleGeometry)
    );
    let changes = [change(&current, &proposed)];
    assert_eq!(
        stage_environmental_delta(
            &current,
            current.pins(),
            Basis {
                revision: revision(2, 7),
                ..basis()
            },
            Some(&changes),
            limits(),
        ),
        Err(EnvironmentalDeltaError::Checkpoint(
            CheckpointError::StaleBasis
        ))
    );
    let mut wrong_pins = pins();
    wrong_pins.rules.handler_digest = ContentDigest([9; 32]);
    assert_eq!(
        stage_environmental_delta(&current, &wrong_pins, basis(), Some(&changes), limits(),),
        Err(EnvironmentalDeltaError::Checkpoint(
            CheckpointError::RulesMismatch
        ))
    );
    assert_eq!(current, before);
}

#[test]
fn faction_entity_moves_only_for_exact_admitted_event_destination_and_accepted_time() {
    let current = checkpoint(causal_state());
    let before = current.clone();
    let mappings = destinations(&current);
    let delta = schedule(&current, &mappings, false).unwrap();
    assert_eq!(
        delta
            .movements
            .iter()
            .map(|move_| (move_.event, move_.entity, move_.before, move_.after))
            .collect::<Vec<_>>(),
        vec![
            (record(10), entity(6), Some(entity(4)), entity(5)),
            (record(11), entity(4), Some(entity(4)), entity(5)),
        ]
    );
    assert_eq!(delta.due.proposed_time, time(10));
    assert_eq!(delta.due.remaining_due(), 0);
    assert_eq!(current, before);
}

#[test]
fn paused_due_faction_and_travel_events_return_backlog_without_movement() {
    let mut state = causal_state();
    state.logical_time = time(10);
    let current = checkpoint(state);
    let before = current.clone();
    let delta = schedule(&current, &destinations(&current), true).unwrap();
    assert!(delta.movements.is_empty());
    assert!(delta.due.events.is_empty());
    assert_eq!(delta.due.remaining_due(), 2);
    assert_eq!(delta.due.cursor.last_processed, None);
    assert_eq!(delta.due.proposed_time, time(10));
    assert_eq!(current, before);
}

#[test]
fn travel_leg_preserves_route_source_travelers_and_resource_stock_through_due_proposal() {
    let current = checkpoint(causal_state());
    let travel = current.state().continuity.travel.clone();
    let stock = current.state().resources.clone();
    let delta = schedule(&current, &destinations(&current), false).unwrap();
    assert_eq!(delta.movements[1].entity, travel[0].travelers[0]);
    assert_eq!(delta.movements[1].after, travel[0].destination);
    assert_eq!(delta.due.proposed_time, travel[0].arrives);
    assert_eq!(current.state().continuity.travel, travel);
    assert_eq!(current.state().resources, stock);
    // A TravelLeg is canonical data, not an automatic due-event policy producer.
    let mut no_events = causal_state();
    no_events.schedules.clear();
    let current = checkpoint(no_events);
    let empty = schedule(&current, &[], false).unwrap();
    assert!(empty.movements.is_empty());
    assert_eq!(current.state().continuity.travel, travel);
}

#[test]
fn unaccepted_time_wrong_event_definition_or_previous_location_refuses() {
    let current = checkpoint(causal_state());
    let before = current.clone();
    let mut mappings = destinations(&current);
    assert_eq!(
        stage_schedule_advancement(&current, current.pins(), None, &mappings, schedule_limits(),),
        Err(ScheduleAdvancementError::TimeNotAccepted)
    );
    let wrong_definition = storm();
    mappings[0].definition = &wrong_definition;
    assert_eq!(
        schedule(&current, &mappings, false),
        Err(ScheduleAdvancementError::StaleDestination)
    );
    let mut mappings = destinations(&current);
    mappings[1].expected_location = Some(entity(5));
    assert_eq!(
        schedule(&current, &mappings, false),
        Err(ScheduleAdvancementError::StaleLocation)
    );
    assert_eq!(current, before);
}

#[test]
fn world_proposals_preserve_rules_resources_inventory_draws_and_source_bounds() {
    let current = checkpoint(causal_state());
    let resources = current.state().resources.clone();
    let inventory = current.state().inventory.clone();
    let draws = current.state().draws.clone();
    let proposed = replacement();
    stage(&current, &[change(&current, &proposed)]).unwrap();
    schedule(&current, &destinations(&current), false).unwrap();
    assert_eq!(current.state().resources, resources);
    assert_eq!(current.state().inventory, inventory);
    assert_eq!(current.state().draws, draws);
    assert_eq!(current.state().resources[0].minimum, 0);
    assert_eq!(current.state().resources[0].maximum, 8);
    assert_eq!(current.state().resources[0].source, rule());
}

#[test]
fn checkpoint_refuses_self_raised_resource_bounds_and_out_of_range_stock() {
    let mut raised = causal_state();
    raised.resources[0].maximum = 9;
    let mut overflow = causal_state();
    overflow.resources[0].value = 9;
    let mut deficit = causal_state();
    deficit.resources[0].value = -1;
    for state in [raised, overflow, deficit] {
        assert_eq!(
            admitted_checkpoint(state),
            Err(CheckpointError::InvalidResource)
        );
    }
}

#[test]
fn bounded_replay_is_deterministic_and_source_checkpoint_unchanged() {
    let current = checkpoint(causal_state());
    let before = current.clone();
    let mappings = destinations(&current);
    let first = schedule(&current, &mappings, false).unwrap();
    let mut shuffled = destinations(&current);
    shuffled.reverse();
    assert_eq!(schedule(&current, &shuffled, false).unwrap(), first);
    let proposed = replacement();
    let environmental = stage(&current, &[change(&current, &proposed)]).unwrap();
    assert_eq!(
        stage(&current, &[change(&current, &proposed)]).unwrap(),
        environmental
    );
    assert_eq!(first.movements.len(), 2);
    assert!(first.accounted_output_bytes <= 64 * 1024);
    assert!(environmental.accounted_output_bytes <= 64 * 1024);
    assert_eq!(current, before);
}

#[test]
fn contract_owner_and_gap_record_covers_exact_four_original_families() {
    let decision = include_str!("causal_state_contract.json");
    for family in ["faction", "weather", "travel", "resource"] {
        assert!(decision.contains(&format!("\"family\": \"{family}\"")));
    }
    // The JSON is a retained design decision, not a source-policy parser or runtime authority.
    assert!(decision.contains("\"new_public_production_contract\": false"));
    assert!(decision.contains("\"sole_commit_owner\": \"df-session\""));
    assert!(decision.contains("\"fixture_only\": true"));
}
