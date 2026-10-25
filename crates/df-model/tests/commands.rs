use df_model::checkpoint::*;
use df_types::{
    BuildIdentity, ClientBindingId, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId,
    SessionId, SessionRevision,
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
fn limits() -> CheckpointLimits {
    CheckpointLimits {
        maximum_records: 100,
        maximum_text_bytes: 256,
        maximum_total_text_bytes: 1024,
        maximum_retained_bytes: 1024 * 1024,
    }
}
fn state() -> GameState {
    GameState {
        mode: ExecutionMode::Replay,
        logical_time: LogicalTime {
            ticks: 120,
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
fn checkpoint(state: GameState) -> Result<Checkpoint, CheckpointError> {
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let resource_constraints = resource_constraints();
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        state,
        ReferenceInventory {
            rules: &rules,
            content: &content_entries,
            resources: &resource_constraints,
            assets: &[],
        },
        limits(),
    )
}
fn fact(value: u8, ordinal: u32) -> GameFact {
    GameFact {
        id: FactId::from_bytes(&[value; 16]).unwrap(),
        revision: basis().revision,
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        ordinal,
        cause: None,
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: content(),
            subjects: vec![entity(4)],
        },
    }
}

fn pending() -> PendingResolution {
    PendingResolution {
        id: ResolutionId::from_bytes(&[9; 16]).unwrap(),
        basis: basis(),
        continuation: label("fixture-continuation-position-1"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[10; 16]).unwrap(),
            phase: TriggerPhase::BeforeDraw,
            causal_fact: FactId::from_bytes(&[7; 16]).unwrap(),
            source: rule(),
            timer: None,
        },
        next: PendingInput::Reaction {
            remaining: vec![OfferedResponse {
                participant: member(3),
                offer: label("fixture-offer-1"),
                options: vec![label("fixture-option-1")],
                source: rule(),
            }],
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![ResourceSpend {
            owner: entity(4),
            resource: label("fixture-resource-1"),
            amount: 1,
            source: rule(),
        }],
        rulings: vec![],
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

use df_model::commands::{CommandError, CommandLimits, validate_client_command};

fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 8,
        maximum_text_bytes: 32,
        maximum_retained_bytes: 8192,
    }
}
fn game(command: GameCommand) -> GameInput {
    GameInput::Game(CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        member: member(3),
        command,
    })
}
fn host(command: HostCommand) -> GameInput {
    GameInput::Host(HostInput {
        basis: basis(),
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        host: member(3),
        command,
    })
}
fn admit(input: &GameInput, current_checkpoint: &Checkpoint) -> Result<(), CommandError> {
    admit_with_limits(input, current_checkpoint, command_limits())
}
fn admit_with_limits(
    input: &GameInput,
    current_checkpoint: &Checkpoint,
    bounds: CommandLimits,
) -> Result<(), CommandError> {
    validate_client_command(
        input,
        current_checkpoint,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        bounds,
    )
}
fn response(kind: u8) -> GameInput {
    let pending = pending();
    let resolution = pending.id;
    let window = pending.window.id;
    let offer = label("fixture-offer-1");
    let option = label("fixture-option-1");
    match kind {
        0 => game(GameCommand::SelectChoice {
            resolution,
            window,
            offer,
            option,
        }),
        1 => game(GameCommand::SelectReaction {
            resolution,
            window,
            offer,
            option,
        }),
        2 => game(GameCommand::SubmitRoll { resolution, window }),
        3 => host(HostCommand::ResolveRuling {
            resolution,
            window,
            offer,
            option,
        }),
        _ => unreachable!(),
    }
}
fn waiting(kind: u8) -> Checkpoint {
    let mut supplied = state();
    supplied.facts.push(fact(7, 0));
    let mut pending = pending();
    let offered = vec![OfferedResponse {
        participant: member(3),
        offer: label("fixture-offer-1"),
        options: vec![label("fixture-option-1")],
        source: rule(),
    }];
    pending.next = match kind {
        0 => PendingInput::Choice { remaining: offered },
        1 => PendingInput::Reaction { remaining: offered },
        2 => PendingInput::Roll {
            participant: member(3),
            sides: vec![20],
            source: rule(),
        },
        3 => PendingInput::Ruling {
            permitted: offered,
            source: rule(),
        },
        _ => unreachable!(),
    };
    supplied.pending.push(pending);
    checkpoint(supplied).unwrap()
}
fn proposal() -> GameInput {
    game(GameCommand::ProposeAction {
        actor: entity(4),
        action: content(),
        targets: vec![entity(4)],
        choices: vec![(
            label("fixture-action-offer"),
            label("fixture-action-selection"),
        )],
    })
}
fn draft() -> GameInput {
    game(GameCommand::SubmitCharacterDraft {
        draft: CharacterDraft {
            entity: entity(4),
            member: member(3),
            ancestry: Some(content()),
            background: None,
            classes: vec![content()],
            choices: vec![AcceptedChoice {
                participant: member(3),
                offer: label("fixture-build-offer"),
                selected: label("fixture-build-selection"),
                source: rule(),
            }],
            phase: CreationPhase::AwaitingValidation,
        },
    })
}

#[test]
fn incomplete_character_draft_remains_editable_and_accepted_character_cannot_be_resubmitted() {
    let current_checkpoint = checkpoint(state()).unwrap();
    let before = current_checkpoint.clone();
    let mut incomplete = draft();
    let GameInput::Game(input) = &mut incomplete else {
        unreachable!()
    };
    let GameCommand::SubmitCharacterDraft { draft } = &mut input.command else {
        unreachable!()
    };
    draft.ancestry = None;
    draft.background = None;
    draft.classes.clear();
    draft.choices.clear();
    draft.phase = CreationPhase::Selecting;

    assert_eq!(admit(&incomplete, &current_checkpoint), Ok(()));
    let mut edited = incomplete.clone();
    let GameInput::Game(input) = &mut edited else {
        unreachable!()
    };
    let GameCommand::SubmitCharacterDraft { draft } = &mut input.command else {
        unreachable!()
    };
    draft.background = Some(content());
    draft.classes.push(content());
    draft.phase = CreationPhase::AwaitingValidation;
    assert_eq!(admit(&edited, &current_checkpoint), Ok(()));
    assert_eq!(current_checkpoint, before);

    let mut accepted_state = state();
    accepted_state.continuity.creation.push(CharacterDraft {
        entity: entity(4),
        member: member(3),
        ancestry: None,
        background: None,
        classes: vec![],
        choices: vec![],
        phase: CreationPhase::Accepted,
    });
    let accepted = checkpoint(accepted_state).unwrap();
    let accepted_before = accepted.clone();
    let edited_before = edited.clone();
    assert_eq!(admit(&edited, &accepted), Err(CommandError::InvalidDraft));
    assert_eq!(edited, edited_before);
    assert_eq!(accepted, accepted_before);
}

#[test]
fn accepts_each_closed_command_family_without_producing_state_or_draws() {
    for kind in 0..4 {
        let current_checkpoint = waiting(kind);
        let before = current_checkpoint.clone();
        let input = response(kind);
        let original = input.clone();
        assert_eq!(admit(&input, &current_checkpoint), Ok(()));
        assert_eq!(input, original);
        assert_eq!(current_checkpoint, before);
        assert!(current_checkpoint.state().draws.is_empty());
    }
    let current_checkpoint = checkpoint(state()).unwrap();
    for input in [
        proposal(),
        draft(),
        game(GameCommand::Speak {
            speaker: entity(4),
            text: "hello".to_owned(),
            conversation: None,
        }),
        host(HostCommand::SetPause {
            paused: true,
            policy: content(),
        }),
        host(HostCommand::RequestCheckpoint),
    ] {
        assert_eq!(admit(&input, &current_checkpoint), Ok(()));
    }
}

#[test]
fn rejects_native_success_failure_cancellation_and_timer_at_client_command_boundary() {
    let current_checkpoint = checkpoint(state()).unwrap();
    for outcome in [
        JobOutcome::Ai {
            semantic_output: "arbitrary invented outcome".to_owned(),
            policy: content(),
            model: label("fixture-model"),
        },
        JobOutcome::Media {
            asset: AssetReference {
                key: label("fixture-asset"),
                digest: ContentDigest([1; 32]),
                byte_length: 1,
                kind: AssetKind::Image,
            },
            demand: RecordId::from_bytes(&[21; 16]).unwrap(),
        },
        JobOutcome::MemoryCandidates { records: vec![] },
        JobOutcome::JobCancelled {
            target: JobId::from_bytes(&[22; 16]).unwrap(),
        },
        JobOutcome::TimerCancelled {
            target: TimerId::from_bytes(&[23; 16]).unwrap(),
        },
        JobOutcome::Failed(NativeFailure::Unavailable),
    ] {
        let input = GameInput::Job(JobCompletion {
            basis: basis(),
            operation: OperationId::from_bytes(&[20; 16]).unwrap(),
            job: JobId::from_bytes(&[22; 16]).unwrap(),
            generation: 1,
            outcome,
        });
        assert_eq!(
            admit(&input, &current_checkpoint),
            Err(CommandError::InternalInput)
        );
    }
    let input = GameInput::Timer(TimerExpiry {
        basis: basis(),
        timer: TimerId::from_bytes(&[23; 16]).unwrap(),
        generation: 1,
        observed_time: current_checkpoint.state().logical_time,
    });
    assert_eq!(
        admit(&input, &current_checkpoint),
        Err(CommandError::InternalInput)
    );
}

#[test]
fn routes_presentation_observations_to_the_separate_delivery_boundary() {
    let current_checkpoint = checkpoint(state()).unwrap();
    let input = GameInput::Presentation(PresentationReport {
        basis: basis(),
        binding: ClientBindingId::from_bytes(&[24; 16]).unwrap(),
        cue: RecordId::from_bytes(&[21; 16]).unwrap(),
        observation: PresentationObservation::Finished,
        presentation_ticks: 10,
    });
    assert_eq!(
        admit(&input, &current_checkpoint),
        Err(CommandError::PresentationReport)
    );
}

#[test]
fn rejects_every_cross_kind_pending_response_and_replaced_window() {
    for pending_kind in 0..4 {
        let current_checkpoint = waiting(pending_kind);
        for command_kind in 0..4 {
            if command_kind != pending_kind {
                assert_eq!(
                    admit(&response(command_kind), &current_checkpoint),
                    Err(CommandError::WrongPendingKind)
                );
            }
        }
    }
    let current_checkpoint = waiting(1);
    let GameInput::Game(mut input) = response(1) else {
        unreachable!()
    };
    if let GameCommand::SelectReaction { window, .. } = &mut input.command {
        *window = WindowId::from_bytes(&[99; 16]).unwrap();
    }
    assert_eq!(
        admit(&GameInput::Game(input), &current_checkpoint),
        Err(CommandError::StaleWindow)
    );
}

#[test]
fn rejects_unknown_offers_options_and_ineligible_remaining_participants() {
    for kind in [0, 1, 3] {
        for changed in 0..3 {
            let current_checkpoint = waiting(kind);
            let mut input = response(kind);
            match &mut input {
                GameInput::Game(input) => match &mut input.command {
                    GameCommand::SelectChoice { offer, option, .. }
                    | GameCommand::SelectReaction { offer, option, .. } => match changed {
                        0 => *offer = label("unoffered"),
                        1 => *option = label("unoffered"),
                        2 => input.member = member(5),
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                },
                GameInput::Host(input) => {
                    if let HostCommand::ResolveRuling { offer, option, .. } = &mut input.command {
                        match changed {
                            0 => *offer = label("unoffered"),
                            1 => *option = label("unoffered"),
                            2 => input.host = member(5),
                            _ => unreachable!(),
                        }
                    }
                }
                _ => unreachable!(),
            }
            let expected = if changed == 2 {
                CommandError::UnknownMember
            } else {
                CommandError::UnofferedResponse
            };
            assert_eq!(admit(&input, &current_checkpoint), Err(expected));
        }
    }
    let mut supplied = waiting(2).state().clone();
    supplied.members.push(MembershipLink {
        member: member(5),
        character: None,
    });
    let current_checkpoint = checkpoint(supplied).unwrap();
    let GameInput::Game(mut input) = response(2) else {
        unreachable!()
    };
    input.member = member(5);
    assert_eq!(
        admit(&GameInput::Game(input), &current_checkpoint),
        Err(CommandError::UnofferedResponse)
    );
}

#[test]
fn permits_unrelated_sequence_advances_but_refuses_retired_epochs_future_and_other_runs() {
    let current_checkpoint = waiting(1);
    let GameInput::Game(mut input) = response(1) else {
        unreachable!()
    };
    input.basis.revision = revision(2, 7);
    input.observed_revision = revision(2, 7);
    assert_eq!(
        admit(&GameInput::Game(input.clone()), &current_checkpoint),
        Ok(())
    );
    for revision in [revision(1, 7), revision(3, 7), revision(2, 9)] {
        let mut changed = input.clone();
        changed.observed_revision = revision;
        assert_eq!(
            admit(&GameInput::Game(changed), &current_checkpoint),
            Err(CommandError::StaleRevision)
        );
        let mut changed = input.clone();
        changed.basis.revision = revision;
        assert_eq!(
            admit(&GameInput::Game(changed), &current_checkpoint),
            Err(CommandError::StaleRevision)
        );
    }
    let mut changed = input.clone();
    changed.basis.session = SessionId::from_bytes(&[99; 16]).unwrap();
    assert_eq!(
        admit(&GameInput::Game(changed), &current_checkpoint),
        Err(CommandError::WrongSession)
    );
    input.basis.run = RunId::from_bytes(&[99; 16]).unwrap();
    assert_eq!(
        admit(&GameInput::Game(input), &current_checkpoint),
        Err(CommandError::WrongRun)
    );
}

#[test]
fn rejects_unadmitted_content_unknown_entities_and_duplicate_selections() {
    let current_checkpoint = checkpoint(state()).unwrap();
    for changed in 0..5 {
        let GameInput::Game(mut input) = proposal() else {
            unreachable!()
        };
        if let GameCommand::ProposeAction {
            actor,
            action,
            targets,
            choices,
        } = &mut input.command
        {
            match changed {
                0 => action.entry = label("unadmitted"),
                1 => *actor = entity(99),
                2 => targets.push(entity(99)),
                3 => targets.push(entity(4)),
                4 => choices.push(choices[0].clone()),
                _ => unreachable!(),
            }
        }
        let expected = if changed < 3 {
            CommandError::InvalidReference
        } else {
            CommandError::DuplicateSelection
        };
        assert_eq!(
            admit(&GameInput::Game(input), &current_checkpoint),
            Err(expected)
        );
    }
    let pause = host(HostCommand::SetPause {
        paused: true,
        policy: ContentReference {
            package: content().package,
            entry: label("unadmitted"),
        },
    });
    assert_eq!(
        admit(&pause, &current_checkpoint),
        Err(CommandError::InvalidReference)
    );
}

#[test]
fn rejects_client_claimed_accepted_build_foreign_member_and_unadmitted_build_sources() {
    let current_checkpoint = checkpoint(state()).unwrap();
    for changed in 0..4 {
        let GameInput::Game(mut input) = draft() else {
            unreachable!()
        };
        if let GameCommand::SubmitCharacterDraft { draft } = &mut input.command {
            match changed {
                0 => draft.phase = CreationPhase::Accepted,
                1 => draft.member = member(5),
                2 => draft.choices[0].source.clause = label("unadmitted"),
                3 => draft.choices[0].participant = member(5),
                _ => unreachable!(),
            }
        }
        assert_eq!(
            admit(&GameInput::Game(input), &current_checkpoint),
            Err(CommandError::InvalidDraft)
        );
    }
}

#[test]
fn bounds_utf8_text_collections_and_spare_capacity_without_input_mutation() {
    let current_checkpoint = checkpoint(state()).unwrap();
    let input = game(GameCommand::Speak {
        speaker: entity(4),
        text: "é".repeat(17),
        conversation: None,
    });
    assert_eq!(
        admit(&input, &current_checkpoint),
        Err(CommandError::Capacity)
    );
    let mut bounds = command_limits();
    bounds.maximum_records = 1;
    assert_eq!(
        admit_with_limits(&proposal(), &current_checkpoint, bounds),
        Err(CommandError::Capacity)
    );
    let input = game(GameCommand::ProposeAction {
        actor: entity(4),
        action: content(),
        targets: Vec::with_capacity(4096),
        choices: vec![],
    });
    let retained = input.retained_bytes();
    let original = input.clone();
    assert_eq!(
        admit(&input, &current_checkpoint),
        Err(CommandError::Capacity)
    );
    assert_eq!(input, original);
    assert_eq!(input.retained_bytes(), retained);
    let mut text = String::with_capacity(16_384);
    text.push_str("hello");
    let input = game(GameCommand::Speak {
        speaker: entity(4),
        text,
        conversation: None,
    });
    assert_eq!(
        admit(&input, &current_checkpoint),
        Err(CommandError::Capacity)
    );
    for bounds in [
        CommandLimits {
            maximum_records: 0,
            ..command_limits()
        },
        CommandLimits {
            maximum_text_bytes: 0,
            ..command_limits()
        },
        CommandLimits {
            maximum_retained_bytes: 0,
            ..command_limits()
        },
    ] {
        assert_eq!(
            admit_with_limits(&proposal(), &current_checkpoint, bounds),
            Err(CommandError::Capacity)
        );
    }
}

#[test]
fn checks_conversation_reference_and_speaker_without_interpreting_speech_as_game_facts() {
    let mut supplied = state();
    let conversation_id = RecordId::from_bytes(&[25; 16]).unwrap();
    supplied.conversations.push(ConversationState {
        id: conversation_id,
        participants: vec![entity(4)],
        topic: content(),
        accepted_facts: vec![],
    });
    let current_checkpoint = checkpoint(supplied).unwrap();
    let before = current_checkpoint.clone();
    let input = game(GameCommand::Speak {
        speaker: entity(4),
        text: "I declare an invented outcome".to_owned(),
        conversation: Some(conversation_id),
    });
    assert_eq!(admit(&input, &current_checkpoint), Ok(()));
    assert_eq!(current_checkpoint, before);
    assert!(current_checkpoint.state().facts.is_empty());
    let input = game(GameCommand::Speak {
        speaker: entity(4),
        text: "hello".to_owned(),
        conversation: Some(RecordId::from_bytes(&[99; 16]).unwrap()),
    });
    assert_eq!(
        admit(&input, &current_checkpoint),
        Err(CommandError::InvalidReference)
    );
}
