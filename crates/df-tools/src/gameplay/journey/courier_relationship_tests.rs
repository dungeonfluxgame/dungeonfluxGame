use super::super::{
    MEMBERS, stage_with_supplier,
    tests::{input, opening_story},
};
use super::*;
use df_engine::relationship_staging::{RelationshipReactionHandler, RelationshipStagingError};
use df_rules::{RulesCommandHandler, RulesCommandInput};
use df_types::MemberId;
use std::cell::Cell;

#[path = "courier_relationship_session_tests.rs"]
mod session_tests;

fn member(index: usize) -> MemberId {
    MemberId::from_bytes(&MEMBERS[index]).unwrap()
}

fn escorted() -> Checkpoint {
    let opening = opening_story();
    stage_with_supplier(
        &opening,
        &input(&opening, member(0), 6, "escort-courier", vec![]),
        &mut |_| panic!("escort cannot draw"),
    )
    .unwrap()
}

fn defend_input(current: &Checkpoint, index: usize, operation: u8) -> GameInput {
    input(current, member(index), operation, "defend-courier", vec![])
}

fn reaction_facts(current: &Checkpoint) -> Vec<&GameFact> {
    current
        .state()
        .facts
        .iter()
        .filter(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "courier-escort-reaction")
        })
        .collect()
}

fn relation(current: &Checkpoint, hero: EntityId) -> &Relationship {
    current
        .state()
        .relationships
        .iter()
        .find(|record| record.subject == courier().unwrap() && record.object == hero)
        .unwrap()
}

#[test]
fn registered_courier_relationship_consumer_preserves_exact_base_facts_draws_and_one_direction() {
    let current = escorted();
    let before = current.clone();
    let command = defend_input(&current, 0, 7);
    let base = super::super::stage_using(&current, &command, &mut |_| Ok(10)).unwrap();
    assert_eq!(base.state().relationships, current.state().relationships);
    let selected = stage_with_supplier(&current, &command, &mut |_| Ok(10)).unwrap();
    let hero = entity(ENTITIES[0]).unwrap();
    let other = entity(ENTITIES[1]).unwrap();
    let mut expected = base.state().clone();
    expected
        .relationships
        .iter_mut()
        .find(|record| record.subject == courier().unwrap() && record.object == hero)
        .unwrap()
        .state = model::label(ESCORT_SUPPORTED).unwrap();
    assert_eq!(selected.state(), &expected);
    assert_eq!(selected.pins(), base.pins());
    assert_eq!(selected.basis(), base.basis());
    assert_eq!(relation(&selected, other), relation(&current, other));
    assert_eq!(
        selected.state().draws.len(),
        current.state().draws.len() + 3
    );
    let facts = reaction_facts(&selected);
    let [fact] = facts.as_slice() else {
        panic!("exactly one authored reaction fact")
    };
    let proposal = propose_on_defend(&current, hero).unwrap().unwrap();
    assert_eq!(fact.cause, Some(proposal.event));
    assert_eq!(fact.audience, AudienceScope::Shared);
    assert!(
        matches!(&fact.value, FactValue::ContentEvent { subjects, .. }
        if subjects.as_slice() == [courier().unwrap(), hero])
    );
    let decision = selected.state().decisions.last().unwrap();
    assert_eq!(decision.facts.get(fact.ordinal as usize), Some(&fact.id));
    assert_eq!(fact.operation, decision.operation);
    assert_eq!(fact.revision, decision.revision);
    assert_eq!(decision.source_policy.as_str(), THREAD_POLICY);
    assert_eq!(decision.draws, vec![0, 1, 2]);
    assert_eq!(
        stage_with_supplier(&current, &command, &mut |_| Ok(10)).unwrap(),
        selected
    );
    assert!(
        stage_with_supplier(&selected, &command, &mut |_| {
            panic!("stale retry cannot draw")
        })
        .is_err()
    );
    assert_eq!(current, before);
}

#[test]
fn registered_courier_relationship_absent_wrong_witness_ignorance_and_other_hero_do_not_invent_reaction()
 {
    let current = escorted();
    for case in 0..5 {
        let mut state = current.state().clone();
        match case {
            0 => state.continuity.witnesses.clear(),
            1 => state.continuity.witnesses[0].observer = entity(ENTITIES[1]).unwrap(),
            2 => {
                state.continuity.witnesses[0].source =
                    model::content("courier-escort-reaction-policy").unwrap()
            }
            3 => state.continuity.npcs[0].known_facts.clear(),
            _ => {}
        }
        let checkpoint = model::checkpoint(current.basis(), state).unwrap();
        let original = checkpoint.clone();
        let command = defend_input(&checkpoint, usize::from(case == 4), 7);
        let selected = stage_with_supplier(&checkpoint, &command, &mut |_| Ok(10)).unwrap();
        assert_eq!(
            selected.state().relationships,
            checkpoint.state().relationships
        );
        assert!(reaction_facts(&selected).is_empty());
        assert_eq!(selected.state().continuity, checkpoint.state().continuity);
        assert_eq!(checkpoint, original);
    }
}

#[test]
fn registered_courier_relationship_stale_witness_source_actor_and_pins_refuse_without_draw_or_state_change()
 {
    let current = escorted();
    for case in 0..3 {
        let mut state = current.state().clone();
        match case {
            0 => state.continuity.witnesses[0].perceived_at.ticks = 1,
            1 => {
                state.decisions.last_mut().unwrap().source_policy =
                    model::label("unadmitted-escort-source").unwrap()
            }
            _ => {}
        }
        let checkpoint = model::checkpoint(current.basis(), state).unwrap();
        let original = checkpoint.clone();
        let mut command = defend_input(&checkpoint, 0, 7);
        if case == 2 {
            let GameInput::Game(input) = &mut command else {
                panic!("command")
            };
            let GameCommand::ProposeAction { actor, .. } = &mut input.command else {
                panic!("action")
            };
            *actor = entity(ENTITIES[1]).unwrap();
        }
        let samples = Cell::new(0);
        assert!(
            stage_with_supplier(&checkpoint, &command, &mut |_| {
                samples.set(samples.get() + 1);
                Ok(10)
            })
            .is_err()
        );
        // Native preparation precedes reaction selection; a bad witness discards its
        // tentative initiative samples, while actor/source refusal stops before them.
        assert_eq!(samples.get(), if case == 0 { 3 } else { 0 });
        assert_eq!(checkpoint.state().draws, original.state().draws);
        assert_eq!(checkpoint, original);
    }
    let mut stale = current.pins().clone();
    stale.content.package_digest = ContentDigest([0x91; 32]);
    let recovery = model::recovery().unwrap();
    let checkpoint = Checkpoint::new(
        current.schema(),
        current.basis(),
        stale,
        current.state().clone(),
        ReferenceInventory {
            rules: &recovery.rules,
            content: &recovery.content,
            resources: &super::super::resources_for(current.state()).unwrap(),
            assets: &recovery.assets,
        },
        model::limits(),
    )
    .unwrap();
    let original = checkpoint.clone();
    assert!(
        stage_with_supplier(&checkpoint, &defend_input(&checkpoint, 0, 7), &mut |_| {
            panic!("stale pins cannot draw")
        })
        .is_err()
    );
    assert_eq!(checkpoint, original);
}

struct CountedJourney<'a> {
    inner: super::super::JourneyHandler,
    calls: &'a Cell<usize>,
    refuse: bool,
}
impl RulesCommandHandler for CountedJourney<'_> {
    type Rejection = RepositoryError;
    fn pins(&self) -> &CheckpointPins {
        self.inner.pins()
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        self.calls.set(self.calls.get() + 1);
        if self.refuse {
            return Err(RepositoryError::InvalidCandidate);
        }
        self.inner.stage(input, current)
    }
}

fn registered_with<H: RulesCommandHandler>(
    current: &Checkpoint,
    input: &GameInput,
    draws: &[ActualDraw],
    handler: &H,
) -> Result<Checkpoint, df_engine::command_entry::CommandRejection<H::Rejection>> {
    let pins = model::pins().unwrap();
    let source = super::super::rule().unwrap();
    let selector = model::label("local-journey-handler-1").unwrap();
    let manifest = model::source_manifest();
    let entries = [df_content::catalog::CatalogEntry::new(&source, &manifest)];
    let catalog = df_content::catalog::CatalogSnapshot::from_published(
        &pins.rules.catalog,
        &pins,
        &manifest,
        &entries,
        df_content::catalog::CatalogLimits {
            max_complete_bytes: 4096,
            max_entries: 1,
            max_item_bytes: 4096,
            max_total_item_bytes: 4096,
        },
    )
    .unwrap();
    let registrations = [df_rules::HandlerRegistration::new(
        &selector, &source, handler,
    )];
    let registry = df_rules::DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    let rules = [model::rule().unwrap(), source.clone()];
    let content = model::contents().unwrap();
    let resources = super::super::resources_for(current.state()).unwrap();
    df_engine::command_entry::decide_registered_command(
        RulesCommandInput {
            command: input,
            supplied_draws: draws,
        },
        current,
        df_engine::command_entry::CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: &pins,
            inventory: ReferenceInventory {
                rules: &rules,
                content: &content,
                resources: &resources,
                assets: &[],
            },
            limits: df_engine::command_entry::CommandEntryLimits {
                command: df_model::commands::CommandLimits {
                    maximum_records: 32,
                    maximum_text_bytes: 128,
                    maximum_retained_bytes: 8192,
                },
                maximum_staged_bytes: 1024 * 1024,
            },
        },
        &registry,
        &selector,
        &source,
    )
}

#[test]
fn registered_courier_relationship_unsupported_policy_handler_refusal_and_final_constructor_discard_whole_candidate()
 {
    let current = escorted();
    let before = current.clone();
    let command = defend_input(&current, 0, 7);
    let prepared = super::super::stage_using(&current, &command, &mut |_| Ok(10)).unwrap();
    let GameInput::Game(input) = &command else {
        panic!("command")
    };
    let draws = prepared
        .state()
        .draws
        .iter()
        .filter(|draw| draw.operation == input.operation)
        .cloned()
        .collect::<Vec<_>>();
    for case in 0..3 {
        let calls = Cell::new(0);
        let inner = CountedJourney {
            inner: super::super::JourneyHandler {
                pins: current.pins().clone(),
            },
            calls: &calls,
            refuse: case == 1,
        };
        let owner = AuthoredCourierSource { current: &current };
        let requests = [request_on_defend(&current, entity(ENTITIES[0]).unwrap())
            .unwrap()
            .unwrap()];
        let mut entries = [entry().unwrap()];
        if case == 0 {
            entries[0].to_state = model::label("unadmitted-social-effect").unwrap();
        }
        let mut limits = relationship_limits();
        if case == 2 {
            limits.checkpoint.maximum_records = 1;
        }
        let rules = [model::rule().unwrap(), super::super::rule().unwrap()];
        let content = model::contents().unwrap();
        let resources = super::super::resources_for(current.state()).unwrap();
        let handler = RelationshipReactionHandler {
            command_handler: &inner,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: ReferenceInventory {
                rules: &rules,
                content: &content,
                resources: &resources,
                assets: &[],
            },
            requests: &requests,
            entries: &entries,
            limits,
        };
        let result = registered_with(&current, &command, &draws, &handler);
        assert!(result.is_err());
        assert_eq!(calls.get(), usize::from(case != 0));
        if case == 0 {
            assert!(matches!(
                result,
                Err(df_engine::command_entry::CommandRejection::Invocation(
                    df_rules::InvocationError::Handler(RelationshipStagingError::Source(
                        RepositoryError::InvalidCandidate
                    ))
                ))
            ));
        }
        assert_eq!(current, before);
    }
}
