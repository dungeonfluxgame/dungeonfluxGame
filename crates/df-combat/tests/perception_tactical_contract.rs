use df_combat::candidates::{
    CandidateContext, CandidateLimits, LegalOfferOwner, enumerate_candidates,
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

const DECISION_FIXTURE: &str = include_str!("fixtures/perception_tactical_contract.json");

#[derive(Clone, Debug, Eq, PartialEq)]
struct Offer {
    id: u8,
    legal: bool,
}

struct OfferOwner;

impl LegalOfferOwner for OfferOwner {
    type Observation = ();
    type Offer = Offer;
    type Error = ();

    fn validate_observation(&self, _: &(), _: CandidateContext<'_>) -> Result<(), ()> {
        Ok(())
    }

    fn is_current_legal(&self, _: &(), _: CandidateContext<'_>, offer: &Offer) -> Result<bool, ()> {
        Ok(offer.legal)
    }

    fn same_offer(&self, left: &Offer, right: &Offer) -> Result<bool, ()> {
        Ok(left.id == right.id)
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

fn ranking_limits() -> UtilityLimits {
    UtilityLimits {
        candidates: 4,
        contributions: 8,
        knowledge_records: 16,
        criteria: 2,
    }
}

struct Fixture {
    basis: Basis,
    pins: CheckpointPins,
    policy: ContentReference,
    criteria: [ContentReference; 1],
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
            policy: reference("utility-policy"),
            criteria: [reference("permitted-observation")],
        }
    }

    fn observation(&self, observer: KnowledgeObserver) -> RankingObservation<'_> {
        RankingObservation {
            basis: &self.basis,
            pins: &self.pins,
            observer,
            logical_time: LogicalTime {
                ticks: 10,
                ticks_per_second: 1,
            },
            cause: OperationId::from_bytes(&[10; 16]).unwrap(),
            policy: &self.policy,
        }
    }

    fn policy(&self) -> UtilityPolicy<'_> {
        UtilityPolicy {
            basis: &self.basis,
            pins: &self.pins,
            definition: &self.policy,
            criteria: &self.criteria,
        }
    }

    fn admitted<'a>(
        &'a self,
        offers: &'a [Offer],
    ) -> df_combat::candidates::AdmittedCandidates<'a, Offer> {
        let current = CandidateContext {
            basis: &self.basis,
            pins: &self.pins,
        };
        enumerate_candidates(
            &OfferOwner,
            &(),
            current,
            current,
            offers,
            CandidateLimits {
                max_offers: 4,
                max_identity_comparisons: 6,
                max_candidates: 4,
                max_candidate_bytes: 256,
            },
        )
        .unwrap()
    }
}

fn contribution(evidence: UtilityEvidence, value: i64) -> UtilityContribution {
    UtilityContribution {
        criterion: reference("permitted-observation"),
        evidence,
        value,
    }
}

#[test]
fn paired_hidden_enemy_states_cannot_change_tactics_using_the_same_actor_evidence() {
    let fixture = Fixture::new();
    let observer = KnowledgeObserver::Entity(entity(7));
    let observation = fixture.observation(observer);
    let policy = fixture.policy();
    let offers = [Offer { id: 1, legal: true }, Offer { id: 2, legal: true }];
    let admitted = fixture.admitted(&offers);
    let preferred = [contribution(UtilityEvidence::Belief(record(6)), 7)];
    let empty = [];
    let assignments = [
        UtilityAssignment {
            offer: &offers[0],
            contributions: &empty,
        },
        UtilityAssignment {
            offer: &offers[1],
            contributions: &preferred,
        },
    ];
    let acting_npc = NpcState {
        entity: entity(7),
        personality: reference("acting-npc"),
        role: reference("acting-npc"),
        motivations: vec![],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: vec![fact(4)],
        beliefs: vec![record(6)],
        secrets: vec![],
    };
    let attributed_belief = AttributedClaim {
        id: record(6),
        holder: entity(7),
        subject: entity(8),
        claim: "the bridge is safe".to_owned(),
        evidence: vec![fact(4)],
        audience: AudienceScope::Host,
        source: reference("witnessed-report"),
    };
    let hidden_enemy_a = NpcState {
        entity: entity(8),
        personality: reference("enemy"),
        role: reference("enemy"),
        motivations: vec![],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: vec![fact(99)],
        beliefs: vec![record(9)],
        secrets: vec![],
    };
    let hidden_enemy_b = NpcState {
        known_facts: vec![fact(98), fact(99), fact(100)],
        beliefs: vec![record(9), record(11)],
        ..hidden_enemy_a.clone()
    };
    let enemy_belief = AttributedClaim {
        id: record(9),
        holder: entity(8),
        subject: entity(7),
        claim: "the bridge is unsafe".to_owned(),
        evidence: vec![fact(99)],
        audience: AudienceScope::Host,
        source: reference("enemy-report"),
    };
    let enemy_belief_extra = AttributedClaim {
        id: record(11),
        holder: entity(8),
        subject: entity(7),
        claim: "the gate is open".to_owned(),
        evidence: vec![fact(100)],
        audience: AudienceScope::Host,
        source: reference("enemy-rumor"),
    };
    let own_beliefs = [attributed_belief];
    let beliefs_a = [own_beliefs[0].clone(), enemy_belief.clone()];
    let beliefs_b = [
        own_beliefs[0].clone(),
        enemy_belief.clone(),
        enemy_belief_extra,
    ];
    let npcs_a = [acting_npc.clone(), hidden_enemy_a];
    let npcs_b = [acting_npc, hidden_enemy_b];
    let grants = [];
    let knowledge_a = TacticalKnowledge {
        basis: &fixture.basis,
        pins: &fixture.pins,
        observer,
        grants: &grants,
        beliefs: &beliefs_a,
        npcs: &npcs_a,
    };
    let knowledge_b = TacticalKnowledge {
        basis: &fixture.basis,
        pins: &fixture.pins,
        observer,
        grants: &grants,
        beliefs: &beliefs_b,
        npcs: &npcs_b,
    };
    let first = rank_tactics(
        &observation,
        &admitted,
        &assignments,
        &knowledge_a,
        &policy,
        ranking_limits(),
    )
    .unwrap();
    let second = rank_tactics(
        &observation,
        &admitted,
        &assignments,
        &knowledge_b,
        &policy,
        ranking_limits(),
    )
    .unwrap();

    let first_result: Vec<_> = first
        .tactics()
        .iter()
        .map(|tactic| (tactic.offer().id, tactic.utility()))
        .collect();
    let second_result: Vec<_> = second
        .tactics()
        .iter()
        .map(|tactic| (tactic.offer().id, tactic.utility()))
        .collect();
    assert_eq!(first_result, [(2, 7), (1, 0)]);
    assert_eq!(second_result, first_result);
}

#[test]
fn only_the_current_actor_grant_or_attributed_belief_authorizes_evidence() {
    let fixture = Fixture::new();
    let offers = [Offer { id: 1, legal: true }];
    let admitted = fixture.admitted(&offers);
    let member_observer = KnowledgeObserver::Member(member(3));
    let observation = fixture.observation(member_observer);
    let policy = fixture.policy();
    let grants = [KnowledgeGrant {
        observer: member(3),
        fact: fact(4),
        source: fact(5),
    }];
    let enemy = NpcState {
        entity: entity(8),
        personality: reference("enemy"),
        role: reference("enemy"),
        motivations: vec![],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: vec![fact(99)],
        beliefs: vec![record(9)],
        secrets: vec![],
    };
    let enemy_belief = AttributedClaim {
        id: record(9),
        holder: entity(8),
        subject: entity(7),
        claim: "hidden claim".to_owned(),
        evidence: vec![fact(99)],
        audience: AudienceScope::Host,
        source: reference("enemy-report"),
    };
    let npcs = [enemy];
    let beliefs = [enemy_belief];
    let knowledge = TacticalKnowledge {
        basis: &fixture.basis,
        pins: &fixture.pins,
        observer: member_observer,
        grants: &grants,
        beliefs: &beliefs,
        npcs: &npcs,
    };
    let visible = [contribution(UtilityEvidence::Fact(fact(4)), 1)];
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &visible,
    }];
    let ranked = rank_tactics(
        &observation,
        &admitted,
        &assignments,
        &knowledge,
        &policy,
        ranking_limits(),
    )
    .unwrap();
    assert_eq!(ranked.preferred().unwrap().utility(), 1);

    for evidence in [
        UtilityEvidence::Fact(fact(99)),
        UtilityEvidence::Belief(record(9)),
    ] {
        let undisclosed = [contribution(evidence, i64::MAX)];
        let assignments = [UtilityAssignment {
            offer: &offers[0],
            contributions: &undisclosed,
        }];
        assert!(matches!(
            rank_tactics(
                &observation,
                &admitted,
                &assignments,
                &knowledge,
                &policy,
                ranking_limits(),
            ),
            Err(UtilityRankingError::UndisclosedEvidence)
        ));
    }

    let actor_observer = KnowledgeObserver::Entity(entity(7));
    let actor_observation = fixture.observation(actor_observer);
    let actor_npc = NpcState {
        entity: entity(7),
        personality: reference("acting-npc"),
        role: reference("acting-npc"),
        motivations: vec![],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: vec![fact(4)],
        beliefs: vec![record(6)],
        secrets: vec![],
    };
    let actor_belief = AttributedClaim {
        id: record(6),
        holder: entity(7),
        subject: entity(8),
        claim: "the bridge is safe".to_owned(),
        evidence: vec![fact(4)],
        audience: AudienceScope::Host,
        source: reference("witnessed-report"),
    };
    let actor_beliefs = [actor_belief, beliefs[0].clone()];
    let actor_npcs = [actor_npc, npcs[0].clone()];
    let actor_knowledge = TacticalKnowledge {
        basis: &fixture.basis,
        pins: &fixture.pins,
        observer: actor_observer,
        grants: &[],
        beliefs: &actor_beliefs,
        npcs: &actor_npcs,
    };
    for evidence in [
        UtilityEvidence::Fact(fact(4)),
        UtilityEvidence::Belief(record(6)),
    ] {
        let terms = [contribution(evidence, 2)];
        let assignments = [UtilityAssignment {
            offer: &offers[0],
            contributions: &terms,
        }];
        assert_eq!(
            rank_tactics(
                &actor_observation,
                &admitted,
                &assignments,
                &actor_knowledge,
                &policy,
                ranking_limits(),
            )
            .unwrap()
            .preferred()
            .unwrap()
            .utility(),
            2
        );
    }
    for evidence in [
        UtilityEvidence::Fact(fact(99)),
        UtilityEvidence::Belief(record(9)),
    ] {
        let terms = [contribution(evidence, 2)];
        let assignments = [UtilityAssignment {
            offer: &offers[0],
            contributions: &terms,
        }];
        assert!(matches!(
            rank_tactics(
                &actor_observation,
                &admitted,
                &assignments,
                &actor_knowledge,
                &policy,
                ranking_limits(),
            ),
            Err(UtilityRankingError::UndisclosedEvidence)
        ));
    }
}

#[test]
fn actor_scope_mismatch_rejects_the_whole_ranking_request() {
    let fixture = Fixture::new();
    let offers = [Offer { id: 1, legal: true }];
    let admitted = fixture.admitted(&offers);
    let observation = fixture.observation(KnowledgeObserver::Entity(entity(7)));
    let knowledge = TacticalKnowledge {
        basis: &fixture.basis,
        pins: &fixture.pins,
        observer: KnowledgeObserver::Entity(entity(8)),
        grants: &[],
        beliefs: &[],
        npcs: &[],
    };
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &[],
    }];
    assert!(matches!(
        rank_tactics(
            &observation,
            &admitted,
            &assignments,
            &knowledge,
            &fixture.policy(),
            ranking_limits(),
        ),
        Err(UtilityRankingError::ObserverMismatch)
    ));
}

#[test]
fn stale_knowledge_revision_is_rejected_before_evidence_is_ranked() {
    let fixture = Fixture::new();
    let observer = KnowledgeObserver::Entity(entity(7));
    let observation = fixture.observation(observer);
    let offers = [Offer { id: 1, legal: true }];
    let admitted = fixture.admitted(&offers);
    let stale_basis = Basis {
        revision: fixture.basis.revision.next_sequence().unwrap(),
        ..fixture.basis
    };
    let knowledge = TacticalKnowledge {
        basis: &stale_basis,
        pins: &fixture.pins,
        observer,
        grants: &[],
        beliefs: &[],
        npcs: &[],
    };
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &[],
    }];
    assert!(matches!(
        rank_tactics(
            &observation,
            &admitted,
            &assignments,
            &knowledge,
            &fixture.policy(),
            ranking_limits(),
        ),
        Err(UtilityRankingError::StaleBasis)
    ));
}

#[test]
fn decision_fixture_records_the_bounded_consumer_claim_and_open_producer_gate() {
    assert!(DECISION_FIXTURE.contains("\"production_privacy_qualified\": false"));
    assert!(DECISION_FIXTURE.contains("df-combat has no direct df-knowledge dependency"));
    assert!(DECISION_FIXTURE.contains("tactics cannot read undisclosed enemy facts"));
}

#[test]
fn undisclosed_player_evidence_rejects_before_any_score_or_criterion_is_inspected() {
    let fixture = Fixture::new();
    let observer = KnowledgeObserver::Member(member(3));
    let observation = fixture.observation(observer);
    let offers = [Offer { id: 1, legal: true }];
    let admitted = fixture.admitted(&offers);
    let grants = [
        KnowledgeGrant {
            observer: member(3),
            fact: fact(4),
            source: fact(6),
        },
        KnowledgeGrant {
            observer: member(3),
            fact: fact(5),
            source: fact(6),
        },
        KnowledgeGrant {
            observer: member(8),
            fact: fact(99),
            source: fact(6),
        },
    ];
    let knowledge = TacticalKnowledge {
        basis: &fixture.basis,
        pins: &fixture.pins,
        observer,
        grants: &grants,
        beliefs: &[],
        npcs: &[],
    };
    let contributions = [
        contribution(UtilityEvidence::Fact(fact(4)), i64::MAX),
        contribution(UtilityEvidence::Fact(fact(5)), 1),
        UtilityContribution {
            criterion: reference("unsupported-private-criterion"),
            evidence: UtilityEvidence::Fact(fact(99)),
            value: i64::MAX,
        },
    ];
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &contributions,
    }];
    assert!(matches!(
        rank_tactics(
            &observation,
            &admitted,
            &assignments,
            &knowledge,
            &fixture.policy(),
            ranking_limits()
        ),
        Err(UtilityRankingError::UndisclosedEvidence)
    ));
}

#[test]
fn retaining_another_holders_belief_id_does_not_admit_the_private_claim() {
    let fixture = Fixture::new();
    let observer = KnowledgeObserver::Entity(entity(7));
    let observation = fixture.observation(observer);
    let offers = [Offer { id: 1, legal: true }];
    let admitted = fixture.admitted(&offers);
    let npcs = [NpcState {
        entity: entity(7),
        personality: reference("actor"),
        role: reference("actor"),
        motivations: vec![],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: vec![],
        beliefs: vec![record(9)],
        secrets: vec![],
    }];
    let beliefs = [AttributedClaim {
        id: record(9),
        holder: entity(8),
        subject: entity(7),
        claim: "another player's private tactical claim".to_owned(),
        evidence: vec![fact(99)],
        audience: AudienceScope::Shared,
        source: reference("hidden-source"),
    }];
    let knowledge = TacticalKnowledge {
        basis: &fixture.basis,
        pins: &fixture.pins,
        observer,
        grants: &[],
        beliefs: &beliefs,
        npcs: &npcs,
    };
    let contributions = [contribution(UtilityEvidence::Belief(record(9)), 50)];
    let assignments = [UtilityAssignment {
        offer: &offers[0],
        contributions: &contributions,
    }];
    assert!(matches!(
        rank_tactics(
            &observation,
            &admitted,
            &assignments,
            &knowledge,
            &fixture.policy(),
            ranking_limits()
        ),
        Err(UtilityRankingError::UndisclosedEvidence)
    ));
}
