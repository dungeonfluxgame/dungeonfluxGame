use df_model::checkpoint::{
    AcceptedDecision, AudienceScope, Basis, Checkpoint, CommandInput, FactValue, GameCommand,
    GameFact, GameInput, GameState,
};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_types::{OperationId, SessionRevision};

use super::model;

pub(super) fn revision(value: SessionRevision) -> rpc::SessionRevision {
    rpc::SessionRevision {
        epoch: Some(rpc::RecoveryEpoch {
            value: Some(value.epoch().get()),
        }),
        sequence: Some(value.sequence()),
    }
}
pub(super) fn receipt(
    basis: Basis,
    operation: OperationId,
    outcome: rpc::decision_receipt::Outcome,
    replayed: bool,
) -> rpc::DecisionReceipt {
    rpc::DecisionReceipt {
        operation_id: Some(rpc::OperationId {
            value: Some(operation.as_bytes().to_vec()),
        }),
        revision: Some(revision(basis.revision)),
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        replayed,
        outcome: Some(outcome),
    }
}
pub(super) fn rejected(
    basis: Basis,
    operation: OperationId,
    code: rpc::RejectionCode,
) -> rpc::DecisionReceipt {
    receipt(
        basis,
        operation,
        rpc::decision_receipt::Outcome::Rejected(rpc::Rejection { code: code as i32 }),
        false,
    )
}
pub(super) fn committed(receipt: rpc::DecisionReceipt) -> rpc::SubmitActionResponse {
    rpc::SubmitActionResponse {
        outcome: Some(rpc::submit_action_response::Outcome::CommittedDecision(
            Box::new(receipt),
        )),
    }
}
pub(super) fn observation(code: rpc::RejectionCode) -> rpc::SubmitActionResponse {
    rpc::SubmitActionResponse {
        outcome: Some(rpc::submit_action_response::Outcome::OperationObservation(
            rpc::OperationObservation { code: code as i32 },
        )),
    }
}
pub(super) fn action_kind(entry: &str) -> Option<rpc::GameplayActionKind> {
    Some(match entry {
        "script-choice" => rpc::GameplayActionKind::ExamineHarborSeal,
        "resolve-errand" => rpc::GameplayActionKind::ResolveErrand,
        "start-new-adventure" => rpc::GameplayActionKind::StartNewAdventure,
        "create-character" => rpc::GameplayActionKind::CreateCharacter,
        "begin-story" => rpc::GameplayActionKind::BeginStory,
        "ask-courier" => rpc::GameplayActionKind::AskCourier,
        "escort-courier" => rpc::GameplayActionKind::EscortCourier,
        "defend-courier" => rpc::GameplayActionKind::DefendCourier,
        "greatsword-attack" => rpc::GameplayActionKind::GreatswordAttack,
        "second-wind" => rpc::GameplayActionKind::SecondWind,
        "end-turn" => rpc::GameplayActionKind::EndTurn,
        "move" => rpc::GameplayActionKind::Move,
        "dash" => rpc::GameplayActionKind::Dash,
        "dodge" => rpc::GameplayActionKind::Dodge,
        "disengage" => rpc::GameplayActionKind::Disengage,
        "opportunity-attack" => rpc::GameplayActionKind::OpportunityAttack,
        "decline-reaction" => rpc::GameplayActionKind::DeclineReaction,
        "stand" => rpc::GameplayActionKind::Stand,
        "pick-up-weapon" => rpc::GameplayActionKind::PickUpWeapon,
        "short-rest" => rpc::GameplayActionKind::ShortRest,
        "choose-harbor-scene" => rpc::GameplayActionKind::ChooseHarborScene,
        _ => return None,
    })
}
pub(super) fn action_entry(kind: rpc::GameplayActionKind) -> Option<&'static str> {
    Some(match kind {
        rpc::GameplayActionKind::ExamineHarborSeal => "script-choice",
        rpc::GameplayActionKind::ResolveErrand => "resolve-errand",
        rpc::GameplayActionKind::StartNewAdventure => "start-new-adventure",
        rpc::GameplayActionKind::CreateCharacter => "create-character",
        rpc::GameplayActionKind::BeginStory => "begin-story",
        rpc::GameplayActionKind::AskCourier => "ask-courier",
        rpc::GameplayActionKind::EscortCourier => "escort-courier",
        rpc::GameplayActionKind::DefendCourier => "defend-courier",
        rpc::GameplayActionKind::GreatswordAttack => "greatsword-attack",
        rpc::GameplayActionKind::SecondWind => "second-wind",
        rpc::GameplayActionKind::EndTurn => "end-turn",
        rpc::GameplayActionKind::Move => "move",
        rpc::GameplayActionKind::Dash => "dash",
        rpc::GameplayActionKind::Dodge => "dodge",
        rpc::GameplayActionKind::Disengage => "disengage",
        rpc::GameplayActionKind::OpportunityAttack => "opportunity-attack",
        rpc::GameplayActionKind::DeclineReaction => "decline-reaction",
        rpc::GameplayActionKind::Stand => "stand",
        rpc::GameplayActionKind::PickUpWeapon => "pick-up-weapon",
        rpc::GameplayActionKind::ShortRest => "short-rest",
        rpc::GameplayActionKind::ChooseHarborScene => "choose-harbor-scene",
        _ => return None,
    })
}
pub(super) fn unrelated_payload(
    request: &rpc::SubmitActionRequest,
    kind: rpc::GameplayActionKind,
) -> bool {
    (if kind == rpc::GameplayActionKind::ChooseHarborScene {
        request.destination != rpc::HarborDestination::HarborInn as i32
    } else {
        request.destination != rpc::HarborDestination::Unspecified as i32
    }) || (kind != rpc::GameplayActionKind::CreateCharacter && request.character.is_some())
        || (kind != rpc::GameplayActionKind::GreatswordAttack
            && (request.savage_attacker || request.graze))
}
pub(super) fn journey_input(
    request: &rpc::SubmitActionRequest,
    member: df_types::MemberId,
    current: &Checkpoint,
) -> Result<GameInput, tonic::Status> {
    let invalid = || tonic::Status::invalid_argument("required bounded action fields invalid");
    let session = df_api::session_id(request.session_id.as_ref()).map_err(|_| invalid())?;
    let run = df_api::run_id(request.run_id.as_ref()).map_err(|_| invalid())?;
    let observed =
        df_api::session_revision(request.observed_revision.as_ref()).map_err(|_| invalid())?;
    let operation = df_api::operation_id(request.operation_id.as_ref()).map_err(|_| invalid())?;
    let kind = rpc::GameplayActionKind::try_from(request.action_kind)
        .map_err(|_| tonic::Status::invalid_argument("unknown action kind"))?;
    let entry =
        action_entry(kind).ok_or_else(|| tonic::Status::invalid_argument("unsupported action"))?;
    if request.offer_id.len() > 96 {
        return Err(tonic::Status::invalid_argument("offer exceeds bound"));
    }
    // A fight action carries at most its choice from the list and the two attack variants
    // (weapon mastery, Savage Attacker).
    let observation_offer = kind == rpc::GameplayActionKind::ExamineHarborSeal
        && super::beat_script_runtime::is_observation_offer(current, &request.offer_id)
            .map_err(|_| invalid())?;
    if request.action_choices.len() > 3
        || (!super::journey::tactical::present(current.state())
            && !observation_offer
            && !request.action_choices.is_empty())
    {
        return Err(invalid());
    }
    let mut choices = request
        .action_choices
        .iter()
        .map(|choice| {
            Ok((
                model::label(&choice.group_id).map_err(|_| invalid())?,
                model::label(&choice.option_id).map_err(|_| invalid())?,
            ))
        })
        .collect::<Result<Vec<_>, tonic::Status>>()?;
    if let Some(id) = super::beat_script_runtime::submitted_id(&request.offer_id) {
        choices.push((
            model::label(super::beat_script_runtime::CHOICE_GROUP).map_err(|_| invalid())?,
            model::label(id).map_err(|_| invalid())?,
        ));
    }
    if kind == rpc::GameplayActionKind::ChooseHarborScene {
        let selected = match rpc::HarborDestination::try_from(request.destination) {
            Ok(rpc::HarborDestination::HarborInn) => "harbor-inn",
            Ok(rpc::HarborDestination::LoadingPier) => "loading-pier",
            Ok(rpc::HarborDestination::LanternWharf) => "lantern-wharf",
            _ => "invalid-destination",
        };
        choices.push((
            model::label("harbor-destination").map_err(|_| invalid())?,
            model::label(selected).map_err(|_| invalid())?,
        ));
    }
    if let Some(character) = &request.character {
        if character.name.len() > 48 || character.choices.len() > 16 {
            return Err(tonic::Status::invalid_argument(
                "character choices exceed bound",
            ));
        }
        let encoded_name = if character.name.is_empty() {
            "empty-name".to_owned()
        } else {
            super::journey::hex(character.name.as_bytes())
        };
        choices.push((
            model::label("name").map_err(|_| invalid())?,
            model::label(&encoded_name).map_err(|_| invalid())?,
        ));
        for selection in &character.choices {
            if selection.group_id.len() > 64 || selection.option_id.len() > 64 {
                return Err(tonic::Status::invalid_argument(
                    "character option exceeds bound",
                ));
            }
            choices.push((
                model::label(&selection.group_id).map_err(|_| invalid())?,
                model::label(&selection.option_id).map_err(|_| invalid())?,
            ));
        }
    }
    if super::journey::tactical::present(current.state())
        && (request.savage_attacker || request.graze)
    {
        return Err(invalid());
    }
    if kind == rpc::GameplayActionKind::GreatswordAttack
        && !super::journey::tactical::present(current.state())
    {
        for (key, value) in [
            ("savage-attacker", request.savage_attacker),
            ("graze", request.graze),
        ] {
            choices.push((
                model::label(key).map_err(|_| invalid())?,
                model::label(if value { "yes" } else { "no" }).map_err(|_| invalid())?,
            ));
        }
    }
    Ok(GameInput::Game(CommandInput {
        basis: Basis {
            session,
            run,
            revision: observed,
        },
        operation,
        member,
        observed_revision: observed,
        command: GameCommand::ProposeAction {
            actor: super::journey::player_entity(member, current)
                .map_err(|_| tonic::Status::permission_denied("character binding unavailable"))?,
            action: model::content(entry).map_err(|_| invalid())?,
            targets: Vec::new(),
            choices,
        },
    }))
}
fn attack_group(id: &str, label: &str, description: &str) -> rpc::JourneyGroup {
    rpc::JourneyGroup {
        group_id: id.to_owned(),
        label: label.to_owned(),
        options: vec![
            rpc::JourneyOption {
                option_id: "no".to_owned(),
                label: "Do not use".to_owned(),
                description: String::new(),
                ..Default::default()
            },
            rpc::JourneyOption {
                option_id: "yes".to_owned(),
                label: "Use".to_owned(),
                description: description.to_owned(),
                ..Default::default()
            },
        ],
    }
}
fn journey_tactical_creation() -> bool {
    super::journey::tactical::enabled()
}
pub(super) fn creation_offer() -> Result<rpc::CharacterCreationOffer, RepositoryError> {
    use super::pack;
    pack::with_content(|content| {
        let tactical = journey_tactical_creation();
        let mut groups = Vec::new();
        for group in &content.creation.groups {
            let options = group
                .options
                .iter()
                .map(|option| rpc::JourneyOption {
                    option_id: option.id.clone(),
                    label: option.label.clone(),
                    description: match &group.description_template {
                        Some(template) => pack::with_creation(|rules| {
                            let array = rules
                                .ability_arrays
                                .iter()
                                .find(|array| array.id == option.id)
                                .expect("validated creation array reference");
                            pack::render_array_description(
                                template,
                                array.scores,
                                pack::BackgroundBonus {
                                    strength: rules.background_bonus.strength,
                                    constitution: rules.background_bonus.constitution,
                                },
                            )
                        }),
                        None => option.description.clone().unwrap_or_default(),
                    },
                    ..Default::default()
                })
                .collect();
            if !tactical || !["array", "equipment", "style-masteries"].contains(&group.id.as_str())
            {
                groups.push(rpc::JourneyGroup {
                    group_id: group.id.clone(),
                    label: group.label.clone(),
                    options,
                });
            }
            if group.id == "array" {
                let lists = &content.creation.pick_lists;
                for (id, label, values) in [
                    ("alignment", "Alignment", &lists.alignment),
                    ("language-1", "First additional language", &lists.language),
                    ("language-2", "Second additional language", &lists.language),
                    ("skill-1", "First Fighter skill", &lists.skill),
                    ("skill-2", "Second Fighter skill", &lists.skill),
                    ("gaming-set", "Gaming set proficiency", &lists.gaming_set),
                ] {
                    groups.push(rpc::JourneyGroup {
                        group_id: id.to_owned(),
                        label: label.to_owned(),
                        options: values
                            .iter()
                            .map(|value| rpc::JourneyOption {
                                option_id: value.clone(),
                                label: value.clone(),
                                description: String::new(),
                                ..Default::default()
                            })
                            .collect(),
                    });
                }
            }
        }
        if tactical {
            let packages = pack::build_packages().map_err(|_| RepositoryError::InvalidCandidate)?;
            let package_ids: Vec<String> =
                packages.iter().map(|package| package.id.clone()).collect();
            {
                groups.push(rpc::JourneyGroup {
                    group_id: "build-package".to_owned(),
                    label: "Starting package".to_owned(),
                    options: packages
                        .into_iter()
                        .map(|package| rpc::JourneyOption {
                            description: package.abilities_text(),
                            option_id: package.id,
                            label: package.label,
                            ..Default::default()
                        })
                        .collect(),
                });
            }
            groups.push(rpc::JourneyGroup {
                group_id: "portrait".to_owned(),
                label: "How your hero looks".to_owned(),
                options: [
                    ("portrait-hero-1", "Greatsword hero"),
                    ("portrait-hero-2", "Second hero"),
                    ("portrait-hero-3", "Woman with a bow"),
                ]
                .into_iter()
                .map(|(id, label)| rpc::JourneyOption {
                    option_id: id.to_owned(),
                    label: label.to_owned(),
                    description: String::new(),
                    // An appearance is offered only with the packages it may be chosen with.
                    requires_group: "build-package".to_owned(),
                    requires_options: package_ids
                        .iter()
                        .filter(|package| pack::appearance_fits_package(id, package))
                        .cloned()
                        .collect(),
                })
                .collect(),
            });
        }
        Ok(rpc::CharacterCreationOffer {
            description: content.creation.description.clone(),
            groups,
            source_revision: model::source_revision(),
        })
    })
}

fn party_member(
    state: &df_model::checkpoint::GameState,
    who: df_model::checkpoint::EntityId,
    ready: bool,
) -> Result<rpc::PartyMemberView, RepositoryError> {
    use super::journey;
    let active = state
        .encounters
        .first()
        .and_then(|encounter| encounter.active_turn)
        == Some(who);
    let mut view = rpc::PartyMemberView {
        name: if ready {
            journey::name(state, who)?
        } else {
            "Creating a hero…".to_owned()
        },
        character_ready: ready,
        hit_points: if ready {
            journey::value(state, who, "hit-points")?
        } else {
            0
        },
        maximum_hit_points: if who.as_bytes() == &[0x65; 16] {
            u32::from(super::pack::bandit().hit_points)
        } else if ready {
            journey::maximum_hit_points(state, who)?
        } else {
            0
        },
        active_turn: active,
        unconscious: ready && journey::value(state, who, "unconscious")? != 0,
        participant_id: who.as_bytes().to_vec(),

        ..Default::default()
    };
    if ready
        && let Some(character) = state
            .characters
            .iter()
            .find(|character| character.entity == who)
        && let Some(portrait) = character
            .choices
            .iter()
            .find(|choice| choice.offer.as_str() == "portrait")
    {
        view.portrait = portrait.selected.as_str().to_owned();
    }
    if journey::tactical::present(state) && ready {
        journey::tactical::project_member(state, who, &mut view)?;
    }
    Ok(view)
}

fn defeat_context(participants: &[rpc::PartyMemberView]) -> Option<String> {
    let mut enemies = std::collections::BTreeMap::<&str, usize>::new();
    for participant in participants.iter().filter(|member| member.side == 2) {
        let kind = participant
            .name
            .rsplit_once(' ')
            .filter(|(_, index)| index.parse::<usize>().is_ok())
            .map_or(participant.name.as_str(), |(kind, _)| kind);
        *enemies.entry(kind).or_default() += 1;
    }
    if enemies.is_empty() {
        return None;
    }

    let enemy_groups = enemies
        .into_iter()
        .map(|(kind, count)| counted_creatures(kind, count))
        .collect::<Vec<_>>();
    let enemy_description = sentence_case(&join_names(&enemy_groups));
    let fallen = participants
        .iter()
        .filter(|member| member.side == 1 && member.unconscious)
        .map(|member| member.name.as_str())
        .collect::<Vec<_>>();
    Some(if fallen.is_empty() {
        format!("{enemy_description} overwhelmed the party.")
    } else {
        format!(
            "{enemy_description} overwhelmed the party. {} fell.",
            join_names(&fallen)
        )
    })
}

/// "three giant weasels", "a bandit": a creature kind and how many of it, in words.
fn counted_creatures(kind: &str, count: usize) -> String {
    let noun = kind.to_lowercase();
    if count == 1 {
        let article = if noun.starts_with(['a', 'e', 'i', 'o', 'u']) {
            "an"
        } else {
            "a"
        };
        return format!("{article} {noun}");
    }
    let number = match count {
        2 => "two".to_owned(),
        3 => "three".to_owned(),
        4 => "four".to_owned(),
        5 => "five".to_owned(),
        6 => "six".to_owned(),
        more => more.to_string(),
    };
    let plural = match noun.strip_suffix("wolf") {
        Some(stem) => format!("{stem}wolves"),
        None => format!("{noun}s"),
    };
    format!("{number} {plural}")
}

fn sentence_case(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

fn join_names<T: AsRef<str>>(names: &[T]) -> String {
    match names {
        [] => String::new(),
        [only] => only.as_ref().to_owned(),
        [first, second] => format!("{} and {}", first.as_ref(), second.as_ref()),
        _ => {
            let all_but_last = names[..names.len() - 1]
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{all_but_last} and {}", names[names.len() - 1].as_ref())
        }
    }
}

fn shared_source_decision(
    state: &GameState,
    selected: &[&GameFact],
    source: &GameFact,
    terminal_entry: &str,
    phase: rpc::JourneyPhase,
) -> Result<(), RepositoryError> {
    let mut owners = state.decisions.iter().filter(|decision| {
        decision.operation == source.operation
            && decision.revision == source.revision
            && decision.source_policy.as_str() == super::journey::THREAD_POLICY
            && decision.facts.get(source.ordinal as usize) == Some(&source.id)
    });
    let owner: &AcceptedDecision = owners
        .next()
        .ok_or_else(|| refused("view-source-decision-1"))?;
    if owners.next().is_some() || super::journey::accepted(owner)?.phase != phase as i32 {
        return Err(refused("view-source-decision-2"));
    }
    let terminal = selected
        .iter()
        .copied()
        .find(|fact| Some(&fact.id) == owner.facts.last())
        .ok_or_else(|| refused("view-source-decision-3"))?;
    let preceding = match super::beat_script_runtime::choice(state, owner)? {
        Some((marker, _, _)) if marker.ordinal > source.ordinal => {
            if source.ordinal.checked_add(1) != Some(marker.ordinal)
                || marker.cause != Some(source.id)
                || marker.ordinal.checked_add(1) != Some(terminal.ordinal)
            {
                return Err(refused("view-source-decision-4"));
            }
            marker
        }
        _ => source,
    };
    if source.audience != AudienceScope::Shared
        || terminal.audience != AudienceScope::Shared
        || preceding.ordinal.checked_add(1) != Some(terminal.ordinal)
        || terminal.cause != Some(preceding.id)
        || !matches!(&terminal.value, FactValue::ContentEvent { definition, subjects }
            if *definition == model::content(terminal_entry)? && subjects.is_empty())
    {
        return Err(refused("view-source-decision-5"));
    }
    Ok(())
}

fn witnessed_courier_support(
    current: &Checkpoint,
    audience_observer: df_knowledge::perception::ObserverScope,
    selected: &[&GameFact],
    party: &[rpc::PartyMemberView],
) -> Result<Option<String>, RepositoryError> {
    let state = current.state();
    let reaction_definition = model::content("courier-escort-reaction")?;
    let mut reactions = selected.iter().copied().filter(|fact| {
        matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if *definition == reaction_definition)
    });
    let Some(reaction) = reactions.next() else {
        return Ok(None);
    };
    if reactions.next().is_some() {
        return Err(refused("view-courier-witness-1"));
    }
    let courier = super::journey::entity([0x67; 16])?;
    let FactValue::ContentEvent { subjects, .. } = &reaction.value else {
        return Err(refused("view-courier-witness-2"));
    };
    let [observer, hero] = subjects.as_slice() else {
        return Err(refused("view-courier-witness-3"));
    };
    if *observer != courier {
        return Err(refused("view-courier-witness-4"));
    }
    let contact = selected
        .iter()
        .copied()
        .find(|fact| Some(fact.id) == reaction.cause)
        .ok_or_else(|| refused("view-courier-witness-5"))?;
    if !matches!(&contact.value, FactValue::ContentEvent { definition, subjects }
        if *definition == model::content("courier-escort-contact")? && subjects.as_slice() == [*hero])
    {
        return Err(refused("view-courier-witness-6"));
    }
    shared_source_decision(
        state,
        selected,
        contact,
        "escort-courier",
        rpc::JourneyPhase::Dialogue,
    )?;
    shared_source_decision(
        state,
        selected,
        reaction,
        "defend-courier",
        rpc::JourneyPhase::Combat,
    )?;
    let witness_source = model::content("courier-escort-perception")?;
    let expected_witness = super::journey::courier_reaction::witness_id(contact.id)?;
    let (audience_basis, witness, witnessed) =
        df_knowledge::witness_projection::perceive_witnessed_fact(
            current,
            current.basis(),
            &model::pins()?,
            audience_observer,
            expected_witness,
            df_knowledge::perception::PerceptionLimits {
                maximum_scan_records: model::limits().maximum_records,
                maximum_member_comparisons: model::limits().maximum_records,
                maximum_selected_facts: model::limits().maximum_records,
            },
        )
        .map_err(|error| match error {
            df_knowledge::perception::PerceptionError::ObserverUnavailable => {
                RepositoryError::Unauthorized
            }
            df_knowledge::perception::PerceptionError::ScanCapacity
            | df_knowledge::perception::PerceptionError::ComparisonCapacity
            | df_knowledge::perception::PerceptionError::ResultCapacity
            | df_knowledge::perception::PerceptionError::AllocationCapacity => {
                RepositoryError::Capacity
            }
            _ => refused("view-courier-witness-7"),
        })?
        .ok_or_else(|| refused("view-courier-witness-8"))?;
    if audience_basis != current.basis()
        || witness.observer != courier
        || witnessed.id != contact.id
        || witness.source != witness_source
    {
        return Err(refused("view-courier-witness-9"));
    }
    let relationship_policy = model::content("courier-escort-relationship")?;
    let courier_definition = model::content("lantern-wharf-courier")?;
    let courier_revision = model::label("lantern-wharf-courier-1")?;
    let personality = model::content("cautious-courier")?;
    let motivation = model::content("deliver-dispatch")?;
    if !state.entities.iter().any(|entity| {
        entity.id == courier
            && entity.definition == courier_definition
            && entity.identity_revision == courier_revision
            && entity.location.is_none()
            && entity.position.is_none()
    }) || !state.continuity.npcs.iter().any(|npc| {
        npc.entity == courier
            && npc.personality == personality
            && npc.motivations.as_slice() == std::slice::from_ref(&motivation)
            && npc.known_facts.contains(&contact.id)
    }) || !state.relationships.iter().any(|relationship| {
        relationship.subject == courier
            && relationship.object == *hero
            && relationship.policy == relationship_policy
            && relationship.state.as_str() == "escort-supported"
    }) {
        return Err(refused("view-courier-witness-10"));
    }
    let member = party
        .iter()
        .find(|member| {
            member.character_ready && member.participant_id.as_slice() == hero.as_bytes()
        })
        .ok_or_else(|| refused("view-courier-witness-11"))?;
    Ok(Some(member.name.clone()))
}

/// The characters on stage in a scene besides the heroes. `speaking` marks the one whose line
/// is the scene's current speaker line.
pub(super) fn scene_cast(scene: &super::pack::Scene) -> Vec<rpc::SceneActor> {
    use super::pack::CastRole;
    scene
        .cast
        .iter()
        .map(|actor| rpc::SceneActor {
            id: actor.id.clone(),
            name: actor.name.clone(),
            role: match actor.role {
                CastRole::Patron => rpc::SceneRole::Patron,
                CastRole::Antagonist => rpc::SceneRole::Antagonist,
                CastRole::Contact => rpc::SceneRole::Contact,
                CastRole::Bystander => rpc::SceneRole::Bystander,
            } as i32,
            sprite: actor.sprite.clone(),
            portrait: actor.portrait.clone(),
            speaking: !scene.dialogue.is_empty() && actor.name == scene.speaker,
            live_sprite: String::new(),
        })
        .collect()
}
/// Typed semantics of the public scene selected by this same audience projection.
/// These contain no private clue or mechanical statistics; editable prose is not a contract.
pub(super) struct PublicJourneyProjection {
    pub(super) view: rpc::ViewMessage,
    pub(super) scene: super::pack::SceneKey,
    pub(super) rested: bool,
}

pub(super) fn public_scene_asset(current: &Checkpoint) -> Result<String, RepositoryError> {
    let state = current.state();
    if stage(
        "view-native-inn",
        super::beat_script_runtime::native_inn(state),
    )? {
        Ok(super::journey::inn_scene().scene_asset)
    } else {
        Ok(stage("view-scene", super::beat_script_runtime::scene(state))?.asset)
    }
}

/// Notes which stage of the view projection refused, so the server's `UNAVAILABLE` line names
/// it after `step=`. The first note of a call wins, so a stage that names its own refusal
/// inside is not overwritten by the stage that called it.
fn stage<T>(step: &'static str, result: Result<T, RepositoryError>) -> Result<T, RepositoryError> {
    result.inspect_err(|_| super::refusal_detail::note(step))
}

/// An `InvalidCandidate` refusal that names its step for the log.
fn refused(step: &'static str) -> RepositoryError {
    super::refusal_detail::note(step);
    RepositoryError::InvalidCandidate
}

fn rested_text(
    authored: &super::pack::Scene,
    rested: bool,
) -> Result<(String, Option<String>), RepositoryError> {
    if rested {
        let (story, rules) = super::pack::rested_story_and_rules(authored)
            .map_err(|_| refused("rested-story-shape"))?;
        Ok((story, Some(rules)))
    } else {
        Ok((authored.narration.clone(), None))
    }
}

fn project_scene_rules_note(
    authored: &str,
    rested_note: Option<&str>,
    epilogue_replaces: bool,
) -> String {
    if epilogue_replaces && rested_note.is_none() {
        String::new()
    } else {
        let mut note = authored.to_owned();
        if let Some(rested_note) = rested_note {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(rested_note);
        }
        note
    }
}

pub(super) fn journey_view(
    current: &Checkpoint,
    role: df_persistence::local_demo_scope::LocalDemoRole,
    principal: df_types::MemberId,
) -> Result<rpc::ViewMessage, RepositoryError> {
    project_journey(current, role, principal)
        .inspect_err(|_| super::refusal_detail::note("view-projection"))
        .map(|projection| projection.view)
}

pub(super) fn attach_notebook(
    view: &mut rpc::ViewMessage,
    entries: &[super::notebook::EntryView],
) -> Result<(), &'static str> {
    let Some(audience) = view.audience.as_ref() else {
        return Err("notebook audience absent");
    };
    let standing = match audience {
        rpc::view_message::Audience::Display(_) => Vec::new(),
        rpc::view_message::Audience::Player(player) => {
            let Some(journey) = player.journey.as_ref() else {
                return Err("notebook journey absent");
            };
            journey.standing.clone()
        }
    };
    let Some(audience) = view.audience.as_mut() else {
        return Err("notebook audience absent");
    };
    let journey = match audience {
        rpc::view_message::Audience::Display(display) => {
            display.journey.as_mut().ok_or("notebook journey absent")?
        }
        rpc::view_message::Audience::Player(player) => {
            player.journey.as_mut().ok_or("notebook journey absent")?
        }
    };
    super::notebook::validate_view_entries(entries).map_err(|_| "notebook view invalid")?;
    journey.notebook = entries
        .iter()
        .map(|entry| {
            let attitude = (entry.kind == super::notebook::Kind::Person)
                .then(|| {
                    standing
                        .iter()
                        .find(|item| item.name.eq_ignore_ascii_case(&entry.title))
                        .and_then(|item| match item.attitude {
                            1 => Some("hostile"),
                            2 => Some("indifferent"),
                            3 => Some("friendly"),
                            _ => None,
                        })
                        .map(str::to_owned)
                })
                .flatten();
            let kind = match entry.kind {
                super::notebook::Kind::Person => rpc::NotebookKind::Person,
                super::notebook::Kind::Place => rpc::NotebookKind::Place,
                super::notebook::Kind::Task => rpc::NotebookKind::Task,
                super::notebook::Kind::Clue => rpc::NotebookKind::Clue,
                super::notebook::Kind::Note => rpc::NotebookKind::Note,
            };
            rpc::NotebookEntry {
                kind: kind as i32,
                title: entry.title.clone(),
                text: entry.text.clone(),
                scene: entry.scene.clone(),
                order: entry.order,
                owner: entry.owner.clone(),
                author: entry.author.clone(),
                attitude,
            }
        })
        .collect();
    Ok(())
}

/// What the players can choose right now, as labels only, for the shared display: every ready
/// hero's current offers merged in offer order without repeats. It carries no offer id and
/// nothing private. A fight's turn belongs to the one hero acting, so it is left empty there.
/// In a conversation the one choice is the public next step of the story: the members' offers
/// there also depend on a private note's audience, which the shared display must not reflect.
fn table_choices(
    current: &Checkpoint,
    phase: rpc::JourneyPhase,
) -> Result<Vec<String>, RepositoryError> {
    match phase {
        rpc::JourneyPhase::Combat => return Ok(Vec::new()),
        rpc::JourneyPhase::Dialogue => {
            if let Some(labels) = super::beat_script_runtime::dialogue_labels(current)? {
                return Ok(labels);
            }
            return Ok(vec![super::pack::offer_label(
                rpc::GameplayActionKind::DefendCourier,
            )]);
        }
        _ => {}
    }
    let state = current.state();
    let mut labels: Vec<String> = Vec::new();
    for link in super::journey::participants(current) {
        let Some(hero) = link.character else {
            continue;
        };
        if !state
            .characters
            .iter()
            .any(|character| character.entity == hero)
        {
            continue;
        }
        // The phones read the same labels: a scripted chapter's authored text, never a handler's.
        for (kind, label, _) in super::beat_script_runtime::view_offers(current, link.member)? {
            if kind != rpc::GameplayActionKind::CreateCharacter && !labels.contains(&label) {
                labels.push(label);
            }
        }
    }
    Ok(labels)
}

pub(super) fn project_journey(
    current: &Checkpoint,
    role: df_persistence::local_demo_scope::LocalDemoRole,
    principal: df_types::MemberId,
) -> Result<PublicJourneyProjection, RepositoryError> {
    use super::journey;
    use df_persistence::local_demo_scope::LocalDemoRole;
    let observer = match role {
        LocalDemoRole::Display => df_knowledge::perception::ObserverScope::Shared,
        LocalDemoRole::Player => df_knowledge::perception::ObserverScope::Member(principal),
    };
    let perceived = df_knowledge::perception::perceive(
        current,
        current.basis(),
        &model::pins()?,
        observer,
        // The admitted local checkpoint caps aggregate records, including
        // membership and nested audience entries, at the same bounded model limit.
        df_knowledge::perception::PerceptionLimits {
            maximum_scan_records: model::limits().maximum_records,
            maximum_member_comparisons: model::limits().maximum_records,
            maximum_selected_facts: model::limits().maximum_records,
        },
    )
    .map_err(|error| match error {
        df_knowledge::perception::PerceptionError::ObserverUnavailable => {
            RepositoryError::Unauthorized
        }
        df_knowledge::perception::PerceptionError::ScanCapacity
        | df_knowledge::perception::PerceptionError::ComparisonCapacity
        | df_knowledge::perception::PerceptionError::ResultCapacity
        | df_knowledge::perception::PerceptionError::AllocationCapacity => {
            RepositoryError::Capacity
        }
        _ => refused("view-perceive"),
    })?;
    let state = current.state();
    let phase = stage("view-phase", journey::phase(current))?;
    let mut party = Vec::new();
    for member in journey::participants(current) {
        let who = member
            .character
            .ok_or_else(|| refused("view-party-character"))?;
        party.push(party_member(
            state,
            who,
            state
                .characters
                .iter()
                .any(|character| character.entity == who),
        )?);
    }
    let mut own = None;
    let mut creation = None;
    let mut offers = Vec::new();
    let mut private_clue = String::new();
    if role == LocalDemoRole::Player {
        let who = stage(
            "view-player-entity",
            journey::player_entity(principal, current),
        )?;
        if state
            .characters
            .iter()
            .any(|character| character.entity == who)
        {
            own = Some(stage(
                "view-character-sheet",
                journey::character_sheet(state, who),
            )?);
        } else {
            creation = Some(stage("view-creation-offer", creation_offer())?);
        }
        let mut observation_added = false;
        for (kind, label, offer_id) in stage(
            "view-offers",
            super::beat_script_runtime::view_offers(current, principal),
        )? {
            if kind == rpc::GameplayActionKind::ExamineHarborSeal
                && super::beat_script_runtime::is_observation_offer(current, &offer_id)?
            {
                if observation_added {
                    continue;
                }
                observation_added = true;
            }
            offers.push(rpc::GameplayActionOffer {
                offer_id: offer_id.clone(),
                action_kind: kind as i32,
                label: label.to_owned(),
                destinations: if kind == rpc::GameplayActionKind::ChooseHarborScene {
                    vec![rpc::HarborSceneOption {
                        destination: rpc::HarborDestination::HarborInn as i32,
                        label: super::pack::destination_label(),
                    }]
                } else {
                    Vec::new()
                },
                input_groups: if journey::tactical::present(state) {
                    stage(
                        "view-tactical-groups",
                        journey::tactical::groups(current, principal, kind),
                    )?
                } else if kind == rpc::GameplayActionKind::GreatswordAttack {
                    vec![
                        attack_group(
                            "savage-attacker",
                            "Savage Attacker",
                            "Roll weapon damage twice and take the higher total",
                        ),
                        attack_group(
                            "graze",
                            "Graze",
                            "On a miss, apply your Strength modifier as damage",
                        ),
                    ]
                } else if kind == rpc::GameplayActionKind::ExamineHarborSeal
                    && stage(
                        "view-observation-offer",
                        super::beat_script_runtime::is_observation_offer(current, &offer_id),
                    )?
                {
                    vec![stage(
                        "view-observation-options",
                        observation_group(current),
                    )?]
                } else {
                    Vec::new()
                },
            });
        }
        let source = model::content("private-courier-note")?;
        if perceived.facts().iter().any(|fact| {
            matches!(
                &fact.value,
                df_model::checkpoint::FactValue::ContentEvent { definition, subjects }
                if *definition == source && subjects.is_empty()
            )
        }) {
            // Canonical audience selection only gates eligibility. The existing
            // courier owner still checks the exact authored reveal, recipient,
            // committed recording and saved response before rendering its text.
            private_clue = stage(
                "view-private-clue",
                super::courier_ai::saved_response(current, principal),
            )?
            .unwrap_or_default()
            .to_owned();
        }
    }
    let mut combat = None;
    if let Some(encounter) = state.encounters.first() {
        let mut participants = party.clone();
        if journey::tactical::present(state) {
            participants = encounter
                .participants
                .iter()
                .map(|who| party_member(state, *who, true))
                .collect::<Result<Vec<_>, _>>()
                .inspect_err(|_| super::refusal_detail::note("view-combat-participants"))?;
        } else {
            participants.push(party_member(state, journey::entity([0x65; 16])?, true)?);
        }
        let outcomes = state
            .decisions
            .iter()
            .filter(|decision| decision.source_policy.as_str() == journey::THREAD_POLICY)
            .map(journey::accepted)
            .collect::<Result<Vec<_>, _>>()
            .inspect_err(|_| super::refusal_detail::note("view-combat-outcomes"))?
            .into_iter()
            .flat_map(|accepted| accepted.combat)
            .collect::<Vec<_>>();
        // The Dungeon Master closing line for a finished fight, picked deterministically
        // from the full recorded history (before the log window below keeps only the last
        // blows). Victory is every enemy down to zero; anything else finished is a defeat.
        let finished = encounter.active_turn.is_none();
        let closing_lines = if finished {
            let heroes: std::collections::HashSet<&str> =
                party.iter().map(|member| member.name.as_str()).collect();
            let mut last_hp: std::collections::HashMap<&str, u32> =
                std::collections::HashMap::new();
            let mut attacks = 0u32;
            let mut kills = 0u32;
            for outcome in &outcomes {
                if !matches!(
                    rpc::OutcomeKind::try_from(outcome.kind),
                    Ok(rpc::OutcomeKind::Attack | rpc::OutcomeKind::OpportunityAttack)
                ) {
                    continue;
                }
                attacks += 1;
                last_hp.insert(outcome.target_name.as_str(), outcome.target_hit_points);
                if outcome.knocked_out && !heroes.contains(outcome.target_name.as_str()) {
                    kills += 1;
                }
            }
            let victory = participants
                .iter()
                .filter(|member| member.side == 2)
                .all(|member| last_hp.get(member.name.as_str()).is_some_and(|hp| *hp == 0));
            super::fight_lines::closing(victory, attacks, kills)
        } else {
            Vec::new()
        };
        // One clock: every resolved outcome carries its place in the fight's recorded
        // history (1-based, stable across views because decisions are append-only), so the
        // display presents beats in exactly this order and the pacing gate can release the
        // next actor only past beats the table has had time to see.
        let latest_sequence = u64::try_from(outcomes.len()).unwrap_or(u64::MAX);
        let outcomes: Vec<rpc::CombatOutcome> = outcomes
            .into_iter()
            .enumerate()
            .map(|(index, mut outcome)| {
                outcome.sequence = u64::try_from(index).unwrap_or(u64::MAX) + 1;
                outcome
            })
            .collect();
        let (outcomes, move_outcomes, death_save_outcomes) = journey::tactical::split_outcomes(
            outcomes,
            usize::from(super::pack::combat().combat_log_entries),
        );
        combat = Some(rpc::CombatView {
            participants,
            active_actor: encounter
                .active_turn
                .map(|who| journey::name(state, who))
                .transpose()?
                .unwrap_or_default(),
            round: journey::combat_round(current).map_err(|error| match error {
                journey::CombatRoundError::Capacity => RepositoryError::Capacity,
                _ => refused("view-combat-round"),
            })?,
            outcomes,
            move_outcomes,
            death_save_outcomes,
            closing_lines,
            latest_sequence,
            // Filled by the pacing gate (`TableLog::attach`) per audience; an unattributed
            // projection (tests) reads as fully released.
            released_sequence: latest_sequence,
            finished,
            source_revision: model::source_revision(),
            active_participant_id: encounter
                .active_turn
                .map_or_else(Vec::new, |who| who.as_bytes().to_vec()),
            initiative: journey::initiative_rolls(current).map_err(|error| match error {
                journey::CombatRoundError::Capacity => RepositoryError::Capacity,
                _ => refused("view-initiative"),
            })?,

            ..Default::default()
        });
    }
    if journey::tactical::present(state)
        && let Some(view) = combat.as_mut()
    {
        let own = if role == LocalDemoRole::Player {
            Some(stage(
                "view-player-entity",
                journey::player_entity(principal, current),
            )?)
        } else {
            None
        };
        stage(
            "view-tactical-combat",
            journey::tactical::project_combat(current, own, view),
        )?;
    }
    use super::pack::{self, SceneKey};
    let authored = stage("view-scene", super::beat_script_runtime::scene(state))?;
    let key = authored.key;
    let rested = state
        .narrative
        .completed_beats
        .contains(&model::content("short-rest-complete")?);
    let (narration, rested_rules_note) = rested_text(&authored, rested)?;
    // The story text and the rules wording are separate strings in the pack and stay separate
    // on the wire: the story is narrated and spoken, the rules note is only read.
    let narration = match stage(
        "view-courier-support",
        witnessed_courier_support(current, observer, perceived.facts(), &party),
    )? {
        Some(name) => pack::with_content(|content| {
            format!(
                "{narration} {}",
                content.escort_support_suffix.replace("{name}", &name)
            )
        }),
        None => narration,
    };
    let narration = if super::pack::active()
        .is_some_and(|loaded| !loaded.departures.is_empty() && loaded.tuning.tactical.is_none())
    {
        format!("{narration} Custom rules: {}.", model::source_revision())
    } else {
        narration
    };
    let narration = pack::with_content(|content| {
        if key == SceneKey::Inn
            && let Some(epilogue) = &content.campaign_epilogue
        {
            return if state.facts.iter().any(|fact| matches!(&fact.value,
                FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "private-courier-note")) {
                epilogue.asked.clone()
            } else {
                epilogue.escorted.clone()
            };
        }
        narration
    });
    let mut narration = if key == SceneKey::Defeat {
        combat
            .as_ref()
            .and_then(|view| defeat_context(&view.participants))
            .map_or(narration.clone(), |context| {
                format!("{context} {narration}")
            })
    } else {
        narration
    };
    if role == LocalDemoRole::Player
        && phase == rpc::JourneyPhase::Dialogue
        && !authored.findings.is_empty()
    {
        let actor = stage(
            "view-player-entity",
            journey::player_entity(principal, current),
        )?;
        // SRD 5.2.1 p. 22: "Passive Perception = 10 + Wisdom (Perception) check modifier."
        let passive = stage(
            "view-passive-perception",
            super::beat_script_runtime::passive_perception(state, actor),
        )?;
        let visible = passive_findings(&authored, passive)
            .into_iter()
            .map(|finding| finding.sentence.as_str())
            .collect::<Vec<_>>();
        if !visible.is_empty() {
            narration = format!("{narration} You notice: {}", visible.join(" "));
        }
    }
    // An epilogue replaces the authored story and rules; a rested scene carries its rules note.
    let epilogue_replaces =
        key == SceneKey::Inn && pack::with_content(|content| content.campaign_epilogue.is_some());
    let mut rules_note = project_scene_rules_note(
        &authored.rules_note,
        rested_rules_note.as_deref(),
        epilogue_replaces,
    );
    if key == SceneKey::Combat && super::journey::tactical::present(current.state()) {
        rules_note.push_str(" Disclosed custom rule tactical.cover-centre-line: cover is adjudicated along the segment between cell centres.");
        add_scan_barrier_reach_note(&mut rules_note);
        rules_note.push_str(&journey::tactical::unmodelled_features_note(
            current.state(),
        )?);
    }
    let table_choices = if role == LocalDemoRole::Display {
        stage("view-table-choices", table_choices(current, phase))?
    } else {
        Vec::new()
    };
    let mut journey = rpc::JourneyView {
        phase: if creation.is_some() {
            rpc::JourneyPhase::CharacterCreation as i32
        } else {
            phase as i32
        },
        room_code: journey::room_code(),
        join_path: format!("/gameplay/player?room={}", journey::room_code()),
        join_url: super::join_link::join_url(&journey::room_code()),
        party,
        creation,
        own_character: own,
        combat,
        speaker: authored.speaker.clone(),
        dialogue: authored.dialogue.clone(),
        table_log: Vec::new(),
        notebook: Vec::new(),
        tempo: None,
        standing: Vec::new(),
        live_faces: Vec::new(),
        table_choices,
        presentation_sequence: 0,
        reply_speaker: String::new(),
        home: (phase == rpc::JourneyPhase::Room)
            .then(|| super::home_theme::lobby_view(&authored.asset, authored.setting.as_ref())),
        adventure_ended: stage(
            "view-adventure-ended",
            super::beat_script_runtime::ended(state),
        )?,
        preparing_adventure: super::journey::new_adventure_requested(current),
        adventure_title: stage(
            "view-adventure-title",
            super::beat_script_runtime::adventure_title(state),
        )?,
        // The room's setting is attached to every view by the owner (see `TableSettings::attach`).
        choice_hints_hidden: false,
        // PLAY-V12: the opening scene's title is the adventure's own title on every view.
        adventure_name: pack::scene(SceneKey::Opening).title,
        composing: Vec::new(),
        // PLAY-V10: set per view for the display's lobby by `home_video::apply`; none here.
        home_video_state: rpc::HomeVideoState::None as i32,
        home_video_path: String::new(),
    };
    // The persisted decision outputs carry no cast; the view adds who is on stage.
    let mut scene = if stage(
        "view-native-inn",
        super::beat_script_runtime::native_inn(state),
    )? {
        journey::inn_scene()
    } else {
        rpc::GameplayScene {
            destination: rpc::HarborDestination::Unspecified as i32,
            title: authored.title.clone(),
            scene_asset: authored.asset.clone(),
            speaker_portrait: authored.speaker_portrait.clone(),
            cast: Vec::new(),
            rules_note: String::new(),
            place: String::new(),
            location_id: String::new(),
            live_backdrop: String::new(),
            live_plate: String::new(),
            plate_layout: None,
            plate_state: rpc::PlateState::Unspecified as i32,
            scene_video_state: rpc::HomeVideoState::None as i32,
            scene_video_path: String::new(),
            scene_contents_json: String::new(),
            item: None,
        }
    };
    scene.cast = scene_cast(&authored);
    scene.rules_note = rules_note;
    // A chapter's closing scenes are story scenes, not the aftermath of its fight: the screens
    // draw them as the peaceful chapters' resolution, epilogue and end card.
    if stage("view-closing", super::beat_script_runtime::closing(state))?
        || super::beat_script_runtime::settled_defeat(state)?
    {
        scene.destination = rpc::HarborDestination::HarborInn as i32;
        journey.combat = None;
    }
    scene.place = authored
        .setting
        .as_ref()
        .map(|setting| setting.name.clone())
        .unwrap_or_default();
    // The confrontation setting's id is the adventure's location, which keys the battlefield.
    scene.location_id = authored
        .setting
        .as_ref()
        .map(|setting| setting.place_id.clone())
        .unwrap_or_default();

    let audience = match role {
        LocalDemoRole::Display => rpc::view_message::Audience::Display(rpc::DisplayGameplayView {
            narration: narration.clone(),
            scene_asset: scene.scene_asset.clone(),
            scene: Some(scene),
            journey: Some(journey),
            scene_graph: None,
            live_plate_graph_revision: 0,
            check: None,
        }),
        LocalDemoRole::Player => rpc::view_message::Audience::Player(rpc::PlayerGameplayView {
            offer_id: String::new(),
            narration,
            check: None,
            private_clue,
            action_available: !offers.is_empty(),
            offers,
            scene: Some(scene),
            journey: Some(journey),
        }),
    };
    Ok(PublicJourneyProjection {
        view: rpc::ViewMessage {
            revision: Some(revision(current.basis().revision)),
            audience: Some(audience),
        },
        scene: key,
        rested,
    })
}

fn observation_group(current: &Checkpoint) -> Result<rpc::JourneyGroup, RepositoryError> {
    let scene = stage(
        "view-observation-scene",
        super::beat_script_runtime::scene(current.state()),
    )?;
    let mut options = scene
        .findings
        .iter()
        .map(|finding| {
            let label = finding
                .id
                .split('-')
                .map(|word| {
                    let mut characters = word.chars();
                    characters.next().map_or_else(String::new, |first| {
                        first.to_uppercase().chain(characters).collect()
                    })
                })
                .collect::<Vec<_>>()
                .join(" ");
            rpc::JourneyOption {
                option_id: finding.id.clone(),
                label,
                description: format!("Examine the {}.", finding.id.replace('-', " ")),
                requires_group: String::new(),
                requires_options: Vec::new(),
            }
        })
        .collect::<Vec<_>>();
    options.extend(scene.cast.iter().map(|actor| rpc::JourneyOption {
        option_id: format!("watch-{}", actor.id),
        label: format!("Watch {}", actor.name),
        description: format!("Watch {} for a change in expression or manner.", actor.name),
        requires_group: String::new(),
        requires_options: Vec::new(),
    }));
    if options.is_empty() || options.len() > 8 {
        crate::gameplay::refusal_detail::note("view-observation-options");
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(rpc::JourneyGroup {
        group_id: "finding".to_owned(),
        label: "Look around".to_owned(),
        options,
    })
}

fn passive_findings(
    scene: &super::pack::Scene,
    passive_perception: i16,
) -> Vec<&super::pack::Finding> {
    scene
        .findings
        .iter()
        .filter(|finding| {
            finding.skill == super::pack::FindingSkill::Perception
                && i16::from(finding.dc) <= passive_perception
        })
        .collect()
}

fn add_scan_barrier_reach_note(rules_note: &mut String) {
    rules_note.push_str(" The 2024 rules do not say whether melee reach crosses a barrier below half the target's height; this game refuses melee reach across every blocked step.");
}

#[cfg(test)]
#[path = "perception_tests.rs"]
mod perception_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rested_scene_keeps_story_and_rules_in_their_own_fields() {
        let mut authored = super::super::pack::scene(super::super::pack::SceneKey::Opening);
        authored.narration_rested = Some(
            "Calla sets out tea. An uninterrupted hour has passed without Hit Point Dice or healing. Eligible heroes recover one Second Wind use; dead or stable 0-HP heroes are not automatically revived. Rain moves past the window.".to_owned(),
        );
        authored.rules_note = "The authored rules note remains.".to_owned();

        let (story, rest_rules) = rested_text(&authored, true).unwrap();
        assert!(story.contains("The party spends an hour catching its breath."));
        assert!(!story.contains("Hit Point Dice"));
        assert!(!story.contains("Second Wind"));
        assert!(!story.contains("stable 0-HP"));
        let rules = project_scene_rules_note(&authored.rules_note, rest_rules.as_deref(), false);
        assert!(rules.starts_with("The authored rules note remains."));
        assert!(rules.contains("Hit Point Dice"));
        assert!(rules.contains("Second Wind"));
        assert!(rules.contains("stable creature"));
    }

    #[test]
    fn scan_barrier_melee_reach_note_discloses_the_table_ruling_srd_p15() {
        let mut rules_note = String::new();
        add_scan_barrier_reach_note(&mut rules_note);
        assert!(rules_note.contains("do not say whether melee reach crosses a barrier below half"));
        assert!(rules_note.contains("refuses melee reach across every blocked step"));
    }

    #[test]
    fn passive_perception_reveals_only_perception_findings_at_or_below_its_score_srd_521_p22() {
        // SRD 5.2.1 p. 22: "Passive Perception = 10 + Wisdom (Perception) check modifier."
        let mut scene = super::super::pack::scene(super::super::pack::SceneKey::Opening);
        scene.findings = vec![
            super::super::pack::Finding {
                id: "nearby".to_owned(),
                sentence: "A footprint crosses the dust.".to_owned(),
                skill: super::super::pack::FindingSkill::Perception,
                dc: 12,
                private: false,
                consequence: None,
            },
            super::super::pack::Finding {
                id: "hidden".to_owned(),
                sentence: "A map is hidden under the desk.".to_owned(),
                skill: super::super::pack::FindingSkill::Perception,
                dc: 15,
                private: false,
                consequence: None,
            },
            super::super::pack::Finding {
                id: "study".to_owned(),
                sentence: "The map is marked with a route.".to_owned(),
                skill: super::super::pack::FindingSkill::Investigation,
                dc: 10,
                private: false,
                consequence: None,
            },
        ];
        assert_eq!(
            passive_findings(&scene, 12)
                .iter()
                .map(|finding| finding.id.as_str())
                .collect::<Vec<_>>(),
            ["nearby"]
        );
    }

    #[test]
    fn defeat_context_names_enemy_kind_count_and_fallen_heroes() {
        let participants = vec![
            rpc::PartyMemberView {
                name: "Brynn".to_owned(),
                side: 1,
                unconscious: true,
                ..Default::default()
            },
            rpc::PartyMemberView {
                name: "Vale".to_owned(),
                side: 1,
                unconscious: true,
                ..Default::default()
            },
            rpc::PartyMemberView {
                name: "Giant Weasel 1".to_owned(),
                side: 2,
                ..Default::default()
            },
            rpc::PartyMemberView {
                name: "Giant Weasel 2".to_owned(),
                side: 2,
                ..Default::default()
            },
        ];

        assert_eq!(
            defeat_context(&participants).as_deref(),
            Some("Two giant weasels overwhelmed the party. Brynn and Vale fell.")
        );
    }
}

pub(super) fn current_combat_turn(
    current: &Checkpoint,
    member: df_types::MemberId,
) -> Result<bool, RepositoryError> {
    if super::journey::tactical::present(current.state()) {
        return super::journey::tactical::has_turn(current, member);
    }
    Ok(current
        .state()
        .encounters
        .first()
        .and_then(|encounter| encounter.active_turn)
        == Some(super::journey::player_entity(member, current)?))
}
pub(super) fn tactical_selection_rejected(
    current: &Checkpoint,
    member: df_types::MemberId,
    kind: rpc::GameplayActionKind,
    choices: &[rpc::JourneyChoice],
) -> Result<bool, RepositoryError> {
    if !super::journey::tactical::present(current.state()) {
        return Ok(false);
    }
    Ok(!super::journey::tactical::valid_selection(
        current, member, kind, choices,
    )?)
}

/// Shared submitted-offer admission used by the native owner before durable staging.
pub(super) fn action_offer_rejection(
    current: &Checkpoint,
    member: df_types::MemberId,
    kind: rpc::GameplayActionKind,
    offer: &str,
    choices: &[rpc::JourneyChoice],
) -> Result<Option<rpc::RejectionCode>, RepositoryError> {
    if kind == rpc::GameplayActionKind::StartNewAdventure
        && super::journey::session_machine::SessionState::of(current)?
            != super::journey::session_machine::SessionState::Ended
    {
        super::refusal_detail::note("new-adventure-machine");
        return Ok(Some(rpc::RejectionCode::StaleOffer));
    }
    if stage("view-phase", super::journey::phase(current))? == rpc::JourneyPhase::Combat
        && !current_combat_turn(current, member)?
    {
        return Ok(Some(rpc::RejectionCode::WrongTurn));
    }
    // The same list the player's phone shows, so an offer the phone lists is never stale here.
    if !super::beat_script_runtime::matches_id(current, kind, offer)?
        || !super::beat_script_runtime::view_offers(current, member)?
            .iter()
            .any(|(offered, _, _)| *offered == kind)
    {
        return Ok(Some(rpc::RejectionCode::StaleOffer));
    }
    if tactical_selection_rejected(current, member, kind, choices)? {
        return Ok(Some(rpc::RejectionCode::InvalidSelection));
    }
    Ok(None)
}
