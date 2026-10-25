use df_combat::candidates::{
    AdmittedCandidates, CandidateContext, CandidateLimits, LegalOfferOwner, enumerate_candidates,
};
use df_combat::ranking::{
    KnowledgeObserver, RankingObservation, TacticalKnowledge, UtilityAssignment,
    UtilityContribution, UtilityEvidence, UtilityLimits, UtilityPolicy, UtilityRankingError,
    rank_tactics,
};
use df_model::checkpoint::{
    AttributedClaim, AudienceScope, Basis, CheckpointPins, ContentDigest, ContentPins,
    ContentReference, EntityId, FactId, KnowledgeGrant, LogicalTime, NpcState, RecordId, RulesMode,
    RulesPins,
};
use df_types::{
    BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Offer {
    offer_id: u8,
    is_legal: bool,
}

struct FixtureOwner;

impl LegalOfferOwner for FixtureOwner {
    type Observation = ();
    type Offer = Offer;
    type Error = ();

    fn validate_observation(&self, _: &(), _: CandidateContext<'_>) -> Result<(), ()> {
        Ok(())
    }

    fn is_current_legal(&self, _: &(), _: CandidateContext<'_>, offer: &Offer) -> Result<bool, ()> {
        Ok(offer.is_legal)
    }

    fn same_offer(&self, left: &Offer, right: &Offer) -> Result<bool, ()> {
        Ok(left.offer_id == right.offer_id)
    }

    fn offer_bytes(&self, _: &Offer) -> Result<usize, ()> {
        Ok(std::mem::size_of::<Offer>())
    }
}

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}

fn reference(entry: &str) -> ContentReference {
    ContentReference {
        package: label("fixture-package"),
        entry: label(entry),
    }
}

fn fact(value: u8) -> FactId {
    FactId::from_bytes(&[value; 16]).unwrap()
}

fn member(value: u8) -> MemberId {
    MemberId::from_bytes(&[value; 16]).unwrap()
}

fn entity(value: u8) -> EntityId {
    EntityId::from_bytes(&[value; 16]).unwrap()
}

fn record(value: u8) -> RecordId {
    RecordId::from_bytes(&[value; 16]).unwrap()
}

fn limits() -> UtilityLimits {
    UtilityLimits {
        candidates: 8,
        contributions: 16,
        knowledge_records: 8,
        criteria: 4,
    }
}

struct Fixture {
    basis: Basis,
    pins: CheckpointPins,
    definition: ContentReference,
    criteria: Vec<ContentReference>,
    grants: Vec<KnowledgeGrant>,
    beliefs: Vec<AttributedClaim>,
    npcs: Vec<NpcState>,
}

struct Context<'a> {
    observation: RankingObservation<'a>,
    knowledge: TacticalKnowledge<'a>,
    policy: UtilityPolicy<'a>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            basis: Basis {
                session: SessionId::from_bytes(&[1; 16]).unwrap(),
                run: RunId::from_bytes(&[2; 16]).unwrap(),
                revision: SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 4),
            },
            pins: CheckpointPins {
                rules: RulesPins {
                    mode: RulesMode::Standard2024,
                    ruleset: label("fixture-rules"),
                    catalog: label("fixture-catalog"),
                    catalog_digest: ContentDigest([1; 32]),
                    source_manifest: label("fixture-sources"),
                    source_manifest_digest: ContentDigest([2; 32]),
                    handler: label("fixture-handler"),
                    handler_digest: ContentDigest([3; 32]),
                },
                content: ContentPins {
                    content: label("fixture-content"),
                    content_digest: ContentDigest([4; 32]),
                    package: label("fixture-package"),
                    package_digest: ContentDigest([5; 32]),
                },
                build: BuildIdentity::new(
                    Some("source"),
                    Some("native"),
                    Some("wasm"),
                    Some("configuration"),
                    Some("content"),
                )
                .unwrap(),
            },
            definition: reference("utility-policy"),
            criteria: vec![reference("criterion-a"), reference("criterion-b")],
            grants: vec![KnowledgeGrant {
                observer: member(3),
                fact: fact(4),
                source: fact(5),
            }],
            npcs: vec![NpcState {
                entity: entity(7),
                personality: reference("npc-personality"),
                role: reference("npc-personality"),
                motivations: vec![],
                goals: vec![],
                needs: vec![],
                fears: vec![],
                known_facts: vec![],
                beliefs: vec![record(6)],
                secrets: vec![],
            }],
            beliefs: vec![AttributedClaim {
                id: record(6),
                holder: entity(7),
                subject: entity(8),
                claim: "attributed-false-claim".to_owned(),
                evidence: vec![fact(9)],
                audience: AudienceScope::Host,
                source: reference("belief-source"),
            }],
        }
    }

    fn context(&self, observer: KnowledgeObserver) -> Context<'_> {
        Context {
            observation: RankingObservation {
                basis: &self.basis,
                pins: &self.pins,
                observer,
                logical_time: LogicalTime {
                    ticks: 10,
                    ticks_per_second: 1,
                },
                cause: OperationId::from_bytes(&[10; 16]).unwrap(),
                policy: &self.definition,
            },
            knowledge: TacticalKnowledge {
                basis: &self.basis,
                pins: &self.pins,
                observer,
                grants: &self.grants,
                beliefs: &self.beliefs,
                npcs: &self.npcs,
            },
            policy: UtilityPolicy {
                basis: &self.basis,
                pins: &self.pins,
                definition: &self.definition,
                criteria: &self.criteria,
            },
        }
    }

    fn admitted<'a>(&'a self, offers: &'a [Offer]) -> AdmittedCandidates<'a, Offer> {
        let context = CandidateContext {
            basis: &self.basis,
            pins: &self.pins,
        };
        enumerate_candidates(
            &FixtureOwner,
            &(),
            context,
            context,
            offers,
            CandidateLimits {
                max_offers: 8,
                max_identity_comparisons: 28,
                max_candidates: 8,
                max_candidate_bytes: 1024,
            },
        )
        .unwrap()
    }
}

fn contribution(criterion: &str, evidence: UtilityEvidence, value: i64) -> UtilityContribution {
    UtilityContribution {
        criterion: reference(criterion),
        evidence,
        value,
    }
}

#[test]
fn legal_receipt_ranks_exact_sums_and_explanations_with_stable_ties() {
    let fixture = Fixture::new();
    let context = fixture.context(KnowledgeObserver::Member(member(3)));
    let offers = [
        Offer {
            offer_id: 1,
            is_legal: true,
        },
        Offer {
            offer_id: 2,
            is_legal: true,
        },
        Offer {
            offer_id: 3,
            is_legal: false,
        },
        Offer {
            offer_id: 4,
            is_legal: true,
        },
    ];
    let admitted = fixture.admitted(&offers);
    let first = [
        contribution("criterion-a", UtilityEvidence::Fact(fact(4)), 8),
        contribution("criterion-b", UtilityEvidence::Fact(fact(4)), -3),
    ];
    let second = [contribution(
        "criterion-a",
        UtilityEvidence::Fact(fact(4)),
        5,
    )];
    let fourth = [contribution(
        "criterion-a",
        UtilityEvidence::Fact(fact(4)),
        -1,
    )];
    let assignments = [
        UtilityAssignment {
            offer: &offers[0],
            contributions: &first,
        },
        UtilityAssignment {
            offer: &offers[1],
            contributions: &second,
        },
        UtilityAssignment {
            offer: &offers[3],
            contributions: &fourth,
        },
    ];
    let ranked = rank_tactics(
        &context.observation,
        &admitted,
        &assignments,
        &context.knowledge,
        &context.policy,
        limits(),
    )
    .unwrap();
    assert_eq!(
        ranked
            .tactics()
            .iter()
            .map(|t| t.offer().offer_id)
            .collect::<Vec<_>>(),
        [1, 2, 4]
    );
    assert_eq!(
        ranked
            .tactics()
            .iter()
            .map(|t| t.utility())
            .collect::<Vec<_>>(),
        [5, 5, -1]
    );
    assert!(std::ptr::eq(
        ranked.preferred().unwrap().offer(),
        &offers[0]
    ));
    assert!(std::ptr::eq(
        ranked.tactics()[0].contributions(),
        first.as_slice()
    ));
    assert!(std::ptr::eq(ranked.observation(), &context.observation));
}

#[test]
fn an_npc_uses_its_attributed_belief_without_reading_or_changing_world_truth() {
    let fixture = Fixture::new();
    let original = fixture.beliefs.clone();
    let context = fixture.context(KnowledgeObserver::Entity(entity(7)));
    let offers = [
        Offer {
            offer_id: 1,
            is_legal: true,
        },
        Offer {
            offer_id: 2,
            is_legal: true,
        },
    ];
    let admitted = fixture.admitted(&offers);
    let belief = [contribution(
        "criterion-a",
        UtilityEvidence::Belief(record(6)),
        12,
    )];
    let none = [];
    let assignments = [
        UtilityAssignment {
            offer: &offers[0],
            contributions: &none,
        },
        UtilityAssignment {
            offer: &offers[1],
            contributions: &belief,
        },
    ];
    let ranked = rank_tactics(
        &context.observation,
        &admitted,
        &assignments,
        &context.knowledge,
        &context.policy,
        limits(),
    )
    .unwrap();
    assert_eq!(ranked.preferred().unwrap().offer().offer_id, 2);
    assert_eq!(ranked.preferred().unwrap().contributions(), belief);
    assert_eq!(fixture.beliefs, original);
}

#[test]
fn secret_values_and_criteria_cannot_affect_the_safe_permission_rejection() {
    let fixture = Fixture::new();
    let context = fixture.context(KnowledgeObserver::Member(member(3)));
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    for (criterion, value) in [("criterion-a", i64::MAX), ("unknown-secret-term", i64::MIN)] {
        let secret = [contribution(
            criterion,
            UtilityEvidence::Fact(fact(99)),
            value,
        )];
        let assignments = [UtilityAssignment {
            offer: &offers[0],
            contributions: &secret,
        }];
        assert!(matches!(
            rank_tactics(
                &context.observation,
                &admitted,
                &assignments,
                &context.knowledge,
                &context.policy,
                limits(),
            ),
            Err(UtilityRankingError::UndisclosedEvidence)
        ));
    }
}

#[test]
fn another_holder_and_unmapped_member_facts_are_unavailable_to_an_npc() {
    let fixture = Fixture::new();
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    for (observer, evidence) in [
        (
            KnowledgeObserver::Entity(entity(8)),
            UtilityEvidence::Belief(record(6)),
        ),
        (
            KnowledgeObserver::Entity(entity(7)),
            UtilityEvidence::Fact(fact(4)),
        ),
        (
            KnowledgeObserver::Member(member(2)),
            UtilityEvidence::Fact(fact(4)),
        ),
        (
            KnowledgeObserver::Member(member(3)),
            UtilityEvidence::Belief(record(6)),
        ),
    ] {
        let context = fixture.context(observer);
        let contributions = [contribution("criterion-a", evidence, 1)];
        let assignments = [UtilityAssignment {
            offer: &offers[0],
            contributions: &contributions,
        }];
        assert!(matches!(
            rank_tactics(
                &context.observation,
                &admitted,
                &assignments,
                &context.knowledge,
                &context.policy,
                limits(),
            ),
            Err(UtilityRankingError::UndisclosedEvidence)
        ));
    }
}

#[test]
fn duplicate_terms_unknown_policy_and_overflow_reject_the_entire_batch() {
    let fixture = Fixture::new();
    let context = fixture.context(KnowledgeObserver::Member(member(3)));
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    let cases = [
        (
            vec![
                contribution("criterion-a", UtilityEvidence::Fact(fact(4)), 1),
                contribution("criterion-a", UtilityEvidence::Fact(fact(4)), 2),
            ],
            UtilityRankingError::DuplicateContribution,
        ),
        (
            vec![contribution("unknown", UtilityEvidence::Fact(fact(4)), 1)],
            UtilityRankingError::UnsupportedCriterion,
        ),
        (
            vec![
                contribution("criterion-a", UtilityEvidence::Fact(fact(4)), i64::MAX),
                contribution("criterion-b", UtilityEvidence::Fact(fact(4)), 1),
            ],
            UtilityRankingError::UtilityOverflow,
        ),
        (
            vec![
                contribution("criterion-a", UtilityEvidence::Fact(fact(4)), i64::MIN),
                contribution("criterion-b", UtilityEvidence::Fact(fact(4)), -1),
            ],
            UtilityRankingError::UtilityOverflow,
        ),
    ];
    for (contributions, expected) in cases {
        let assignments = [UtilityAssignment {
            offer: &offers[0],
            contributions: &contributions,
        }];
        assert!(matches!(rank_tactics(
            &context.observation, &admitted, &assignments, &context.knowledge, &context.policy, limits(),
        ), Err(actual) if actual == expected));
    }
}

#[test]
fn a_score_cannot_be_reattached_to_an_equal_but_unadmitted_payload() {
    let fixture = Fixture::new();
    let context = fixture.context(KnowledgeObserver::Member(member(3)));
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    let clone = offers[0].clone();
    let assignments = [UtilityAssignment {
        offer: &clone,
        contributions: &[],
    }];
    assert!(matches!(
        rank_tactics(
            &context.observation,
            &admitted,
            &assignments,
            &context.knowledge,
            &context.policy,
            limits(),
        ),
        Err(UtilityRankingError::AssignmentMismatch)
    ));
    assert!(matches!(
        rank_tactics(
            &context.observation,
            &admitted,
            &[],
            &context.knowledge,
            &context.policy,
            limits(),
        ),
        Err(UtilityRankingError::AssignmentMismatch)
    ));
}

#[test]
fn every_input_snapshot_must_match_the_admitted_basis_and_pins() {
    let fixture = Fixture::new();
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &[],
    }];
    let stale_basis = Basis {
        revision: fixture.basis.revision.next_sequence().unwrap(),
        ..fixture.basis
    };
    let mut stale_pins = fixture.pins.clone();
    stale_pins.content.content_digest = ContentDigest([99; 32]);
    for component in 0..6 {
        let mut context = fixture.context(KnowledgeObserver::Member(member(3)));
        match component {
            0 => context.observation.basis = &stale_basis,
            1 => context.observation.pins = &stale_pins,
            2 => context.knowledge.basis = &stale_basis,
            3 => context.knowledge.pins = &stale_pins,
            4 => context.policy.basis = &stale_basis,
            _ => context.policy.pins = &stale_pins,
        }
        assert!(matches!(
            rank_tactics(
                &context.observation,
                &admitted,
                &assignments,
                &context.knowledge,
                &context.policy,
                limits(),
            ),
            Err(UtilityRankingError::StaleBasis)
        ));
    }
}

#[test]
fn current_observer_and_policy_must_match_the_requested_observation() {
    let fixture = Fixture::new();
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &[],
    }];
    let mut context = fixture.context(KnowledgeObserver::Member(member(3)));
    context.knowledge.observer = KnowledgeObserver::Member(member(4));
    assert!(matches!(
        rank_tactics(
            &context.observation,
            &admitted,
            &assignments,
            &context.knowledge,
            &context.policy,
            limits(),
        ),
        Err(UtilityRankingError::ObserverMismatch)
    ));
    let other_policy = reference("other-policy");
    let mut context = fixture.context(KnowledgeObserver::Member(member(3)));
    context.policy.definition = &other_policy;
    assert!(matches!(
        rank_tactics(
            &context.observation,
            &admitted,
            &assignments,
            &context.knowledge,
            &context.policy,
            limits(),
        ),
        Err(UtilityRankingError::PolicyMismatch)
    ));
}

#[test]
fn explicit_admission_bounds_are_exact_and_have_no_partial_result() {
    let fixture = Fixture::new();
    let context = fixture.context(KnowledgeObserver::Member(member(3)));
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    let contributions = [contribution(
        "criterion-a",
        UtilityEvidence::Fact(fact(4)),
        1,
    )];
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &contributions,
    }];
    for (field, expected) in [
        (0, UtilityRankingError::CandidateCapacity),
        (1, UtilityRankingError::ContributionCapacity),
        (2, UtilityRankingError::KnowledgeCapacity),
        (3, UtilityRankingError::PolicyCapacity),
    ] {
        let mut bounded = limits();
        match field {
            0 => bounded.candidates = 0,
            1 => bounded.contributions = 0,
            2 => bounded.knowledge_records = 1,
            _ => bounded.criteria = 1,
        }
        assert!(matches!(rank_tactics(
            &context.observation, &admitted, &assignments, &context.knowledge, &context.policy, bounded,
        ), Err(actual) if actual == expected));
    }
    let exact = UtilityLimits {
        candidates: 1,
        contributions: 1,
        knowledge_records: 4,
        criteria: 2,
    };
    assert_eq!(
        rank_tactics(
            &context.observation,
            &admitted,
            &assignments,
            &context.knowledge,
            &context.policy,
            exact,
        )
        .unwrap()
        .tactics()
        .len(),
        1
    );
}

#[test]
fn an_empty_legal_inventory_has_an_explicit_no_action_result() {
    let fixture = Fixture::new();
    let context = fixture.context(KnowledgeObserver::Entity(entity(7)));
    let offers = [];
    let admitted = fixture.admitted(&offers);
    let ranked = rank_tactics(
        &context.observation,
        &admitted,
        &[],
        &context.knowledge,
        &context.policy,
        limits(),
    )
    .unwrap();
    assert!(ranked.tactics().is_empty());
    assert!(ranked.preferred().is_none());
}

#[test]
fn irrelevant_hidden_claim_changes_leave_a_successful_ranking_identical() {
    let fixture = Fixture::new();
    let offers = [Offer {
        offer_id: 1,
        is_legal: true,
    }];
    let admitted = fixture.admitted(&offers);
    let contributions = [contribution(
        "criterion-a",
        UtilityEvidence::Fact(fact(4)),
        7,
    )];
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &contributions,
    }];
    let first_context = fixture.context(KnowledgeObserver::Member(member(3)));
    let first = rank_tactics(
        &first_context.observation,
        &admitted,
        &assignments,
        &first_context.knowledge,
        &first_context.policy,
        limits(),
    )
    .unwrap();
    let mut hidden = fixture.beliefs.clone();
    hidden[0].claim = "different-hidden-claim".to_owned();
    hidden[0].evidence = vec![fact(98)];
    let mut second_context = fixture.context(KnowledgeObserver::Member(member(3)));
    second_context.knowledge.beliefs = &hidden;
    let second = rank_tactics(
        &second_context.observation,
        &admitted,
        &assignments,
        &second_context.knowledge,
        &second_context.policy,
        limits(),
    )
    .unwrap();
    assert_eq!(
        first.preferred().unwrap().utility(),
        second.preferred().unwrap().utility()
    );
    assert!(std::ptr::eq(
        first.preferred().unwrap().offer(),
        second.preferred().unwrap().offer()
    ));
    assert_eq!(
        first.preferred().unwrap().contributions(),
        second.preferred().unwrap().contributions()
    );
}
