//! Source-bound perception and categorical reaction for the courier at Lantern Wharf.

use df_interaction::reactions::{
    NoReactionReason, ReactionEntry, ReactionLimits, ReactionOutcome, ReactionPolicy,
    ReactionProposal, ReactionRequest, ReactionSourceOwner, ReactionSourceRefusal, react,
};
use df_model::checkpoint::*;
use df_persistence::local_demo_scope::PLAYER;
use df_session::submission::RepositoryError;
use df_types::OperationId;
use sha2::{Digest, Sha256};

use super::{ENTITIES, THREAD_POLICY, accepted, bad, entity};
use crate::gameplay::model;

pub(super) const COURIER: [u8; 16] = [0x67; 16];
pub(in crate::gameplay) const CONTENT_ENTRIES: &[&str] = &[
    "lantern-wharf-courier",
    "cautious-courier",
    "deliver-dispatch",
    "courier-escort-relationship",
    "courier-escort-perception",
    "courier-escort-contact",
    "courier-escort-reaction-policy",
    "courier-escort-reaction",
];
const UNFAMILIAR: &str = "unfamiliar";
const ESCORT_SUPPORTED: &str = "escort-supported";
// The retained proposal carries both complete seven-axis relationship states.
const REACTION_PROPOSAL_BYTES: usize = 8 * 1024;

pub(super) fn courier() -> Result<EntityId, RepositoryError> {
    entity(COURIER)
}

fn courier_present(state: &GameState) -> Result<bool, RepositoryError> {
    let courier = courier()?;
    let definition = model::content("lantern-wharf-courier")?;
    let revision = model::label("lantern-wharf-courier-1")?;
    Ok(state.entities.iter().any(|world| {
        world.id == courier
            && world.definition == definition
            && world.identity_revision == revision
            && world.location.is_none()
            && world.position.is_none()
    }))
}

fn joined_hero(state: &GameState, hero: EntityId) -> bool {
    state.members.iter().any(|link| {
        link.member.as_bytes() != &PLAYER
            && link.character == Some(hero)
            && state
                .characters
                .iter()
                .any(|character| character.entity == hero && character.owner == link.member)
    })
}

/// The opening scene places this courier in front of both heroes. Identity and
/// initial attitudes are canonical even before the courier observes an action.
pub(super) fn enter_opening(state: &mut GameState) -> Result<(), RepositoryError> {
    let courier = courier()?;
    if state.entities.iter().any(|entity| entity.id == courier)
        || state
            .continuity
            .npcs
            .iter()
            .any(|npc| npc.entity == courier)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    state.entities.push(WorldEntity {
        id: courier,
        definition: model::content("lantern-wharf-courier")?,
        location: None,
        position: None,
        identity_revision: model::label("lantern-wharf-courier-1")?,
    });
    state.continuity.npcs.push(NpcState {
        entity: courier,
        personality: model::content("cautious-courier")?,
        role: model::content("cautious-courier")?,
        motivations: vec![model::content("deliver-dispatch")?],
        goals: vec![],
        needs: vec![],
        fears: vec![],
        known_facts: Vec::new(),
        beliefs: Vec::new(),
        secrets: Vec::new(),
    });
    for bytes in ENTITIES {
        let hero = entity(bytes)?;
        if !state
            .characters
            .iter()
            .any(|character| character.entity == hero)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        state.relationships.push(Relationship {
            subject: courier,
            object: hero,
            policy: model::content("courier-escort-relationship")?,
            state: model::label(UNFAMILIAR)?,
            trust: RelationshipAxisState {
                value: model::label(UNFAMILIAR)?,
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: model::content("courier-escort-relationship")?,
                },
            },
            affection: RelationshipAxisState {
                value: model::label(UNFAMILIAR)?,
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: model::content("courier-escort-relationship")?,
                },
            },
            respect: RelationshipAxisState {
                value: model::label(UNFAMILIAR)?,
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: model::content("courier-escort-relationship")?,
                },
            },
            fear: RelationshipAxisState {
                value: model::label(UNFAMILIAR)?,
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: model::content("courier-escort-relationship")?,
                },
            },
            suspicion: RelationshipAxisState {
                value: model::label(UNFAMILIAR)?,
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: model::content("courier-escort-relationship")?,
                },
            },
            debt: RelationshipAxisState {
                value: model::label(UNFAMILIAR)?,
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: model::content("courier-escort-relationship")?,
                },
            },
            familiarity: RelationshipAxisState {
                value: model::label(UNFAMILIAR)?,
                provenance: RelationshipAxisProvenance::AuthoredBaseline {
                    source: model::content("courier-escort-relationship")?,
                },
            },
        });
    }
    Ok(())
}

pub(in crate::gameplay) fn witness_id(fact: FactId) -> Result<RecordId, RepositoryError> {
    let digest = Sha256::digest(
        [
            b"lantern-wharf-courier-witness-1".as_slice(),
            fact.as_bytes(),
        ]
        .concat(),
    );
    RecordId::from_bytes(&digest[..16]).map_err(bad)
}

/// Only the accepted shared escort scene supplies the courier's own observation.
pub(super) fn perceive_escort(
    state: &mut GameState,
    fact_id: FactId,
    operation: OperationId,
    revision: df_types::SessionRevision,
    target: EntityId,
) -> Result<(), RepositoryError> {
    let courier = courier()?;
    let event = state
        .facts
        .iter()
        .find(|fact| fact.id == fact_id)
        .ok_or(RepositoryError::InvalidCandidate)?;
    if event.operation != operation
        || event.revision != revision
        || event.audience != AudienceScope::Shared
        || !matches!(&event.value, FactValue::ContentEvent { definition, subjects }
            if *definition == model::content("courier-escort-contact")? && subjects.as_slice() == [target])
        || !courier_present(state)?
        || !joined_hero(state, target)
        || state.narrative.active_beats.as_slice() != [model::content("courier-answer-escort")?]
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let npc = state
        .continuity
        .npcs
        .iter_mut()
        .find(|npc| npc.entity == courier)
        .ok_or(RepositoryError::InvalidCandidate)?;
    if npc.personality != model::content("cautious-courier")?
        || npc.motivations.as_slice() != [model::content("deliver-dispatch")?]
        || npc.known_facts.contains(&fact_id)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let id = witness_id(fact_id)?;
    if state
        .continuity
        .witnesses
        .iter()
        .any(|witness| witness.id == id)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    npc.known_facts.push(fact_id);
    state.continuity.witnesses.push(WitnessRecord {
        id,
        observer: courier,
        fact: fact_id,
        perceived_at: state.logical_time,
        source: model::content("courier-escort-perception")?,
    });
    Ok(())
}

fn entry() -> Result<ReactionEntry, RepositoryError> {
    Ok(ReactionEntry {
        source: model::content("courier-escort-reaction-policy")?,
        event: model::content("courier-escort-contact")?,
        personality: model::content("cautious-courier")?,
        motivation: model::content("deliver-dispatch")?,
        relationship_policy: model::content("courier-escort-relationship")?,
        from_state: model::label(UNFAMILIAR)?,
        to_state: model::label(ESCORT_SUPPORTED)?,
    })
}

struct AuthoredCourierSource<'a> {
    current: &'a Checkpoint,
}

impl ReactionSourceOwner for AuthoredCourierSource<'_> {
    fn validate_entry(
        &self,
        basis: Basis,
        pins: &ContentPins,
        proposed: &ReactionEntry,
    ) -> Result<(), ReactionSourceRefusal> {
        let source = model::pins().map_err(|_| ReactionSourceRefusal::NotAdmitted)?;
        let admitted = entry().map_err(|_| ReactionSourceRefusal::NotAdmitted)?;
        if basis != self.current.basis()
            || *pins != self.current.pins().content
            || *pins != source.content
        {
            return Err(ReactionSourceRefusal::StaleAdmission);
        }
        if *proposed != admitted {
            return Err(ReactionSourceRefusal::UnsupportedPolicy);
        }
        Ok(())
    }
}

impl df_engine::relationship_staging::RelationshipApplicationOwner for AuthoredCourierSource<'_> {
    type Refusal = RepositoryError;

    fn admit_application(
        &self,
        current: &Checkpoint,
        command: &CommandInput,
        requests: &[ReactionRequest],
        entries: &[ReactionEntry],
    ) -> Result<(), Self::Refusal> {
        current
            .validate_resume(self.current.basis(), &model::pins()?)
            .map_err(bad)?;
        let GameCommand::ProposeAction {
            actor,
            action,
            targets,
            choices,
        } = &command.command
        else {
            return Err(RepositoryError::InvalidCandidate);
        };
        super::super::beat_script_runtime::select(
            current,
            df_protocol::common::GameplayActionKind::DefendCourier,
            choices,
        )?;
        if current != self.current
            || command.basis != current.basis()
            || command.observed_revision != current.basis().revision
            || *actor != super::player_entity(command.member, current)?
            || *action != model::content("defend-courier")?
            || !targets.is_empty()
            || choices
                .iter()
                .any(|(group, _)| group.as_str() != super::super::beat_script_runtime::CHOICE_GROUP)
            || super::phase(current)? != df_protocol::common::JourneyPhase::Dialogue
            || !super::offered(current, command.member)?
                .iter()
                .any(|(kind, _)| *kind == df_protocol::common::GameplayActionKind::DefendCourier)
            || entries != [entry()?]
            || requests != request_on_defend(current, *actor)?.as_slice()
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    }
}

fn relationship_limits() -> df_engine::relationship_staging::RelationshipStagingLimits {
    df_engine::relationship_staging::RelationshipStagingLimits {
        maximum_reactions: 1,
        maximum_relationships: 512,
        maximum_work: 1024 * 1024,
        maximum_inventory_records: 512,
        maximum_checkpoint_bytes: model::limits().maximum_retained_bytes,
        maximum_pass_bytes: 8 * 1024 * 1024,
        checkpoint: model::limits(),
        reaction: ReactionLimits {
            maximum_entries: 1,
            maximum_policy_bytes: 4096,
            maximum_work: 16_384,
            maximum_proposal_bytes: REACTION_PROPOSAL_BYTES,
        },
    }
}

/// Registered native journey adapter; reactions observe the prior committed checkpoint.
/// The inner journey preserves relationships and emits the authored causal fact. The engine
/// then recomputes/selects the categorical replacement before returning a complete candidate.
pub(super) struct CourierJourneyHandler<'a, Handler> {
    pub command_handler: &'a Handler,
}

impl<Handler> df_rules::RulesCommandHandler for CourierJourneyHandler<'_, Handler>
where
    Handler: df_rules::RulesCommandHandler<Rejection = RepositoryError>,
{
    type Rejection = RepositoryError;

    fn pins(&self) -> &CheckpointPins {
        self.command_handler.pins()
    }

    fn bound_source(&self) -> Option<&RuleReference> {
        self.command_handler.bound_source()
    }

    fn stage(
        &self,
        input: df_rules::RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        let GameInput::Game(command) = input.command else {
            return self.command_handler.stage(input, current);
        };
        let GameCommand::ProposeAction { actor, action, .. } = &command.command else {
            return self.command_handler.stage(input, current);
        };
        if action.entry.as_str() != "defend-courier" {
            return self.command_handler.stage(input, current);
        }
        let source = AuthoredCourierSource { current };
        let request = request_on_defend(current, *actor)?;
        let entries = [entry()?];
        let rules = [model::rule()?, super::rule()?];
        let content = model::contents()?;
        let resources = super::resources_for(current.state())?;
        let handler = df_engine::relationship_staging::RelationshipReactionHandler {
            command_handler: self.command_handler,
            owner: &source,
            current_basis: current.basis(),
            admitted_pins: self.pins(),
            inventory: ReferenceInventory {
                rules: &rules,
                content: &content,
                resources: &resources,
                assets: &[],
            },
            requests: request.as_slice(),
            entries: &entries,
            limits: relationship_limits(),
        };
        df_rules::RulesCommandHandler::stage(&handler, input, current).map_err(bad)
    }
}

/// Evaluate only the prior checkpoint. The current Defend candidate cannot make
/// its own staged facts look committed to df-interaction::react.
fn request_on_defend(
    current: &Checkpoint,
    target: EntityId,
) -> Result<Option<ReactionRequest>, RepositoryError> {
    current
        .validate_resume(current.basis(), &model::pins()?)
        .map_err(bad)?;
    if !courier_present(current.state())? || !joined_hero(current.state(), target) {
        return Err(RepositoryError::InvalidCandidate);
    }
    let escort = model::content("courier-escort-contact")?;
    let mut sources = current.state().facts.iter().filter(|fact| {
        fact.audience == AudienceScope::Shared
            && matches!(&fact.value, FactValue::ContentEvent { definition, subjects }
                if *definition == escort && subjects.len() == 1)
    });
    let Some(event) = sources.next() else {
        return Ok(None);
    };
    if sources.next().is_some() {
        return Err(RepositoryError::InvalidCandidate);
    }
    let mut decisions = current.state().decisions.iter().filter(|decision| {
        decision.operation == event.operation
            && decision.revision == event.revision
            && decision.facts.contains(&event.id)
            && decision.source_policy.as_str() == THREAD_POLICY
    });
    let decision = decisions.next().ok_or(RepositoryError::InvalidCandidate)?;
    if decisions.next().is_some()
        || accepted(decision)?.phase != df_protocol::common::JourneyPhase::Dialogue as i32
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let terminal = decision
        .facts
        .last()
        .and_then(|id| current.state().facts.iter().find(|fact| fact.id == *id))
        .ok_or(RepositoryError::InvalidCandidate)?;
    if terminal.operation != event.operation
        || terminal.revision != event.revision
        || Some(terminal.ordinal) != event.ordinal.checked_add(1)
        || terminal.cause != Some(event.id)
        || terminal.audience != AudienceScope::Shared
        || !matches!(&terminal.value, FactValue::ContentEvent { definition, subjects }
            if *definition == model::content("escort-courier")? && subjects.is_empty())
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let courier = courier()?;
    let perception = model::content("courier-escort-perception")?;
    let expected_witness = witness_id(event.id)?;
    let Some(witness) = current.state().continuity.witnesses.iter().find(|witness| {
        witness.id == expected_witness
            && witness.observer == courier
            && witness.fact == event.id
            && witness.source == perception
    }) else {
        return Ok(None);
    };
    Ok(Some(ReactionRequest {
        expected_basis: current.basis(),
        npc: courier,
        target,
        event: event.id,
        witness: witness.id,
    }))
}

pub(super) fn propose_on_defend(
    current: &Checkpoint,
    target: EntityId,
) -> Result<Option<Box<ReactionProposal>>, RepositoryError> {
    let Some(request) = request_on_defend(current, target)? else {
        return Ok(None);
    };
    let source = AuthoredCourierSource { current };
    let entries = [entry()?];
    let content = model::contents()?;
    let resources = super::resources_for(current.state())?;
    let inventory = ReferenceInventory {
        rules: &[],
        content: &content,
        resources: &resources,
        assets: &[],
    };
    let policy = ReactionPolicy::new(
        current,
        &entries,
        inventory,
        &source,
        ReactionLimits {
            maximum_entries: 1,
            maximum_policy_bytes: 4096,
            maximum_work: 16_384,
            maximum_proposal_bytes: REACTION_PROPOSAL_BYTES,
        },
    )
    .map_err(bad)?;
    match react(current, request, &policy).map_err(bad)? {
        ReactionOutcome::Proposed(proposal) => Ok(Some(proposal)),
        ReactionOutcome::NoReaction(
            NoReactionReason::NotKnown
            | NoReactionReason::NotWitnessed
            | NoReactionReason::UnrelatedTarget
            | NoReactionReason::NoMatchingPolicy
            | NoReactionReason::UnchangedState,
        ) => Ok(None),
        ReactionOutcome::NoReaction(NoReactionReason::UnsupportedEvent) => {
            Err(RepositoryError::InvalidCandidate)
        }
    }
}

pub(super) fn stage_proposal(
    current: &Checkpoint,
    state: &mut GameState,
    proposal: &ReactionProposal,
    operation: OperationId,
    revision: df_types::SessionRevision,
    ordinal: u32,
    id: FactId,
) -> Result<(), RepositoryError> {
    let courier = courier()?;
    let expected = propose_on_defend(current, proposal.original.object)?;
    if expected.as_deref() != Some(proposal)
        || proposal.expected_basis != current.basis()
        || proposal.content_pins != current.pins().content
        || proposal.policy_entry != entry()?
        || proposal.original.subject != courier
        || proposal.proposed.subject != courier
        || proposal.proposed.object != proposal.original.object
        || proposal.proposed.policy != proposal.original.policy
        || proposal.proposed.state != model::label(ESCORT_SUPPORTED)?
        || proposal.event == id
        || operation == proposal.accepted_operation
        || revision <= proposal.accepted_revision
        || ordinal
            != state
                .facts
                .iter()
                .filter(|fact| fact.operation == operation)
                .count() as u32
        || state.facts.iter().any(|fact| fact.id == id)
        || !current.state().facts.iter().any(|fact| {
            fact.id == proposal.event
                && fact.operation == proposal.accepted_operation
                && fact.revision == proposal.accepted_revision
                && fact.cause == proposal.cause
        })
        || !current.state().continuity.witnesses.iter().any(|witness| {
            witness.id == proposal.witness
                && witness.observer == courier
                && witness.fact == proposal.event
        })
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    if !state.relationships.contains(&proposal.original) {
        return Err(RepositoryError::InvalidCandidate);
    }
    // The registered RelationshipReactionHandler owns the categorical replacement.
    // The base journey contributes only this existing authored fact to its decision.
    state.facts.push(GameFact {
        id,
        revision,
        operation,
        ordinal,
        cause: Some(proposal.event),
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: model::content("courier-escort-reaction")?,
            subjects: vec![courier, proposal.original.object],
        },
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::input;
    use super::super::{MEMBERS, stage_with_supplier};
    use super::*;

    fn first_member() -> df_types::MemberId {
        df_types::MemberId::from_bytes(&MEMBERS[0]).expect("joined member")
    }

    fn escorted() -> Checkpoint {
        let opening = super::super::tests::opening_story();
        let member = first_member();
        stage_with_supplier(
            &opening,
            &input(&opening, member, 6, "escort-courier", vec![]),
            &mut |_| panic!("escort cannot draw"),
        )
        .expect("accepted shared escort")
    }

    fn attitude(checkpoint: &Checkpoint, hero: EntityId) -> &str {
        checkpoint
            .state()
            .relationships
            .iter()
            .find(|relationship| {
                relationship.subject == courier().unwrap() && relationship.object == hero
            })
            .expect("courier relationship")
            .state
            .as_str()
    }

    #[test]
    fn persistent_axis_reaction_obeys_proposal_budget_and_preserves_checkpoint() {
        use df_interaction::reactions::{ReactionError, ReactionLimit};
        let current = escorted();
        let original = current.clone();
        let hero = entity(ENTITIES[0]).unwrap();
        let request = request_on_defend(&current, hero).unwrap().unwrap();
        let source = AuthoredCourierSource { current: &current };
        let entries = [entry().unwrap()];
        let content = model::contents().unwrap();
        let resources = super::super::resources_for(current.state()).unwrap();
        let inventory = || ReferenceInventory {
            rules: &[],
            content: &content,
            resources: &resources,
            assets: &[],
        };
        let admitted = relationship_limits().reaction;
        let legacy = ReactionPolicy::new(
            &current,
            &entries,
            inventory(),
            &source,
            ReactionLimits {
                maximum_proposal_bytes: 4096,
                ..admitted
            },
        )
        .unwrap();
        assert!(matches!(
            react(&current, request, &legacy),
            Err(ReactionError::LimitExceeded(ReactionLimit::ProposalBytes)),
        ));
        assert_eq!(current, original);
        let policy =
            ReactionPolicy::new(&current, &entries, inventory(), &source, admitted).unwrap();
        let ReactionOutcome::Proposed(proposal) = react(&current, request, &policy).unwrap() else {
            panic!("source-qualified persistent courier reaction");
        };
        let relationship = current
            .state()
            .relationships
            .iter()
            .find(|relationship| {
                relationship.subject == courier().unwrap() && relationship.object == hero
            })
            .unwrap();
        assert_eq!(&proposal.original, relationship);
        assert_eq!(proposal.proposed.state.as_str(), ESCORT_SUPPORTED);
        assert_eq!(proposal.proposed.trust, relationship.trust);
        assert_eq!(proposal.proposed.affection, relationship.affection);
        assert_eq!(proposal.proposed.respect, relationship.respect);
        assert_eq!(proposal.proposed.fear, relationship.fear);
        assert_eq!(proposal.proposed.suspicion, relationship.suspicion);
        assert_eq!(proposal.proposed.debt, relationship.debt);
        assert_eq!(proposal.proposed.familiarity, relationship.familiarity);
        assert_eq!(current, original);
        assert_eq!(admitted.maximum_proposal_bytes, 8 * 1024);
    }

    #[test]
    fn accepted_escort_is_witnessed_then_next_defend_stages_only_directional_reaction() {
        let opening = super::super::tests::opening_story();
        let npc = opening
            .state()
            .continuity
            .npcs
            .first()
            .expect("canonical courier");
        assert_eq!(npc.entity, courier().unwrap());
        assert!(npc.known_facts.is_empty());
        assert!(opening.state().continuity.witnesses.is_empty());
        let current = escorted();
        let hero = entity(ENTITIES[0]).unwrap();
        let other = entity(ENTITIES[1]).unwrap();
        let contact = current
            .state()
            .facts
            .iter()
            .find(|fact| {
                matches!(&fact.value,
            FactValue::ContentEvent { definition, .. }
            if *definition == model::content("courier-escort-contact").unwrap())
            })
            .expect("contact fact");
        assert_eq!(contact.audience, AudienceScope::Shared);
        assert!(
            matches!(&contact.value, FactValue::ContentEvent { subjects, .. }
            if subjects.as_slice() == [hero])
        );
        let escort = current.state().facts.last().expect("terminal escort fact");
        assert_eq!(escort.cause, Some(contact.id));
        assert!(
            matches!(&escort.value, FactValue::ContentEvent { definition, subjects }
            if *definition == model::content("escort-courier").unwrap() && subjects.is_empty())
        );
        assert_eq!(
            current.state().continuity.npcs[0].known_facts,
            vec![contact.id]
        );
        assert_eq!(current.state().continuity.witnesses.len(), 1);
        assert_eq!(attitude(&current, hero), UNFAMILIAR);
        let command = input(&current, first_member(), 7, "defend-courier", vec![]);
        let reacted = stage_with_supplier(&current, &command, &mut |_| Ok(10))
            .expect("registered Defend candidate");
        let replay = stage_with_supplier(&current, &command, &mut |_| Ok(10))
            .expect("same prior committed basis");
        assert_eq!(reacted, replay);
        assert_eq!(attitude(&reacted, hero), ESCORT_SUPPORTED);
        assert_eq!(attitude(&reacted, other), UNFAMILIAR);
        assert_eq!(reacted.state().relationships.len(), 2);
        let reaction = reacted
            .state()
            .facts
            .iter()
            .find(|fact| {
                matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                if *definition == model::content("courier-escort-reaction").unwrap())
            })
            .expect("source-linked reaction fact");
        assert_eq!(reaction.cause, Some(contact.id));
        assert_eq!(
            reaction.operation,
            df_types::OperationId::from_bytes(&[7; 16]).unwrap()
        );
        assert_eq!(reaction.audience, AudienceScope::Shared);
        assert!(
            reacted
                .state()
                .decisions
                .last()
                .unwrap()
                .facts
                .contains(&reaction.id)
        );
        assert!(
            stage_with_supplier(&reacted, &command, &mut |_| panic!("duplicate cannot draw"))
                .is_err()
        );
    }

    #[test]
    fn private_cue_never_gives_courier_perception_or_reaction() {
        let opening = super::super::tests::opening_story();
        let private = stage_with_supplier(
            &opening,
            &input(&opening, first_member(), 6, "ask-courier", vec![]),
            &mut |_| panic!("private cue cannot draw"),
        )
        .expect("private cue");
        assert!(private.state().continuity.npcs[0].known_facts.is_empty());
        assert!(private.state().continuity.witnesses.is_empty());
        let defended = stage_with_supplier(
            &private,
            &input(&private, first_member(), 7, "defend-courier", vec![]),
            &mut |_| Ok(10),
        )
        .expect("private route still starts combat");
        assert_eq!(
            attitude(&defended, entity(ENTITIES[0]).unwrap()),
            UNFAMILIAR
        );
        assert!(
            !defended
                .state()
                .facts
                .iter()
                .any(|fact| matches!(&fact.value,
            FactValue::ContentEvent { definition, .. }
            if *definition == model::content("courier-escort-reaction").unwrap()))
        );
    }

    #[test]
    fn missing_or_wrong_evidence_and_other_target_do_not_propose() {
        let current = escorted();
        let hero = entity(ENTITIES[0]).unwrap();
        let other = entity(ENTITIES[1]).unwrap();
        assert!(propose_on_defend(&current, other).unwrap().is_none());
        let mut no_knowledge = current.state().clone();
        no_knowledge.continuity.npcs[0].known_facts.clear();
        let no_knowledge = model::checkpoint(current.basis(), no_knowledge).unwrap();
        assert!(propose_on_defend(&no_knowledge, hero).unwrap().is_none());
        let mut no_witness = current.state().clone();
        no_witness.continuity.witnesses.clear();
        let no_witness = model::checkpoint(current.basis(), no_witness).unwrap();
        assert!(propose_on_defend(&no_witness, hero).unwrap().is_none());
        let mut wrong_witness = current.state().clone();
        wrong_witness.continuity.witnesses[0].source =
            model::content("courier-escort-reaction-policy").unwrap();
        let wrong_witness = model::checkpoint(current.basis(), wrong_witness).unwrap();
        assert!(propose_on_defend(&wrong_witness, hero).unwrap().is_none());
        let mut wrong_source = current.state().clone();
        wrong_source.decisions.last_mut().unwrap().source_policy =
            model::label("wrong-escort-source").unwrap();
        let wrong_source = model::checkpoint(current.basis(), wrong_source).unwrap();
        assert!(propose_on_defend(&wrong_source, hero).is_err());
    }

    #[test]
    fn restored_checkpoint_keeps_one_witness_and_same_reaction_candidate() {
        let current = escorted();
        let restored = model::checkpoint(current.basis(), current.state().clone()).unwrap();
        assert_eq!(restored, current);
        let command = input(&restored, first_member(), 7, "defend-courier", vec![]);
        let reacted = stage_with_supplier(&restored, &command, &mut |_| Ok(10)).unwrap();
        assert_eq!(
            attitude(&reacted, entity(ENTITIES[0]).unwrap()),
            ESCORT_SUPPORTED
        );
        assert_eq!(reacted.state().continuity.witnesses.len(), 1);
        assert_eq!(reacted.state().continuity.npcs[0].known_facts.len(), 1);
    }
}

#[cfg(test)]
#[path = "courier_relationship_tests.rs"]
mod relationship_consumer_tests;
