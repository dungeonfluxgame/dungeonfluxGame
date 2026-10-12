use std::cell::Cell;

use df_encounter::challenge::{
    AuthoredChallengeReference, ChallengeBudget, ChallengeMode, ChallengeReferenceKind,
    EncounterError, EncounterRequest, EncounterSourceOwner, SourceAdmission,
};
use df_encounter::escalation::{
    EncounterPurpose, EscalationAdmission, EscalationCandidate, EscalationError, EscalationLimits,
    EscalationSourceOwner, SelectedEncounter, WorldPrerequisite, select_encounter,
};
use df_encounter::participants::{ParticipantOwner, ParticipantStatus};

include!("objectives/fixture.rs");

fn definitions() -> Vec<ContentReference> {
    vec![content()]
}

fn selection_limits() -> EscalationLimits {
    EscalationLimits {
        maximum_candidates: 4,
        maximum_records: 100,
        maximum_comparisons: 1000,
        maximum_input_bytes: 1_000_000,
        maximum_output_bytes: 1_000_000,
    }
}

struct Authority {
    admission: Cell<EscalationAdmission>,
    source: Cell<SourceAdmission>,
    participant: Cell<ParticipantStatus>,
    calls: Cell<usize>,
    skip_first: bool,
    completion: Option<FactId>,
}

impl Default for Authority {
    fn default() -> Self {
        Self {
            admission: Cell::new(EscalationAdmission::Eligible),
            source: Cell::new(SourceAdmission::Admitted),
            participant: Cell::new(ParticipantStatus::Available),
            calls: Cell::new(0),
            skip_first: false,
            completion: None,
        }
    }
}

impl ParticipantOwner for Authority {
    type Actor = EntityId;
    type Basis = Basis;
    type Error = ();

    fn validate_basis(&self, proposed: &Basis, current: &Basis) -> Result<(), ()> {
        if proposed != current {
            return Err(());
        }
        Ok(())
    }

    fn participant_status(
        &self,
        _current: &Basis,
        _actor: &EntityId,
    ) -> Result<ParticipantStatus, ()> {
        Ok(self.participant.get())
    }
}

impl EncounterSourceOwner for Authority {
    fn admit_reference(
        &self,
        current: &Checkpoint,
        _mode: ChallengeMode,
        _purpose: ChallengeReferenceKind,
        authored: &AuthoredChallengeReference,
    ) -> Result<SourceAdmission, ()> {
        assert_eq!(current.pins().content.package, authored.definition.package);
        assert_eq!(authored.source, rule());
        Ok(self.source.get())
    }
}

impl EscalationSourceOwner for Authority {
    fn admit_candidate(
        &self,
        current: &Checkpoint,
        candidate: &EscalationCandidate<'_>,
        order: usize,
    ) -> Result<EscalationAdmission, ()> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(candidate.policy, &content());
        assert_eq!(current.pins(), &pins());
        // This fixture source policy explicitly defines this canonical content event
        // as its completed one-shot. Presence alone is not a universal game rule.
        if let Some(id) = self.completion
            && current.state().facts.iter().any(|fact| {
                fact.id == id
                    && matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition == &content())
                    && current.state().decisions.iter().any(|decision| {
                        decision.operation == fact.operation
                            && decision.revision == fact.revision
                            && decision.facts.contains(&id)
                    })
            })
        {
            return Ok(EscalationAdmission::CompletedOneShot);
        }
        if self.skip_first && order == 0 {
            return Ok(EscalationAdmission::NotEligible);
        }
        Ok(self.admission.get())
    }
}

struct Fixture {
    current: Checkpoint,
    authored: AuthoredChallengeReference,
    rules: Vec<RuleReference>,
    contents: Vec<ContentReference>,
    actors: [EntityId; 1],
    identity: RevisionLabel,
}

impl Fixture {
    fn new(committed: bool, audience: AudienceScope) -> Self {
        let mut game = state();
        game.encounters.push(EncounterState {
            id: RecordId::from_bytes(&[9; 16]).unwrap(),
            definition: content(),
            participants: vec![entity(4)],
            turn_order: vec![entity(4)],
            active_turn: Some(entity(4)),
            objectives: vec![content()],
            combat_policy: content(),
            tactical: None,
        });
        let mut event = fact(7, 0);
        event.audience = audience;
        game.facts.push(event.clone());
        if committed {
            game.decisions.push(AcceptedDecision {
                operation: event.operation,
                revision: event.revision,
                facts: vec![event.id],
                draws: vec![],
                effects: vec![],
                source_policy: label("fixture-source-policy"),
                semantic_output: None,
            });
        }
        Self {
            current: checkpoint(game).unwrap(),
            authored: AuthoredChallengeReference {
                definition: content(),
                source: rule(),
            },
            rules: vec![rule()],
            contents: definitions(),
            actors: [entity(4)],
            identity: label("fixture-entity-1"),
        }
    }

    fn world(&self) -> [WorldPrerequisite<'_>; 1] {
        [WorldPrerequisite {
            entity: entity(4),
            identity_revision: &self.identity,
        }]
    }

    fn inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: &self.rules,
            content: &self.contents,
            resources: &[],
            assets: &[],
        }
    }

    fn candidate<'a>(
        &'a self,
        world: &'a [WorldPrerequisite<'a>],
        facts: &'a [FactId],
        purpose: EncounterPurpose<'a>,
    ) -> EscalationCandidate<'a> {
        EscalationCandidate {
            policy: &self.authored.definition,
            purpose,
            challenge: EncounterRequest {
                current: &self.current,
                expected_basis: self.current.basis(),
                admitted_pins: self.current.pins(),
                inventory: self.inventory(),
                mode: ChallengeMode::Social,
                definition: &self.authored,
                participants: &self.actors,
                objectives: std::slice::from_ref(&self.authored),
                escape_condition: None,
                consequences: &[],
                budget: ChallengeBudget {
                    max_participants: 1,
                    max_objectives: 1,
                    max_escape_references: 0,
                    max_consequence_references: 0,
                    max_comparisons: 100,
                    max_input_bytes: 1_000_000,
                    max_output_bytes: 1_000_000,
                },
            },
            world,
            committed_facts: facts,
        }
    }
}

#[test]
fn current_source_and_world_prerequisites_select_exact_escalation_proposal() {
    let f = Fixture::new(true, AudienceScope::Shared);
    let before = f.current.clone();
    let world = f.world();
    let facts = [fact(7, 0).id];
    let candidates = [
        f.candidate(&world, &facts, EncounterPurpose::New),
        f.candidate(
            &world,
            &facts,
            EncounterPurpose::Escalate(&f.current.state().encounters[0]),
        ),
    ];
    let owner = Authority {
        skip_first: true,
        ..Authority::default()
    };
    let selected = select_encounter(
        &owner,
        &f.current,
        basis(),
        f.current.pins(),
        &candidates,
        selection_limits(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.order(), 1);
    assert_eq!(selected.plan().definition, f.authored);
    assert_eq!(selected.plan().participants, f.actors);
    assert_eq!(selected.plan().objectives[0].authored, f.authored);
    selected
        .validate(&owner, &f.current, f.inventory(), selection_limits())
        .unwrap();
    assert_eq!(f.current, before);
}

#[test]
fn missing_foreign_uncommitted_or_hidden_prerequisite_refuses() {
    for (committed, audience, ids, expected) in [
        (
            true,
            AudienceScope::Shared,
            vec![FactId::from_bytes(&[99; 16]).unwrap()],
            EscalationError::MissingFactPrerequisite,
        ),
        (
            false,
            AudienceScope::Shared,
            vec![fact(7, 0).id],
            EscalationError::UncommittedFactPrerequisite,
        ),
        (
            true,
            AudienceScope::Members(vec![member(3)]),
            vec![fact(7, 0).id],
            EscalationError::HiddenPrerequisite,
        ),
    ] {
        let f = Fixture::new(committed, audience);
        let world = f.world();
        let candidates = [f.candidate(&world, &ids, EncounterPurpose::New)];
        let owner = Authority::default();
        let result = select_encounter(
            &owner,
            &f.current,
            basis(),
            f.current.pins(),
            &candidates,
            selection_limits(),
        );
        match result {
            Err(error) => assert_eq!(error, expected),
            Ok(_) => panic!("invalid prerequisite accepted"),
        }
        assert_eq!(owner.calls.get(), 0);
    }
    let f = Fixture::new(true, AudienceScope::Shared);
    let unknown = [WorldPrerequisite {
        entity: entity(88),
        identity_revision: &f.identity,
    }];
    let candidates = [f.candidate(&unknown, &[], EncounterPurpose::New)];
    assert!(matches!(
        select_encounter(
            &Authority::default(),
            &f.current,
            basis(),
            f.current.pins(),
            &candidates,
            selection_limits()
        ),
        Err(EscalationError::MissingWorldPrerequisite)
    ));
}

#[test]
fn reused_composition_revalidates_current_source_world_and_participants() {
    let f = Fixture::new(true, AudienceScope::Shared);
    let world = f.world();
    let ids = [fact(7, 0).id];
    let candidates = [f.candidate(
        &world,
        &ids,
        EncounterPurpose::Reuse(&f.current.state().encounters[0]),
    )];
    let owner = Authority::default();
    let selected = select_encounter(
        &owner,
        &f.current,
        basis(),
        f.current.pins(),
        &candidates,
        selection_limits(),
    )
    .unwrap()
    .unwrap();
    selected
        .validate(&owner, &f.current, f.inventory(), selection_limits())
        .unwrap();
    owner.source.set(SourceAdmission::Denied);
    assert!(matches!(
        selected.validate(&owner, &f.current, f.inventory(), selection_limits()),
        Err(EscalationError::Challenge(EncounterError::SourceGap(_)))
    ));
    owner.source.set(SourceAdmission::Admitted);
    owner.participant.set(ParticipantStatus::Unavailable);
    assert!(matches!(
        selected.validate(&owner, &f.current, f.inventory(), selection_limits()),
        Err(EscalationError::Challenge(EncounterError::Participant(_)))
    ));
    owner.participant.set(ParticipantStatus::Available);
    let mut changed = f.current.state().clone();
    changed.entities[0].identity_revision = label("changed-identity");
    let changed = checkpoint(changed).unwrap();
    assert!(matches!(
        selected.validate(&owner, &changed, f.inventory(), selection_limits()),
        Err(EscalationError::StaleWorldPrerequisite)
    ));
    owner.admission.set(EscalationAdmission::Denied);
    assert!(matches!(
        selected.validate(&owner, &f.current, f.inventory(), selection_limits()),
        Err(EscalationError::PolicyGap(EscalationAdmission::Denied))
    ));
}

#[test]
fn completed_or_stale_reuse_cannot_reactivate_encounter() {
    let f = Fixture::new(true, AudienceScope::Shared);
    let before = f.current.clone();
    let world = f.world();
    let ids = [fact(7, 0).id];
    let candidates = [f.candidate(
        &world,
        &ids,
        EncounterPurpose::Reuse(&f.current.state().encounters[0]),
    )];
    let owner = Authority {
        completion: Some(ids[0]),
        ..Authority::default()
    };
    assert!(matches!(
        select_encounter(
            &owner,
            &f.current,
            basis(),
            f.current.pins(),
            &candidates,
            selection_limits()
        ),
        Err(EscalationError::PolicyGap(
            EscalationAdmission::CompletedOneShot
        ))
    ));
    let mut stale = f.current.state().encounters[0].clone();
    stale.participants.clear();
    let candidates = [f.candidate(&world, &ids, EncounterPurpose::Reuse(&stale))];
    assert!(matches!(
        select_encounter(
            &Authority::default(),
            &f.current,
            basis(),
            f.current.pins(),
            &candidates,
            selection_limits()
        ),
        Err(EscalationError::StaleEncounter)
    ));
    let mut stale_basis = basis();
    stale_basis.revision = revision(2, 9);
    assert!(matches!(
        select_encounter(
            &Authority::default(),
            &f.current,
            stale_basis,
            f.current.pins(),
            &candidates,
            selection_limits()
        ),
        Err(EscalationError::Snapshot(CheckpointError::StaleBasis))
    ));
    assert_eq!(f.current, before);
}

#[test]
fn missing_source_invalid_bounds_duplicates_and_overflow_refuse() {
    let f = Fixture::new(true, AudienceScope::Shared);
    let world = f.world();
    let ids = [fact(7, 0).id, fact(7, 0).id];
    let candidates = [f.candidate(&world, &ids, EncounterPurpose::New)];
    assert!(matches!(
        select_encounter(
            &Authority::default(),
            &f.current,
            basis(),
            f.current.pins(),
            &candidates,
            selection_limits()
        ),
        Err(EscalationError::DuplicatePrerequisite)
    ));
    let candidates = [f.candidate(&world, &[], EncounterPurpose::New)];
    for admission in [
        EscalationAdmission::Unavailable,
        EscalationAdmission::Unsupported,
        EscalationAdmission::Denied,
    ] {
        let owner = Authority {
            admission: Cell::new(admission),
            ..Authority::default()
        };
        assert!(
            matches!(select_encounter(&owner, &f.current, basis(), f.current.pins(), &candidates, selection_limits()), Err(EscalationError::PolicyGap(value)) if value == admission)
        );
    }
    for limits in [
        EscalationLimits {
            maximum_candidates: 0,
            ..selection_limits()
        },
        EscalationLimits {
            maximum_candidates: usize::MAX,
            ..selection_limits()
        },
        EscalationLimits {
            maximum_records: usize::MAX,
            ..selection_limits()
        },
        EscalationLimits {
            maximum_records: 0,
            ..selection_limits()
        },
        EscalationLimits {
            maximum_input_bytes: 0,
            ..selection_limits()
        },
        EscalationLimits {
            maximum_output_bytes: 0,
            ..selection_limits()
        },
        EscalationLimits {
            maximum_output_bytes: std::mem::size_of::<SelectedEncounter<'_>>() + 1,
            ..selection_limits()
        },
        EscalationLimits {
            maximum_comparisons: 0,
            ..selection_limits()
        },
    ] {
        let owner = Authority::default();
        assert!(matches!(
            select_encounter(
                &owner,
                &f.current,
                basis(),
                f.current.pins(),
                &candidates,
                limits
            ),
            Err(EscalationError::Capacity)
        ));
        assert_eq!(owner.calls.get(), 0);
    }
    let mut candidates = [f.candidate(&world, &[], EncounterPurpose::New)];
    candidates[0].challenge.budget.max_comparisons = 0;
    let owner = Authority::default();
    assert!(matches!(
        select_encounter(
            &owner,
            &f.current,
            basis(),
            f.current.pins(),
            &candidates,
            selection_limits()
        ),
        Err(EscalationError::Capacity)
    ));
    assert_eq!(owner.calls.get(), 0);
}

#[test]
fn proposal_is_deterministic_and_preserves_checkpoint_rules_resources_and_rewards() {
    let f = Fixture::new(true, AudienceScope::Shared);
    let before = f.current.clone();
    let world = f.world();
    let ids = [fact(7, 0).id];
    let candidates = [f.candidate(&world, &ids, EncounterPurpose::New)];
    let first = select_encounter(
        &Authority::default(),
        &f.current,
        basis(),
        f.current.pins(),
        &candidates,
        selection_limits(),
    )
    .unwrap()
    .unwrap();
    let retry = select_encounter(
        &Authority::default(),
        &f.current,
        basis(),
        f.current.pins(),
        &candidates,
        selection_limits(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(first.plan(), retry.plan());
    assert_eq!(f.current, before);
    assert_eq!(f.current.state().resources[0].value, 4);
    assert_eq!(f.current.state().draws.len(), 0);
    assert_eq!(f.current.state().inventory.len(), 0);
    assert_eq!(f.current.state().logical_time.ticks, 120);
}

#[test]
fn empty_and_all_authored_declines_return_no_intervention() {
    let f = Fixture::new(true, AudienceScope::Shared);
    assert!(
        select_encounter(
            &Authority::default(),
            &f.current,
            basis(),
            f.current.pins(),
            &[],
            selection_limits()
        )
        .unwrap()
        .is_none()
    );
    let world = f.world();
    let candidates = [f.candidate(&world, &[], EncounterPurpose::New)];
    let owner = Authority {
        admission: Cell::new(EscalationAdmission::NotEligible),
        ..Authority::default()
    };
    assert!(
        select_encounter(
            &owner,
            &f.current,
            basis(),
            f.current.pins(),
            &candidates,
            selection_limits()
        )
        .unwrap()
        .is_none()
    );
}
