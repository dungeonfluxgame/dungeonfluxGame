use df_model::checkpoint::*;
use df_types::{
    BuildIdentity, MemberId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
};
use df_world::{
    AdmittedScheduleDestination, DueSelectionError, DueSelectionLimits, DueSelectionRequest,
    ScheduleAdvancement, ScheduleAdvancementError, ScheduleAdvancementLimits,
    stage_schedule_advancement,
};
use std::time::Duration;

fn fixture_revision_label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}
fn fixture_entity_id(value: u8) -> EntityId {
    EntityId::from_bytes(&[value; 16]).unwrap()
}
fn fixture_member_id(value: u8) -> MemberId {
    MemberId::from_bytes(&[value; 16]).unwrap()
}
fn fixture_session_revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}
fn fixture_checkpoint_basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: fixture_session_revision(2, 8),
    }
}
fn fixture_content_reference() -> ContentReference {
    ContentReference {
        package: fixture_revision_label("fixture-package-1"),
        entry: fixture_revision_label("fixture-entry-1"),
    }
}
fn fixture_rule_reference() -> RuleReference {
    RuleReference {
        catalog: fixture_revision_label("fixture-catalog-1"),
        source: fixture_revision_label("fixture-source-1"),
        entry: fixture_revision_label("fixture-entry-1"),
        clause: fixture_revision_label("fixture-clause-1"),
    }
}
fn fixture_checkpoint_pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::Standard2024,
            ruleset: fixture_revision_label("fixture-rules-1"),
            catalog: fixture_revision_label("fixture-catalog-1"),
            catalog_digest: ContentDigest([1; 32]),
            source_manifest: fixture_revision_label("fixture-sources-1"),
            source_manifest_digest: ContentDigest([2; 32]),
            handler: fixture_revision_label("fixture-handler-1"),
            handler_digest: ContentDigest([3; 32]),
        },
        content: ContentPins {
            content: fixture_revision_label("fixture-content-1"),
            content_digest: ContentDigest([4; 32]),
            package: fixture_revision_label("fixture-package-1"),
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
fn fixture_resource_constraints() -> Vec<ResourceConstraint> {
    vec![ResourceConstraint {
        owner: fixture_entity_id(4),
        resource: fixture_revision_label("fixture-resource-1"),
        minimum: 0,
        maximum: 8,
        source: fixture_rule_reference(),
    }]
}
fn fixture_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 1024,
        maximum_retained_bytes: 1024 * 1024,
    }
}
fn fixture_game_state() -> GameState {
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 9,
            ticks_per_second: 10,
        },
        members: vec![MembershipLink {
            member: fixture_member_id(3),
            character: Some(fixture_entity_id(4)),
        }],
        entities: vec![WorldEntity {
            id: fixture_entity_id(4),
            definition: fixture_content_reference(),
            location: None,
            position: Some(Position { x: 0, y: 0, z: 0 }),
            identity_revision: fixture_revision_label("fixture-entity-1"),
        }],
        characters: vec![CharacterState {
            entity: fixture_entity_id(4),
            build: fixture_content_reference(),
            owner: fixture_member_id(3),
            choices: vec![],
        }],
        resources: vec![ResourceState {
            owner: fixture_entity_id(4),
            resource: fixture_revision_label("fixture-resource-1"),
            value: 4,
            minimum: 0,
            maximum: 8,
            source: fixture_rule_reference(),
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
            definition: fixture_content_reference(),
            active_beats: vec![],
            completed_beats: vec![],
            open_threads: vec![],
            accepted_facts: vec![],
            remaining_budget: 0,
        },
        encounters: vec![],
        activity: vec![],
        tempo: TempoState {
            policy: fixture_content_reference(),
            presentation_ticks: 0,
            intensity: 0,
            inertia: 0,
            fatigue: vec![],
        },
        presentation: vec![],
        continuity: fixture_continuity_state(),
    }
}
fn fixture_continuity_state() -> ContinuityState {
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

fn checkpoint_with_state(supplied: GameState) -> Checkpoint {
    let mut entries = vec![fixture_content_reference()];
    entries.extend(
        supplied
            .schedules
            .iter()
            .map(|event| event.definition.clone()),
    );
    if let Some(cursor) = &supplied.continuity.catch_up {
        entries.push(cursor.policy.clone());
    }
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        fixture_checkpoint_basis(),
        fixture_checkpoint_pins(),
        supplied,
        ReferenceInventory {
            rules: &[fixture_rule_reference()],
            content: &entries,
            resources: &fixture_resource_constraints(),
            assets: &[],
        },
        fixture_checkpoint_limits(),
    )
    .unwrap()
}
fn fixture_record_id(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}
fn logical_time(ticks: u64) -> LogicalTime {
    LogicalTime {
        ticks,
        ticks_per_second: 10,
    }
}
fn scheduled_event(ticks: u64, value: u8, entry: &str) -> ScheduledEvent {
    ScheduledEvent {
        id: fixture_record_id(value),
        entity: fixture_entity_id(4),
        due: logical_time(ticks),
        definition: ContentReference {
            entry: fixture_revision_label(entry),
            ..fixture_content_reference()
        },
    }
}
fn due_selection_request<'a>(
    target_tick: u64,
    policy: &'a ContentReference,
) -> DueSelectionRequest<'a> {
    DueSelectionRequest {
        expected_basis: fixture_checkpoint_basis(),
        target_time: logical_time(target_tick),
        paused: false,
        deadline_remaining: Duration::from_secs(1),
        policy,
    }
}
fn due_selection_limits() -> DueSelectionLimits {
    DueSelectionLimits {
        queue_events: 4,
        selected_events: 2,
        output_bytes: 1024 * 1024,
    }
}

fn schedule_advancement_limits() -> ScheduleAdvancementLimits {
    ScheduleAdvancementLimits {
        selection: due_selection_limits(),
        entities: 4,
        destinations: 4,
        movements: 2,
        output_bytes: 1024 * 1024,
    }
}
fn scheduled_game_state() -> GameState {
    let mut supplied = fixture_game_state();
    supplied.entities[0].location = Some(fixture_entity_id(5));
    for value in 5..=7 {
        supplied.entities.push(WorldEntity {
            id: fixture_entity_id(value),
            definition: fixture_content_reference(),
            location: None,
            position: None,
            identity_revision: fixture_revision_label("fixture-location-1"),
        });
    }
    supplied.schedules = vec![
        scheduled_event(10, 1, "fixture-entry-1"),
        scheduled_event(11, 2, "fixture-entry-1"),
        scheduled_event(12, 3, "fixture-entry-1"),
    ];
    supplied
}
fn admitted_destinations(definition: &ContentReference) -> Vec<AdmittedScheduleDestination<'_>> {
    vec![
        AdmittedScheduleDestination {
            event: fixture_record_id(1),
            definition,
            entity: fixture_entity_id(4),
            expected_location: Some(fixture_entity_id(5)),
            destination: fixture_entity_id(6),
        },
        AdmittedScheduleDestination {
            event: fixture_record_id(2),
            definition,
            entity: fixture_entity_id(4),
            expected_location: Some(fixture_entity_id(6)),
            destination: fixture_entity_id(7),
        },
        AdmittedScheduleDestination {
            event: fixture_record_id(3),
            definition,
            entity: fixture_entity_id(4),
            expected_location: Some(fixture_entity_id(7)),
            destination: fixture_entity_id(5),
        },
    ]
}
fn stage_schedule<'a>(
    checkpoint: &'a Checkpoint,
    target: u64,
    definition: &ContentReference,
    destinations: &[AdmittedScheduleDestination<'_>],
) -> Result<ScheduleAdvancement<'a>, ScheduleAdvancementError> {
    stage_schedule_advancement(
        checkpoint,
        &fixture_checkpoint_pins(),
        Some(due_selection_request(target, definition)),
        destinations,
        schedule_advancement_limits(),
    )
}

#[test]
fn only_admitted_game_time_stages_due_locations_and_never_mutates_basis() {
    let checkpoint = checkpoint_with_state(scheduled_game_state());
    let original = checkpoint.state().clone();
    let definition = fixture_content_reference();
    let destinations = admitted_destinations(&definition);
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &fixture_checkpoint_pins(),
            None,
            &destinations,
            schedule_advancement_limits()
        ),
        Err(ScheduleAdvancementError::TimeNotAccepted),
    );
    assert!(
        stage_schedule(&checkpoint, 9, &definition, &destinations)
            .unwrap()
            .movements
            .is_empty()
    );
    let proposal = stage_schedule(&checkpoint, 10, &definition, &destinations).unwrap();
    assert_eq!(proposal.movements.len(), 1);
    assert_eq!(proposal.movements[0].before, Some(fixture_entity_id(5)));
    assert_eq!(proposal.movements[0].after, fixture_entity_id(6));
    assert_eq!(proposal.due.proposed_time, logical_time(10));
    assert_eq!(checkpoint.state(), &original);
    assert_eq!(
        stage_schedule(&checkpoint, 10, &definition, &destinations).unwrap(),
        proposal
    );
}

#[test]
fn paused_campaign_retains_location_cursor_and_backlog_without_wall_time() {
    let mut supplied = scheduled_game_state();
    supplied.logical_time = logical_time(12);
    let checkpoint = checkpoint_with_state(supplied);
    let definition = fixture_content_reference();
    let destinations = admitted_destinations(&definition);
    let mut paused = due_selection_request(12, &definition);
    paused.paused = true;
    let proposal = stage_schedule_advancement(
        &checkpoint,
        &fixture_checkpoint_pins(),
        Some(paused),
        &destinations,
        schedule_advancement_limits(),
    )
    .unwrap();
    assert!(proposal.movements.is_empty());
    assert_eq!(proposal.due.cursor.last_processed, None);
    assert_eq!(
        proposal.due.cursor.pending_events,
        vec![
            fixture_record_id(1),
            fixture_record_id(2),
            fixture_record_id(3)
        ]
    );
    assert_eq!(
        checkpoint.state().entities[0].location,
        Some(fixture_entity_id(5))
    );
    let mut paused = due_selection_request(13, &definition);
    paused.paused = true;
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &fixture_checkpoint_pins(),
            Some(paused),
            &destinations,
            schedule_advancement_limits()
        ),
        Err(ScheduleAdvancementError::DueSelection(
            DueSelectionError::InvalidTime
        ))
    );
}

#[test]
fn ordered_movement_chain_preserves_future_events_and_resumes_committed_cursor() {
    let first_checkpoint = checkpoint_with_state(scheduled_game_state());
    let definition = fixture_content_reference();
    let destinations = admitted_destinations(&definition);
    let first = stage_schedule(&first_checkpoint, 12, &definition, &destinations).unwrap();
    assert_eq!(
        first
            .movements
            .iter()
            .map(|movement| movement.event)
            .collect::<Vec<_>>(),
        vec![fixture_record_id(1), fixture_record_id(2)]
    );
    assert_eq!(first.movements[1].before, Some(first.movements[0].after));
    assert_eq!(first.due.cursor.pending_events, vec![fixture_record_id(3)]);
    let mut committed = first_checkpoint.state().clone();
    committed.logical_time = first.due.proposed_time;
    committed.continuity.catch_up = Some(first.due.cursor.clone());
    committed.entities[0].location = Some(fixture_entity_id(7));
    let resumed = checkpoint_with_state(committed);
    let next = stage_schedule(&resumed, 12, &definition, &destinations).unwrap();
    assert_eq!(next.movements.len(), 1);
    assert_eq!(next.movements[0].event, fixture_record_id(3));
    assert_eq!(next.movements[0].before, Some(fixture_entity_id(7)));
    assert_eq!(next.due.remaining_due(), 0);
    let mut completed = resumed.state().clone();
    completed.continuity.catch_up = Some(next.due.cursor.clone());
    completed.entities[0].location = Some(fixture_entity_id(5));
    assert!(
        stage_schedule(
            &checkpoint_with_state(completed),
            12,
            &definition,
            &destinations
        )
        .unwrap()
        .movements
        .is_empty()
    );
}

#[test]
fn schedule_and_mapping_arrival_order_do_not_change_candidates() {
    let definition = fixture_content_reference();
    let mut mappings = admitted_destinations(&definition);
    let first_checkpoint = checkpoint_with_state(scheduled_game_state());
    let first = stage_schedule(&first_checkpoint, 12, &definition, &mappings).unwrap();
    let mut shuffled = scheduled_game_state();
    shuffled.schedules.reverse();
    mappings.reverse();
    assert_eq!(
        stage_schedule(&checkpoint_with_state(shuffled), 12, &definition, &mappings).unwrap(),
        first
    );
}

#[test]
fn missing_mapping_never_invents_location_and_keeps_other_selected_consequences() {
    let checkpoint = checkpoint_with_state(scheduled_game_state());
    let definition = fixture_content_reference();
    let proposal = stage_schedule(&checkpoint, 10, &definition, &[]).unwrap();
    assert!(proposal.movements.is_empty());
    assert_eq!(proposal.due.events[0].id, fixture_record_id(1));
    assert_eq!(
        checkpoint.state().entities[0].location,
        Some(fixture_entity_id(5))
    );
}

#[test]
fn invalid_destination_mapping_and_stale_location_refuse_the_whole_candidate() {
    let checkpoint = checkpoint_with_state(scheduled_game_state());
    let original = checkpoint.state().clone();
    let definition = fixture_content_reference();
    let mut mappings = admitted_destinations(&definition);
    mappings[1].expected_location = Some(fixture_entity_id(5));
    assert_eq!(
        stage_schedule(&checkpoint, 11, &definition, &mappings),
        Err(ScheduleAdvancementError::StaleLocation)
    );
    mappings[1].expected_location = Some(fixture_entity_id(6));
    mappings[0].destination = fixture_entity_id(99);
    assert_eq!(
        stage_schedule(&checkpoint, 10, &definition, &mappings),
        Err(ScheduleAdvancementError::UnknownEntity)
    );
    mappings[0].destination = fixture_entity_id(6);
    mappings[0].event = fixture_record_id(99);
    assert_eq!(
        stage_schedule(&checkpoint, 10, &definition, &mappings),
        Err(ScheduleAdvancementError::UnknownEvent)
    );
    mappings[0].event = fixture_record_id(2);
    assert_eq!(
        stage_schedule(&checkpoint, 10, &definition, &mappings),
        Err(ScheduleAdvancementError::DuplicateDestination)
    );
    mappings[0].event = fixture_record_id(1);
    mappings[0].entity = fixture_entity_id(5);
    assert_eq!(
        stage_schedule(&checkpoint, 10, &definition, &mappings),
        Err(ScheduleAdvancementError::StaleDestination)
    );
    mappings[0].entity = fixture_entity_id(4);
    let wrong = ContentReference {
        entry: fixture_revision_label("unrelated-entry"),
        ..fixture_content_reference()
    };
    mappings[0].definition = &wrong;
    assert_eq!(
        stage_schedule(&checkpoint, 10, &definition, &mappings),
        Err(ScheduleAdvancementError::StaleDestination)
    );
    assert_eq!(checkpoint.state(), &original);
}

#[test]
fn stale_basis_and_changed_content_are_not_reinterpreted_as_accepted_time() {
    let checkpoint = checkpoint_with_state(scheduled_game_state());
    let definition = fixture_content_reference();
    let mappings = admitted_destinations(&definition);
    let mut stale = due_selection_request(10, &definition);
    stale.expected_basis.revision = fixture_session_revision(2, 7);
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &fixture_checkpoint_pins(),
            Some(stale),
            &mappings,
            schedule_advancement_limits()
        ),
        Err(ScheduleAdvancementError::DueSelection(
            DueSelectionError::Checkpoint(CheckpointError::StaleBasis)
        ))
    );
    let mut changed = fixture_checkpoint_pins();
    changed.content.package_digest = ContentDigest([99; 32]);
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &changed,
            Some(due_selection_request(10, &definition)),
            &mappings,
            schedule_advancement_limits()
        ),
        Err(ScheduleAdvancementError::DueSelection(
            DueSelectionError::Checkpoint(CheckpointError::ContentMismatch)
        ))
    );
}

#[test]
fn cursor_with_missing_remaining_event_is_refused_before_movement() {
    let mut supplied = scheduled_game_state();
    supplied.logical_time = logical_time(11);
    supplied.continuity.catch_up = Some(CatchUpCursor {
        last_processed: Some(fixture_record_id(1)),
        pending_events: vec![],
        policy: fixture_content_reference(),
        time: logical_time(11),
    });
    let checkpoint = checkpoint_with_state(supplied);
    let definition = fixture_content_reference();
    assert_eq!(
        stage_schedule(
            &checkpoint,
            12,
            &definition,
            &admitted_destinations(&definition)
        ),
        Err(ScheduleAdvancementError::DueSelection(
            DueSelectionError::StaleCursor
        ))
    );
}

#[test]
fn output_and_movement_capacity_fail_atomically_instead_of_dropping_consequences() {
    let checkpoint = checkpoint_with_state(scheduled_game_state());
    let definition = fixture_content_reference();
    let mappings = admitted_destinations(&definition);
    let proposal = stage_schedule(&checkpoint, 11, &definition, &mappings).unwrap();
    let mut bounded = schedule_advancement_limits();
    bounded.output_bytes = proposal.accounted_output_bytes - 1;
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &fixture_checkpoint_pins(),
            Some(due_selection_request(11, &definition)),
            &mappings,
            bounded
        ),
        Err(ScheduleAdvancementError::Capacity)
    );
    bounded = schedule_advancement_limits();
    bounded.movements = 1;
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &fixture_checkpoint_pins(),
            Some(due_selection_request(11, &definition)),
            &mappings,
            bounded
        ),
        Err(ScheduleAdvancementError::Capacity)
    );
    bounded = schedule_advancement_limits();
    bounded.entities = 3;
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &fixture_checkpoint_pins(),
            Some(due_selection_request(11, &definition)),
            &mappings,
            bounded
        ),
        Err(ScheduleAdvancementError::Capacity)
    );
}

#[test]
fn maximum_logical_tick_is_supplied_exactly_and_oversize_counts_are_refused_before_allocation() {
    let mut supplied = scheduled_game_state();
    supplied.schedules.truncate(1);
    supplied.schedules[0].due = logical_time(u64::MAX);
    let checkpoint = checkpoint_with_state(supplied);
    let definition = fixture_content_reference();
    let mappings = admitted_destinations(&definition);
    let one = &mappings[..1];
    let proposal = stage_schedule(&checkpoint, u64::MAX, &definition, one).unwrap();
    assert_eq!(proposal.due.proposed_time.ticks, u64::MAX);
    assert_eq!(proposal.movements.len(), 1);
    for bound in [0, usize::MAX] {
        let mut invalid = schedule_advancement_limits();
        invalid.movements = bound;
        assert_eq!(
            stage_schedule_advancement(
                &checkpoint,
                &fixture_checkpoint_pins(),
                Some(due_selection_request(u64::MAX, &definition)),
                one,
                invalid
            ),
            Err(ScheduleAdvancementError::InvalidLimits)
        );
    }
}
