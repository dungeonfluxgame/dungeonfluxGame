use df_combat::candidates::{CandidateContext, CandidateError, CandidateLimits};
use df_combat::current_response::{
    CurrentResponseRanking, CurrentResponseRankingError, ResponseUtility, SelectedTacticalResponse,
    stage_preferred_response,
};
use df_combat::ranking::{
    KnowledgeObserver, RankingObservation, UtilityContribution, UtilityEvidence, UtilityLimits,
    UtilityPolicy, UtilityRankingError,
};
use df_combat::registered_candidates::{
    RegisteredCandidateError, RegisteredCandidateLimits, RegisteredCandidateRequest,
    enumerate_registered_responses,
};
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_model::commands::CommandLimits;
use df_rules::current_responses::ResponsePreparationError;
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionLimits, PreconditionedCommandHandler, PreconditionedRejection,
    RuleDependency, RulePreconditions,
};
use df_rules::{
    DispatchRegistry, HandlerRegistration, InvocationError, RulesCommandHandler, RulesCommandInput,
};
use std::cell::Cell;
include!("support/combat-I02-response-fixture.rs");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixtureRefusal {
    Refused,
    ActualDrawsRequired,
    Invalid,
}

// Invocation fixture only. No D&D source outcome, bonus, cost or tactic is invented.
struct FixtureHandler {
    pins: CheckpointPins,
    calls: Cell<usize>,
    refuse_on_call: Option<usize>,
    draw_required_on_call: Option<usize>,
}

impl FixtureHandler {
    fn new(refuse_on_call: Option<usize>) -> Self {
        Self {
            pins: pins(),
            calls: Cell::new(0),
            refuse_on_call,
            draw_required_on_call: None,
        }
    }
}

impl RulesCommandHandler for FixtureHandler {
    type Rejection = FixtureRefusal;

    fn pins(&self) -> &CheckpointPins {
        &self.pins
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, FixtureRefusal> {
        assert!(
            input.supplied_draws.is_empty(),
            "Choice/Reaction fixture borrows explicit empty draws"
        );
        let call = self.calls.get() + 1;
        self.calls.set(call);
        // Controlled source-boundary probe: a preview cannot invent a required outcome.
        if self.draw_required_on_call == Some(call) {
            return Err(FixtureRefusal::ActualDrawsRequired);
        }
        if self.refuse_on_call == Some(call) {
            return Err(FixtureRefusal::Refused);
        }
        let GameInput::Game(input) = input.command else {
            return Err(FixtureRefusal::Invalid);
        };
        let mut next = current.basis();
        next.revision = next
            .revision
            .next_sequence()
            .map_err(|_| FixtureRefusal::Invalid)?;
        let mut state = current.state().clone();
        state.pending.clear();
        state.decisions.push(AcceptedDecision {
            operation: input.operation,
            revision: next.revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-policy"),
            semantic_output: None,
        });
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            next,
            current.pins().clone(),
            state,
            ReferenceInventory {
                rules: &[rule()],
                content: &[content()],
                resources: &resource_constraints(),
                assets: &[],
            },
            limits(),
        )
        .map_err(|_| FixtureRefusal::Invalid)
    }
}

fn waiting(change: impl FnOnce(&mut GameState)) -> Checkpoint {
    let mut state = state();
    state.facts.push(fact(7, 0));
    state.knowledge.push(KnowledgeGrant {
        observer: member(3),
        fact: FactId::from_bytes(&[7; 16]).unwrap(),
        source: FactId::from_bytes(&[7; 16]).unwrap(),
    });
    state.beliefs.push(AttributedClaim {
        id: RecordId::from_bytes(&[18; 16]).unwrap(),
        holder: entity(4),
        subject: entity(4),
        claim: "private-attributed-false-belief".to_owned(),
        evidence: vec![FactId::from_bytes(&[7; 16]).unwrap()],
        audience: AudienceScope::Host,
        source: content(),
    });
    state.continuity.npcs.push(NpcState {
        entity: entity(4),
        personality: content(),
        role: content(),
        motivations: vec![],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: vec![FactId::from_bytes(&[7; 16]).unwrap()],
        beliefs: vec![RecordId::from_bytes(&[18; 16]).unwrap()],
        secrets: vec![],
    });
    let mut pending = pending();
    let mut second = match &pending.next {
        PendingInput::Reaction { remaining } => remaining.first().unwrap().clone(),
        _ => panic!("fixture reaction"),
    };
    second.offer = label("fixture-offer-2");
    second.options = vec![label("fixture-option-2")];
    if let PendingInput::Reaction { remaining } = &mut pending.next {
        remaining.push(second);
    }
    state.pending.push(pending);
    change(&mut state);
    checkpoint(state).unwrap()
}

fn template() -> CommandInput {
    CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        member: member(3),
        command: GameCommand::SelectReaction {
            resolution: pending().id,
            window: pending().window.id,
            offer: label("ignored-client-label"),
            option: label("ignored-client-label"),
        },
    }
}

fn copied_context<'a>(context: &CurrentRuleContext<'a>) -> CurrentRuleContext<'a> {
    CurrentRuleContext {
        checkpoint: context.checkpoint,
        basis: context.basis,
        pins: context.pins,
        inventory: ReferenceInventory {
            rules: context.inventory.rules,
            content: context.inventory.content,
            resources: context.inventory.resources,
            assets: context.inventory.assets,
        },
        command_limits: context.command_limits,
    }
}

#[derive(Debug, Eq, PartialEq)]
enum FixtureError {
    Admission(RegisteredCandidateError<FixtureRefusal>),
    Ranking(CurrentResponseRankingError<FixtureRefusal>),
}

fn choose(
    current: &Checkpoint,
    handler: &FixtureHandler,
    observer: KnowledgeObserver,
    evidence: UtilityEvidence,
    values: &[i64],
) -> Result<Option<SelectedTacticalResponse>, FixtureError> {
    let source = rule();
    let sources = [source.clone()];
    let contents = [content()];
    let resources = resource_constraints();
    let context = CurrentRuleContext {
        checkpoint: current,
        basis: current.basis(),
        pins: current.pins(),
        inventory: ReferenceInventory {
            rules: &sources,
            content: &contents,
            resources: &resources,
            assets: &[],
        },
        command_limits: CommandLimits {
            maximum_records: 32,
            maximum_text_bytes: 64,
            maximum_retained_bytes: 16384,
        },
    };
    let dependencies = [RuleDependency::PendingResolution(pending().id)];
    let wrapped = PreconditionedCommandHandler::new(
        handler,
        &source,
        copied_context(&context),
        RulePreconditions {
            prepared: current,
            sources: &sources,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 16,
            maximum_comparisons: 8 * 1024 * 1024,
            maximum_checkpoint_bytes: 1024 * 1024,
        },
    );
    let selector = pending().continuation;
    let registrations = [HandlerRegistration::new(&selector, &source, &wrapped)];
    let entries = [CatalogEntry::new(&source, b"synthetic invocation fixture")];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic invocation fixture",
        &entries,
        CatalogLimits {
            max_complete_bytes: 1024,
            max_entries: 8,
            max_item_bytes: 1024,
            max_total_item_bytes: 1024,
        },
    )
    .unwrap();
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 8).unwrap();
    let mut template = template();
    if matches!(
        current.state().pending.first().map(|pending| &pending.next),
        Some(PendingInput::Choice { .. })
    ) {
        template.command = GameCommand::SelectChoice {
            resolution: pending().id,
            window: pending().window.id,
            offer: label("ignored-client-label"),
            option: label("ignored-client-label"),
        };
    }
    let current_basis = current.basis();
    let admitted = enumerate_registered_responses(
        copied_context(&context),
        &registry,
        RegisteredCandidateRequest {
            template: &template,
            current: CandidateContext {
                basis: &current_basis,
                pins: current.pins(),
            },
            member: template.member,
            operation: template.operation,
            selector: &selector,
        },
        RegisteredCandidateLimits {
            candidates: CandidateLimits {
                max_offers: 8,
                max_identity_comparisons: 28,
                max_candidates: 8,
                max_candidate_bytes: 8192,
            },
            maximum_checkpoint_bytes: 1024 * 1024,
            maximum_inventory_records: 16,
            maximum_rules_preparations: 16,
            maximum_staged_bytes: 1024 * 1024,
        },
    )
    .map_err(FixtureError::Admission)?;
    let terms = [content()];
    let policy = UtilityPolicy {
        basis: &current_basis,
        pins: current.pins(),
        definition: &contents[0],
        criteria: &terms,
    };
    let observation = RankingObservation {
        basis: &current_basis,
        pins: current.pins(),
        observer,
        logical_time: current.state().logical_time,
        cause: template.operation,
        policy: &contents[0],
    };
    let contributions: Vec<_> = values
        .iter()
        .map(|value| {
            [UtilityContribution {
                criterion: content(),
                evidence,
                value: *value,
            }]
        })
        .collect();
    let assignments: Vec<_> = admitted
        .offers()
        .iter()
        .zip(&contributions)
        .map(|(response, scores)| ResponseUtility {
            response,
            option: response.options.first().unwrap(),
            contributions: scores,
        })
        .collect();
    stage_preferred_response(
        CurrentResponseRanking {
            template: &template,
            observation: &observation,
            assignments: &assignments,
            policy: &policy,
            limits: UtilityLimits {
                candidates: 8,
                contributions: 16,
                knowledge_records: 16,
                criteria: 4,
            },
            maximum_staged_bytes: 1024 * 1024,
        },
        &admitted,
        context,
        &registry,
        &selector,
    )
    .map_err(FixtureError::Ranking)
}

#[test]
fn actual_registered_candidates_rank_and_restage_the_selected_exact_response() {
    let current = waiting(|_| {});
    let before = current.clone();
    let handler = FixtureHandler::new(None);
    let selected = choose(
        &current,
        &handler,
        KnowledgeObserver::Member(member(3)),
        UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
        &[2, 9],
    )
    .unwrap()
    .unwrap();
    let GameInput::Game(input) = selected.input() else {
        panic!("canonical game response");
    };
    assert_eq!(
        input.command,
        GameCommand::SelectReaction {
            resolution: pending().id,
            window: pending().window.id,
            offer: label("fixture-offer-2"),
            option: label("fixture-option-2"),
        }
    );
    assert_eq!(input.operation, template().operation);
    assert_eq!(input.basis, current.basis());
    assert_eq!(handler.calls.get(), 3);
    let staged = selected.into_staged();
    assert_eq!(
        staged.basis().revision,
        current.basis().revision.next_sequence().unwrap()
    );
    assert!(staged.state().pending.is_empty());
    assert_eq!(current, before);
}

#[test]
fn stable_ties_preserve_canonical_offer_order_in_the_real_consumer() {
    let current = waiting(|_| {});
    let handler = FixtureHandler::new(None);
    let selected = choose(
        &current,
        &handler,
        KnowledgeObserver::Member(member(3)),
        UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
        &[5, 5],
    )
    .unwrap()
    .unwrap();
    let GameInput::Game(input) = selected.input() else {
        panic!("game response");
    };
    assert!(
        matches!(&input.command, GameCommand::SelectReaction { offer, .. } if offer == &label("fixture-offer-1"))
    );
}

#[test]
fn current_revoked_member_grant_prevents_selection_after_rules_admission() {
    let current = waiting(|state| state.knowledge.clear());
    let handler = FixtureHandler::new(None);
    let result = choose(
        &current,
        &handler,
        KnowledgeObserver::Member(member(3)),
        UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
        &[9, 2],
    );
    assert!(matches!(
        result,
        Err(FixtureError::Ranking(CurrentResponseRankingError::Ranking(
            UtilityRankingError::UndisclosedEvidence
        )))
    ));
    assert_eq!(handler.calls.get(), 2);
    assert_eq!(current.state().decisions.len(), 0);
}

#[test]
fn retained_private_belief_affects_npc_choice_without_leaking_into_the_chosen_command() {
    let current = waiting(|_| {});
    let handler = FixtureHandler::new(None);
    let selected = choose(
        &current,
        &handler,
        KnowledgeObserver::Entity(entity(4)),
        UtilityEvidence::Belief(RecordId::from_bytes(&[18; 16]).unwrap()),
        &[1, 8],
    )
    .unwrap()
    .unwrap();
    let published_command = format!("{:?}", selected.input());
    assert!(!published_command.contains("private-attributed-false-belief"));
    assert!(!published_command.contains("UtilityContribution"));
    assert!(!published_command.contains("RecordId"));
    let GameInput::Game(input) = selected.input() else {
        panic!("game response");
    };
    assert!(
        matches!(&input.command, GameCommand::SelectReaction { offer, .. } if offer == &label("fixture-offer-2"))
    );
}

#[test]
fn revoking_npc_retention_blocks_a_still_existing_attributed_claim() {
    let current = waiting(|state| state.continuity.npcs.first_mut().unwrap().beliefs.clear());
    let handler = FixtureHandler::new(None);
    let result = choose(
        &current,
        &handler,
        KnowledgeObserver::Entity(entity(4)),
        UtilityEvidence::Belief(RecordId::from_bytes(&[18; 16]).unwrap()),
        &[1, 8],
    );
    assert!(matches!(
        result,
        Err(FixtureError::Ranking(CurrentResponseRankingError::Ranking(
            UtilityRankingError::UndisclosedEvidence
        )))
    ));
    assert_eq!(handler.calls.get(), 2);
}

#[test]
fn refused_rules_candidate_produces_no_admitted_ranking_input() {
    let current = waiting(|_| {});
    let handler = FixtureHandler::new(Some(1));
    assert!(matches!(
        choose(
            &current,
            &handler,
            KnowledgeObserver::Member(member(3)),
            UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
            &[9, 2]
        ),
        Err(FixtureError::Admission(_))
    ));
    assert_eq!(handler.calls.get(), 1);
    assert!(current.state().decisions.is_empty());
    let before = current.clone();
    let mut draw_required = FixtureHandler::new(None);
    draw_required.draw_required_on_call = Some(2);
    let result = choose(
        &current,
        &draw_required,
        KnowledgeObserver::Member(member(3)),
        UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
        &[9, 2],
    );
    let Err(FixtureError::Admission(RegisteredCandidateError::Enumeration(CandidateError::Owner(
        error,
    )))) = result
    else {
        panic!("required draw must refuse the entire admission after the first preview");
    };
    assert_eq!(
        error,
        ResponsePreparationError::Invocation(InvocationError::Handler(
            PreconditionedRejection::Handler(FixtureRefusal::ActualDrawsRequired)
        ))
    );
    assert_eq!(draw_required.calls.get(), 2);
    assert_eq!(current, before);
}

#[test]
fn a_refusal_on_selected_revalidation_returns_no_staged_choice() {
    let current = waiting(|_| {});
    let handler = FixtureHandler::new(Some(3));
    assert!(matches!(
        choose(
            &current,
            &handler,
            KnowledgeObserver::Member(member(3)),
            UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
            &[9, 2]
        ),
        Err(FixtureError::Ranking(
            CurrentResponseRankingError::Preparation(_)
        ))
    ));
    assert_eq!(handler.calls.get(), 3);
    assert!(current.state().decisions.is_empty());
    let before = current.clone();
    let mut draw_required = FixtureHandler::new(None);
    draw_required.draw_required_on_call = Some(3);
    let result = choose(
        &current,
        &draw_required,
        KnowledgeObserver::Member(member(3)),
        UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
        &[9, 2],
    );
    let Err(FixtureError::Ranking(CurrentResponseRankingError::Preparation(error))) = result else {
        panic!("required draw must refuse selection without fabricating outcomes");
    };
    assert_eq!(
        error,
        ResponsePreparationError::Invocation(InvocationError::Handler(
            PreconditionedRejection::Handler(FixtureRefusal::ActualDrawsRequired)
        ))
    );
    assert_eq!(draw_required.calls.get(), 3);
    assert_eq!(current, before);
}

#[test]
fn a_choice_uses_the_exact_choice_variant_instead_of_fabricating_a_reaction() {
    let current = waiting(|state| {
        let pending = state.pending.first_mut().unwrap();
        if let PendingInput::Reaction { remaining } = &pending.next {
            pending.next = PendingInput::Choice {
                remaining: remaining.clone(),
            };
        }
    });
    let handler = FixtureHandler::new(None);
    let selected = choose(
        &current,
        &handler,
        KnowledgeObserver::Member(member(3)),
        UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
        &[9, 2],
    )
    .unwrap()
    .unwrap();
    let GameInput::Game(input) = selected.input() else {
        panic!("choice");
    };
    assert!(
        matches!(&input.command, GameCommand::SelectChoice { offer, .. } if offer == &label("fixture-offer-1"))
    );
    assert_eq!(handler.calls.get(), 3);
}

#[test]
fn npc_fact_permission_uses_current_retention_and_does_not_inherit_a_member_grant() {
    let known = waiting(|_| {});
    let handler = FixtureHandler::new(None);
    assert!(
        choose(
            &known,
            &handler,
            KnowledgeObserver::Entity(entity(4)),
            UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
            &[9, 2]
        )
        .unwrap()
        .is_some()
    );
    let revoked = waiting(|state| {
        state
            .continuity
            .npcs
            .first_mut()
            .unwrap()
            .known_facts
            .clear()
    });
    let handler = FixtureHandler::new(None);
    assert!(matches!(
        choose(
            &revoked,
            &handler,
            KnowledgeObserver::Entity(entity(4)),
            UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
            &[9, 2]
        ),
        Err(FixtureError::Ranking(CurrentResponseRankingError::Ranking(
            UtilityRankingError::UndisclosedEvidence
        )))
    ));
    assert_eq!(revoked.state().knowledge.len(), 1);
    assert_eq!(handler.calls.get(), 2);
}

#[test]
fn another_participants_only_offers_return_no_action_without_staging_or_ranking_them() {
    let current = waiting(|state| {
        state.members.push(MembershipLink {
            member: member(12),
            character: None,
        });
        if let PendingInput::Reaction { remaining } = &mut state.pending.first_mut().unwrap().next {
            for response in remaining {
                response.participant = member(12);
            }
        }
    });
    let handler = FixtureHandler::new(None);
    assert!(
        choose(
            &current,
            &handler,
            KnowledgeObserver::Member(member(3)),
            UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
            &[]
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(handler.calls.get(), 0);
    assert!(current.state().decisions.is_empty());
}

#[test]
fn hidden_player_and_npc_context_cannot_change_the_registered_selected_command() {
    let make = |hidden: &str, extra_grant: bool| {
        waiting(|state| {
            state.members.push(MembershipLink {
                member: member(12),
                character: None,
            });
            state.knowledge.push(KnowledgeGrant {
                observer: member(12),
                fact: FactId::from_bytes(&[7; 16]).unwrap(),
                source: FactId::from_bytes(&[7; 16]).unwrap(),
            });
            if extra_grant {
                state.knowledge.push(KnowledgeGrant {
                    observer: member(12),
                    fact: FactId::from_bytes(&[8; 16]).unwrap(),
                    source: FactId::from_bytes(&[7; 16]).unwrap(),
                });
                state.facts.push(fact(8, 1));
            }
            state.beliefs.push(AttributedClaim {
                id: RecordId::from_bytes(&[19; 16]).unwrap(),
                holder: entity(4),
                subject: entity(4),
                claim: hidden.to_owned(),
                evidence: vec![FactId::from_bytes(&[7; 16]).unwrap()],
                audience: AudienceScope::Members(vec![member(12)]),
                source: content(),
            });
        })
    };
    let first = make("private player weakness", false);
    let second = make("different private player plan", true);
    for (observer, evidence) in [
        (
            KnowledgeObserver::Member(member(3)),
            UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
        ),
        (
            KnowledgeObserver::Entity(entity(4)),
            UtilityEvidence::Belief(RecordId::from_bytes(&[18; 16]).unwrap()),
        ),
    ] {
        let first_handler = FixtureHandler::new(None);
        let second_handler = FixtureHandler::new(None);
        let selected_first = choose(&first, &first_handler, observer, evidence, &[1, 8])
            .unwrap()
            .unwrap();
        let selected_second = choose(&second, &second_handler, observer, evidence, &[1, 8])
            .unwrap()
            .unwrap();
        assert_eq!(selected_first.input(), selected_second.input());
        assert_eq!(first_handler.calls.get(), 3);
        assert_eq!(second_handler.calls.get(), 3);
        assert!(first.state().decisions.is_empty());
        assert!(second.state().decisions.is_empty());
        let safe = format!("{:?}", selected_first.input());
        assert!(!safe.contains("private player"));
        assert!(!safe.contains("RecordId"));
    }
}

#[test]
fn hidden_member_grant_cannot_authorize_selection_after_the_actor_grant_is_revoked() {
    let current = waiting(|state| {
        state.members.push(MembershipLink {
            member: member(12),
            character: None,
        });
        state.knowledge.first_mut().unwrap().observer = member(12);
    });
    let before = current.clone();
    let handler = FixtureHandler::new(None);
    assert!(matches!(
        choose(
            &current,
            &handler,
            KnowledgeObserver::Member(member(3)),
            UtilityEvidence::Fact(FactId::from_bytes(&[7; 16]).unwrap()),
            &[i64::MAX, 1]
        ),
        Err(FixtureError::Ranking(CurrentResponseRankingError::Ranking(
            UtilityRankingError::UndisclosedEvidence
        )))
    ));
    assert_eq!(handler.calls.get(), 2);
    assert_eq!(current, before);
}
