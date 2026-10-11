use df_knowledge::perception::{ObserverScope, PerceptionError, PerceptionLimits};
use df_knowledge::witness_projection::perceive_witnessed_fact;
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
        members: vec![
            MembershipLink {
                member: member(3),
                character: None,
            },
            MembershipLink {
                member: member(5),
                character: None,
            },
        ],
        entities: vec![WorldEntity {
            id: entity(4),
            definition: content(),
            location: None,
            position: Some(Position { x: 0, y: 0, z: 0 }),
            identity_revision: label("fixture-entity-1"),
        }],
        characters: vec![],
        resources: vec![],
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
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis(),
        pins(),
        state,
        ReferenceInventory {
            rules: &rules,
            content: &content_entries,
            resources: &[],
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
            subjects: vec![],
        },
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

fn selection_limits() -> PerceptionLimits {
    PerceptionLimits {
        maximum_scan_records: 16,
        maximum_member_comparisons: 16,
        maximum_selected_facts: 8,
    }
}

fn witness_id() -> RecordId {
    RecordId::from_bytes(&[20; 16]).unwrap()
}

fn witnessed_state(audience: AudienceScope) -> GameState {
    let mut current = state();
    let mut event = fact(10, 0);
    event.audience = audience;
    current.continuity.witnesses.push(WitnessRecord {
        id: witness_id(),
        observer: entity(4),
        fact: event.id,
        perceived_at: LogicalTime {
            ticks: 100,
            ticks_per_second: 10,
        },
        source: content(),
    });
    current.facts.push(event);
    current
}

fn projection(
    current: &Checkpoint,
    observer: ObserverScope,
) -> Option<(Basis, &WitnessRecord, &GameFact)> {
    perceive_witnessed_fact(
        current,
        current.basis(),
        &pins(),
        observer,
        witness_id(),
        selection_limits(),
    )
    .unwrap()
}

#[test]
fn current_audience_basis_is_separate_from_retained_event_and_witness_provenance() {
    let current = checkpoint(witnessed_state(AudienceScope::Shared)).unwrap();
    let (audience_basis, witness, event) = projection(&current, ObserverScope::Shared).unwrap();
    assert_eq!(audience_basis, current.basis());
    assert!(std::ptr::eq(
        witness,
        &current.state().continuity.witnesses[0]
    ));
    assert!(std::ptr::eq(event, &current.state().facts[0]));
    assert_eq!(event.operation, OperationId::from_bytes(&[6; 16]).unwrap());
    assert_eq!(event.ordinal, 0);
    assert_eq!(witness.perceived_at.ticks, 100);
    let mut next_basis = current.basis();
    next_basis.revision = revision(2, 9);
    let next = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        next_basis,
        pins(),
        current.state().clone(),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    let (next_audience, next_witness, next_event) =
        projection(&next, ObserverScope::Shared).unwrap();
    assert_eq!(next_audience, next_basis);
    assert_ne!(next_audience.revision, next_event.revision);
    assert_eq!(next_event, event);
    assert_eq!(next_witness, witness);
}

#[test]
fn member_audience_is_selected_by_current_authenticated_observer_not_npc_witness() {
    let current = checkpoint(witnessed_state(AudienceScope::Members(vec![member(3)]))).unwrap();
    assert!(projection(&current, ObserverScope::Member(member(3))).is_some());
    assert!(projection(&current, ObserverScope::Member(member(5))).is_none());
    assert!(projection(&current, ObserverScope::Shared).is_none());
}

#[test]
fn irrelevant_hidden_fact_changes_preserve_exact_public_provenance() {
    let mut left = witnessed_state(AudienceScope::Shared);
    let mut hidden = fact(11, 1);
    hidden.audience = AudienceScope::Host;
    left.facts.push(hidden);
    let mut right = left.clone();
    right.facts[1].value = FactValue::ContentEvent {
        definition: content(),
        subjects: vec![entity(4)],
    };
    let left = checkpoint(left).unwrap();
    let right = checkpoint(right).unwrap();
    let public = projection(&left, ObserverScope::Shared).unwrap();
    let paired = projection(&right, ObserverScope::Shared).unwrap();
    assert_eq!(public, paired);
    assert_ne!(left.state().facts[1], right.state().facts[1]);
}

#[test]
fn stale_basis_and_source_pins_refuse_before_provenance_selection() {
    let current = checkpoint(witnessed_state(AudienceScope::Shared)).unwrap();
    let mut stale = current.basis();
    stale.revision = revision(2, 7);
    assert!(matches!(
        perceive_witnessed_fact(
            &current,
            stale,
            &pins(),
            ObserverScope::Shared,
            witness_id(),
            selection_limits()
        ),
        Err(PerceptionError::Checkpoint(_))
    ));
    let mut wrong_pins = pins();
    wrong_pins.content.content_digest = ContentDigest([99; 32]);
    assert!(matches!(
        perceive_witnessed_fact(
            &current,
            current.basis(),
            &wrong_pins,
            ObserverScope::Shared,
            witness_id(),
            selection_limits()
        ),
        Err(PerceptionError::Checkpoint(_))
    ));
}

#[test]
fn removed_membership_refuses_even_when_retained_fact_is_shared() {
    let mut current = witnessed_state(AudienceScope::Shared);
    current.members.retain(|link| link.member != member(3));
    let current = checkpoint(current).unwrap();
    assert!(matches!(
        perceive_witnessed_fact(
            &current,
            current.basis(),
            &pins(),
            ObserverScope::Member(member(3)),
            witness_id(),
            selection_limits()
        ),
        Err(PerceptionError::ObserverUnavailable)
    ));
    assert!(projection(&current, ObserverScope::Shared).is_some());
}

#[test]
fn missing_witness_or_hidden_witnessed_fact_returns_no_fabricated_proof() {
    let current = checkpoint(witnessed_state(AudienceScope::Shared)).unwrap();
    assert!(
        perceive_witnessed_fact(
            &current,
            current.basis(),
            &pins(),
            ObserverScope::Shared,
            RecordId::from_bytes(&[21; 16]).unwrap(),
            selection_limits(),
        )
        .unwrap()
        .is_none()
    );
    let hidden = checkpoint(witnessed_state(AudienceScope::Host)).unwrap();
    assert!(projection(&hidden, ObserverScope::Shared).is_none());
    assert!(projection(&hidden, ObserverScope::Member(member(3))).is_none());
}

#[test]
fn whole_lookup_work_and_perception_output_have_typed_finite_bounds() {
    let current = checkpoint(witnessed_state(AudienceScope::Shared)).unwrap();
    let mut work = selection_limits();
    work.maximum_scan_records = 3;
    assert!(
        perceive_witnessed_fact(
            &current,
            current.basis(),
            &pins(),
            ObserverScope::Shared,
            witness_id(),
            work,
        )
        .unwrap()
        .is_some()
    );
    work.maximum_scan_records = 2;
    assert!(matches!(
        perceive_witnessed_fact(
            &current,
            current.basis(),
            &pins(),
            ObserverScope::Shared,
            witness_id(),
            work,
        ),
        Err(PerceptionError::ScanCapacity)
    ));
    work = selection_limits();
    work.maximum_selected_facts = 0;
    assert!(matches!(
        perceive_witnessed_fact(
            &current,
            current.basis(),
            &pins(),
            ObserverScope::Shared,
            witness_id(),
            work,
        ),
        Err(PerceptionError::ResultCapacity)
    ));
    work = selection_limits();
    work.maximum_member_comparisons = 0;
    assert!(matches!(
        perceive_witnessed_fact(
            &current,
            current.basis(),
            &pins(),
            ObserverScope::Member(member(3)),
            witness_id(),
            work,
        ),
        Err(PerceptionError::ComparisonCapacity)
    ));
}
