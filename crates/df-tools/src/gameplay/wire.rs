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
        "create-character" => rpc::GameplayActionKind::CreateCharacter,
        "begin-story" => rpc::GameplayActionKind::BeginStory,
        "ask-courier" => rpc::GameplayActionKind::AskCourier,
        "escort-courier" => rpc::GameplayActionKind::EscortCourier,
        "defend-courier" => rpc::GameplayActionKind::DefendCourier,
        "greatsword-attack" => rpc::GameplayActionKind::GreatswordAttack,
        "second-wind" => rpc::GameplayActionKind::SecondWind,
        "end-turn" => rpc::GameplayActionKind::EndTurn,
        "short-rest" => rpc::GameplayActionKind::ShortRest,
        "choose-harbor-scene" => rpc::GameplayActionKind::ChooseHarborScene,
        _ => return None,
    })
}
pub(super) fn action_entry(kind: rpc::GameplayActionKind) -> Option<&'static str> {
    Some(match kind {
        rpc::GameplayActionKind::CreateCharacter => "create-character",
        rpc::GameplayActionKind::BeginStory => "begin-story",
        rpc::GameplayActionKind::AskCourier => "ask-courier",
        rpc::GameplayActionKind::EscortCourier => "escort-courier",
        rpc::GameplayActionKind::DefendCourier => "defend-courier",
        rpc::GameplayActionKind::GreatswordAttack => "greatsword-attack",
        rpc::GameplayActionKind::SecondWind => "second-wind",
        rpc::GameplayActionKind::EndTurn => "end-turn",
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
    let mut choices = Vec::new();
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
    if kind == rpc::GameplayActionKind::GreatswordAttack {
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
            },
            rpc::JourneyOption {
                option_id: "yes".to_owned(),
                label: "Use".to_owned(),
                description: description.to_owned(),
            },
        ],
    }
}
pub(super) fn creation_offer() -> rpc::CharacterCreationOffer {
    use df_rules::local_journey as rules;
    let group = |id: &str, label: &str, values: &[(&str, &str, &str)]| rpc::JourneyGroup {
        group_id: id.to_owned(),
        label: label.to_owned(),
        options: values
            .iter()
            .map(|(id, label, description)| rpc::JourneyOption {
                option_id: (*id).to_owned(),
                label: (*label).to_owned(),
                description: (*description).to_owned(),
            })
            .collect(),
    };
    let choice_group = |id: &str, label: &str, values: &[&str]| {
        group(
            id,
            label,
            &values.iter().map(|id| (*id, *id, "")).collect::<Vec<_>>(),
        )
    };
    rpc::CharacterCreationOffer {description:"Supported adventure builds: Level 1 Dwarf Fighter / Soldier. Choose one option in each group. Other classes and species remain in development; every submitted build is validated on the server.".to_owned(),
        groups:vec![
            group("species","Ancestry",&[("dwarf","Dwarf","Dwarven Toughness, Darkvision, Dwarven Resilience and Stonecunning retained.")]),
            group("class","Class",&[("fighter-1","Fighter · level 1","Weapon training, Fighting Style, Second Wind, three masteries.")]),
            group("background","Background",&[("soldier","Soldier","Athletics, Intimidation, gaming set and Savage Attacker.")]),
            group("array","Ability training",&[("stalwart","Stalwart","STR17 DEX13 CON15 INT10 WIS12 CHA8 · Soldier +2STR/+1CON"),
                ("vanguard","Vanguard","STR17 DEX14 CON14 INT8 WIS10 CHA12 · Soldier +2STR/+1CON")]),
            choice_group("alignment","Alignment",rules::ALIGNMENTS),
            choice_group("language-1","First additional language",rules::STANDARD_LANGUAGES),
            choice_group("language-2","Second additional language",rules::STANDARD_LANGUAGES),
            choice_group("skill-1","First Fighter skill",rules::FIGHTER_SKILLS),
            choice_group("skill-2","Second Fighter skill",rules::FIGHTER_SKILLS),
            choice_group("gaming-set","Gaming set proficiency",rules::GAMING_SETS),
            group("equipment","Starting equipment",&[("fighter-a-soldier-a","Fighter A + Soldier A","Chain Mail, Greatsword, Flail, 8 Javelins, Dungeoneer's Pack; Spear, Shortbow, 20 arrows, gaming set, Healer's Kit, Quiver, Traveler's Clothes; 18 GP.")]),
            group("style-masteries","Fighting Style and masteries",&[("defense-greatsword-flail-javelin","Defense · Greatsword / Flail / Javelin","Defense +1 armored AC. Graze / Sap / Slow masteries. This encounter offers Greatsword only.")]),
            group("allied-ties","Initiative tie agreement",&[("join-order","Party agrees: earlier join acts first","Party ties use join order; the GM chooses allies before the Bandit when tied.")]),
        ],source_revision:rules::SOURCE_REVISION.to_owned()}
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
    Ok(rpc::PartyMemberView {
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
            11
        } else if ready {
            13
        } else {
            0
        },
        active_turn: active,
        unconscious: ready && journey::value(state, who, "unconscious")? != 0,
        participant_id: who.as_bytes().to_vec(),
    })
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
    let owner: &AcceptedDecision = owners.next().ok_or(RepositoryError::InvalidCandidate)?;
    if owners.next().is_some() || super::journey::accepted(owner)?.phase != phase as i32 {
        return Err(RepositoryError::InvalidCandidate);
    }
    let terminal = selected
        .iter()
        .copied()
        .find(|fact| Some(&fact.id) == owner.facts.last())
        .ok_or(RepositoryError::InvalidCandidate)?;
    if source.audience != AudienceScope::Shared
        || terminal.audience != AudienceScope::Shared
        || source.ordinal.checked_add(1) != Some(terminal.ordinal)
        || terminal.cause != Some(source.id)
        || !matches!(&terminal.value, FactValue::ContentEvent { definition, subjects }
            if *definition == model::content(terminal_entry)? && subjects.is_empty())
    {
        return Err(RepositoryError::InvalidCandidate);
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
        return Err(RepositoryError::InvalidCandidate);
    }
    let courier = super::journey::entity([0x67; 16])?;
    let FactValue::ContentEvent { subjects, .. } = &reaction.value else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let [observer, hero] = subjects.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if *observer != courier {
        return Err(RepositoryError::InvalidCandidate);
    }
    let contact = selected
        .iter()
        .copied()
        .find(|fact| Some(fact.id) == reaction.cause)
        .ok_or(RepositoryError::InvalidCandidate)?;
    if !matches!(&contact.value, FactValue::ContentEvent { definition, subjects }
        if *definition == model::content("courier-escort-contact")? && subjects.as_slice() == [*hero])
    {
        return Err(RepositoryError::InvalidCandidate);
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
                maximum_scan_records: 512,
                maximum_member_comparisons: 512,
                maximum_selected_facts: 512,
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
            _ => RepositoryError::InvalidCandidate,
        })?
        .ok_or(RepositoryError::InvalidCandidate)?;
    if audience_basis != current.basis()
        || witness.observer != courier
        || witnessed.id != contact.id
        || witness.source != witness_source
    {
        return Err(RepositoryError::InvalidCandidate);
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
        return Err(RepositoryError::InvalidCandidate);
    }
    let member = party
        .iter()
        .find(|member| {
            member.character_ready && member.participant_id.as_slice() == hero.as_bytes()
        })
        .ok_or(RepositoryError::InvalidCandidate)?;
    Ok(Some(member.name.clone()))
}

pub(super) fn journey_view(
    current: &Checkpoint,
    role: df_persistence::local_demo_scope::LocalDemoRole,
    principal: df_types::MemberId,
) -> Result<rpc::ViewMessage, RepositoryError> {
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
        // membership and nested audience entries, at 512.
        df_knowledge::perception::PerceptionLimits {
            maximum_scan_records: 512,
            maximum_member_comparisons: 512,
            maximum_selected_facts: 512,
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
        _ => RepositoryError::InvalidCandidate,
    })?;
    let state = current.state();
    let phase = journey::phase(current)?;
    let mut party = Vec::new();
    for member in journey::participants(current) {
        let who = member.character.ok_or(RepositoryError::InvalidCandidate)?;
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
        let who = journey::player_entity(principal, current)?;
        if state
            .characters
            .iter()
            .any(|character| character.entity == who)
        {
            own = Some(journey::character_sheet(state, who)?);
        } else {
            creation = Some(creation_offer());
        }
        for (kind, label) in journey::offered(current, principal)? {
            offers.push(rpc::GameplayActionOffer {
                offer_id: journey::offer_id(current, kind),
                action_kind: kind as i32,
                label: label.to_owned(),
                destinations: if kind == rpc::GameplayActionKind::ChooseHarborScene {
                    vec![rpc::HarborSceneOption {
                        destination: rpc::HarborDestination::HarborInn as i32,
                        label: "Harbor Inn".to_owned(),
                    }]
                } else {
                    Vec::new()
                },
                input_groups: if kind == rpc::GameplayActionKind::GreatswordAttack {
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
            private_clue = super::courier_ai::saved_response(current, principal)?
                .unwrap_or_default()
                .to_owned();
        }
    }
    let mut combat = None;
    if let Some(encounter) = state.encounters.first() {
        let mut participants = party.clone();
        participants.push(party_member(state, journey::entity([0x65; 16])?, true)?);
        let outcomes = state
            .decisions
            .iter()
            .filter(|decision| decision.source_policy.as_str() == journey::THREAD_POLICY)
            .map(journey::accepted)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flat_map(|accepted| accepted.combat)
            .collect::<Vec<_>>();
        combat = Some(rpc::CombatView {
            participants,
            active_actor: encounter
                .active_turn
                .map(|who| journey::name(state, who))
                .transpose()?
                .unwrap_or_default(),
            round: journey::combat_round(current).map_err(|error| match error {
                journey::CombatRoundError::Capacity => RepositoryError::Capacity,
                _ => RepositoryError::InvalidCandidate,
            })?,
            outcomes: outcomes
                .into_iter()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            finished: encounter.active_turn.is_none(),
            source_revision: df_rules::local_journey::SOURCE_REVISION.to_owned(),
            active_participant_id: encounter
                .active_turn
                .map_or_else(Vec::new, |who| who.as_bytes().to_vec()),
        });
    }
    let (title, narration, speaker, dialogue) = match phase {
        rpc::JourneyPhase::Room => (
            "Gather at the Lantern Wharf",
            "A rain-soaked harbor waits beyond the lanterns. Share the room with your companions, then bring your heroes into the story.",
            "",
            "",
        ),
        rpc::JourneyPhase::Opening => (
            "The Broken Seal",
            "You reach the Lantern Wharf as a frightened courier slips from the shadows. Behind him, a boot scrapes across the boards.",
            "Courier",
            "Please. I can't deliver this alone. The seal was broken before the ship reached port.",
        ),
        rpc::JourneyPhase::Dialogue => {
            if state.narrative.active_beats[0].entry.as_str() == "courier-answer-seal" {
                (
                    "A Whisper in the Rain",
                    "The courier presses the packet against his coat. A dockside enforcer approaches, one hand on a scimitar.",
                    "Courier",
                    "The crest belongs to a ship that never appears on the ledger. I think someone aboard followed me.",
                )
            } else {
                (
                    "An Oath at the Wharf",
                    "The courier steadies himself behind you. The approaching enforcer draws a scimitar and demands the packet.",
                    "Courier",
                    "If you help me reach the inn, I'll tell Vell what I saw aboard that ship. But he won't let us leave.",
                )
            }
        }
        rpc::JourneyPhase::Combat => (
            "Steel Beneath the Lanterns",
            "The Bandit is within 5 feet of both heroes. All weapons are wielded openly: no surprise, cover or movement. This encounter uses nonlethal melee attacks.",
            "Bandit",
            "Hand over the packet. Last warning.",
        ),
        rpc::JourneyPhase::Complete
            if state.narrative.active_beats.as_slice() == [model::content("harbor-inn")?] =>
        {
            (
                "The Harbor Inn",
                "Warm lamplight frames the next chapter with your party and the courier. The sealed packet remains unresolved.",
                "Courier",
                "We'll keep the packet close. Its journey isn't over.",
            )
        }
        rpc::JourneyPhase::Complete => {
            let rested = state
                .narrative
                .completed_beats
                .contains(&model::content("short-rest-complete")?);
            if journey::combat_victory(state)? {
                (
                    "The Courier's Road",
                    if rested {
                        "The Bandit was defeated. An uninterrupted hour has passed without Hit Point Dice or healing. Knocked-out creatures are awake with 1 HP, still Prone; their held weapons remain dropped. The original victory and the courier's unresolved journey remain."
                    } else {
                        "The Bandit falls unconscious with 1 HP. The courier lowers his shaking hands. The packet—and its secret—remain with your party. His Short Rest has begun; campaign time has not completed it."
                    },
                    "Courier",
                    "You stood your ground. Come on. There's a light waiting for us at the inn.",
                )
            } else {
                (
                    "A Costly Encounter",
                    if rested {
                        "Both heroes were defeated by nonlethal attacks. An uninterrupted hour has passed without Hit Point Dice or healing. They awaken with 1 HP, still Prone, and their held weapons remain dropped. That rest does not change the original defeat: the Bandit took the packet and left."
                    } else {
                        "Both heroes are unconscious with 1 HP after nonlethal attacks. Their Short Rests have begun; campaign time has not completed them. The Bandit takes the packet and leaves."
                    },
                    "",
                    "",
                )
            }
        }
        _ => return Err(RepositoryError::InvalidCandidate),
    };
    let narration = match witnessed_courier_support(current, observer, perceived.facts(), &party)? {
        Some(name) => {
            format!("{narration} The courier now looks to {name} for support on the escort.")
        }
        None => narration.to_owned(),
    };
    let journey = rpc::JourneyView {
        phase: if creation.is_some() {
            rpc::JourneyPhase::CharacterCreation as i32
        } else {
            phase as i32
        },
        room_code: journey::ROOM_CODE.to_owned(),
        join_path: format!("/gameplay/player?room={}", journey::ROOM_CODE),
        party,
        creation,
        own_character: own,
        combat,
        speaker: speaker.to_owned(),
        dialogue: dialogue.to_owned(),
    };
    let scene = if state.narrative.active_beats.as_slice() == [model::content("harbor-inn")?] {
        journey::inn_scene()
    } else {
        rpc::GameplayScene {
            destination: rpc::HarborDestination::Unspecified as i32,
            title: title.to_owned(),
            scene_asset: "assets/ui/scenes/mara-harbor-v4.png".to_owned(),
        }
    };

    let audience = match role {
        LocalDemoRole::Display => rpc::view_message::Audience::Display(rpc::DisplayGameplayView {
            narration: narration.clone(),
            scene_asset: scene.scene_asset.clone(),
            scene: Some(scene),
            journey: Some(journey),
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
    Ok(rpc::ViewMessage {
        revision: Some(revision(current.basis().revision)),
        audience: Some(audience),
    })
}

#[cfg(test)]
#[path = "perception_tests.rs"]
mod perception_tests;
