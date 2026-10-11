//! Admission for the current authored Defend Courier candidate before initiative.
use df_encounter::participants::{
    ParticipantError, ParticipantLimits, ParticipantOwner, ParticipantStatus,
    ValidatedParticipants, validate_participants,
};
use df_model::checkpoint::{
    AudienceScope, Basis, Checkpoint, CommandInput, EntityId, FactValue, GameCommand, GameState,
    Position,
};
use df_session::submission::RepositoryError;

use super::{
    BANDIT, THREAD_POLICY, accepted, entity, model, offered, participants, phase, player_entity,
    room_entity, value,
};
use df_protocol::common as rpc;

struct CurrentJourney<'a> {
    current: &'a Checkpoint,
    command: &'a CommandInput,
    staged: &'a GameState,
}

impl ParticipantOwner for CurrentJourney<'_> {
    type Actor = EntityId;
    type Basis = Basis;
    type Error = RepositoryError;

    fn validate_basis(&self, proposal: &Basis, current: &Basis) -> Result<(), Self::Error> {
        if *proposal != *current || *current != self.current.basis() {
            return Err(RepositoryError::RevisionConflict);
        }
        self.current
            .validate_resume(*current, &model::pins()?)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        let GameCommand::ProposeAction {
            actor,
            action,
            targets,
            choices,
        } = &self.command.command
        else {
            return Err(RepositoryError::InvalidCandidate);
        };
        if self.command.observed_revision != current.revision
            || *actor != player_entity(self.command.member, self.current)?
            || *action != model::content("defend-courier")?
            || !targets.is_empty()
            || !choices.is_empty()
            || !offered(self.current, self.command.member)?
                .iter()
                .any(|(kind, _)| *kind == rpc::GameplayActionKind::DefendCourier)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let note = model::content("private-courier-note")?;
        let escort = model::content("escort-courier")?;
        let seal_beat = model::content("courier-answer-seal")?;
        let escort_beat = model::content("courier-answer-escort")?;
        let mut source = false;
        for fact in &self.current.state().facts {
            let FactValue::ContentEvent {
                definition,
                subjects,
            } = &fact.value
            else {
                continue;
            };
            let permitted = (*definition == note
                && matches!(&fact.audience, AudienceScope::Members(members)
                    if members.len() == 1 && members.first().is_some_and(|member|
                        participants(self.current).into_iter().any(|link|
                            link.member == *member && link.character.is_some())))
                && self
                    .current
                    .state()
                    .narrative
                    .active_beats
                    .contains(&seal_beat))
                || (*definition == escort
                    && fact.audience == AudienceScope::Shared
                    && self
                        .current
                        .state()
                        .narrative
                        .active_beats
                        .contains(&escort_beat));
            if !subjects.is_empty()
                || !permitted
                || !self
                    .current
                    .state()
                    .narrative
                    .accepted_facts
                    .contains(&fact.id)
            {
                continue;
            }
            if let Some(decision) = self.current.state().decisions.iter().find(|decision| {
                decision.operation == fact.operation
                    && decision.revision == fact.revision
                    && decision.source_policy.as_str() == THREAD_POLICY
                    && decision.facts.last() == Some(&fact.id)
            }) && accepted(decision)?.phase == rpc::JourneyPhase::Dialogue as i32
            {
                source = true;
            }
        }
        if !source {
            return Err(RepositoryError::InvalidCandidate);
        }
        if !self.current.state().encounters.is_empty()
            || phase(self.current)? != rpc::JourneyPhase::Dialogue
            || self.staged.members != self.current.state().members
            || self.staged.logical_time != self.current.state().logical_time
            || self.staged.entities.len() > 512
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    }

    fn participant_status(
        &self,
        current: &Basis,
        actor: &EntityId,
    ) -> Result<ParticipantStatus, Self::Error> {
        if *current != self.current.basis() {
            return Err(RepositoryError::RevisionConflict);
        }
        let Some(candidate) = self
            .staged
            .entities
            .iter()
            .find(|candidate| candidate.id == *actor)
        else {
            return Ok(ParticipantStatus::Unknown);
        };
        let monster = entity(BANDIT)?;
        // Existing joined character identities are already exposed by the shared party view.
        // The only new disclosed identity here is the pinned authored Defend Courier Bandit.
        if *actor == monster {
            if self
                .current
                .state()
                .entities
                .iter()
                .any(|entity| entity.id == monster)
                || candidate.definition != model::content("bandit")?
                || candidate.identity_revision != model::label("srd521-bandit-1")?
                || candidate.position != Some(Position { x: 0, y: 0, z: 0 })
            {
                return Ok(ParticipantStatus::Undisclosed);
            }
        } else {
            let Some(link) = participants(self.current)
                .into_iter()
                .find(|link| link.character == Some(*actor))
            else {
                return Ok(ParticipantStatus::Undisclosed);
            };
            let build = model::content("dwarf-fighter-soldier")?;
            if !self.current.state().characters.iter().any(|character| {
                character.entity == *actor
                    && character.owner == link.member
                    && character.build == build
            }) {
                return Ok(ParticipantStatus::Undisclosed);
            }
            if value(self.current.state(), *actor, "hit-points")? == 0
                || value(self.current.state(), *actor, "unconscious")? != 0
            {
                return Ok(ParticipantStatus::Unavailable);
            }
            if self
                .current
                .state()
                .entities
                .iter()
                .find(|entity| entity.id == *actor)
                != Some(candidate)
            {
                return Ok(ParticipantStatus::Unavailable);
            }
        }
        u8::try_from(value(self.staged, *actor, "dexterity")?)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        if candidate.location != Some(room_entity()?)
            || candidate.position.is_none()
            || value(self.staged, *actor, "hit-points")? == 0
            || value(self.staged, *actor, "unconscious")? != 0
        {
            return Ok(ParticipantStatus::Unavailable);
        }
        Ok(ParticipantStatus::Available)
    }
}

#[cfg(test)]
pub(super) fn validate(
    current: &Checkpoint,
    command: &CommandInput,
    staged: &GameState,
    actors: &[EntityId],
) -> Result<(), ParticipantError<RepositoryError>> {
    admission(current, command, staged, actors).map(|_| ())
}

/// Retain the canonical owner's exact borrowed admission for Engine composition.
pub(super) fn admission<'a>(
    current: &Checkpoint,
    command: &'a CommandInput,
    staged: &GameState,
    actors: &'a [EntityId],
) -> Result<ValidatedParticipants<'a, EntityId, Basis>, ParticipantError<RepositoryError>> {
    let owner = CurrentJourney {
        current,
        command,
        staged,
    };
    let admitted = validate_participants(
        &owner,
        &command.basis,
        &command.basis,
        actors,
        ParticipantLimits {
            max_participants: 3,
            max_identity_comparisons: 3,
        },
    )?;
    let expected = participants(current)
        .into_iter()
        .map(|link| {
            link.character
                .ok_or(ParticipantError::Owner(RepositoryError::InvalidCandidate))
        })
        .chain([entity(BANDIT).map_err(ParticipantError::Owner)])
        .collect::<Result<Vec<_>, _>>()?;
    if expected.len() != 3 || admitted.actors() != expected.as_slice() {
        return Err(ParticipantError::Owner(RepositoryError::InvalidCandidate));
    }
    Ok(admitted)
}

#[cfg(test)]
mod tests {
    use super::super::{
        ENTITIES, MEMBERS, stage_with_supplier,
        tests::{input, opening_story},
    };
    use super::*;
    use df_model::checkpoint::{ContentDigest, GameInput, ReferenceInventory};
    use df_types::MemberId;

    fn dialogue() -> Checkpoint {
        let opening = opening_story();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "ask-courier", vec![]),
            &mut |_| panic!("dialogue cannot roll"),
        )
        .unwrap()
    }
    fn proposal(current: &Checkpoint) -> CommandInput {
        let GameInput::Game(command) = input(
            current,
            MemberId::from_bytes(&MEMBERS[0]).unwrap(),
            9,
            "defend-courier",
            vec![],
        ) else {
            panic!("game input")
        };
        command
    }
    fn candidate(current: &Checkpoint) -> Checkpoint {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        stage_with_supplier(
            current,
            &input(current, first, 7, "defend-courier", vec![]),
            &mut |_| Ok(10),
        )
        .unwrap()
    }

    fn engine_order(
        current: &Checkpoint,
        input: &GameInput,
        staged: &GameState,
        admitted: &ValidatedParticipants<'_, EntityId, Basis>,
        draws: &[df_model::checkpoint::ActualDraw],
        resources: &[df_model::checkpoint::ResourceConstraint],
        pins: &df_model::checkpoint::CheckpointPins,
    ) -> Result<[EntityId; 3], df_engine::encounter_start::EncounterStartError> {
        let source = super::super::rule().unwrap();
        df_engine::encounter_start::local_journey_turn_order(
            df_rules::RulesCommandInput {
                command: input,
                supplied_draws: draws,
            },
            current,
            df_engine::command_entry::CommandEntryContext {
                current_basis: current.basis(),
                admitted_pins: pins,
                inventory: ReferenceInventory {
                    rules: &[model::rule().unwrap(), source.clone()],
                    content: &model::contents().unwrap(),
                    resources,
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
            admitted,
            staged,
            &source,
        )
    }

    #[test]
    fn canonical_engine_initiative_preserves_source_draws_and_admitted_identities() {
        let current = dialogue();
        let untouched = current.clone();
        let combat = candidate(&current);
        let staged_before = combat.state().clone();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let input = input(&current, member, 7, "defend-courier", vec![]);
        let GameInput::Game(command) = &input else {
            panic!("game input")
        };
        let actors = combat.state().encounters[0].participants.clone();
        let admitted = admission(&current, command, combat.state(), &actors).unwrap();
        assert_eq!(admitted.current_basis(), &command.basis);
        assert_eq!(admitted.actors().as_ptr(), actors.as_ptr());
        let draws = combat
            .state()
            .draws
            .iter()
            .filter(|draw| draw.operation == command.operation)
            .cloned()
            .collect::<Vec<_>>();
        let original_draws = draws.clone();
        let pins = model::pins().unwrap();
        let resources = super::super::resources().unwrap();
        assert_eq!(
            engine_order(
                &current,
                &input,
                combat.state(),
                &admitted,
                &draws,
                &resources,
                &pins
            ),
            Ok(actors.clone().try_into().unwrap())
        );
        assert_eq!(draws, original_draws);
        assert_eq!(combat.state(), &staged_before);
        assert_eq!(current, untouched);
        let mut faces = [8, 14, 4].into_iter();
        let mixed = stage_with_supplier(&current, &input, &mut |sides| {
            assert_eq!(sides, 20);
            Ok(faces
                .next()
                .expect("exactly three supplied initiative faces"))
        })
        .unwrap();
        assert_eq!(
            mixed.state().encounters[0].turn_order,
            vec![actors[1], actors[0], actors[2]]
        );
        assert_eq!(faces.next(), None);
        assert_eq!(current, untouched);
        println!(
            "F11_INITIATIVE_NATIVE_WITNESS source_revision={} draws={} ordered={:?} pins={pins:?}",
            df_rules::local_journey::SOURCE_REVISION,
            draws.len(),
            actors
        );
    }

    #[test]
    fn canonical_engine_initiative_refuses_stale_inputs_and_pin_context() {
        let current = dialogue();
        let combat = candidate(&current);
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let input = input(&current, member, 7, "defend-courier", vec![]);
        let GameInput::Game(command) = &input else {
            panic!("game input")
        };
        let actors = combat.state().encounters[0].participants.clone();
        let admitted = admission(&current, command, combat.state(), &actors).unwrap();
        let draws = combat
            .state()
            .draws
            .iter()
            .filter(|draw| draw.operation == command.operation)
            .cloned()
            .collect::<Vec<_>>();
        let pins = model::pins().unwrap();
        let resources = super::super::resources().unwrap();
        let before = current.clone();
        let mut stale_pins = pins.clone();
        stale_pins.rules.handler_digest = ContentDigest([99; 32]);
        assert!(
            engine_order(
                &current,
                &input,
                combat.state(),
                &admitted,
                &draws,
                &resources,
                &stale_pins
            )
            .is_err()
        );
        let mut stale_input = input.clone();
        let GameInput::Game(stale_command) = &mut stale_input else {
            panic!("game input")
        };
        stale_command.basis.revision = stale_command.basis.revision.next_sequence().unwrap();
        assert!(
            engine_order(
                &current,
                &stale_input,
                combat.state(),
                &admitted,
                &draws,
                &resources,
                &pins
            )
            .is_err()
        );
        assert_eq!(current, before);
    }

    #[test]
    fn canonical_engine_initiative_rejects_malformed_supplied_draw_envelopes_without_mutation() {
        use df_engine::encounter_start::EncounterStartError;
        use df_model::checkpoint::{ResolutionId, WindowId};
        let current = dialogue();
        let before = current.clone();
        let combat = candidate(&current);
        let staged_before = combat.state().clone();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let input = input(&current, member, 7, "defend-courier", vec![]);
        let GameInput::Game(command) = &input else {
            panic!("game input")
        };
        let actors = combat.state().encounters[0].participants.clone();
        let admitted = admission(&current, command, combat.state(), &actors).unwrap();
        let exact = combat
            .state()
            .draws
            .iter()
            .filter(|draw| draw.operation == command.operation)
            .cloned()
            .collect::<Vec<_>>();
        let pins = model::pins().unwrap();
        let resources = super::super::resources().unwrap();
        for case in 0..11 {
            let mut draws = exact.clone();
            match case {
                0 => {
                    draws.pop();
                }
                1 => draws.push(exact[2].clone()),
                2 => draws[0].operation = df_types::OperationId::from_bytes(&[0x88; 16]).unwrap(),
                3 => draws[0].ordinal += 1,
                4 => draws[0].resolution = ResolutionId::from_bytes(&[0x88; 16]).unwrap(),
                5 => draws[0].window = WindowId::from_bytes(&[0x88; 16]).unwrap(),
                6 => draws[0].sides = 6,
                7 => draws[0].value = 0,
                8 => draws[0].value = 21,
                9 => draws[0].source = model::rule().unwrap(),
                _ => draws.swap(0, 1),
            }
            let supplied_before = draws.clone();
            assert_eq!(
                engine_order(
                    &current,
                    &input,
                    combat.state(),
                    &admitted,
                    &draws,
                    &resources,
                    &pins
                ),
                Err(EncounterStartError::InvalidDraws),
                "draw case {case}"
            );
            assert_eq!(draws, supplied_before);
            assert_eq!(combat.state(), &staged_before);
            assert_eq!(current, before);
        }
    }

    #[test]
    fn canonical_engine_initiative_rejects_forged_missing_and_duplicate_dexterity_resources() {
        use df_engine::encounter_start::EncounterStartError;
        let current = dialogue();
        let before = current.clone();
        let combat = candidate(&current);
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let input = input(&current, member, 7, "defend-courier", vec![]);
        let GameInput::Game(command) = &input else {
            panic!("game input")
        };
        let actors = combat.state().encounters[0].participants.clone();
        let admitted = admission(&current, command, combat.state(), &actors).unwrap();
        let draws = combat
            .state()
            .draws
            .iter()
            .filter(|draw| draw.operation == command.operation)
            .cloned()
            .collect::<Vec<_>>();
        let pins = model::pins().unwrap();
        let exact_resources = super::super::resources().unwrap();
        for case in 0..11 {
            let mut staged = combat.state().clone();
            let mut resources = exact_resources.clone();
            let state_index = staged
                .resources
                .iter()
                .position(|record| {
                    record.owner == actors[0] && record.resource.as_str() == "dexterity"
                })
                .unwrap();
            let constraint_index = resources
                .iter()
                .position(|record| {
                    record.owner == actors[0] && record.resource.as_str() == "dexterity"
                })
                .unwrap();
            match case {
                0 => {
                    staged.resources.remove(state_index);
                }
                1 => staged.resources.push(staged.resources[state_index].clone()),
                2 => staged.resources[state_index].source = model::rule().unwrap(),
                3 => staged.resources[state_index].minimum = 0,
                4 => staged.resources[state_index].maximum = 21,
                5 => staged.resources[state_index].value = 0,
                6 => staged.resources[state_index].value = 21,
                7 => {
                    resources.remove(constraint_index);
                }
                8 => resources.push(resources[constraint_index].clone()),
                9 => resources[constraint_index].source = model::rule().unwrap(),
                _ => resources[constraint_index].maximum = 21,
            }
            let staged_before = staged.clone();
            let resources_before = resources.clone();
            assert_eq!(
                engine_order(
                    &current, &input, &staged, &admitted, &draws, &resources, &pins
                ),
                Err(EncounterStartError::InvalidDexterity),
                "resource case {case}"
            );
            assert_eq!(staged, staged_before);
            assert_eq!(resources, resources_before);
            assert_eq!(current, before);
        }
    }
    #[test]
    fn actual_combat_start_admits_exact_shared_party_and_authored_bandit_before_ordered_initiative()
    {
        let current = dialogue();
        let untouched = current.clone();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut calls = 0;
        let combat = stage_with_supplier(
            &current,
            &input(&current, first, 7, "defend-courier", vec![]),
            &mut |sides| {
                calls += 1;
                assert_eq!(sides, 20);
                Ok(10)
            },
        )
        .unwrap();
        let actors = vec![
            entity(ENTITIES[0]).unwrap(),
            entity(ENTITIES[1]).unwrap(),
            entity(BANDIT).unwrap(),
        ];
        assert_eq!(
            calls, 3,
            "unchanged three genuine initiative samples, no monster turn yet"
        );
        assert_eq!(combat.state().encounters[0].participants, actors);
        assert_eq!(combat.state().encounters[0].turn_order, actors);
        assert_eq!(
            combat
                .state()
                .draws
                .iter()
                .map(|draw| draw.ordinal)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(combat.state().logical_time, current.state().logical_time);
        assert_eq!(combat.state().knowledge, current.state().knowledge);
        assert_eq!(combat.state().intents, current.state().intents);
        assert_eq!(current, untouched);
    }
    #[test]
    fn actual_unavailable_party_refuses_before_any_draw_and_preserves_canonical_checkpoint() {
        let current = dialogue();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        for case in 0..4 {
            let mut state = current.state().clone();
            if case == 0 {
                state
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == super::entity(ENTITIES[1]).unwrap())
                    .unwrap()
                    .location = None;
            } else if case == 3 {
                state.resources.retain(|record| {
                    !(record.owner == super::entity(ENTITIES[1]).unwrap()
                        && record.resource.as_str() == "dexterity")
                });
            } else if case == 2 {
                state
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == super::entity(ENTITIES[1]).unwrap())
                    .unwrap()
                    .position = None;
            } else {
                state
                    .resources
                    .iter_mut()
                    .find(|record| {
                        record.owner == super::entity(ENTITIES[1]).unwrap()
                            && record.resource.as_str() == "unconscious"
                    })
                    .unwrap()
                    .value = 1;
            }
            let unavailable = model::checkpoint(current.basis(), state).unwrap();
            let before = unavailable.clone();
            let mut calls = 0;
            let result = stage_with_supplier(
                &unavailable,
                &input(&unavailable, first, 8, "defend-courier", vec![]),
                &mut |_| {
                    calls += 1;
                    Ok(10)
                },
            );
            assert_eq!(result, Err(RepositoryError::InvalidCandidate));
            assert_eq!(calls, 0);
            assert_eq!(unavailable, before);
        }
    }

    #[test]
    fn actual_start_rejects_duplicate_joined_alias_missing_link_and_world_present_unadmitted_actor_without_draws()
     {
        let current = dialogue();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        for case in 0..3 {
            let mut state = current.state().clone();
            state
                .characters
                .retain(|character| character.owner != second);
            let link = state
                .members
                .iter_mut()
                .find(|link| link.member == second)
                .unwrap();
            link.character = match case {
                0 => Some(entity(ENTITIES[0]).unwrap()),
                1 => None,
                _ => Some(room_entity().unwrap()),
            };
            let invalid = model::checkpoint(current.basis(), state).unwrap();
            let before = invalid.clone();
            let mut calls = 0;
            assert_eq!(
                stage_with_supplier(
                    &invalid,
                    &input(&invalid, first, 8, "defend-courier", vec![]),
                    &mut |_| {
                        calls += 1;
                        Ok(10)
                    }
                ),
                Err(RepositoryError::InvalidCandidate)
            );
            assert_eq!(calls, 0, "case {case} must fail before initiative");
            assert_eq!(
                invalid, before,
                "no threat, Bandit or encounter is published"
            );
        }
    }
    #[test]
    fn actual_start_requires_current_accepted_courier_source_not_just_staged_bandit_presence() {
        let current = dialogue();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        for case in 0..3 {
            let mut state = current.state().clone();
            let source = state.facts.iter().find(|fact| matches!(&fact.value,
                FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "private-courier-note")).unwrap();
            let id = source.id;
            let operation = source.operation;
            match case {
                0 => state.narrative.accepted_facts.retain(|fact| *fact != id),
                1 => {
                    state
                        .decisions
                        .iter_mut()
                        .find(|decision| decision.operation == operation)
                        .unwrap()
                        .source_policy = model::label("unadmitted-disclosure-policy").unwrap()
                }
                _ => {
                    let FactValue::ContentEvent { definition, .. } = &mut state
                        .facts
                        .iter_mut()
                        .find(|fact| fact.id == id)
                        .unwrap()
                        .value
                    else {
                        panic!("source")
                    };
                    *definition = model::content("ask-courier").unwrap();
                }
            }
            let invalid = model::checkpoint(current.basis(), state).unwrap();
            let before = invalid.clone();
            let mut calls = 0;
            assert_eq!(
                stage_with_supplier(
                    &invalid,
                    &input(&invalid, first, 8, "defend-courier", vec![]),
                    &mut |_| {
                        calls += 1;
                        Ok(10)
                    }
                ),
                Err(RepositoryError::InvalidCandidate)
            );
            assert_eq!(calls, 0);
            assert_eq!(invalid, before);
        }
    }

    #[test]
    fn actual_source_shape_rejects_subjects_and_bootstrap_or_unjoined_private_recipient_without_draws()
     {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        for case in 0..4 {
            let current = if case == 3 {
                let opening = opening_story();
                stage_with_supplier(
                    &opening,
                    &input(&opening, first, 6, "escort-courier", vec![]),
                    &mut |_| panic!("escort cannot roll"),
                )
                .unwrap()
            } else {
                dialogue()
            };
            let mut state = current.state().clone();
            let source = state
                .facts
                .iter_mut()
                .find(|fact| {
                    matches!(&fact.value,
                FactValue::ContentEvent { definition, .. } if definition.entry.as_str() ==
                    if case == 3 { "escort-courier" } else { "private-courier-note" })
                })
                .unwrap();
            if case == 0 || case == 3 {
                let FactValue::ContentEvent { subjects, .. } = &mut source.value else {
                    panic!("source event")
                };
                subjects.push(entity(ENTITIES[0]).unwrap());
            } else {
                source.audience = AudienceScope::Members(vec![if case == 1 {
                    super::super::bootstrap_member().unwrap()
                } else {
                    second
                }]);
            }
            if case == 2 {
                state
                    .characters
                    .retain(|character| character.owner != second);
                state
                    .members
                    .iter_mut()
                    .find(|link| link.member == second)
                    .unwrap()
                    .character = None;
            }
            let invalid = model::checkpoint(current.basis(), state).unwrap();
            let before = invalid.clone();
            let proposed = proposal(&invalid);
            let owner = CurrentJourney {
                current: &invalid,
                command: &proposed,
                staged: invalid.state(),
            };
            assert_eq!(
                owner.validate_basis(&proposed.basis, &invalid.basis()),
                Err(RepositoryError::InvalidCandidate),
                "exact source admission must reject case {case}"
            );
            let mut calls = 0;
            assert_eq!(
                stage_with_supplier(
                    &invalid,
                    &input(&invalid, first, 8, "defend-courier", vec![]),
                    &mut |_| {
                        calls += 1;
                        Ok(10)
                    }
                ),
                Err(RepositoryError::InvalidCandidate)
            );
            assert_eq!(calls, 0, "source case {case} cannot roll");
            assert_eq!(
                invalid, before,
                "no candidate or threat/Bandit state escapes"
            );
        }
    }

    #[test]
    fn actual_private_start_boundary_rejects_missing_world_actor_and_forged_basis_without_draws() {
        let current = dialogue();
        let command = proposal(&current);
        for case in 0..2 {
            let mut staged = current.state().clone();
            let mut proposed = command.clone();
            if case == 0 {
                staged
                    .entities
                    .retain(|entity| entity.id != super::entity(ENTITIES[1]).unwrap());
            } else {
                proposed.basis.run = df_types::RunId::from_bytes(&[99; 16]).unwrap();
            }
            let before = current.clone();
            let mut calls = 0;
            let mut draws = Vec::new();
            let mut outcomes = Vec::new();
            assert_eq!(
                super::super::start_combat(
                    &current,
                    &proposed,
                    &mut staged,
                    &mut draws,
                    &mut outcomes,
                    &mut |_| {
                        calls += 1;
                        Ok(10)
                    }
                ),
                Err(RepositoryError::InvalidCandidate)
            );
            assert_eq!(calls, 0);
            assert!(draws.is_empty() && outcomes.is_empty());
            assert_eq!(current, before);
        }
    }

    #[test]
    fn actual_stale_command_and_checkpoint_pins_refuse_before_initiative() {
        let current = dialogue();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut command = input(&current, first, 8, "defend-courier", vec![]);
        let GameInput::Game(ref mut command) = command else {
            panic!("game input")
        };
        command.basis.revision = command.basis.revision.next_sequence().unwrap();
        let mut calls = 0;
        assert!(
            stage_with_supplier(&current, &GameInput::Game(command.clone()), &mut |_| {
                calls += 1;
                Ok(10)
            })
            .is_err()
        );
        assert_eq!(calls, 0);
        let mut pins = current.pins().clone();
        pins.content.package_digest = ContentDigest([99; 32]);
        let stale = Checkpoint::new(
            current.schema(),
            current.basis(),
            pins,
            current.state().clone(),
            ReferenceInventory {
                rules: &[model::rule().unwrap(), super::super::rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &super::super::resources().unwrap(),
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        let before = stale.clone();
        assert!(
            stage_with_supplier(
                &stale,
                &input(&stale, first, 9, "defend-courier", vec![]),
                &mut |_| {
                    calls += 1;
                    Ok(10)
                }
            )
            .is_err()
        );
        assert_eq!(calls, 0);
        assert_eq!(stale, before);
    }
    #[test]
    fn exact_validator_rejects_duplicate_unknown_and_existing_undisclosed_actor_with_no_partial_result()
     {
        let current = dialogue();
        let combat = candidate(&current);
        let staged = combat.state();
        let actors = combat.state().encounters[0].participants.clone();
        assert_eq!(
            validate(&current, &proposal(&current), staged, &actors),
            Ok(())
        );
        let mut duplicate = actors.clone();
        duplicate[1] = duplicate[0];
        assert_eq!(
            validate(&current, &proposal(&current), staged, &duplicate),
            Err(ParticipantError::Duplicate {
                first: 0,
                duplicate: 1
            })
        );
        let mut unknown = actors.clone();
        unknown[1] = entity([0x99; 16]).unwrap();
        assert_eq!(
            validate(&current, &proposal(&current), staged, &unknown),
            Err(ParticipantError::Inadmissible {
                index: 1,
                status: ParticipantStatus::Unknown
            })
        );
        let mut undisclosed = actors.clone();
        undisclosed[1] = room_entity().unwrap();
        assert_eq!(
            validate(&current, &proposal(&current), staged, &undisclosed),
            Err(ParticipantError::Inadmissible {
                index: 1,
                status: ParticipantStatus::Undisclosed
            })
        );
        assert_eq!(
            current,
            dialogue(),
            "validator cannot mutate canonical source"
        );
    }
    #[test]
    fn exact_validator_rejects_stale_basis_capacity_omission_and_changed_identity_before_any_result()
     {
        let current = dialogue();
        let combat = candidate(&current);
        let actors = combat.state().encounters[0].participants.clone();
        let mut stale = proposal(&current);
        stale.basis.revision = stale.basis.revision.next_sequence().unwrap();
        assert_eq!(
            validate(&current, &stale, combat.state(), &actors),
            Err(ParticipantError::Owner(RepositoryError::RevisionConflict))
        );
        let mut crowded = actors.clone();
        crowded.push(actors[0]);
        assert_eq!(
            validate(&current, &proposal(&current), combat.state(), &crowded),
            Err(ParticipantError::ParticipantCapacity {
                required: 4,
                limit: 3
            })
        );
        assert_eq!(
            validate(&current, &proposal(&current), combat.state(), &actors[..2]),
            Err(ParticipantError::Owner(RepositoryError::InvalidCandidate))
        );
        let mut changed = combat.state().clone();
        changed
            .entities
            .iter_mut()
            .find(|entity| entity.id == actors[0])
            .unwrap()
            .identity_revision = model::label("unadmitted-identity").unwrap();
        assert_eq!(
            validate(&current, &proposal(&current), &changed, &actors),
            Err(ParticipantError::Inadmissible {
                index: 0,
                status: ParticipantStatus::Unavailable
            })
        );
        let before = combat.clone();
        assert_eq!(
            validate(&current, &proposal(&current), combat.state(), &actors),
            Ok(())
        );
        assert_eq!(combat, before);
    }

    const ENCOUNTER_ADMISSION_DECISION: &str = include_str!("encounter_admission_contract.json");

    #[test]
    fn encounter_design_admission_is_not_a_mechanical_commit() {
        let current = dialogue();
        let canonical = current.clone();
        let staged = candidate(&current);
        let mechanical = staged.clone();
        let actors = staged.state().encounters[0].participants.clone();
        assert_eq!(
            validate(&current, &proposal(&current), staged.state(), &actors),
            Ok(())
        );
        assert_eq!(current, canonical);
        assert_eq!(staged, mechanical);
        assert!(current.state().encounters.is_empty());
        assert_eq!(current.state().draws, canonical.state().draws);
        assert_eq!(current.state().facts, canonical.state().facts);
        assert_eq!(current.pins(), canonical.pins());
        assert_eq!(current.basis(), canonical.basis());
    }

    #[test]
    fn encounter_design_handler_requires_complete_source_qualified_initiative() {
        let current = dialogue();
        let canonical = current.clone();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let command = input(&current, member, 7, "defend-courier", vec![]);
        let staged = candidate(&current);
        let operation = super::super::command(&command).unwrap().operation;
        let exact = staged
            .state()
            .draws
            .iter()
            .filter(|draw| draw.operation == operation)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(exact.len(), 3);
        assert!(
            exact
                .iter()
                .all(|draw| draw.source == super::super::rule().unwrap())
        );
        let base = super::super::JourneyHandler {
            pins: model::pins().unwrap(),
        };
        let handler = super::super::courier_reaction::CourierJourneyHandler {
            command_handler: &base,
        };
        assert_eq!(
            df_rules::RulesCommandHandler::stage(
                &handler,
                df_rules::RulesCommandInput {
                    command: &command,
                    supplied_draws: &exact
                },
                &current
            )
            .unwrap(),
            staged
        );
        for case in 0..9 {
            let mut malformed = exact.clone();
            match case {
                0 => {
                    malformed.pop();
                }
                1 => malformed.push(exact[2].clone()),
                2 => malformed[0].ordinal += 1,
                3 => {
                    malformed[0].operation =
                        df_types::OperationId::from_bytes(&[0x88; 16]).unwrap();
                }
                4 => {
                    assert_ne!(model::rule().unwrap(), super::super::rule().unwrap());
                    malformed[0].source = model::rule().unwrap();
                }
                5 => malformed[0].sides = 6,
                6 => malformed[0].value = 0,
                7 => malformed[0].value = 21,
                _ => malformed.swap(0, 1),
            }
            assert_eq!(
                df_rules::RulesCommandHandler::stage(
                    &handler,
                    df_rules::RulesCommandInput {
                        command: &command,
                        supplied_draws: &malformed
                    },
                    &current
                ),
                Err(RepositoryError::InvalidCandidate),
                "actual source-qualified replay rejects malformed case {case}"
            );
            assert_eq!(current, canonical);
        }
    }

    #[test]
    fn encounter_design_sampler_failure_has_no_accepted_failure_exit() {
        let current = dialogue();
        let canonical = current.clone();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        for fail_at in [2, 3] {
            let command = input(&current, member, 7, "defend-courier", vec![]);
            let mut calls = 0;
            assert_eq!(
                stage_with_supplier(&current, &command, &mut |sides| {
                    assert_eq!(sides, 20);
                    calls += 1;
                    if calls == fail_at {
                        Err(RepositoryError::InvalidCandidate)
                    } else {
                        Ok(10)
                    }
                }),
                Err(RepositoryError::InvalidCandidate)
            );
            assert_eq!(
                calls, fail_at,
                "prior sampling is an actual performed side effect"
            );
            assert_eq!(current, canonical);
            assert!(current.state().encounters.is_empty());
            let retried = candidate(&current);
            assert_eq!(retried.state().encounters.len(), 1);
            assert_eq!(current, canonical);
        }
    }

    #[test]
    fn encounter_design_executed_contract_binds_current_source_pins() {
        use sha2::Digest;
        let current = dialogue();
        let canonical = current.clone();
        let staged = candidate(&current);
        let pins = model::pins().unwrap();
        assert_eq!(current.pins(), &pins);
        assert_eq!(staged.pins(), &pins);
        assert_eq!(staged.state().encounters[0].participants.len(), 3);
        assert_eq!(staged.state().draws.len() - current.state().draws.len(), 3);
        assert!(
            staged.state().draws[current.state().draws.len()..]
                .iter()
                .all(|draw| draw.source == super::super::rule().unwrap())
        );
        assert_eq!(current, canonical);
        assert!(!ENCOUNTER_ADMISSION_DECISION.is_empty());
        println!(
            "ENCOUNTER_D02_NATIVE_WITNESS fixture_sha256={:x} source_manifest_sha256={:x} pins={pins:?}",
            sha2::Sha256::digest(ENCOUNTER_ADMISSION_DECISION.as_bytes()),
            sha2::Sha256::digest(model::source_manifest())
        );
    }
}
