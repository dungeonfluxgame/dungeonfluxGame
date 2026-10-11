use df_model::checkpoint::*;
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
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

#[test]
fn owns_detached_state_and_preserves_exact_resume_basis() {
    let supplied = state();
    let retained = supplied.clone();
    let checkpoint = checkpoint(supplied).unwrap();
    assert_eq!(checkpoint.state(), &retained);
    assert_eq!(checkpoint.basis(), basis());
    assert_eq!(
        checkpoint.validate_resume(basis(), &pins()),
        Ok(&checkpoint)
    );
    let mut unrelated = retained.clone();
    unrelated.resources[0].value = 7;
    assert_eq!(checkpoint.state().resources[0].value, 4);
}
#[test]
fn rejects_reference_to_unadmitted_source_or_content() {
    for changed in 0..4 {
        let mut supplied = state();
        match changed {
            0 => supplied.entities[0].definition.entry = label("fixture-unadmitted"),
            1 => supplied.entities[0].definition.package = label("fixture-other-package"),
            2 => supplied.resources[0].source.clause = label("fixture-unadmitted"),
            3 => supplied.resources[0].source.catalog = label("fixture-other-catalog"),
            _ => unreachable!(),
        }
        assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    }
}
#[test]
fn rejects_missing_member_entity_and_duplicate_resource_references() {
    let mut supplied = state();
    supplied.members[0].character = Some(entity(99));
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    let mut supplied = state();
    supplied.characters[0].owner = member(99);
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    let mut supplied = state();
    supplied.resources[0].owner = entity(99);
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    let mut supplied = state();
    supplied.resources.push(supplied.resources[0].clone());
    assert_eq!(
        checkpoint(supplied),
        Err(CheckpointError::DuplicateIdentity)
    );
}
#[test]
fn validates_source_supplied_resource_interval_without_a_universal_cap() {
    for (minimum, maximum, value) in [(0, 8, -1), (0, 8, 9), (9, 8, 8)] {
        let mut supplied = state();
        supplied.resources[0].minimum = minimum;
        supplied.resources[0].maximum = maximum;
        supplied.resources[0].value = value;
        assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidResource));
    }
    let mut supplied = state();
    supplied.resources[0].minimum = -8;
    supplied.resources[0].value = -4;
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let admitted_bounds = vec![ResourceConstraint {
        owner: entity(4),
        resource: label("fixture-resource-1"),
        minimum: -8,
        maximum: 8,
        source: rule(),
    }];
    assert!(
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis(),
            pins(),
            supplied,
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &admitted_bounds,
                assets: &[]
            },
            limits()
        )
        .is_ok()
    );
}
#[test]
fn refuses_same_sequence_from_other_epoch_and_caller_newer_basis() {
    let checkpoint = checkpoint(state()).unwrap();
    for candidate in [
        revision(1, 8),
        revision(2, 7),
        revision(2, 9),
        revision(3, 8),
    ] {
        let mut expected = basis();
        expected.revision = candidate;
        assert_eq!(
            checkpoint.validate_resume(expected, &pins()),
            Err(CheckpointError::StaleBasis)
        );
    }
    let mut expected = basis();
    expected.session = SessionId::from_bytes(&[9; 16]).unwrap();
    assert_eq!(
        checkpoint.validate_resume(expected, &pins()),
        Err(CheckpointError::WrongSession)
    );
    let mut expected = basis();
    expected.run = RunId::from_bytes(&[9; 16]).unwrap();
    assert_eq!(
        checkpoint.validate_resume(expected, &pins()),
        Err(CheckpointError::WrongRun)
    );
}
#[test]
fn refuses_every_exact_rules_pin_change() {
    let checkpoint = checkpoint(state()).unwrap();
    for changed in 0..8 {
        let mut admitted = pins();
        match changed {
            0 => admitted.rules.mode = RulesMode::DisclosedCustom,
            1 => admitted.rules.ruleset = label("fixture-other"),
            2 => admitted.rules.catalog = label("fixture-other"),
            3 => admitted.rules.catalog_digest = ContentDigest([9; 32]),
            4 => admitted.rules.source_manifest = label("fixture-other"),
            5 => admitted.rules.source_manifest_digest = ContentDigest([9; 32]),
            6 => admitted.rules.handler = label("fixture-other"),
            7 => admitted.rules.handler_digest = ContentDigest([9; 32]),
            _ => unreachable!(),
        }
        assert_eq!(
            checkpoint.validate_resume(basis(), &admitted),
            Err(CheckpointError::RulesMismatch)
        );
    }
}
#[test]
fn refuses_content_package_digest_and_build_changes() {
    let checkpoint = checkpoint(state()).unwrap();
    for changed in 0..4 {
        let mut admitted = pins();
        match changed {
            0 => admitted.content.content = label("fixture-other"),
            1 => admitted.content.content_digest = ContentDigest([9; 32]),
            2 => admitted.content.package = label("fixture-other"),
            3 => admitted.content.package_digest = ContentDigest([9; 32]),
            _ => unreachable!(),
        }
        assert_eq!(
            checkpoint.validate_resume(basis(), &admitted),
            Err(CheckpointError::ContentMismatch)
        );
    }
    let mut admitted = pins();
    admitted.build = BuildIdentity::new(
        Some("fixture-other"),
        Some("fixture-native-1"),
        Some("fixture-wasm-1"),
        Some("fixture-config-1"),
        Some("fixture-content-1"),
    )
    .unwrap();
    assert_eq!(
        checkpoint.validate_resume(basis(), &admitted),
        Err(CheckpointError::BuildMismatch)
    );
}
#[test]
fn accepts_historical_facts_without_rewriting_their_recovery_epoch() {
    let mut supplied = state();
    let mut historical = fact(7, 0);
    historical.revision = revision(1, 40);
    supplied.facts.push(historical.clone());
    let checkpoint = checkpoint(supplied).unwrap();
    assert_eq!(checkpoint.state().facts, vec![historical]);
}
#[test]
fn rejects_forward_causal_reference_and_out_of_order_facts() {
    let mut supplied = state();
    let mut first = fact(7, 0);
    let next = fact(8, 1);
    first.cause = Some(next.id);
    supplied.facts = vec![first, next];
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    let mut supplied = state();
    supplied.facts = vec![fact(7, 1)];
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidOrder));
    let mut supplied = state();
    supplied.facts = vec![fact(7, 0), fact(8, 2)];
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidOrder));
}
#[test]
fn preserves_actual_draw_values_and_rejects_invalid_dice() {
    let draw = ActualDraw {
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        ordinal: 0,
        resolution: ResolutionId::from_bytes(&[7; 16]).unwrap(),
        window: WindowId::from_bytes(&[8; 16]).unwrap(),
        sides: 20,
        value: 13,
        source: rule(),
    };
    let mut supplied = state();
    supplied.draws.push(draw.clone());
    assert_eq!(
        checkpoint(supplied).unwrap().state().draws,
        vec![draw.clone()]
    );
    for (sides, value) in [(0, 1), (20, 0), (20, 21)] {
        let mut supplied = state();
        let mut changed = draw.clone();
        changed.sides = sides;
        changed.value = value;
        supplied.draws.push(changed);
        assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidDraw));
    }
    let mut supplied = state();
    supplied.draws = vec![draw.clone(), draw];
    assert_eq!(
        checkpoint(supplied),
        Err(CheckpointError::DuplicateIdentity)
    );
}
fn accepted_draw_decision() -> GameState {
    let mut supplied = state();
    let operation = OperationId::from_bytes(&[6; 16]).unwrap();
    let draw = ActualDraw {
        operation,
        ordinal: 0,
        resolution: ResolutionId::from_bytes(&[7; 16]).unwrap(),
        window: WindowId::from_bytes(&[8; 16]).unwrap(),
        sides: 20,
        value: 13,
        source: rule(),
    };
    let mut accepted = fact(9, 0);
    accepted.value = FactValue::DrawAccepted {
        operation,
        ordinal: draw.ordinal,
    };
    let intent = DurableIntent {
        id: EffectId::from_bytes(&[11; 16]).unwrap(),
        basis: basis(),
        operation,
        slot: 0,
        kind: EffectKind::PublishPresentation,
        job: None,
        timer: None,
        generation: 1,
        status: DurableStatus::Pending,
        definition: content(),
    };
    supplied.draws.push(draw);
    supplied.facts.push(accepted.clone());
    supplied.intents.push(intent.clone());
    supplied.decisions.push(AcceptedDecision {
        operation,
        revision: basis().revision,
        facts: vec![accepted.id],
        draws: vec![0],
        effects: vec![intent.id],
        source_policy: label("fixture-policy-1"),
        semantic_output: Some("accepted exact outcome".to_owned()),
    });
    supplied
}
#[test]
fn reconstructs_exact_accepted_decision_with_source_policy_and_pins() {
    let supplied = accepted_draw_decision();
    let retained = supplied.clone();
    let original = checkpoint(supplied).unwrap();
    let reconstructed = checkpoint(retained.clone()).unwrap();
    assert_eq!(reconstructed, original);
    assert_eq!(reconstructed.state(), &retained);
    assert_eq!(reconstructed.pins(), &pins());
    assert_eq!(reconstructed.state().draws[0].value, 13);
    assert_eq!(reconstructed.state().draws[0].source, rule());
    assert_eq!(
        reconstructed.state().decisions[0].source_policy,
        label("fixture-policy-1")
    );
    assert_eq!(
        reconstructed.state().decisions[0]
            .semantic_output
            .as_deref(),
        Some("accepted exact outcome")
    );
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let resource_constraints = resource_constraints();
    assert_eq!(
        reconstructed.validate_admitted(
            basis(),
            &pins(),
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &resource_constraints,
                assets: &[],
            },
            limits(),
        ),
        Ok(&reconstructed)
    );
    let mut historical = retained;
    historical.facts[0].revision = revision(2, 7);
    historical.decisions[0].revision = revision(2, 7);
    historical.intents[0].basis.revision = revision(2, 7);
    assert_eq!(checkpoint(historical.clone()).unwrap().state(), &historical);
}
#[test]
fn decision_provenance_refuses_foreign_draw_fact_and_effect_operation() {
    let original = checkpoint(accepted_draw_decision()).unwrap();
    let mut foreign_draw = accepted_draw_decision();
    let other_operation = OperationId::from_bytes(&[16; 16]).unwrap();
    foreign_draw.draws[0].operation = other_operation;
    foreign_draw.facts[0].value = FactValue::DrawAccepted {
        operation: other_operation,
        ordinal: 0,
    };
    foreign_draw.decisions[0].draws.clear();
    assert_eq!(
        checkpoint(foreign_draw),
        Err(CheckpointError::InvalidReference)
    );

    let mut foreign_effect = accepted_draw_decision();
    foreign_effect.intents[0].operation = other_operation;
    assert_eq!(
        checkpoint(foreign_effect),
        Err(CheckpointError::InvalidReference)
    );
    assert_eq!(
        original.state().decisions,
        accepted_draw_decision().decisions
    );
}
#[test]
fn decision_provenance_refuses_unadmitted_draw_source_and_capacity() {
    let original = checkpoint(accepted_draw_decision()).unwrap();
    let mut unadmitted = accepted_draw_decision();
    unadmitted.draws[0].source.clause = label("unadmitted-clause");
    assert_eq!(
        checkpoint(unadmitted),
        Err(CheckpointError::InvalidReference)
    );
    let mut narrower = limits();
    narrower.maximum_records = 1;
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let resource_constraints = resource_constraints();
    assert_eq!(
        original.validate_admitted(
            basis(),
            &pins(),
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &resource_constraints,
                assets: &[],
            },
            narrower,
        ),
        Err(CheckpointError::Capacity)
    );
    assert_eq!(original.state(), &accepted_draw_decision());
}
#[test]
fn rejects_decision_fact_from_another_operation() {
    let mut supplied = state();
    let record = fact(7, 0);
    supplied.facts.push(record.clone());
    supplied.decisions.push(AcceptedDecision {
        operation: OperationId::from_bytes(&[8; 16]).unwrap(),
        revision: basis().revision,
        facts: vec![record.id],
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-policy-1"),
        semantic_output: None,
    });
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
}
#[test]
fn keeps_attributed_false_claim_separate_from_canonical_facts() {
    let mut supplied = state();
    let record = fact(7, 0);
    supplied.facts.push(record.clone());
    supplied.beliefs.push(AttributedClaim {
        id: RecordId::from_bytes(&[9; 16]).unwrap(),
        holder: entity(4),
        subject: entity(4),
        claim: "synthetic contradictory claim".to_owned(),
        evidence: vec![record.id],
        audience: AudienceScope::Members(vec![member(3)]),
        source: content(),
    });
    let checkpoint = checkpoint(supplied).unwrap();
    assert_eq!(checkpoint.state().facts, vec![record]);
    assert_eq!(
        checkpoint.state().beliefs[0].claim,
        "synthetic contradictory claim"
    );
}

fn observer_truth_state() -> GameState {
    let mut supplied = state();
    let mut cause = fact(70, 0);
    cause.revision = revision(1, 3);
    let mut outcome = fact(71, 1);
    outcome.revision = cause.revision;
    outcome.cause = Some(cause.id);
    outcome.value = FactValue::ResourceChanged {
        entity: entity(4),
        resource: label("fixture-resource-1"),
        before: 5,
        after: 4,
        source: rule(),
    };
    supplied.decisions.push(AcceptedDecision {
        operation: cause.operation,
        revision: cause.revision,
        facts: vec![cause.id, outcome.id],
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-policy-1"),
        semantic_output: None,
    });
    supplied.knowledge.push(KnowledgeGrant {
        observer: member(3),
        fact: outcome.id,
        source: cause.id,
    });
    supplied.beliefs.push(AttributedClaim {
        id: RecordId::from_bytes(&[72; 16]).unwrap(),
        holder: entity(4),
        subject: entity(4),
        claim: "I gained a resource; none was spent".to_owned(),
        evidence: vec![outcome.id],
        audience: AudienceScope::Members(vec![member(3)]),
        source: content(),
    });
    supplied.memories.push(MemoryEpisode {
        id: RecordId::from_bytes(&[73; 16]).unwrap(),
        holder: entity(4),
        source_facts: vec![outcome.id],
        retained_text: "I remember having six resources afterward".to_owned(),
        audience: AudienceScope::Members(vec![member(3)]),
        source_revision: outcome.revision,
    });
    supplied.continuity.rumors.push(RumorTransmission {
        id: RecordId::from_bytes(&[74; 16]).unwrap(),
        claim: supplied.beliefs[0].id,
        sender: entity(4),
        recipient: entity(4),
        evidence: vec![outcome.id],
        policy: content(),
        remaining_hops: 1,
        audience: AudienceScope::Members(vec![member(3)]),
    });
    supplied.continuity.journal.push(JournalEntry {
        id: RecordId::from_bytes(&[75; 16]).unwrap(),
        source_facts: vec![outcome.id],
        attributed_claims: vec![supplied.beliefs[0].id],
        audience: AudienceScope::Members(vec![member(3)]),
        text: "Attributed recollection: six resources".to_owned(),
    });
    supplied.continuity.summaries.push(MemorySummary {
        id: RecordId::from_bytes(&[76; 16]).unwrap(),
        episodes: vec![supplied.memories[0].id],
        derived_claims: vec![supplied.beliefs[0].id],
        source_digest: pins().content.content_digest,
        source_revision: outcome.revision,
        summarizer: label("fixture-summarizer"),
        model: label("fixture-model"),
        policy: content(),
        audience: AudienceScope::Members(vec![member(3)]),
        text: "The observer recalls gaining resources".to_owned(),
        incomplete: false,
    });
    supplied.facts = vec![cause, outcome];
    supplied
}

#[test]
fn observer_edits_and_compaction_preserve_exact_canonical_history() {
    let original = checkpoint(observer_truth_state()).unwrap();
    let mut edited = original.state().clone();
    edited.beliefs[0].claim = "I now believe the cost was three".to_owned();
    edited.memories[0].retained_text = "I no longer recall the exact quantity".to_owned();
    edited.continuity.rumors[0].remaining_hops = 0;
    edited.continuity.journal[0].text = "The observer revised their recollection".to_owned();
    edited.continuity.summaries[0].text = "Uncertain resource recollection".to_owned();
    edited.continuity.summaries[0].incomplete = true;
    let revised = checkpoint(edited).unwrap();
    assert_ne!(revised.state().beliefs, original.state().beliefs);
    assert_ne!(revised.state().memories, original.state().memories);
    assert_ne!(
        revised.state().continuity.summaries,
        original.state().continuity.summaries
    );

    let mut compacted = revised.state().clone();
    compacted.knowledge.clear();
    compacted.beliefs.clear();
    compacted.memories.clear();
    compacted.continuity.rumors.clear();
    compacted.continuity.journal.clear();
    compacted.continuity.summaries.clear();
    let compacted = checkpoint(compacted).unwrap();
    for candidate in [&revised, &compacted] {
        assert_eq!(candidate.state().facts, original.state().facts);
        assert_eq!(candidate.state().decisions, original.state().decisions);
        assert_eq!(candidate.state().resources, original.state().resources);
        assert_eq!(candidate.state().draws, original.state().draws);
        assert_eq!(
            candidate.state().logical_time,
            original.state().logical_time
        );
        assert_eq!(candidate.basis(), original.basis());
        assert_eq!(candidate.pins(), original.pins());
        assert_eq!(candidate.validate_resume(basis(), &pins()), Ok(candidate));
    }
    assert_eq!(original.state().facts[1].revision, revision(1, 3));
    assert_eq!(
        original.state().facts[1].cause,
        Some(original.state().facts[0].id)
    );
    assert_eq!(
        original.state().beliefs[0].claim,
        "I gained a resource; none was spent"
    );
}

#[test]
fn forged_observer_provenance_refuses_without_changing_the_original_checkpoint() {
    let original = checkpoint(observer_truth_state()).unwrap();
    let retained = original.clone();
    for case in [
        "claim-holder",
        "claim-subject",
        "claim-evidence",
        "claim-source",
        "knowledge-observer",
        "knowledge-fact",
        "knowledge-source",
        "memory-fact",
        "rumor-evidence",
        "summary-episode",
        "summary-digest",
    ] {
        let mut candidate = original.state().clone();
        match case {
            "claim-holder" => candidate.beliefs[0].holder = entity(99),
            "claim-subject" => candidate.beliefs[0].subject = entity(99),
            "claim-evidence" => candidate.beliefs[0].evidence[0] = fact(99, 0).id,
            "claim-source" => candidate.beliefs[0].source.entry = label("unadmitted-source"),
            "knowledge-observer" => candidate.knowledge[0].observer = member(99),
            "knowledge-fact" => candidate.knowledge[0].fact = fact(99, 0).id,
            "knowledge-source" => candidate.knowledge[0].source = fact(99, 0).id,
            "memory-fact" => candidate.memories[0].source_facts[0] = fact(99, 0).id,
            "rumor-evidence" => candidate.continuity.rumors[0].evidence[0] = fact(99, 0).id,
            "summary-episode" => {
                candidate.continuity.summaries[0].episodes[0] =
                    RecordId::from_bytes(&[99; 16]).unwrap();
            }
            "summary-digest" => {
                candidate.continuity.summaries[0].source_digest = ContentDigest([99; 32]);
            }
            _ => unreachable!(),
        }
        let expected = if case == "summary-digest" {
            CheckpointError::ContentMismatch
        } else {
            CheckpointError::InvalidReference
        };
        assert_eq!(checkpoint(candidate), Err(expected), "{case}");
        assert_eq!(original, retained, "{case}");
    }
}

#[test]
fn observer_records_cannot_stand_in_for_missing_canonical_sources() {
    let original = checkpoint(observer_truth_state()).unwrap();
    for case in ["knowledge", "belief", "memory", "summary"] {
        let mut candidate = original.state().clone();
        candidate.facts.clear();
        candidate.decisions.clear();
        candidate.knowledge.clear();
        candidate.beliefs.clear();
        candidate.memories.clear();
        candidate.continuity.rumors.clear();
        candidate.continuity.journal.clear();
        candidate.continuity.summaries.clear();
        match case {
            "knowledge" => candidate.knowledge = original.state().knowledge.clone(),
            "belief" => candidate.beliefs = original.state().beliefs.clone(),
            "memory" => candidate.memories = original.state().memories.clone(),
            "summary" => {
                let mut summary = original.state().continuity.summaries[0].clone();
                summary.derived_claims.clear();
                candidate.continuity.summaries.push(summary);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            checkpoint(candidate),
            Err(CheckpointError::InvalidReference),
            "{case}"
        );
        assert_eq!(original.state().facts[1].revision, revision(1, 3));
        assert_eq!(
            original.state().facts[1].cause,
            Some(original.state().facts[0].id)
        );
        assert_eq!(original.state().resources[0].value, 4);
    }
}

#[test]
fn rejects_zero_time_scale_unknown_private_audience_and_capacity_overflow() {
    let mut supplied = state();
    supplied.logical_time.ticks_per_second = 0;
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidTime));
    let mut supplied = state();
    let mut record = fact(7, 0);
    record.audience = AudienceScope::Members(vec![member(99)]);
    supplied.facts.push(record);
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let resource_constraints = resource_constraints();
    let mut admitted = limits();
    admitted.maximum_records = 1;
    assert_eq!(
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis(),
            pins(),
            state(),
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &resource_constraints,
                assets: &[]
            },
            admitted
        ),
        Err(CheckpointError::Capacity)
    );
}
#[test]
fn rejects_unsupported_schema_before_admitting_state() {
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let resource_constraints = resource_constraints();
    for schema in [0, 1, u16::MAX] {
        assert_eq!(
            Checkpoint::new(
                schema,
                basis(),
                pins(),
                state(),
                ReferenceInventory {
                    rules: &rules,
                    content: &content_entries,
                    resources: &resource_constraints,
                    assets: &[]
                },
                limits()
            ),
            Err(CheckpointError::UnsupportedSchema)
        );
    }
}

#[test]
fn npc_references_and_directional_axis_provenance_require_admitted_sources() {
    let mut supplied = state();
    let mut target = supplied.entities[0].clone();
    target.id = entity(5);
    supplied.entities.push(target);
    let accepted = fact(70, 0);
    let accepted_policy = label("axis-policy");
    supplied.facts.push(accepted.clone());
    supplied.decisions.push(AcceptedDecision {
        operation: accepted.operation,
        revision: accepted.revision,
        facts: vec![accepted.id],
        draws: vec![],
        effects: vec![],
        source_policy: accepted_policy.clone(),
        semantic_output: None,
    });
    let baseline = || RelationshipAxisState {
        value: label("authored-baseline"),
        provenance: RelationshipAxisProvenance::AuthoredBaseline { source: content() },
    };
    supplied.relationships.push(Relationship {
        subject: entity(4),
        object: entity(5),
        policy: content(),
        state: label("authored-category"),
        trust: RelationshipAxisState {
            value: label("trust-current"),
            provenance: RelationshipAxisProvenance::AcceptedFact {
                source: content(),
                fact: accepted.id,
                source_policy: accepted_policy,
                witness: None,
            },
        },
        affection: baseline(),
        respect: baseline(),
        fear: baseline(),
        suspicion: baseline(),
        debt: baseline(),
        familiarity: baseline(),
    });
    supplied.continuity.npcs.push(NpcState {
        entity: entity(4),
        role: content(),
        personality: content(),
        motivations: vec![content()],
        goals: vec![content()],
        needs: vec![content()],
        fears: vec![content()],
        known_facts: vec![accepted.id],
        beliefs: vec![],
        secrets: vec![],
    });

    assert!(checkpoint(supplied.clone()).is_ok());

    let mut missing_fact = supplied.clone();
    missing_fact.relationships[0].fear.provenance = RelationshipAxisProvenance::AcceptedFact {
        source: content(),
        fact: FactId::from_bytes(&[71; 16]).unwrap(),
        source_policy: label("axis-policy"),
        witness: None,
    };
    assert_eq!(
        checkpoint(missing_fact),
        Err(CheckpointError::InvalidReference)
    );

    let mut wrong_policy = supplied.clone();
    wrong_policy.relationships[0].trust.provenance = RelationshipAxisProvenance::AcceptedFact {
        source: content(),
        fact: accepted.id,
        source_policy: label("different-policy"),
        witness: None,
    };
    assert_eq!(
        checkpoint(wrong_policy),
        Err(CheckpointError::InvalidReference)
    );

    let mut wrong_axis_source = supplied.clone();
    if let RelationshipAxisProvenance::AcceptedFact { source, .. } =
        &mut wrong_axis_source.relationships[0].trust.provenance
    {
        source.entry = label("unadmitted-axis-source");
    }
    assert_eq!(
        checkpoint(wrong_axis_source),
        Err(CheckpointError::InvalidReference)
    );

    let mut missing_witness = supplied.clone();
    if let RelationshipAxisProvenance::AcceptedFact { witness, .. } =
        &mut missing_witness.relationships[0].trust.provenance
    {
        *witness = Some(RecordId::from_bytes(&[74; 16]).unwrap());
    }
    assert_eq!(
        checkpoint(missing_witness),
        Err(CheckpointError::InvalidReference)
    );

    let mut wrong_package = supplied.clone();
    wrong_package.continuity.npcs[0].role.package = label("unadmitted-package");
    assert_eq!(
        checkpoint(wrong_package),
        Err(CheckpointError::InvalidReference)
    );

    let mut duplicate_goal = supplied;
    duplicate_goal.continuity.npcs[0].goals.push(content());
    assert_eq!(
        checkpoint(duplicate_goal),
        Err(CheckpointError::DuplicateIdentity)
    );
}

#[test]
fn obligation_requires_explicit_agreement_lineage_and_consistent_lifecycle() {
    let mut supplied = state();
    let mut target = supplied.entities[0].clone();
    target.id = entity(5);
    supplied.entities.push(target);
    let mut agreement_fact = fact(72, 0);
    agreement_fact.value = FactValue::ContentEvent {
        definition: content(),
        subjects: vec![entity(4), entity(5)],
    };
    let agreement_policy = label("agreement-policy");
    supplied.facts.push(agreement_fact.clone());
    supplied.decisions.push(AcceptedDecision {
        operation: agreement_fact.operation,
        revision: agreement_fact.revision,
        facts: vec![agreement_fact.id],
        draws: vec![],
        effects: vec![],
        source_policy: agreement_policy.clone(),
        semantic_output: None,
    });
    supplied.obligations.push(Obligation {
        id: RecordId::from_bytes(&[72; 16]).unwrap(),
        obligor: entity(4),
        beneficiary: entity(5),
        definition: content(),
        terms: content(),
        due: None,
        agreement: ObligationAgreement {
            source: content(),
            fact: agreement_fact.id,
            source_policy: agreement_policy,
            at: supplied.logical_time,
        },
        status: ObligationStatus::Active,
        transition: None,
    });
    assert!(checkpoint(supplied.clone()).is_ok());

    let mut missing_agreement = supplied.clone();
    missing_agreement.obligations[0].agreement.fact = FactId::from_bytes(&[73; 16]).unwrap();
    assert_eq!(
        checkpoint(missing_agreement),
        Err(CheckpointError::InvalidReference)
    );

    let mut wrong_agreement_policy = supplied.clone();
    wrong_agreement_policy.obligations[0]
        .agreement
        .source_policy = label("unaccepted-agreement-policy");
    assert_eq!(
        checkpoint(wrong_agreement_policy),
        Err(CheckpointError::InvalidReference)
    );

    let mut terminal_without_action = supplied.clone();
    terminal_without_action.obligations[0].status = ObligationStatus::Fulfilled;
    assert_eq!(
        checkpoint(terminal_without_action),
        Err(CheckpointError::InvalidReference)
    );

    let mut deadline_before_agreement = supplied;
    deadline_before_agreement.obligations[0].due = Some(LogicalTime {
        ticks: 100,
        ticks_per_second: 10,
    });
    assert_eq!(
        checkpoint(deadline_before_agreement),
        Err(CheckpointError::InvalidTime)
    );
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
#[test]
fn preserves_exact_reaction_window_continuation_and_spent_resources() {
    let mut supplied = state();
    supplied.facts.push(fact(7, 0));
    let mut before = pending();
    before.choices.push(AcceptedChoice {
        participant: member(3),
        offer: label("fixture-old-offer"),
        selected: label("fixture-old-selection"),
        source: rule(),
    });
    supplied.pending.push(before.clone());
    let checkpoint = checkpoint(supplied).unwrap();
    assert_eq!(checkpoint.state().pending, vec![before]);
    assert_eq!(checkpoint.state().resources[0].value, 4);
}
#[test]
fn rejects_stale_pending_basis_empty_offer_and_unknown_spent_resource() {
    for changed in 0..3 {
        let mut supplied = state();
        supplied.facts.push(fact(7, 0));
        let mut continuation = pending();
        match changed {
            0 => continuation.basis.revision = revision(2, 7),
            1 => continuation.next = PendingInput::Choice { remaining: vec![] },
            2 => continuation.spent[0].resource = label("fixture-unknown-resource"),
            _ => unreachable!(),
        }
        supplied.pending.push(continuation);
        assert_eq!(
            checkpoint(supplied),
            Err(match changed {
                0 => CheckpointError::StaleBasis,
                1 => CheckpointError::InvalidContinuation,
                2 => CheckpointError::InvalidReference,
                _ => unreachable!(),
            })
        );
    }
}
#[test]
fn preserves_unknown_dispatch_status_and_refuses_timer_kind_without_owned_instance() {
    let mut supplied = state();
    let intent = DurableIntent {
        id: EffectId::from_bytes(&[11; 16]).unwrap(),
        basis: basis(),
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        slot: 0,
        kind: EffectKind::RunAi,
        job: Some(JobId::from_bytes(&[12; 16]).unwrap()),
        timer: None,
        generation: 2,
        status: DurableStatus::SentUnknown,
        definition: content(),
    };
    supplied.intents.push(intent.clone());
    assert_eq!(
        checkpoint(supplied).unwrap().state().intents,
        vec![intent.clone()]
    );
    let mut supplied = state();
    let mut invalid = intent;
    invalid.kind = EffectKind::ArmTimer;
    invalid.job = None;
    invalid.timer = Some(TimerId::from_bytes(&[13; 16]).unwrap());
    supplied.intents.push(invalid);
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidIntent));
}
#[test]
fn counts_retained_claim_text_in_utf8_bytes() {
    let mut supplied = state();
    supplied.beliefs.push(AttributedClaim {
        id: RecordId::from_bytes(&[9; 16]).unwrap(),
        holder: entity(4),
        subject: entity(4),
        claim: "é".repeat(129),
        evidence: vec![],
        audience: AudienceScope::Shared,
        source: content(),
    });
    assert_eq!(checkpoint(supplied), Err(CheckpointError::Capacity));
}

#[test]
fn checkpoint_cannot_authorize_its_own_resource_maximum() {
    let mut supplied = state();
    supplied.resources[0].maximum = 100;
    supplied.resources[0].value = 99;
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidResource));
    let mut supplied = state();
    supplied.resources[0].minimum = -100;
    supplied.resources[0].value = -99;
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidResource));
    let rules = vec![rule()];
    let content_entries = vec![content()];
    assert_eq!(
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis(),
            pins(),
            state(),
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &[],
                assets: &[]
            },
            limits()
        ),
        Err(CheckpointError::InvalidResource)
    );
}

#[test]
fn rejects_reserved_heap_capacity_even_when_the_vector_has_no_live_records() {
    let normal = checkpoint(state()).unwrap();
    let mut supplied = state();
    supplied.facts = Vec::with_capacity(10_000);
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let resource_constraints = resource_constraints();
    let mut admitted = limits();
    admitted.maximum_retained_bytes = normal.retained_bytes().unwrap() + 128;
    assert_eq!(
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis(),
            pins(),
            supplied,
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &resource_constraints,
                assets: &[]
            },
            admitted
        ),
        Err(CheckpointError::Capacity)
    );
    assert!(normal.retained_bytes().unwrap() > normal.retained_heap_bytes().unwrap());
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

#[test]
fn rejects_missing_family_references_and_spare_nested_capacity() {
    let mut supplied = state();
    supplied.continuity.witnesses.push(WitnessRecord {
        id: RecordId::from_bytes(&[20; 16]).unwrap(),
        observer: entity(4),
        fact: FactId::from_bytes(&[99; 16]).unwrap(),
        perceived_at: supplied.logical_time,
        source: content(),
    });
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    let mut supplied = state();
    supplied.continuity.journal.push(JournalEntry {
        id: RecordId::from_bytes(&[20; 16]).unwrap(),
        source_facts: vec![],
        attributed_claims: vec![],
        audience: AudienceScope::Shared,
        text: String::with_capacity(2 * 1024 * 1024),
    });
    assert_eq!(checkpoint(supplied), Err(CheckpointError::Capacity));
}
#[test]
fn forbids_private_payload_routing_to_shared_room_and_locked_listener() {
    use df_types::ClientBindingId;
    let binding = ClientBindingId::from_bytes(&[30; 16]).unwrap();
    for changed in 0..2 {
        let mut supplied = state();
        supplied.continuity.presence.push(ParticipantPresence {
            member: member(3),
            bindings: vec![binding],
            state: PresenceKind::Connected,
            policy: content(),
        });
        supplied.continuity.audio = Some(AudioTopology {
            policy: content(),
            outputs: vec![AudioOutputLease {
                id: RecordId::from_bytes(&[31; 16]).unwrap(),
                destination: if changed == 0 {
                    AudioDestination::PublicRoom(binding)
                } else {
                    AudioDestination::PrivateListener {
                        member: member(3),
                        binding,
                    }
                },
                generation: 1,
                audience: AudienceScope::Members(vec![member(3)]),
                device_unlocked: false,
            }],
            captures: vec![],
        });
        assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
    }
}
#[test]
fn keeps_presence_separate_from_resources_logical_time_and_gameplay_membership() {
    use df_types::ClientBindingId;
    let mut supplied = state();
    let before_time = supplied.logical_time;
    let before_resource = supplied.resources.clone();
    let before_members = supplied.members.clone();
    supplied.continuity.presence.push(ParticipantPresence {
        member: member(3),
        bindings: vec![ClientBindingId::from_bytes(&[30; 16]).unwrap()],
        state: PresenceKind::VoluntaryAfk,
        policy: content(),
    });
    let checkpoint = checkpoint(supplied).unwrap();
    assert_eq!(checkpoint.state().logical_time, before_time);
    assert_eq!(checkpoint.state().resources, before_resource);
    assert_eq!(checkpoint.state().members, before_members);
}
#[test]
fn derived_summary_cannot_replace_missing_episode_or_canonical_fact() {
    let mut supplied = state();
    supplied.continuity.summaries.push(MemorySummary {
        id: RecordId::from_bytes(&[31; 16]).unwrap(),
        episodes: vec![RecordId::from_bytes(&[99; 16]).unwrap()],
        derived_claims: vec![],
        source_digest: pins().content.content_digest,
        source_revision: basis().revision,
        summarizer: label("fixture-summarizer"),
        model: label("fixture-model"),
        policy: content(),
        audience: AudienceScope::Shared,
        text: "synthetic summary".into(),
        incomplete: false,
    });
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
}
#[test]
fn redacted_or_unavailable_checkpoint_never_claims_intact_resume() {
    let mut supplied = state();
    supplied
        .continuity
        .recovery
        .redacted_records
        .push(RecordId::from_bytes(&[31; 16]).unwrap());
    let retained = checkpoint(supplied).unwrap();
    assert_eq!(
        retained.validate_resume(basis(), &pins()),
        Err(CheckpointError::RedactedCheckpoint)
    );
    let mut supplied = state();
    supplied
        .continuity
        .recovery
        .unavailable_sources
        .push(label("fixture-source-1"));
    let checkpoint = checkpoint(supplied).unwrap();
    assert_eq!(
        checkpoint.validate_resume(basis(), &pins()),
        Err(CheckpointError::UnavailableCheckpoint)
    );
}
#[test]
fn debug_fork_and_disaster_restore_require_distinct_supplied_origin_bindings() {
    for kind in [RestoreKind::DebugFork, RestoreKind::Disaster] {
        let mut supplied = state();
        supplied.continuity.recovery.origin = Some(RestoreOrigin {
            kind,
            checkpoint_digest: ContentDigest([31; 32]),
            origin: basis(),
            process_generation: 1,
        });
        assert_eq!(
            checkpoint(supplied),
            Err(match kind {
                RestoreKind::DebugFork => CheckpointError::WrongRun,
                RestoreKind::Disaster => CheckpointError::StaleBasis,
            })
        );
    }
    let mut supplied = state();
    let mut origin = basis();
    origin.revision = revision(1, 99);
    supplied.continuity.recovery.origin = Some(RestoreOrigin {
        kind: RestoreKind::Disaster,
        checkpoint_digest: ContentDigest([31; 32]),
        origin,
        process_generation: 2,
    });
    supplied
        .continuity
        .recovery
        .retired_epochs
        .push(RecoveryEpoch::new(1).unwrap());
    supplied
        .continuity
        .recovery
        .lost_ranges
        .push(LostGameRange {
            from: revision(1, 90),
            through: revision(1, 99),
        });
    assert!(checkpoint(supplied).is_ok());
}
#[test]
fn unverified_asset_or_asset_dependency_cycle_rejects() {
    let a = AssetReference {
        key: label("fixture-asset-a"),
        digest: ContentDigest([31; 32]),
        byte_length: 100,
        kind: AssetKind::Image,
    };
    let b = AssetReference {
        key: label("fixture-asset-b"),
        digest: ContentDigest([32; 32]),
        byte_length: 100,
        kind: AssetKind::Image,
    };
    let mut supplied = state();
    supplied.continuity.asset_dependencies = vec![
        AssetDependency {
            asset: a.clone(),
            prerequisites: vec![b.clone()],
        },
        AssetDependency {
            asset: b.clone(),
            prerequisites: vec![a.clone()],
        },
    ];
    assert_eq!(
        checkpoint(supplied.clone()),
        Err(CheckpointError::InvalidReference)
    );
    let rules = vec![rule()];
    let content_entries = vec![content()];
    let constraints = resource_constraints();
    assert_eq!(
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis(),
            pins(),
            supplied,
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &constraints,
                assets: &[a, b]
            },
            limits()
        ),
        Err(CheckpointError::InvalidReference)
    );
}

#[test]
fn native_input_retained_bytes_include_string_and_vector_capacities() {
    let input = GameInput::Game(CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[40; 16]).unwrap(),
        member: member(3),
        command: GameCommand::Speak {
            speaker: entity(4),
            text: String::with_capacity(4096),
            conversation: None,
        },
    });
    assert_eq!(input.retained_heap_bytes(), Some(4096));
    assert_eq!(
        input.retained_bytes(),
        Some(std::mem::size_of::<GameInput>() + 4096)
    );
    let input = GameInput::Job(JobCompletion {
        basis: basis(),
        operation: OperationId::from_bytes(&[40; 16]).unwrap(),
        job: JobId::from_bytes(&[41; 16]).unwrap(),
        generation: 1,
        outcome: JobOutcome::MemoryCandidates {
            records: Vec::with_capacity(16),
        },
    });
    assert_eq!(
        input.retained_heap_bytes(),
        Some(16 * std::mem::size_of::<RetrievedMemory>())
    );
}
#[test]
fn checkpoint_debug_output_omits_private_payload_and_choices() {
    let mut supplied = state();
    supplied.continuity.journal.push(JournalEntry {
        id: RecordId::from_bytes(&[43; 16]).unwrap(),
        source_facts: vec![],
        attributed_claims: vec![],
        audience: AudienceScope::Members(vec![member(3)]),
        text: "synthetic private witness report".into(),
    });
    let checkpoint = checkpoint(supplied).unwrap();
    let diagnostic = format!("{checkpoint:?}");
    assert!(diagnostic.contains("Checkpoint"));
    assert!(!diagnostic.contains("synthetic private witness report"));
    assert!(!diagnostic.contains("journal"));
}

#[test]
fn pending_roll_request_retains_only_the_current_resolution_and_window() {
    let resolution = ResolutionId::from_bytes(&[44; 16]).unwrap();
    let window = WindowId::from_bytes(&[45; 16]).unwrap();
    let input = GameInput::Game(CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[46; 16]).unwrap(),
        member: member(3),
        command: GameCommand::SubmitRoll { resolution, window },
    });
    assert_eq!(input.retained_heap_bytes(), Some(0));
    assert!(matches!(input, GameInput::Game(CommandInput {
        command: GameCommand::SubmitRoll { resolution: observed_resolution, window: observed_window }, ..
    }) if observed_resolution == resolution && observed_window == window));
}

#[test]
fn pending_retains_prior_window_draw_but_never_another_resolutions_draw() {
    let mut supplied = state();
    supplied.facts.push(fact(7, 0));
    let mut resolution = pending();
    let draw = ActualDraw {
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        ordinal: 0,
        resolution: resolution.id,
        window: WindowId::from_bytes(&[50; 16]).unwrap(),
        sides: 6,
        value: 4,
        source: rule(),
    };
    resolution
        .draw_ordinals
        .push((draw.operation, draw.ordinal));
    supplied.draws.push(draw);
    supplied.pending.push(resolution);
    assert!(checkpoint(supplied.clone()).is_ok());
    supplied.draws[0].resolution = ResolutionId::from_bytes(&[51; 16]).unwrap();
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidReference));
}

#[test]
fn duplicate_schedule_ids_reject_even_with_different_due_times() {
    let mut supplied = state();
    let first = ScheduledEvent {
        id: RecordId::from_bytes(&[52; 16]).unwrap(),
        entity: entity(4),
        due: supplied.logical_time,
        definition: content(),
    };
    let mut next = first.clone();
    next.due.ticks += 1;
    supplied.schedules = vec![first, next];
    assert_eq!(
        checkpoint(supplied),
        Err(CheckpointError::DuplicateIdentity)
    );
}

#[test]
fn private_output_and_capture_require_the_named_members_connection() {
    use df_types::ClientBindingId;
    let binding = ClientBindingId::from_bytes(&[53; 16]).unwrap();
    let mut supplied = state();
    supplied.members.push(MembershipLink {
        member: member(54),
        character: None,
    });
    supplied.continuity.presence.push(ParticipantPresence {
        member: member(3),
        bindings: vec![binding],
        state: PresenceKind::Connected,
        policy: content(),
    });
    supplied.continuity.audio = Some(AudioTopology {
        policy: content(),
        outputs: vec![AudioOutputLease {
            id: RecordId::from_bytes(&[55; 16]).unwrap(),
            destination: AudioDestination::PrivateListener {
                member: member(3),
                binding,
            },
            generation: 1,
            audience: AudienceScope::Members(vec![member(3)]),
            device_unlocked: true,
        }],
        captures: vec![CaptureLease {
            id: RecordId::from_bytes(&[56; 16]).unwrap(),
            member: member(3),
            binding,
            generation: 1,
            permitted_offer: None,
            format: label("fixture-audio"),
        }],
    });
    assert!(checkpoint(supplied.clone()).is_ok());
    let mut wrong_output = supplied.clone();
    let audio = wrong_output.continuity.audio.as_mut().unwrap();
    audio.outputs[0].destination = AudioDestination::PrivateListener {
        member: member(54),
        binding,
    };
    audio.outputs[0].audience = AudienceScope::Members(vec![member(54)]);
    assert_eq!(
        checkpoint(wrong_output),
        Err(CheckpointError::InvalidReference)
    );
    supplied.continuity.audio.as_mut().unwrap().captures[0].member = member(54);
    assert_eq!(checkpoint(supplied), Err(CheckpointError::InvalidIntent));
}

fn retrieved_state() -> GameState {
    let mut supplied = state();
    supplied.continuity.retrieval.push(RetrievalRequest {
        id: RecordId::from_bytes(&[57; 16]).unwrap(),
        basis: basis(),
        observer: member(3),
        purpose: RetrievalPurpose::PlayerRecall,
        topics: vec![],
        entities: vec![],
        from: supplied.logical_time,
        through: supplied.logical_time,
        maximum_items: 1,
        maximum_bytes: 2,
        maximum_tokens: 1,
        access_generation: 4,
        index_generation: 8,
        source_digest: pins().content.content_digest,
        policy: content(),
    });
    supplied.continuity.retrieved.push(RetrievedMemory {
        request: RecordId::from_bytes(&[57; 16]).unwrap(),
        episodes: vec![],
        facts: vec![],
        attributed_claims: vec![],
        snippets: vec!["ok".into()],
        source_revision: basis().revision,
        index_generation: 8,
        access_generation: 4,
        incomplete: false,
    });
    supplied
}

#[test]
fn retrieved_memory_matches_request_provenance_and_exact_byte_item_bounds() {
    assert!(checkpoint(retrieved_state()).is_ok());
    for change in 0..5 {
        let mut supplied = retrieved_state();
        let result = &mut supplied.continuity.retrieved[0];
        match change {
            0 => result.access_generation = 3,
            1 => result.index_generation = 7,
            2 => result.source_revision = revision(1, 8),
            3 => result.snippets[0].push('!'),
            4 => result.snippets.push(String::new()),
            _ => unreachable!(),
        }
        assert_eq!(
            checkpoint(supplied),
            Err(match change {
                0 | 1 => CheckpointError::InvalidReference,
                2 => CheckpointError::StaleBasis,
                _ => CheckpointError::Capacity,
            })
        );
    }
}

#[test]
fn aggregate_record_budget_counts_audience_members_in_all_state_families() {
    for continuity in [false, true] {
        let mut supplied = state();
        if continuity {
            supplied.continuity.journal.push(JournalEntry {
                id: RecordId::from_bytes(&[58; 16]).unwrap(),
                source_facts: vec![],
                attributed_claims: vec![],
                audience: AudienceScope::Members(vec![member(3)]),
                text: String::new(),
            });
        } else {
            let mut record = fact(7, 0);
            record.audience = AudienceScope::Members(vec![member(3)]);
            if let FactValue::ContentEvent { subjects, .. } = &mut record.value {
                subjects.clear();
            }
            supplied.facts.push(record);
        }
        // Four base records, one fact/journal record and one audience identity.
        let inventory = ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        };
        let mut bounds = limits();
        bounds.maximum_records = 6;
        assert!(
            Checkpoint::new(
                CHECKPOINT_SCHEMA,
                basis(),
                pins(),
                supplied.clone(),
                inventory,
                bounds
            )
            .is_ok()
        );
        bounds.maximum_records = 5;
        let inventory = ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        };
        assert_eq!(
            Checkpoint::new(
                CHECKPOINT_SCHEMA,
                basis(),
                pins(),
                supplied,
                inventory,
                bounds
            ),
            Err(CheckpointError::Capacity)
        );
    }
}

#[test]
fn duplicate_encounter_identity_cannot_preserve_conflicting_turn_state() {
    let mut supplied = state();
    let encounter = EncounterState {
        id: RecordId::from_bytes(&[60; 16]).unwrap(),
        definition: content(),
        participants: vec![entity(4)],
        turn_order: vec![entity(4)],
        active_turn: Some(entity(4)),
        objectives: vec![],
        combat_policy: content(),
    };
    supplied.encounters.push(encounter.clone());
    assert!(checkpoint(supplied.clone()).is_ok());
    let mut conflicting = encounter;
    conflicting.active_turn = None;
    supplied.encounters.push(conflicting);
    assert_eq!(
        checkpoint(supplied),
        Err(CheckpointError::DuplicateIdentity)
    );
}

#[test]
fn duplicate_asset_dependency_cannot_hide_a_self_cycle_or_conflicting_edges() {
    let asset = AssetReference {
        key: label("fixture-dependency-a"),
        digest: ContentDigest([60; 32]),
        byte_length: 1,
        kind: AssetKind::Image,
    };
    let dependency = AssetReference {
        key: label("fixture-dependency-b"),
        digest: ContentDigest([61; 32]),
        byte_length: 1,
        kind: AssetKind::Image,
    };
    for repeated in [false, true] {
        let mut supplied = state();
        supplied.continuity.asset_dependencies = vec![
            AssetDependency {
                asset: asset.clone(),
                prerequisites: vec![],
            },
            AssetDependency {
                asset: if repeated {
                    asset.clone()
                } else {
                    dependency.clone()
                },
                prerequisites: vec![asset.clone()],
            },
        ];
        let result = Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis(),
            pins(),
            supplied,
            ReferenceInventory {
                rules: &[rule()],
                content: &[content()],
                resources: &resource_constraints(),
                assets: &[asset.clone(), dependency.clone()],
            },
            limits(),
        );
        if repeated {
            assert_eq!(result, Err(CheckpointError::DuplicateIdentity));
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn records_with_owned_ids_are_unique_within_each_canonical_family() {
    let id = RecordId::from_bytes(&[62; 16]).unwrap();
    for family in 0..5 {
        let mut supplied = state();
        match family {
            0 => {
                let value = ThreatClock {
                    id,
                    definition: content(),
                    progress: 0,
                    capacity: 3,
                };
                supplied.threats.push(value.clone());
                assert!(checkpoint(supplied.clone()).is_ok());
                supplied.threats.push(value);
            }
            1 => {
                let value = ConversationState {
                    id,
                    participants: vec![entity(4)],
                    topic: content(),
                    accepted_facts: vec![],
                };
                supplied.conversations.push(value.clone());
                assert!(checkpoint(supplied.clone()).is_ok());
                supplied.conversations.push(value);
            }
            2 => {
                let agreement_fact = FactId::from_bytes(&[8; 16]).unwrap();
                let agreement_operation = OperationId::from_bytes(&[8; 16]).unwrap();
                supplied.facts.push(GameFact {
                    id: agreement_fact,
                    revision: basis().revision,
                    operation: agreement_operation,
                    ordinal: 0,
                    cause: None,
                    audience: AudienceScope::Shared,
                    value: FactValue::ContentEvent {
                        definition: content(),
                        subjects: vec![entity(4), entity(4)],
                    },
                });
                supplied.decisions.push(AcceptedDecision {
                    operation: agreement_operation,
                    revision: basis().revision,
                    facts: vec![agreement_fact],
                    draws: vec![],
                    effects: vec![],
                    source_policy: label("explicit-agreement-policy"),
                    semantic_output: None,
                });
                let value = Obligation {
                    id,
                    obligor: entity(4),
                    beneficiary: entity(4),
                    definition: content(),
                    terms: content(),
                    due: None,
                    agreement: ObligationAgreement {
                        source: content(),
                        fact: agreement_fact,
                        source_policy: label("explicit-agreement-policy"),
                        at: supplied.logical_time,
                    },
                    status: ObligationStatus::Active,
                    transition: None,
                };
                supplied.obligations.push(value.clone());
                assert!(checkpoint(supplied.clone()).is_ok());
                supplied.obligations.push(value);
            }
            3 => {
                let value = PresentationDemand {
                    id,
                    definition: content(),
                    audience: AudienceScope::Shared,
                    causal_facts: vec![],
                    source_revision: basis().revision,
                };
                supplied.presentation.push(value.clone());
                assert!(checkpoint(supplied.clone()).is_ok());
                supplied.presentation.push(value);
            }
            4 => {
                let pending = pending();
                supplied.facts.push(fact(7, 0));
                supplied.pending.push(pending.clone());
                let value = CriticalCueEligibility {
                    id,
                    fact: fact(7, 0).id,
                    resolution: pending.id,
                    audience: AudienceScope::Shared,
                    ready_assets: vec![],
                    policy: content(),
                    maximum_duration_ticks: 1,
                };
                supplied.continuity.critical_cues.push(value.clone());
                assert!(checkpoint(supplied.clone()).is_ok());
                supplied.continuity.critical_cues.push(value);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            checkpoint(supplied),
            Err(CheckpointError::DuplicateIdentity)
        );
    }
}

fn retained_mode_demand(mode: ExecutionMode, maximum_bytes: u64) -> GameState {
    let mut state = state();
    let moment = RecordId::from_bytes(&[91; 16]).unwrap();
    state.continuity.moments.push(NarrativeMoment {
        id: moment,
        location: entity(4),
        characters: vec![entity(4)],
        facts: vec![],
        attributed_claims: vec![],
        audience: AudienceScope::Members(vec![member(3)]),
        semantic_focus: content(),
        identity_revision: label("fixture-entity-1"),
    });
    state.continuity.demands.push(AssetDemand {
        id: RecordId::from_bytes(&[92; 16]).unwrap(),
        basis: basis(),
        key: AssetRequestKey {
            schema: CHECKPOINT_SCHEMA,
            source: pins().content.content_digest,
            moment,
            identity: label("fixture-entity-1"),
            style: label("fixture-style-1"),
            voice: None,
            provider: label("fixture-provider-1"),
            model: label("fixture-model-1"),
            format: label("png-v1"),
            references: vec![],
            audience: AudienceScope::Members(vec![member(3)]),
            parameters: label("fixture-parameters-1"),
        },
        priority: DemandPriority::Optional,
        mode,
        expires: LogicalTime {
            ticks: 240,
            ticks_per_second: 10,
        },
        budget_reservation: label("fixture-reservation-1"),
        maximum_bytes,
        policy: content(),
    });
    state
}

#[test]
fn retained_mode_demand_keeps_its_admitted_mode_when_default_changes() {
    let modes = [
        ExecutionMode::Live,
        ExecutionMode::PreparedOnly,
        ExecutionMode::Replay,
    ];
    for admitted in modes {
        for configured in modes {
            let mut supplied = retained_mode_demand(admitted, 1024);
            let retained = supplied.continuity.demands.clone();
            supplied.mode = configured;
            let checkpoint = checkpoint(supplied).unwrap();
            assert_eq!(checkpoint.state().mode, configured);
            assert_eq!(checkpoint.state().continuity.demands, retained);
            checkpoint.validate_resume(basis(), &pins()).unwrap();
        }
    }
}

#[test]
fn retained_mode_difference_does_not_relax_demand_byte_bound() {
    for configured in [ExecutionMode::Live, ExecutionMode::PreparedOnly] {
        let mut supplied = retained_mode_demand(ExecutionMode::Replay, 0);
        supplied.mode = configured;
        assert_eq!(checkpoint(supplied), Err(CheckpointError::Capacity));
    }
}

fn retained_bookend_fallback() -> AssetReference {
    AssetReference {
        key: label("retained-bookend-still-1"),
        digest: ContentDigest([93; 32]),
        byte_length: 128,
        kind: AssetKind::Image,
    }
}

// Canonical stored-plan fixtures only; this does not authorize cinematic jobs or media access.
fn retained_bookend_state(kind: BookendKind, admitted: ExecutionMode) -> GameState {
    let mut supplied = state();
    supplied.mode = admitted;
    supplied.facts.push(fact(7, 0));
    let claim = RecordId::from_bytes(&[94; 16]).unwrap();
    supplied.beliefs.push(AttributedClaim {
        id: claim,
        holder: entity(4),
        subject: entity(4),
        claim: "Synthetic attributed belief".into(),
        evidence: vec![FactId::from_bytes(&[7; 16]).unwrap()],
        audience: AudienceScope::Members(vec![member(3)]),
        source: content(),
    });
    let moment = RecordId::from_bytes(&[95; 16]).unwrap();
    let audience = AudienceScope::Members(vec![member(3)]);
    supplied.continuity.moments.push(NarrativeMoment {
        id: moment,
        location: entity(4),
        characters: vec![entity(4)],
        facts: vec![FactId::from_bytes(&[7; 16]).unwrap()],
        attributed_claims: vec![claim],
        audience: audience.clone(),
        semantic_focus: content(),
        identity_revision: label("fixture-entity-1"),
    });
    supplied.continuity.bookends.push(BookendPlan {
        spec: BookendSpec {
            id: RecordId::from_bytes(&[96; 16]).unwrap(),
            basis: basis(),
            kind,
            from: basis().revision,
            through: basis().revision,
            audience: audience.clone(),
            locale: df_types::LocaleTag::parse("en").unwrap(),
            policy: content(),
            maximum_shots: 1,
            maximum_duration_ticks: 40,
            maximum_text_bytes: 128,
            maximum_asset_bytes: 256,
            expires: LogicalTime {
                ticks: 240,
                ticks_per_second: 10,
            },
            mode: admitted,
            budget_reservation: label("synthetic-retained-budget-1"),
        },
        selection: FactSelection {
            facts: vec![FactId::from_bytes(&[7; 16]).unwrap()],
            attributed_claims: vec![claim],
            audience: audience.clone(),
        },
        shots: vec![ShotPlan {
            id: RecordId::from_bytes(&[97; 16]).unwrap(),
            moment,
            subjects: vec![entity(4)],
            audience,
            duration_ticks: 20,
            definition: content(),
            references: vec![retained_bookend_fallback()],
            performance: PerformanceHint {
                definition: content(),
                voice: None,
                emphasis_facts: vec![FactId::from_bytes(&[7; 16]).unwrap()],
            },
        }],
        captions: vec!["Synthetic established event and attributed belief".into()],
        fallback: retained_bookend_fallback(),
    });
    supplied
}

fn restore_retained_bookend(
    supplied: GameState,
    admitted_limits: CheckpointLimits,
) -> Result<Checkpoint, CheckpointError> {
    let rules = [rule()];
    let contents = [content()];
    let constraints = resource_constraints();
    let assets = [retained_bookend_fallback()];
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        supplied,
        ReferenceInventory {
            rules: &rules,
            content: &contents,
            resources: &constraints,
            assets: &assets,
        },
        admitted_limits,
    )
}

#[test]
fn retained_bookend_modes_restore_exact_recap_and_trailer_across_all_defaults() {
    let modes = [
        ExecutionMode::Live,
        ExecutionMode::PreparedOnly,
        ExecutionMode::Replay,
    ];
    for kind in [BookendKind::Recap, BookendKind::SpeculativeTrailer] {
        for admitted in modes {
            let original =
                restore_retained_bookend(retained_bookend_state(kind, admitted), limits()).unwrap();
            for configured in modes {
                let mut supplied = original.state().clone();
                supplied.mode = configured;
                let expected = supplied.clone();
                let restored = restore_retained_bookend(supplied, limits()).unwrap();
                assert_eq!(restored.state(), &expected);
                assert_eq!(
                    restored.state().continuity.bookends,
                    original.state().continuity.bookends
                );
                assert_eq!(restored.state().continuity.bookends[0].spec.mode, admitted);
                assert_eq!(restored.state().mode, configured);
                restored.validate_resume(basis(), &pins()).unwrap();
            }
        }
    }
}

#[test]
fn retained_bookend_mode_difference_preserves_session_run_basis_and_range_checks() {
    for kind in [BookendKind::Recap, BookendKind::SpeculativeTrailer] {
        let mut original = retained_bookend_state(kind, ExecutionMode::Replay);
        original.mode = ExecutionMode::Live;
        let retained = restore_retained_bookend(original.clone(), limits()).unwrap();
        for mutation in 0..6 {
            let mut supplied = original.clone();
            let spec = &mut supplied.continuity.bookends[0].spec;
            match mutation {
                0 => spec.basis.session = SessionId::from_bytes(&[98; 16]).unwrap(),
                1 => spec.basis.run = RunId::from_bytes(&[98; 16]).unwrap(),
                2 => spec.basis.revision = revision(3, 0),
                3 => spec.basis.revision = revision(2, 9),
                4 => spec.from = revision(2, 9),
                _ => spec.through = revision(2, 9),
            }
            assert_eq!(
                restore_retained_bookend(supplied, limits()),
                Err(if mutation < 4 {
                    CheckpointError::StaleBasis
                } else {
                    CheckpointError::InvalidReference
                })
            );
            assert_eq!(retained.state(), &original);
        }
    }
}

#[test]
fn retained_bookend_mode_difference_preserves_audience_and_exact_reference_checks() {
    for kind in [BookendKind::Recap, BookendKind::SpeculativeTrailer] {
        let mut original = retained_bookend_state(kind, ExecutionMode::Replay);
        original.mode = ExecutionMode::PreparedOnly;
        let retained = restore_retained_bookend(original.clone(), limits()).unwrap();
        for mutation in 0..10 {
            let mut supplied = original.clone();
            let plan = &mut supplied.continuity.bookends[0];
            match mutation {
                0 => plan.selection.audience = AudienceScope::Host,
                1 => plan.shots[0].audience = AudienceScope::Host,
                2 => plan.spec.policy.entry = label("missing-cinematic-policy"),
                3 => plan.fallback.key = label("unpublished-fallback"),
                4 => plan.selection.facts[0] = FactId::from_bytes(&[98; 16]).unwrap(),
                5 => plan.selection.attributed_claims[0] = RecordId::from_bytes(&[98; 16]).unwrap(),
                6 => plan.shots[0].subjects[0] = entity(98),
                7 => plan.shots[0].moment = RecordId::from_bytes(&[98; 16]).unwrap(),
                8 => plan.shots[0].references[0].digest = ContentDigest([98; 32]),
                _ => {
                    plan.shots[0].performance.definition.entry = label("missing-performance-policy")
                }
            }
            assert_eq!(
                restore_retained_bookend(supplied, limits()),
                Err(CheckpointError::InvalidReference)
            );
            assert_eq!(retained.state(), &original);
        }
    }
}

#[test]
fn retained_bookend_mode_difference_preserves_declared_bounds_and_expiry_validation() {
    for kind in [BookendKind::Recap, BookendKind::SpeculativeTrailer] {
        let mut original = retained_bookend_state(kind, ExecutionMode::Replay);
        original.mode = ExecutionMode::Live;
        for mutation in 0..6 {
            let mut supplied = original.clone();
            let plan = &mut supplied.continuity.bookends[0];
            match mutation {
                0 => plan.spec.maximum_shots = 0,
                1 => plan.spec.maximum_duration_ticks = 0,
                2 => plan.spec.maximum_text_bytes = 0,
                3 => plan.spec.maximum_asset_bytes = 0,
                4 => plan.shots[0].duration_ticks = 0,
                _ => plan.spec.expires.ticks_per_second = 0,
            }
            assert_eq!(
                restore_retained_bookend(supplied, limits()),
                Err(if mutation == 5 {
                    CheckpointError::InvalidTime
                } else {
                    CheckpointError::InvalidReference
                })
            );
        }
    }
}

#[test]
fn retained_bookend_mode_difference_preserves_shot_text_record_and_owned_byte_caps() {
    for kind in [BookendKind::Recap, BookendKind::SpeculativeTrailer] {
        let mut original = retained_bookend_state(kind, ExecutionMode::Replay);
        original.mode = ExecutionMode::PreparedOnly;
        let retained = restore_retained_bookend(original.clone(), limits()).unwrap();
        for mutation in 0..5 {
            let mut supplied = original.clone();
            let mut admitted = limits();
            match mutation {
                0 => {
                    let shot = supplied.continuity.bookends[0].shots[0].clone();
                    supplied.continuity.bookends[0].shots.push(shot);
                }
                1 => {
                    supplied.continuity.bookends[0].captions[0] =
                        "x".repeat(admitted.maximum_text_bytes + 1)
                }
                2 => admitted.maximum_records = 1,
                3 => admitted.maximum_total_text_bytes = 1,
                _ => admitted.maximum_retained_bytes = retained.retained_bytes().unwrap() - 1,
            }
            assert_eq!(
                restore_retained_bookend(supplied, admitted),
                Err(CheckpointError::Capacity)
            );
            assert_eq!(retained.state(), &original);
        }
    }
}

#[test]
fn retained_bookend_mode_difference_does_not_bypass_resume_basis_or_source_pins() {
    let mut supplied = retained_bookend_state(BookendKind::Recap, ExecutionMode::Replay);
    supplied.mode = ExecutionMode::Live;
    let retained = restore_retained_bookend(supplied, limits()).unwrap();
    let mut expected = basis();
    expected.session = SessionId::from_bytes(&[98; 16]).unwrap();
    assert_eq!(
        retained.validate_resume(expected, &pins()),
        Err(CheckpointError::WrongSession)
    );
    let mut expected = basis();
    expected.run = RunId::from_bytes(&[98; 16]).unwrap();
    assert_eq!(
        retained.validate_resume(expected, &pins()),
        Err(CheckpointError::WrongRun)
    );
    let mut expected = basis();
    expected.revision = revision(3, 0);
    assert_eq!(
        retained.validate_resume(expected, &pins()),
        Err(CheckpointError::StaleBasis)
    );
    let mut source = pins();
    source.content.content_digest = ContentDigest([98; 32]);
    assert_eq!(
        retained.validate_resume(basis(), &source),
        Err(CheckpointError::ContentMismatch)
    );
}

#[test]
fn committed_outcome_reference_borrows_exact_retained_fact_and_decision() {
    let retained = checkpoint(accepted_draw_decision()).unwrap();
    let id = retained.state().facts[0].id;
    let (fact, decision) = retained.committed_outcome(id).unwrap();

    assert!(std::ptr::eq(fact, &retained.state().facts[0]));
    assert!(std::ptr::eq(decision, &retained.state().decisions[0]));
    assert_eq!(fact.operation, decision.operation);
    assert_eq!(fact.revision, decision.revision);
    assert_eq!(decision.facts, vec![id]);
    assert_eq!(decision.source_policy, label("fixture-policy-1"));
    assert_eq!(
        decision.semantic_output.as_deref(),
        Some("accepted exact outcome")
    );
    assert!(
        retained
            .committed_outcome(FactId::from_bytes(&[120; 16]).unwrap())
            .is_none()
    );
}

#[test]
fn seed_facts_remain_valid_without_committed_outcome_provenance() {
    let mut supplied = state();
    let seeded = fact(118, 0);
    supplied.facts.push(seeded.clone());
    supplied.continuity.moments.push(NarrativeMoment {
        id: RecordId::from_bytes(&[119; 16]).unwrap(),
        location: entity(4),
        characters: vec![entity(4)],
        facts: vec![seeded.id],
        attributed_claims: vec![],
        audience: AudienceScope::Shared,
        semantic_focus: content(),
        identity_revision: label("fixture-entity-1"),
    });
    let mut identity_only = supplied.continuity.moments[0].clone();
    identity_only.id = RecordId::from_bytes(&[120; 16]).unwrap();
    identity_only.facts.clear();
    supplied.continuity.moments.push(identity_only);
    let original = supplied.clone();
    let retained = checkpoint(supplied).unwrap();

    assert!(retained.committed_outcome(seeded.id).is_none());
    assert_eq!(retained.state(), &original);
    assert!(retained.validate_resume(basis(), &pins()).is_ok());
}
