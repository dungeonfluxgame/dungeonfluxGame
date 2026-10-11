use df_knowledge::perception::{ObserverScope, PerceptionError, PerceptionLimits, perceive};
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

fn audience_state() -> GameState {
    let mut current = state();
    for (index, audience) in [
        AudienceScope::Shared,
        AudienceScope::Members(vec![member(3)]),
        AudienceScope::Members(vec![member(5)]),
        AudienceScope::Host,
    ]
    .into_iter()
    .enumerate()
    {
        let mut value = fact(10 + index as u8, index as u32);
        value.audience = audience;
        current.facts.push(value);
    }
    current
}

#[test]
fn current_observers_select_only_explicit_audience_and_borrow_exact_canonical_order() {
    let current = checkpoint(audience_state()).unwrap();
    let original = current.clone();
    for (observer, indices) in [
        (ObserverScope::Shared, vec![0]),
        (ObserverScope::Member(member(3)), vec![0, 1]),
        (ObserverScope::Member(member(5)), vec![0, 2]),
    ] {
        let selected = perceive(&current, basis(), &pins(), observer, selection_limits()).unwrap();
        assert_eq!(selected.basis(), current.basis());
        assert!(std::ptr::eq(selected.pins(), current.pins()));
        assert_eq!(selected.facts().len(), indices.len());
        for (value, index) in selected.facts().iter().zip(indices) {
            assert!(std::ptr::eq(*value, &current.state().facts[index]));
        }
    }
    assert_eq!(current, original);
}

#[test]
fn grants_attributed_claims_and_rumors_never_widen_canonical_audience() {
    let mut supplied = audience_state();
    supplied.knowledge = vec![
        KnowledgeGrant {
            observer: member(3),
            fact: supplied.facts[2].id,
            source: supplied.facts[0].id,
        },
        KnowledgeGrant {
            observer: member(3),
            fact: supplied.facts[3].id,
            source: supplied.facts[0].id,
        },
    ];
    supplied.beliefs.push(AttributedClaim {
        id: RecordId::from_bytes(&[30; 16]).unwrap(),
        holder: entity(4),
        subject: entity(4),
        claim: "A false claim cannot reveal a private canonical fact".to_owned(),
        evidence: vec![supplied.facts[2].id],
        audience: AudienceScope::Shared,
        source: content(),
    });
    supplied.continuity.rumors.push(RumorTransmission {
        id: RecordId::from_bytes(&[31; 16]).unwrap(),
        claim: supplied.beliefs[0].id,
        sender: entity(4),
        recipient: entity(4),
        evidence: vec![supplied.facts[2].id, supplied.facts[3].id],
        policy: content(),
        remaining_hops: 1,
        audience: AudienceScope::Shared,
    });
    let current = checkpoint(supplied).unwrap();
    let selected = perceive(
        &current,
        basis(),
        &pins(),
        ObserverScope::Member(member(3)),
        selection_limits(),
    )
    .unwrap();
    assert_eq!(
        selected
            .facts()
            .iter()
            .map(|fact| fact.id)
            .collect::<Vec<_>>(),
        vec![current.state().facts[0].id, current.state().facts[1].id]
    );
    let shared = perceive(
        &current,
        basis(),
        &pins(),
        ObserverScope::Shared,
        selection_limits(),
    )
    .unwrap();
    assert_eq!(shared.facts().len(), 1);
}

#[test]
fn current_audience_removal_and_current_membership_removal_revoke_selection() {
    let previous = checkpoint(audience_state()).unwrap();
    let mut supplied = previous.state().clone();
    supplied.facts[1].audience = AudienceScope::Host;
    let changed = checkpoint(supplied.clone()).unwrap();
    let selected = perceive(
        &changed,
        basis(),
        &pins(),
        ObserverScope::Member(member(3)),
        selection_limits(),
    )
    .unwrap();
    assert_eq!(selected.facts().len(), 1);
    supplied.members.retain(|link| link.member != member(3));
    let removed = checkpoint(supplied).unwrap();
    assert_eq!(
        perceive(
            &removed,
            basis(),
            &pins(),
            ObserverScope::Member(member(3)),
            selection_limits()
        )
        .unwrap_err(),
        PerceptionError::ObserverUnavailable
    );
    assert_eq!(
        perceive(
            &previous,
            basis(),
            &pins(),
            ObserverScope::Member(member(99)),
            selection_limits()
        )
        .unwrap_err(),
        PerceptionError::ObserverUnavailable
    );
}

#[test]
fn every_basis_identity_and_revision_must_match_current_checkpoint() {
    let current = checkpoint(audience_state()).unwrap();
    let mut session = basis();
    session.session = SessionId::from_bytes(&[99; 16]).unwrap();
    let mut run = basis();
    run.run = RunId::from_bytes(&[99; 16]).unwrap();
    let mut stale = basis();
    stale.revision = revision(2, 7);
    let mut future = basis();
    future.revision = revision(2, 9);
    let mut epoch = basis();
    epoch.revision = revision(1, 8);
    for (expected, error) in [
        (session, CheckpointError::WrongSession),
        (run, CheckpointError::WrongRun),
        (stale, CheckpointError::StaleBasis),
        (future, CheckpointError::StaleBasis),
        (epoch, CheckpointError::StaleBasis),
    ] {
        assert_eq!(
            perceive(
                &current,
                expected,
                &pins(),
                ObserverScope::Shared,
                selection_limits()
            )
            .unwrap_err(),
            PerceptionError::Checkpoint(error)
        );
    }
}

#[test]
fn rules_content_and_build_pins_are_exact_and_cannot_be_self_admitted() {
    let current = checkpoint(audience_state()).unwrap();
    for changed in 0..12 {
        let mut admitted = pins();
        let error = match changed {
            0..=7 => {
                match changed {
                    0 => admitted.rules.mode = RulesMode::DisclosedCustom,
                    1 => admitted.rules.ruleset = label("changed"),
                    2 => admitted.rules.catalog = label("changed"),
                    3 => admitted.rules.catalog_digest.0[0] ^= 1,
                    4 => admitted.rules.source_manifest = label("changed"),
                    5 => admitted.rules.source_manifest_digest.0[0] ^= 1,
                    6 => admitted.rules.handler = label("changed"),
                    _ => admitted.rules.handler_digest.0[0] ^= 1,
                }
                CheckpointError::RulesMismatch
            }
            _ => {
                match changed {
                    8 => admitted.content.content = label("changed"),
                    9 => admitted.content.content_digest.0[0] ^= 1,
                    10 => admitted.content.package = label("changed"),
                    _ => admitted.content.package_digest.0[0] ^= 1,
                }
                CheckpointError::ContentMismatch
            }
        };
        assert_eq!(
            perceive(
                &current,
                basis(),
                &admitted,
                ObserverScope::Shared,
                selection_limits()
            )
            .unwrap_err(),
            PerceptionError::Checkpoint(error)
        );
    }
    for field in 0..5 {
        let mut admitted = pins();
        let mut values = [
            "fixture-source-1",
            "fixture-native-1",
            "fixture-wasm-1",
            "fixture-config-1",
            "fixture-content-1",
        ];
        values[field] = "changed";
        admitted.build = BuildIdentity::new(
            Some(values[0]),
            Some(values[1]),
            Some(values[2]),
            Some(values[3]),
            Some(values[4]),
        )
        .unwrap();
        assert_eq!(
            perceive(
                &current,
                basis(),
                &admitted,
                ObserverScope::Shared,
                selection_limits()
            )
            .unwrap_err(),
            PerceptionError::Checkpoint(CheckpointError::BuildMismatch)
        );
    }
}

#[test]
fn bounded_scans_comparisons_and_results_refuse_without_a_partial_selection() {
    let current = checkpoint(audience_state()).unwrap();
    let original = current.clone();
    let exact = PerceptionLimits {
        maximum_scan_records: 5,
        maximum_member_comparisons: 3,
        maximum_selected_facts: 2,
    };
    assert_eq!(
        perceive(
            &current,
            basis(),
            &pins(),
            ObserverScope::Member(member(3)),
            exact
        )
        .unwrap()
        .facts()
        .len(),
        2
    );
    for (limits, error) in [
        (
            PerceptionLimits {
                maximum_scan_records: 4,
                ..exact
            },
            PerceptionError::ScanCapacity,
        ),
        (
            PerceptionLimits {
                maximum_member_comparisons: 2,
                ..exact
            },
            PerceptionError::ComparisonCapacity,
        ),
        (
            PerceptionLimits {
                maximum_selected_facts: 1,
                ..exact
            },
            PerceptionError::ResultCapacity,
        ),
        (
            PerceptionLimits {
                maximum_scan_records: 0,
                ..exact
            },
            PerceptionError::ScanCapacity,
        ),
        (
            PerceptionLimits {
                maximum_member_comparisons: 0,
                ..exact
            },
            PerceptionError::ComparisonCapacity,
        ),
        (
            PerceptionLimits {
                maximum_selected_facts: 0,
                ..exact
            },
            PerceptionError::ResultCapacity,
        ),
    ] {
        assert_eq!(
            perceive(
                &current,
                basis(),
                &pins(),
                ObserverScope::Member(member(3)),
                limits
            )
            .unwrap_err(),
            error
        );
        assert_eq!(current, original);
    }
    let empty = checkpoint(state()).unwrap();
    let zero = PerceptionLimits {
        maximum_scan_records: 0,
        maximum_member_comparisons: 0,
        maximum_selected_facts: 0,
    };
    assert!(
        perceive(&empty, basis(), &pins(), ObserverScope::Shared, zero)
            .unwrap()
            .facts()
            .is_empty()
    );
    let shared = PerceptionLimits {
        maximum_scan_records: 4,
        maximum_member_comparisons: 0,
        maximum_selected_facts: 1,
    };
    assert_eq!(
        perceive(&current, basis(), &pins(), ObserverScope::Shared, shared)
            .unwrap()
            .facts()
            .len(),
        1
    );
}

#[test]
fn each_explicit_current_recipient_of_a_members_audience_is_selected() {
    let mut state = audience_state();
    state.facts[1].audience = AudienceScope::Members(vec![member(5), member(3)]);
    let current = checkpoint(state).unwrap();
    for observer in [member(3), member(5)] {
        let selected = perceive(
            &current,
            basis(),
            &pins(),
            ObserverScope::Member(observer),
            selection_limits(),
        )
        .unwrap();
        assert!(
            selected
                .facts()
                .iter()
                .any(|fact| fact.id == current.state().facts[1].id)
        );
    }
    let shared = perceive(
        &current,
        basis(),
        &pins(),
        ObserverScope::Shared,
        selection_limits(),
    )
    .unwrap();
    assert_eq!(shared.facts().len(), 1);
}

fn record(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}

fn journal(value: u8, sources: Vec<FactId>, audience: AudienceScope) -> JournalEntry {
    JournalEntry {
        id: record(value),
        source_facts: sources,
        attributed_claims: vec![],
        audience,
        text: "Retained source-bound account".to_owned(),
    }
}

fn accept_facts(state: &mut GameState) {
    state.decisions.push(AcceptedDecision {
        operation: state.facts[0].operation,
        revision: state.facts[0].revision,
        facts: state.facts.iter().map(|fact| fact.id).collect(),
        draws: vec![],
        effects: vec![],
        source_policy: label("accepted-journal-source"),
        semantic_output: None,
    });
}

fn journal_state() -> GameState {
    let mut state = audience_state();
    accept_facts(&mut state);
    for index in 0..state.facts.len() {
        state.continuity.journal.push(journal(
            40 + index as u8,
            vec![state.facts[index].id],
            state.facts[index].audience.clone(),
        ));
    }
    state
}

fn journals(
    current: &Checkpoint,
    observer: ObserverScope,
) -> Result<Vec<&JournalEntry>, PerceptionError> {
    df_knowledge::perception::perceive_journal(
        current,
        current.basis(),
        current.pins(),
        observer,
        1000,
        1000,
        16,
    )
}

#[test]
fn journal_committed_source_and_exact_member_selection_preserve_canonical_identity() {
    let current = checkpoint(journal_state()).unwrap();
    let original = current.clone();
    for (observer, indices) in [
        (ObserverScope::Shared, vec![0]),
        (ObserverScope::Member(member(3)), vec![0, 1]),
        (ObserverScope::Member(member(5)), vec![0, 2]),
    ] {
        let selected = journals(&current, observer).unwrap();
        assert_eq!(selected.len(), indices.len());
        for (entry, index) in selected.iter().zip(indices) {
            assert!(std::ptr::eq(
                *entry,
                &current.state().continuity.journal[index]
            ));
            assert_eq!(entry.source_facts, vec![current.state().facts[index].id]);
        }
        assert_eq!(current, original);
    }
}

#[test]
fn journal_missing_or_unrelated_decision_is_omitted_without_tightening_seed_perception() {
    for unrelated in [false, true] {
        let mut supplied = journal_state();
        supplied.decisions.clear();
        if unrelated {
            supplied.decisions.push(AcceptedDecision {
                operation: OperationId::from_bytes(&[99; 16]).unwrap(),
                revision: revision(2, 7),
                facts: vec![],
                draws: vec![],
                effects: vec![],
                source_policy: label("unrelated-source"),
                semantic_output: None,
            });
        }
        let current = checkpoint(supplied).unwrap();
        assert!(
            journals(&current, ObserverScope::Member(member(3)))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            perceive(
                &current,
                basis(),
                &pins(),
                ObserverScope::Member(member(3)),
                selection_limits()
            )
            .unwrap()
            .facts()
            .len(),
            2
        );
        assert_eq!(current.state().continuity.journal.len(), 4);
    }
}

#[test]
fn journal_visible_text_cannot_disclose_any_private_or_host_cited_source() {
    let mut supplied = journal_state();
    supplied.continuity.journal.clear();
    supplied.continuity.journal.push(journal(
        40,
        vec![supplied.facts[0].id, supplied.facts[1].id],
        AudienceScope::Shared,
    ));
    supplied.continuity.journal.push(journal(
        41,
        vec![supplied.facts[0].id, supplied.facts[3].id],
        AudienceScope::Shared,
    ));
    let current = checkpoint(supplied).unwrap();
    assert!(
        journals(&current, ObserverScope::Shared)
            .unwrap()
            .is_empty()
    );
    assert!(
        journals(&current, ObserverScope::Member(member(5)))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        journals(&current, ObserverScope::Member(member(3)))
            .unwrap()
            .len(),
        1
    );
}

fn journal_claim_state() -> GameState {
    let mut supplied = journal_state();
    supplied.continuity.journal.truncate(1);
    supplied.beliefs.push(AttributedClaim {
        id: record(60),
        holder: entity(4),
        subject: entity(4),
        claim: "The closed gate is open".to_owned(),
        evidence: vec![supplied.facts[0].id],
        audience: AudienceScope::Members(vec![member(3)]),
        source: content(),
    });
    supplied.continuity.journal[0].attributed_claims = vec![record(60)];
    supplied
}

#[test]
fn journal_false_claim_stays_attributed_and_preserves_mechanical_truth() {
    let current = checkpoint(journal_claim_state()).unwrap();
    let original = current.clone();
    let selected = journals(&current, ObserverScope::Member(member(3))).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].attributed_claims, vec![record(60)]);
    assert_eq!(selected[0].source_facts, vec![current.state().facts[0].id]);
    assert_eq!(current.state().beliefs[0].claim, "The closed gate is open");
    assert!(
        journals(&current, ObserverScope::Shared)
            .unwrap()
            .is_empty()
    );
    assert!(
        journals(&current, ObserverScope::Member(member(5)))
            .unwrap()
            .is_empty()
    );
    assert_eq!(current, original);
}

fn secret_policy(audience: AudienceScope) -> SecretPolicy {
    SecretPolicy {
        holder: entity(4),
        claims: vec![record(60)],
        policy: content(),
        permitted_audience: audience,
    }
}

fn npc(secrets: Vec<SecretPolicy>) -> NpcState {
    NpcState {
        entity: entity(4),
        role: content(),
        personality: content(),
        motivations: vec![],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: vec![],
        beliefs: vec![record(60)],
        secrets,
    }
}

#[test]
fn journal_secret_claim_policy_conflicting_ownership_and_hidden_evidence_are_enforced() {
    for case in 0..5 {
        let mut supplied = journal_claim_state();
        supplied.beliefs[0].audience = AudienceScope::Shared;
        let mut policies = vec![secret_policy(AudienceScope::Members(vec![member(3)]))];
        match case {
            0 => {}
            1 => policies[0].permitted_audience = AudienceScope::Host,
            2 => policies.push(secret_policy(AudienceScope::Shared)),
            3 => {
                let mut other = supplied.entities[0].clone();
                other.id = entity(6);
                supplied.entities.push(other);
                policies[0].holder = entity(6);
            }
            _ => supplied.beliefs[0].evidence = vec![supplied.facts[3].id],
        }
        supplied.continuity.npcs.push(npc(policies));
        // Grants do not authorize journal text or its private claim/evidence.
        supplied.knowledge.push(KnowledgeGrant {
            observer: member(5),
            fact: supplied.facts[0].id,
            source: supplied.facts[0].id,
        });
        let current = checkpoint(supplied).unwrap();
        assert!(
            journals(&current, ObserverScope::Shared)
                .unwrap()
                .is_empty()
        );
        assert!(
            journals(&current, ObserverScope::Member(member(5)))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            journals(&current, ObserverScope::Member(member(3)))
                .unwrap()
                .len(),
            usize::from(case == 0)
        );
    }
}

#[test]
fn journal_source_free_notes_are_preserved_but_never_selected_as_committed_outcomes() {
    let mut supplied = journal_claim_state();
    supplied.continuity.journal[0].source_facts.clear();
    let current = checkpoint(supplied).unwrap();
    let original = current.clone();
    assert!(
        journals(&current, ObserverScope::Member(member(3)))
            .unwrap()
            .is_empty()
    );
    assert!(!current.state().continuity.journal[0].text.is_empty());
    assert_eq!(current, original);
}

#[test]
fn journal_stale_basis_pins_and_removed_member_refuse() {
    use df_knowledge::perception::perceive_journal;
    let current = checkpoint(journal_state()).unwrap();
    let mut stale = basis();
    stale.revision = revision(2, 7);
    let mut wrong_pins = pins();
    wrong_pins.content.content_digest.0[0] ^= 1;
    for (expected, admitted, observer, error) in [
        (
            stale,
            pins(),
            ObserverScope::Shared,
            PerceptionError::Checkpoint(CheckpointError::StaleBasis),
        ),
        (
            basis(),
            wrong_pins,
            ObserverScope::Shared,
            PerceptionError::Checkpoint(CheckpointError::ContentMismatch),
        ),
        (
            basis(),
            pins(),
            ObserverScope::Member(member(99)),
            PerceptionError::ObserverUnavailable,
        ),
    ] {
        assert_eq!(
            perceive_journal(&current, expected, &admitted, observer, 1000, 1000, 16).unwrap_err(),
            error
        );
    }
    let mut supplied = current.state().clone();
    supplied.members.retain(|link| link.member != member(3));
    // Remove now-invalid private audience references before canonical restoration.
    for fact in &mut supplied.facts {
        if fact.audience == AudienceScope::Members(vec![member(3)]) {
            fact.audience = AudienceScope::Host;
        }
    }
    for entry in &mut supplied.continuity.journal {
        if entry.audience == AudienceScope::Members(vec![member(3)]) {
            entry.audience = AudienceScope::Host;
        }
    }
    let removed = checkpoint(supplied).unwrap();
    assert_eq!(
        journals(&removed, ObserverScope::Member(member(3))).unwrap_err(),
        PerceptionError::ObserverUnavailable
    );
}

#[test]
fn journal_nested_scan_comparison_and_result_capacity_return_no_partial_output() {
    use df_knowledge::perception::perceive_journal;
    let mut supplied = state();
    supplied.facts.push(fact(10, 0));
    accept_facts(&mut supplied);
    supplied.continuity.journal = vec![
        journal(40, vec![supplied.facts[0].id], AudienceScope::Shared),
        journal(41, vec![supplied.facts[0].id], AudienceScope::Shared),
    ];
    let current = checkpoint(supplied).unwrap();
    let original = current.clone();
    assert_eq!(
        perceive_journal(&current, basis(), &pins(), ObserverScope::Shared, 10, 10, 2)
            .unwrap()
            .len(),
        2
    );
    for (scans, comparisons, selected, error) in [
        (9, 10, 2, PerceptionError::ScanCapacity),
        (10, 9, 2, PerceptionError::ComparisonCapacity),
        (10, 10, 1, PerceptionError::ResultCapacity),
        (0, 10, 2, PerceptionError::ScanCapacity),
        (10, 0, 2, PerceptionError::ComparisonCapacity),
        (10, 10, 0, PerceptionError::ResultCapacity),
    ] {
        assert_eq!(
            perceive_journal(
                &current,
                basis(),
                &pins(),
                ObserverScope::Shared,
                scans,
                comparisons,
                selected
            )
            .unwrap_err(),
            error
        );
        assert_eq!(current, original);
    }
    let empty = checkpoint(state()).unwrap();
    assert!(
        perceive_journal(&empty, basis(), &pins(), ObserverScope::Shared, 0, 0, 0)
            .unwrap()
            .is_empty()
    );
}
