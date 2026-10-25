use df_model::checkpoint::*;
use df_types::{
    BuildIdentity, MemberId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
};
use df_world::{
    AdmittedScheduleDestination, DueSelection, DueSelectionError, DueSelectionLimits,
    DueSelectionRequest, ScheduleAdvancementError, ScheduleAdvancementLimits,
    ScheduledLocationChange, select_due_events, stage_schedule_advancement,
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
fn scheduled_game_state() -> GameState {
    let mut supplied = fixture_game_state();
    supplied.schedules = vec![
        scheduled_event(10, 3, "long-event"),
        scheduled_event(10, 1, "event"),
        scheduled_event(10, 2, "event"),
        scheduled_event(12, 4, "event"),
    ];
    supplied
}
fn selected_event_ids(selected: &DueSelection<'_>) -> Vec<RecordId> {
    selected.events.iter().map(|event| event.id).collect()
}

#[test]
fn paused_campaign_does_not_advance_for_positive_caller_time_budgets() {
    let mut supplied = scheduled_game_state();
    supplied.logical_time = logical_time(10);
    let checkpoint = checkpoint_with_state(supplied);
    let before = checkpoint.state().clone();
    let policy = fixture_content_reference();

    let mut short_budget = due_selection_request(10, &policy);
    short_budget.paused = true;
    short_budget.deadline_remaining = Duration::from_nanos(1);
    let short = select_due_events(
        &checkpoint,
        &fixture_checkpoint_pins(),
        short_budget,
        due_selection_limits(),
    )
    .unwrap();

    let mut long_budget = due_selection_request(10, &policy);
    long_budget.paused = true;
    long_budget.deadline_remaining = Duration::from_secs(60);
    let long = select_due_events(
        &checkpoint,
        &fixture_checkpoint_pins(),
        long_budget,
        due_selection_limits(),
    )
    .unwrap();

    assert_eq!(short, long);
    assert!(short.events.is_empty());
    assert_eq!(
        short.cursor.pending_events,
        vec![
            fixture_record_id(1),
            fixture_record_id(2),
            fixture_record_id(3)
        ]
    );
    assert_eq!(checkpoint.state(), &before);
    assert_eq!(checkpoint.state().logical_time, logical_time(10));
    assert_eq!(checkpoint.state().entities[0].location, None);
}

#[test]
fn pause_and_missing_time_admission_refuse_without_mutating_the_checkpoint() {
    let checkpoint = checkpoint_with_state(scheduled_game_state());
    let before = checkpoint.state().clone();
    let policy = fixture_content_reference();

    let mut paused_future = due_selection_request(11, &policy);
    paused_future.paused = true;
    assert_eq!(
        select_due_events(
            &checkpoint,
            &fixture_checkpoint_pins(),
            paused_future,
            due_selection_limits()
        ),
        Err(DueSelectionError::InvalidTime)
    );
    assert_eq!(
        stage_schedule_advancement(
            &checkpoint,
            &fixture_checkpoint_pins(),
            None,
            &[],
            schedule_advancement_limits()
        ),
        Err(ScheduleAdvancementError::TimeNotAccepted)
    );
    assert_eq!(checkpoint.state(), &before);
}

#[test]
fn stale_basis_pins_cursor_and_time_units_refuse_without_mutation() {
    let policy = fixture_content_reference();
    let current_checkpoint = checkpoint_with_state(scheduled_game_state());
    let before = current_checkpoint.state().clone();

    let mut stale_basis = due_selection_request(10, &policy);
    stale_basis.expected_basis.revision = fixture_session_revision(2, 9);
    assert!(matches!(
        select_due_events(
            &current_checkpoint,
            &fixture_checkpoint_pins(),
            stale_basis,
            due_selection_limits()
        ),
        Err(DueSelectionError::Checkpoint(_))
    ));

    let mut stale_pins = fixture_checkpoint_pins();
    stale_pins.content.content = fixture_revision_label("changed-content");
    assert!(matches!(
        select_due_events(
            &current_checkpoint,
            &stale_pins,
            due_selection_request(10, &policy),
            due_selection_limits()
        ),
        Err(DueSelectionError::Checkpoint(_))
    ));

    assert_eq!(
        select_due_events(
            &current_checkpoint,
            &fixture_checkpoint_pins(),
            due_selection_request(8, &policy),
            due_selection_limits()
        ),
        Err(DueSelectionError::InvalidTime)
    );
    let mut mismatched_units = due_selection_request(10, &policy);
    mismatched_units.target_time.ticks_per_second = 20;
    assert_eq!(
        select_due_events(
            &current_checkpoint,
            &fixture_checkpoint_pins(),
            mismatched_units,
            due_selection_limits()
        ),
        Err(DueSelectionError::InvalidTime)
    );

    let mut stale_cursor_state = scheduled_game_state();
    stale_cursor_state.logical_time = logical_time(10);
    stale_cursor_state.continuity.catch_up = Some(CatchUpCursor {
        last_processed: Some(fixture_record_id(1)),
        pending_events: vec![fixture_record_id(3), fixture_record_id(2)],
        policy: fixture_content_reference(),
        time: logical_time(10),
    });
    let stale_cursor_checkpoint = checkpoint_with_state(stale_cursor_state);
    let stale_cursor_before = stale_cursor_checkpoint.state().clone();
    assert_eq!(
        select_due_events(
            &stale_cursor_checkpoint,
            &fixture_checkpoint_pins(),
            due_selection_request(10, &policy),
            due_selection_limits()
        ),
        Err(DueSelectionError::StaleCursor)
    );
    assert_eq!(stale_cursor_checkpoint.state(), &stale_cursor_before);
    assert_eq!(current_checkpoint.state(), &before);
}

#[test]
fn accepted_unpaused_time_stages_ordered_destination_and_replays_exactly() {
    let mut supplied = scheduled_game_state();
    supplied.entities.push(WorldEntity {
        id: fixture_entity_id(5),
        definition: fixture_content_reference(),
        location: None,
        position: None,
        identity_revision: fixture_revision_label("fixture-destination-1"),
    });
    let checkpoint = checkpoint_with_state(supplied);
    let before = checkpoint.state().clone();
    let policy = fixture_content_reference();
    let definition = ContentReference {
        entry: fixture_revision_label("event"),
        ..fixture_content_reference()
    };
    let destinations = [AdmittedScheduleDestination {
        event: fixture_record_id(1),
        definition: &definition,
        entity: fixture_entity_id(4),
        expected_location: None,
        destination: fixture_entity_id(5),
    }];

    let staged = stage_schedule_advancement(
        &checkpoint,
        &fixture_checkpoint_pins(),
        Some(due_selection_request(10, &policy)),
        &destinations,
        schedule_advancement_limits(),
    )
    .unwrap();
    let replay = stage_schedule_advancement(
        &checkpoint,
        &fixture_checkpoint_pins(),
        Some(due_selection_request(10, &policy)),
        &destinations,
        schedule_advancement_limits(),
    )
    .unwrap();

    assert_eq!(
        selected_event_ids(&staged.due),
        vec![
            fixture_record_id(1),
            fixture_record_id(2),
            fixture_record_id(3)
        ]
    );
    assert_eq!(
        staged.movements,
        vec![ScheduledLocationChange {
            event: fixture_record_id(1),
            entity: fixture_entity_id(4),
            before: None,
            after: fixture_entity_id(5),
        }]
    );
    assert_eq!(staged, replay);
    assert_eq!(checkpoint.state(), &before);
    assert_eq!(checkpoint.state().logical_time, logical_time(9));
    assert_eq!(checkpoint.state().entities[0].location, None);
}

fn schedule_advancement_limits() -> ScheduleAdvancementLimits {
    ScheduleAdvancementLimits {
        selection: DueSelectionLimits {
            selected_events: 4,
            ..due_selection_limits()
        },
        entities: 8,
        destinations: 8,
        movements: 8,
        output_bytes: 1024 * 1024,
    }
}
