//! Game actions for the control surface: every command plays through the
//! session owner's admission, validation, rules and commit path as a seated
//! player. Nothing here writes game state directly.

use axum::{extract::State, http::HeaderMap, response::Json};
use df_protocol::common as rpc;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::oneshot;

use super::{Outcome, Shared, authorized, bad_request, failed, owner_submit_action, seat_view};
use crate::gameplay::dev_control::parse;

/// `POST /dev/join {"seat": "1"}`: joins the room as that seat through the
/// same [`actor`](super::super::actor) join path a phone uses.
pub(super) async fn join_seat(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let seat = body
        .get("seat")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("join needs {\"seat\": \"1\"}"))?;
    let index = super::seat_index(seat).map_err(bad_request)?;
    let operation: [u8; 16] = random_array().await?;
    let mut secret: [u8; 32] = random_array().await?;
    while secret.iter().all(|byte| *byte == 0) {
        secret = random_array().await?;
    }
    let request = rpc::JoinRoomRequest {
        room_code: super::super::journey::room_code(),
        operation_id: Some(rpc::OperationId {
            value: Some(operation.to_vec()),
        }),
        join_secret: secret.to_vec(),
    };
    let (reply, wait) = oneshot::channel();
    state
        .service
        .actor
        .try_submit(super::super::actor::Call::Join { request, reply })
        .map_err(|_| failed("room actor queue full"))?;
    let response = tokio::time::timeout(super::super::timing::active().reply, wait)
        .await
        .map_err(|_| failed("room join deadline"))?
        .map_err(|_| failed("room owner stopped"))?
        .map_err(|status| failed(status.message()))?;
    match response.outcome {
        Some(rpc::join_room_response::Outcome::Joined(joined)) => {
            store_join(&state, index, *joined).await
        }
        Some(rpc::join_room_response::Outcome::Refused(refused)) => Ok((
            axum::http::StatusCode::OK,
            Json(json!({"status": "refused", "code": super::rejection_name(refused.code)})),
        )),
        None => Err(failed("empty join answer")),
    }
}

/// `POST /dev/reclaim {"seat":"1"}` uses the room-code-only phone admission path.
/// It reads no player view first: doing so would keep the abandoned credential active.
pub(super) async fn reclaim_seat(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let seat = body
        .get("seat")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("reclaim needs {\"seat\": \"1\"}"))?;
    let index = super::seat_index(seat).map_err(bad_request)?;
    let member = state.seats.lock().await.seats[index]
        .member
        .clone()
        .ok_or_else(|| bad_request("that seat has not joined"))?;
    let pending = state.seats.lock().await.seats[index].reclaim;
    let pending = match pending {
        Some(pending) => pending,
        None => {
            let (reply, wait) = oneshot::channel();
            state
                .service
                .actor
                .try_submit(super::super::actor::Call::AwaySeats {
                    room_code: super::super::journey::room_code(),
                    reply,
                })
                .map_err(|_| failed("room actor queue full"))?;
            let away = tokio::time::timeout(super::super::timing::active().reply, wait)
                .await
                .map_err(|_| failed("away seat read deadline"))?
                .map_err(|_| failed("room owner stopped"))?
                .map_err(|status| failed(status.message()))?;
            let token = away
                .into_iter()
                .find(|hero| hero.member.as_slice() == member.as_slice())
                .map(|hero| hero.token)
                .unwrap_or([0; 32]);
            let pending = super::PendingReclaim {
                operation: random_array::<16>().await?,
                token,
            };
            let mut seats = state.seats.lock().await;
            seats.seats[index].reclaim = Some(pending);
            super::persist_seats(&state, &seats)
                .map_err(|_| failed("reclaim proof could not be retained"))?;
            pending
        }
    };
    let operation = df_types::OperationId::from_bytes(&pending.operation)
        .map_err(|_| failed("reclaim operation invalid"))?;
    let token = pending.token;
    let (reply, wait) = oneshot::channel();
    state
        .service
        .actor
        .try_submit(super::super::actor::Call::Reclaim {
            request: super::super::room::ReclaimRequest {
                room_code: super::super::journey::room_code(),
                operation,
                token,
            },
            reply,
        })
        .map_err(|_| failed("room actor queue full"))?;
    let answer = tokio::time::timeout(super::super::timing::active().reply, wait)
        .await
        .map_err(|_| failed("room reclaim deadline"))?
        .map_err(|_| failed("room owner stopped"))?
        .map_err(|status| failed(status.message()))?;
    if answer.is_err() {
        let mut seats = state.seats.lock().await;
        seats.seats[index].reclaim = None;
        super::persist_seats(&state, &seats)
            .map_err(|_| failed("reclaim proof could not be cleared"))?;
    }
    match answer {
        Ok(joined) => store_join(&state, index, joined).await,
        Err(reason) => Ok((
            axum::http::StatusCode::OK,
            Json(json!({
                "status": "refused", "reason": format!("{reason:?}"),
                "message": "That hero is being played on another phone."
            })),
        )),
    }
}

async fn store_join(state: &Shared, index: usize, joined: rpc::JoinedRoom) -> Outcome {
    let credential = super::super::journey::unhex(&joined.local_binding)
        .map_err(|_| failed("join binding invalid"))?;
    let credential: [u8; 32] = credential
        .try_into()
        .map_err(|_| failed("join binding length invalid"))?;
    let member = joined
        .client_binding_id
        .and_then(|binding| binding.value)
        .ok_or_else(|| failed("join member binding missing"))?;
    let session_id = joined
        .session_id
        .and_then(|id| id.value)
        .ok_or_else(|| failed("join session missing"))?;
    let run_id = joined
        .run_id
        .and_then(|id| id.value)
        .ok_or_else(|| failed("join run missing"))?;
    let receipt = joined.receipt.map(super::receipt_json);
    let mut seats = state.seats.lock().await;
    seats.session_id = Some(session_id);
    seats.run_id = Some(run_id);
    seats.seats[index].reclaim = None;
    seats.seats[index].credential = Some(credential);
    seats.seats[index].member = Some(member);
    super::persist_seats(state, &seats)
        .map_err(|_| failed("dev control seat state unavailable"))?;
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({"status": "joined", "seat": index + 1, "receipt": receipt})),
    ))
}

async fn random_array<const N: usize>() -> Result<[u8; N], (axum::http::StatusCode, Json<Value>)> {
    super::super::model::random::<N>().map_err(|_| failed("local randomness unavailable"))
}

/// The canonical creation choices: the one source-selected Dwarf Fighter /
/// Soldier the compiled validator admits. The hero name travels only in the
/// selection's name field (the owner hex-encodes it there); repeating it here
/// would be a duplicate group.
fn creation_choices() -> Vec<rpc::JourneyChoice> {
    let mut choices = Vec::new();
    for (group, option) in [
        ("species", "dwarf"),
        ("class", "fighter-1"),
        ("background", "soldier"),
        ("array", "stalwart"),
        ("alignment", "lawful-good"),
        ("language-1", "dwarvish"),
        ("language-2", "elvish"),
        ("skill-1", "perception"),
        ("skill-2", "survival"),
        ("gaming-set", "dice"),
        ("equipment", "fighter-a-soldier-a"),
        ("style-masteries", "defense-greatsword-flail-javelin"),
        ("allied-ties", "join-order"),
    ] {
        choices.push(rpc::JourneyChoice {
            group_id: group.to_owned(),
            option_id: option.to_owned(),
        });
    }
    choices
}

fn choice_values(
    value: Option<&Value>,
) -> Result<Vec<rpc::JourneyChoice>, (axum::http::StatusCode, Json<Value>)> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_object()
        .ok_or_else(|| bad_request("choices must be an object of group ids and option ids"))?;
    if values.len() > 32 {
        return Err(bad_request("at most 32 choice groups"));
    }
    values
        .iter()
        .map(|(group, value)| {
            let option = value
                .as_str()
                .ok_or_else(|| bad_request("choice values must be option ids or labels"))?;
            if group.is_empty() || group.len() > 128 || option.is_empty() || option.len() > 256 {
                return Err(bad_request("choice ids must be bounded and non-empty"));
            }
            Ok(rpc::JourneyChoice {
                group_id: group.clone(),
                option_id: option.into(),
            })
        })
        .collect()
}

fn selected_creation_choices(
    body: &Value,
) -> Result<Vec<rpc::JourneyChoice>, (axum::http::StatusCode, Json<Value>)> {
    let mut choices = creation_choices();
    for choice in choice_values(body.get("choices"))? {
        if let Some(found) = choices
            .iter_mut()
            .find(|found| found.group_id == choice.group_id)
        {
            *found = choice;
        } else {
            choices.push(choice);
        }
    }
    Ok(choices)
}

fn selected_identity(
    body: &Value,
) -> Result<Option<rpc::HeroIdentity>, (axum::http::StatusCode, Json<Value>)> {
    use crate::gameplay::hero_identity::Identity;
    if body.get("identity").is_some() && body.get("identity_choices").is_some() {
        return Err(bad_request(
            "provide identity or identity_choices, not both",
        ));
    }
    let identity = if let Some(value) = body.get("identity") {
        serde_json::from_value::<Identity>(value.clone())
            .map_err(|_| bad_request("identity shape is invalid"))?
            .validate()
    } else if body.get("identity_choices").is_some() {
        let choices = choice_values(body.get("identity_choices"))?;
        Identity::from_menus(
            &choices
                .iter()
                .map(|choice| (choice.group_id.as_str(), choice.option_id.as_str()))
                .collect::<Vec<_>>(),
        )
    } else {
        return Ok(None);
    };
    identity
        .map(|identity| Some(identity.to_rpc()))
        .map_err(|error| bad_request(format!("identity: {error:?}")))
}

/// `POST /dev/create {"seat": "1", "name": "Brynn"}`: creates the canonical
/// hero for that seat through the owner's create-character path.
pub(super) async fn create_hero(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let seat = body
        .get("seat")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("create needs {\"seat\": \"1\", \"name\": \"Brynn\"}"))?;
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("create needs {\"seat\": \"1\", \"name\": \"Brynn\"}"))?;
    if name.is_empty() || name.len() > 48 {
        return Err(bad_request("hero name must be 1 to 48 characters"));
    }
    let index = super::seat_index(seat).map_err(bad_request)?;
    let credential = {
        let seats = state.seats.lock().await;
        seats.seats[index]
            .credential
            .ok_or_else(|| bad_request(format!("seat{} has not joined yet", index + 1)))?
    };
    let (revision, offers) = seat_offers(&state, index).await?;
    let offer = offers
        .iter()
        .find(|offer| offer.action_kind == rpc::GameplayActionKind::CreateCharacter as i32)
        .ok_or_else(|| {
            (
                axum::http::StatusCode::OK,
                Json(json!({"status": "unavailable", "reason": "create-character is not offered to this seat right now"})),
            )
        })?;
    let seats = state.seats.lock().await;
    let (session_id, run_id) = super::scope_of(&seats)?;
    drop(seats);
    let operation: [u8; 16] = random_array().await?;
    let request = rpc::SubmitActionRequest {
        session_id: Some(rpc::SessionId {
            value: Some(session_id),
        }),
        run_id: Some(rpc::RunId {
            value: Some(run_id),
        }),
        observed_revision: Some(revision),
        operation_id: Some(rpc::OperationId {
            value: Some(operation.to_vec()),
        }),
        offer_id: offer.offer_id.clone(),
        action_kind: rpc::GameplayActionKind::CreateCharacter as i32,
        character: Some(rpc::CharacterSelection {
            name: name.to_owned(),
            choices: selected_creation_choices(&body)?,
            identity: selected_identity(&body)?,
        }),
        ..Default::default()
    };
    let (code, Json(answer)) = owner_submit_action(&state, credential, request).await?;
    if answer.get("status").and_then(Value::as_str) == Some("accepted") {
        let mut seats = state.seats.lock().await;
        seats.seats[index].hero = Some(name.to_owned());
        super::persist_seats(&state, &seats)
            .map_err(|_| failed("dev control seat state unavailable"))?;
    }
    Ok((code, Json(answer)))
}

/// Uses the same authenticated, read-only identity preview as the creation page.
pub(super) async fn preview_identity(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let hero = body
        .get("seat")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("preview needs a seat"))?;
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, hero).map_err(bad_request)?
    };
    let credential = state.seats.lock().await.seats[index]
        .credential
        .ok_or_else(|| bad_request("join this seat first"))?;
    let member = state.seats.lock().await.seats[index]
        .member
        .clone()
        .ok_or_else(|| bad_request("join this seat first"))?;
    let (revision, _) = seat_view(&state, index).await?;
    let (session_id, run_id) = {
        let seats = state.seats.lock().await;
        super::scope_of(&seats)?
    };
    let mut request = tonic::Request::new(rpc::PreviewHeroIdentityRequest {
        session_id: Some(rpc::SessionId {
            value: Some(session_id),
        }),
        run_id: Some(rpc::RunId {
            value: Some(run_id),
        }),
        observed_revision: Some(revision),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(member),
        }),
        appearance_text: body
            .get("appearance_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        backstory_text: body
            .get("backstory_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        current_identity: selected_identity(&body)?,
    });
    request.metadata_mut().insert(
        "x-df-local-binding",
        crate::gameplay::journey::hex(&credential)
            .parse()
            .map_err(|_| failed("seat binding invalid"))?,
    );
    let result =
        rpc::action_service_server::ActionService::preview_hero_identity(&state.service, request)
            .await
            .map_err(|status| bad_request(status.message()))?
            .into_inner();
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({"status":"previewed", "source":result.source,
        "identity":result.proposed_identity.as_ref().map(super::reads::identity_json), "warnings":result.warnings})),
    ))
}

/// One offer from a player view, with its selectable inputs.
struct Offer {
    offer_id: String,
    action_kind: i32,
    label: String,
    groups: Vec<rpc::JourneyGroup>,
    destinations: Vec<rpc::HarborSceneOption>,
}

fn player_offers(
    view: &rpc::ViewMessage,
) -> Result<Vec<Offer>, (axum::http::StatusCode, Json<Value>)> {
    let Some(rpc::view_message::Audience::Player(player)) = view.audience.as_ref() else {
        return Err(failed("player view required"));
    };
    Ok(player
        .offers
        .iter()
        .map(|offer| Offer {
            offer_id: offer.offer_id.clone(),
            action_kind: offer.action_kind,
            label: offer.label.clone(),
            groups: offer.input_groups.clone(),
            destinations: offer.destinations.clone(),
        })
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChoiceTarget {
    Offer,
    GroupOption { group: usize, option: usize },
    Destination(usize),
}

fn unique_offer_match(offers: &[Offer], query: &str) -> Option<usize> {
    let matches = |exact: bool| {
        let mut found = Vec::new();
        for (index, offer) in offers.iter().enumerate() {
            let mut names = vec![offer.offer_id.clone(), offer.label.clone()];
            for group in &offer.groups {
                for option in &group.options {
                    names.push(option.option_id.clone());
                    names.push(option.label.clone());
                }
            }
            for destination in &offer.destinations {
                names.push(destination.label.clone());
                names.push(destination.destination.to_string());
            }
            if names.iter().any(|name| {
                if exact {
                    name.eq_ignore_ascii_case(query)
                } else {
                    parse::names_match(name, query)
                }
            }) {
                found.push(index);
            }
        }
        found
    };
    let exact = matches(true);
    let found = if exact.is_empty() {
        matches(false)
    } else {
        exact
    };
    match found.as_slice() {
        [index] => Some(*index),
        _ => None,
    }
}

fn unique_choice_target(offer: &Offer, query: &str) -> Option<ChoiceTarget> {
    let mut names = vec![
        (offer.offer_id.as_str(), ChoiceTarget::Offer),
        (offer.label.as_str(), ChoiceTarget::Offer),
    ];
    for (group_index, group) in offer.groups.iter().enumerate() {
        for (option_index, option) in group.options.iter().enumerate() {
            let target = ChoiceTarget::GroupOption {
                group: group_index,
                option: option_index,
            };
            names.push((option.option_id.as_str(), target));
            names.push((option.label.as_str(), target));
        }
    }
    for (index, destination) in offer.destinations.iter().enumerate() {
        names.push((destination.label.as_str(), ChoiceTarget::Destination(index)));
    }
    let select = |exact: bool| {
        let mut found = Vec::new();
        for (name, target) in &names {
            let matches = if exact {
                name.eq_ignore_ascii_case(query)
            } else {
                parse::names_match(name, query)
            };
            if matches && !found.contains(target) {
                found.push(*target);
            }
        }
        found
    };
    let exact = select(true);
    let found = if exact.is_empty() {
        select(false)
    } else {
        exact
    };
    match found.as_slice() {
        [target] => Some(*target),
        _ => None,
    }
}

/// Reads one seat's offers for name resolution, tolerating the owner's
/// post-commit projection lag: right after a commit the served player view
/// can carry no offers for several seconds (observed up to ~10 s on a loaded
/// box) before the projection catches up, while a settled view for a hero
/// who may act always carries at least one offer. An empty read is therefore
/// re-read a bounded number of times; a settled empty view is returned as-is
/// and reported faithfully. The owner's submit answer stays authoritative.
async fn seat_offers(
    state: &Shared,
    index: usize,
) -> Result<(rpc::SessionRevision, Vec<Offer>), (axum::http::StatusCode, Json<Value>)> {
    let mut attempts = 0;
    loop {
        let (revision, view) = super::seat_view(state, index).await?;
        let offers = player_offers(&view)?;
        if !offers.is_empty() || attempts >= 6 {
            return Ok((revision, offers));
        }
        attempts += 1;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// `POST /dev/act {"hero": "Brynn", "action": "move", "cell": "c5r6"}`: plays
/// one explicit action through the owner's submit path and reports the owner's
/// answer: accepted with the new revision, or refused with the server's reason.
pub(super) async fn act(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let hero = body
        .get("hero")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("act needs {\"hero\": \"Brynn\", \"action\": \"move\", ...}"))?;
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("act needs {\"hero\": \"Brynn\", \"action\": \"move\", ...}"))?;
    if action.eq_ignore_ascii_case("interact") {
        let index = {
            let seats = state.seats.lock().await;
            super::resolve_hero(&seats, hero).map_err(bad_request)?
        };
        let credential = state.seats.lock().await.seats[index]
            .credential
            .ok_or_else(|| bad_request("join this seat first"))?;
        let (revision, _) = seat_view(&state, index).await?;
        let current = state.service.updates.borrow().clone();
        let target = body
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| bad_request("interact needs --target <scene id>"))?;
        let option = body
            .get("option")
            .and_then(Value::as_str)
            .ok_or_else(|| bad_request("interact needs --option <action>"))?;
        let mut submit = rpc::SubmitActionRequest {
            observed_revision: Some(revision),
            offer_id: crate::gameplay::journey::living_scene::offer_id(&current),
            action_kind: rpc::GameplayActionKind::ExamineHarborSeal as i32,
            action_choices: vec![
                rpc::JourneyChoice {
                    group_id: "scene-target".into(),
                    option_id: target.into(),
                },
                rpc::JourneyChoice {
                    group_id: "scene-option".into(),
                    option_id: option.into(),
                },
            ],
            ..Default::default()
        };
        fill_scope(&state, &mut submit).await?;
        submit.operation_id = Some(rpc::OperationId {
            value: Some(random_array::<16>().await?.to_vec()),
        });
        return owner_submit_action(&state, credential, submit).await;
    }
    let mut body = body.clone();
    let action = match parse::parse_action(action) {
        Ok(action) => action,
        Err(_) => {
            if body.get("option").is_some() {
                return Err(bad_request(
                    "use choose with --option, or an offer label directly",
                ));
            }
            body["option"] = json!(action);
            parse::PlayAction::Choose
        }
    };
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, hero).map_err(bad_request)?
    };
    let credential = {
        let seats = state.seats.lock().await;
        seats.seats[index]
            .credential
            .ok_or_else(|| bad_request(format!("seat{} has not joined yet", index + 1)))?
    };
    if action == parse::PlayAction::TableSetting {
        let hidden = body
            .get("choice_hints_hidden")
            .and_then(Value::as_bool)
            .ok_or_else(|| bad_request("table-setting needs choice_hints_hidden: true or false"))?;
        let (revision, _) = seat_view(&state, index).await?;
        let mut submit = rpc::SubmitActionRequest {
            observed_revision: Some(revision),
            action_kind: action.kind() as i32,
            operation_id: Some(rpc::OperationId {
                value: Some(random_array::<16>().await?.to_vec()),
            }),
            table_setting: Some(rpc::TableSettingChange {
                value: Some(rpc::table_setting_change::Value::ChoiceHintsHidden(hidden)),
            }),
            ..Default::default()
        };
        fill_scope(&state, &mut submit).await?;
        return owner_submit_action(&state, credential, submit).await;
    }
    let (revision, offers) = seat_offers(&state, index).await?;
    let offer_index = if action == parse::PlayAction::Choose {
        let option = body
            .get("option")
            .and_then(Value::as_str)
            .filter(|option| !option.trim().is_empty() && option.len() <= 256)
            .ok_or_else(|| bad_request("choose needs {\"option\": \"<label or id>\"}"))?;
        unique_offer_match(&offers, option)
    } else {
        offers
            .iter()
            .position(|offer| offer.action_kind == action.kind() as i32)
    }
    .ok_or_else(|| {
        (
            axum::http::StatusCode::OK,
            Json(json!({
                "status": "unavailable",
                "reason": if action == parse::PlayAction::Choose {
                    "the choice matches none or several current offers".to_owned()
                } else {
                    format!("{action:?} is not offered to this hero right now")
                },
                "offers": offers.iter().map(|offer| offer.label.clone()).collect::<Vec<_>>(),
            })),
        )
    })?;
    let offer = &offers[offer_index];
    let action_kind = if action == parse::PlayAction::Choose {
        rpc::GameplayActionKind::try_from(offer.action_kind)
            .map_err(|_| failed("offered action kind invalid"))?
    } else {
        action.kind()
    };
    let mut submit = rpc::SubmitActionRequest {
        observed_revision: Some(revision),
        offer_id: offer.offer_id.clone(),
        action_kind: action_kind as i32,
        ..Default::default()
    };
    fill_scope(&state, &mut submit).await?;
    let operation: [u8; 16] = random_array().await?;
    submit.operation_id = Some(rpc::OperationId {
        value: Some(operation.to_vec()),
    });
    fill_action_choices(&body, action, offer, &mut submit)?;
    if body.get("confirmation_token").is_some() {
        let mut request = tonic::Request::new(rpc::ConfirmDialogueRequest {
            input_id: Some(rpc::OperationId {
                value: Some(proof_bytes(&body, "input_id", 16)?),
            }),
            confirmation_token: proof_bytes(&body, "confirmation_token", 32)?,
            action: Some(submit),
        });
        request.metadata_mut().insert(
            "x-df-local-binding",
            crate::gameplay::journey::hex(&credential)
                .parse()
                .map_err(|_| failed("seat binding invalid"))?,
        );
        let response =
            rpc::dialogue_service_server::DialogueService::confirm(&state.service, request)
                .await
                .map_err(|status| bad_request(status.message()))?
                .into_inner();
        return Ok((
            axum::http::StatusCode::OK,
            Json(super::submit_json(response)),
        ));
    }
    owner_submit_action(&state, credential, submit).await
}

fn proof_bytes(
    body: &Value,
    key: &str,
    size: usize,
) -> Result<Vec<u8>, (axum::http::StatusCode, Json<Value>)> {
    let text = body
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request(format!("{key} is required")))?;
    let bytes = crate::gameplay::journey::unhex(text)
        .map_err(|_| bad_request(format!("{key} must be hexadecimal")))?;
    if bytes.len() != size {
        return Err(bad_request(format!("{key} must have {size} bytes")));
    }
    Ok(bytes)
}

pub(super) async fn cancel_dialogue(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let hero = body
        .get("hero")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("cancel needs a hero"))?;
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, hero).map_err(bad_request)?
    };
    let credential = state.seats.lock().await.seats[index]
        .credential
        .ok_or_else(|| bad_request("join this seat first"))?;
    let mut request = tonic::Request::new(rpc::CancelDialogueRequest {
        input_id: Some(rpc::OperationId {
            value: Some(proof_bytes(&body, "input_id", 16)?),
        }),
        confirmation_token: proof_bytes(&body, "confirmation_token", 32)?,
    });
    request.metadata_mut().insert(
        "x-df-local-binding",
        crate::gameplay::journey::hex(&credential)
            .parse()
            .map_err(|_| failed("seat binding invalid"))?,
    );
    let response = rpc::dialogue_service_server::DialogueService::cancel(&state.service, request)
        .await
        .map_err(|status| bad_request(status.message()))?
        .into_inner();
    Ok((
        axum::http::StatusCode::OK,
        Json(
            json!({"status":"cancelled","disposition":disposition_name(response.disposition),"revision":super::revision_json(response.revision.as_ref())}),
        ),
    ))
}

async fn fill_scope(
    state: &Shared,
    submit: &mut rpc::SubmitActionRequest,
) -> Result<(), (axum::http::StatusCode, Json<Value>)> {
    let seats = state.seats.lock().await;
    let (session_id, run_id) = super::scope_of(&seats)?;
    submit.session_id = Some(rpc::SessionId {
        value: Some(session_id),
    });
    submit.run_id = Some(rpc::RunId {
        value: Some(run_id),
    });
    Ok(())
}

/// Resolves the action's selections from the current offers. Names match
/// exactly or by unique substring; anything ambiguous is refused without
/// guessing. A well-formed but illegal selection (for example a cell outside
/// reach) is still submitted so the server's own refusal is reported.
fn fill_action_choices(
    body: &Value,
    action: parse::PlayAction,
    offer: &Offer,
    submit: &mut rpc::SubmitActionRequest,
) -> Result<(), (axum::http::StatusCode, Json<Value>)> {
    let explicit = choice_values(body.get("choices"))?;
    let mut selected_body = body.clone();
    if selected_body.get("option").is_none()
        && selected_body.get("target").is_none()
        && selected_body.get("cell").is_none()
        && action != parse::PlayAction::Choose
        && let Some(first) = offer.groups.first()
        && let Some(choice) = explicit
            .iter()
            .find(|choice| choice.group_id == first.group_id)
    {
        selected_body["option"] = json!(choice.option_id);
    }
    default_action_choices(&selected_body, action, offer, submit)?;
    for choice in explicit {
        let group = offer
            .groups
            .iter()
            .find(|group| group.group_id == choice.group_id)
            .ok_or_else(|| {
                bad_request(format!("choice group {:?} is not offered", choice.group_id))
            })?;
        let id = group
            .options
            .iter()
            .find(|option| option.option_id == choice.option_id)
            .map(|option| option.option_id.clone())
            .or_else(|| {
                let labels = group
                    .options
                    .iter()
                    .map(|option| option.label.as_str())
                    .collect::<Vec<_>>();
                parse::match_index(&labels, &choice.option_id)
                    .map(|index| group.options[index].option_id.clone())
            })
            .ok_or_else(|| {
                bad_request(format!(
                    "choice {:?} is absent or ambiguous",
                    choice.option_id
                ))
            })?;
        if let Some(found) = submit
            .action_choices
            .iter_mut()
            .find(|found| found.group_id == choice.group_id)
        {
            found.option_id = id;
        } else {
            submit.action_choices.push(rpc::JourneyChoice {
                group_id: choice.group_id,
                option_id: id,
            });
        }
    }
    Ok(())
}

fn default_action_choices(
    body: &Value,
    action: parse::PlayAction,
    offer: &Offer,
    submit: &mut rpc::SubmitActionRequest,
) -> Result<(), (axum::http::StatusCode, Json<Value>)> {
    if action == parse::PlayAction::Choose {
        let query = body
            .get("option")
            .and_then(Value::as_str)
            .ok_or_else(|| bad_request("choose needs {\"option\": \"<label or id>\"}"))?;
        let target = unique_choice_target(offer, query).ok_or_else(|| {
            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "status": "ambiguous",
                    "reason": format!("{query:?} matches none or several choices; be specific"),
                })),
            )
        })?;
        for (group_index, group) in offer.groups.iter().enumerate() {
            let selected = match target {
                ChoiceTarget::GroupOption { group, option } if group == group_index => Some(option),
                _ => group
                    .options
                    .iter()
                    .position(|option| option.option_id == "no")
                    .or_else(|| (!group.options.is_empty()).then_some(0)),
            }
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::OK,
                    Json(json!({"status": "unavailable", "reason": "this offer has an empty choice group"})),
                )
            })?;
            let option = group.options.get(selected).ok_or_else(|| {
                (
                    axum::http::StatusCode::OK,
                    Json(json!({"status": "unavailable", "reason": "this offer's choice is no longer available"})),
                )
            })?;
            submit.action_choices.push(rpc::JourneyChoice {
                group_id: group.group_id.clone(),
                option_id: option.option_id.clone(),
            });
        }
        submit.destination = match target {
            ChoiceTarget::Destination(index) => offer
                .destinations
                .get(index)
                .map(|destination| destination.destination)
                .ok_or_else(|| {
                    (
                        axum::http::StatusCode::OK,
                        Json(json!({"status": "unavailable", "reason": "this destination is no longer available"})),
                    )
                })?,
            _ => offer
                .destinations
                .first()
                .map_or(rpc::HarborDestination::Unspecified as i32, |destination| {
                    destination.destination
                }),
        };
        return Ok(());
    }
    if action == parse::PlayAction::Search {
        let thing = body
            .get("option")
            .and_then(Value::as_str)
            .filter(|thing| !thing.trim().is_empty() && thing.len() <= 64)
            .ok_or_else(|| bad_request("search needs {\"option\": \"<thing>\"}"))?;
        submit.action_choices = vec![rpc::JourneyChoice {
            group_id: "finding".to_owned(),
            option_id: thing.to_owned(),
        }];
        return Ok(());
    }
    if action == parse::PlayAction::ChooseInn {
        let destination = offer
            .destinations
            .first()
            .map(|option| option.destination)
            .unwrap_or(rpc::HarborDestination::HarborInn as i32);
        submit.destination = destination;
        return Ok(());
    }
    if !action.needs_option() {
        return Ok(());
    }
    let group = offer.groups.first().ok_or_else(|| {
        (
            axum::http::StatusCode::OK,
            Json(json!({"status": "unavailable", "reason": "this offer carries no selectable options"})),
        )
    })?;
    let options: Vec<&str> = group
        .options
        .iter()
        .map(|option| option.option_id.as_str())
        .collect();
    let labels: Vec<&str> = group
        .options
        .iter()
        .map(|option| option.label.as_str())
        .collect();
    let option_id = if action == parse::PlayAction::Move
        && let Some(cell) = body.get("cell").and_then(Value::as_str)
    {
        let parsed = parse::parse_cell(cell).map_err(bad_request)?;
        parsed.option_id()
    } else if let Some(option) = body.get("option").and_then(Value::as_str) {
        if options.contains(&option) {
            option.to_owned()
        } else {
            unique_label_match(&labels, &group.options, option)?
        }
    } else if action == parse::PlayAction::Attack {
        attack_option(body, &group.options)?
    } else if matches!(
        action,
        parse::PlayAction::StabiliseWithHealersKit | parse::PlayAction::StabiliseMedicine
    ) {
        target_option(body, &group.options)?
    } else {
        return Err(bad_request(format!(
            "this action needs a selection; options: {}",
            labels.join(", ")
        )));
    };
    submit.action_choices = vec![rpc::JourneyChoice {
        group_id: group.group_id.clone(),
        option_id,
    }];
    Ok(())
}

fn target_option(
    body: &Value,
    options: &[rpc::JourneyOption],
) -> Result<String, (axum::http::StatusCode, Json<Value>)> {
    let target = body
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("stabilisation needs {\"target\": \"<hero>\"}"))?;
    let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
    unique_label_match(&labels, options, target)
}

fn unique_label_match(
    labels: &[&str],
    options: &[rpc::JourneyOption],
    query: &str,
) -> Result<String, (axum::http::StatusCode, Json<Value>)> {
    match parse::match_index(labels, query) {
        Some(index) => Ok(options[index].option_id.clone()),
        None => Err((
            axum::http::StatusCode::OK,
            Json(json!({
                "status": "ambiguous",
                "reason": format!("{query:?} matches none or several options; be specific"),
                "options": labels,
            })),
        )),
    }
}

/// Picks the attack option naming `target` (and `weapon` when given),
/// preferring the plain action spend without mastery or Savage Attacker extras.
fn attack_option(
    body: &Value,
    options: &[rpc::JourneyOption],
) -> Result<String, (axum::http::StatusCode, Json<Value>)> {
    let target = body
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("attack needs {\"target\": \"<foe>\"}"))?;
    let weapon = body.get("weapon").and_then(Value::as_str).unwrap_or("");
    let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
    let mut matches: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, label)| {
            parse::names_match(label, target)
                && (weapon.is_empty() || parse::names_match(label, weapon))
        })
        .map(|(index, _)| index)
        .collect();
    if matches.is_empty() {
        return Err((
            axum::http::StatusCode::OK,
            Json(json!({
                "status": "unavailable",
                "reason": format!("no offered attack names {target:?}"),
                "options": labels,
            })),
        ));
    }
    matches.sort_by_key(|index| {
        let label = labels[*index];
        (
            u8::from(label.contains('·')),
            u8::from(label.contains("bonus") || label.contains("Nick")),
        )
    });
    let plain: Vec<usize> = matches
        .iter()
        .copied()
        .filter(|index| !labels[*index].contains('·'))
        .collect();
    if plain.len() == 1 || (weapon.is_empty() && plain.len() > 1) {
        let many = plain.len() > 1;
        if many {
            // Several plain spends still name this target (melee and throw, for
            // example): take the first rather than guess, and say so.
            return Ok(options[plain[0]].option_id.clone());
        }
        return Ok(options[plain[0]].option_id.clone());
    }
    if plain.is_empty() && matches.len() == 1 {
        return Ok(options[matches[0]].option_id.clone());
    }
    Err((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "ambiguous",
            "reason": format!("{target:?} matches several attacks; name the weapon too"),
            "options": matches.iter().map(|index| labels[*index]).collect::<Vec<_>>(),
        })),
    ))
}

/// `POST /dev/say {"hero": "Brynn", "text": "..."}`: speaks one line to the
/// scene through the owner's dialogue path, exactly as a typed phone line.
/// With providers off the authored fallback answers. A clarification ticket
/// (confirmation-required) is reported, not auto-confirmed: confirming a
/// choice is what `act` is for.
pub(super) async fn say(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let hero = body
        .get("hero")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("say needs {\"hero\": \"Brynn\", \"text\": \"...\"}"))?;
    let text = body
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("say needs {\"hero\": \"Brynn\", \"text\": \"...\"}"))?;
    if text.is_empty() || text.len() > 4096 {
        return Err(bad_request("text must be 1 to 4096 characters"));
    }
    let context = match body
        .get("context")
        .and_then(Value::as_str)
        .unwrap_or("question")
    {
        "question" => rpc::DialogueContext::Question,
        "social" => rpc::DialogueContext::Social,
        "plan" => rpc::DialogueContext::PlanOnly,
        "meta" => rpc::DialogueContext::Meta,
        "joke" => rpc::DialogueContext::Joke,
        "clarify" => rpc::DialogueContext::Clarify,
        _ => {
            return Err(bad_request(
                "context must be question, social, plan, meta, joke or clarify",
            ));
        }
    };
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, hero).map_err(bad_request)?
    };
    let credential = {
        let seats = state.seats.lock().await;
        seats.seats[index]
            .credential
            .ok_or_else(|| bad_request(format!("seat{} has not joined yet", index + 1)))?
    };
    let (revision, _) = seat_view(&state, index).await?;
    let scope = state.seats.lock().await;
    let (session_id, run_id) = super::scope_of(&scope)?;
    drop(scope);
    let input: [u8; 16] = random_array().await?;
    let request = rpc::RawDialogueRequest {
        session_id: Some(rpc::SessionId {
            value: Some(session_id),
        }),
        run_id: Some(rpc::RunId {
            value: Some(run_id),
        }),
        observed_revision: Some(revision),
        input_id: Some(rpc::OperationId {
            value: Some(input.to_vec()),
        }),
        text: text.to_owned(),
        context: context as i32,
        finality: rpc::DialogueFinality::Final as i32,
        offer_id: String::new(),
    };
    let started = std::time::Instant::now();
    let answer = state
        .service
        .submit_line(credential, request)
        .await
        .map_err(|status| failed(status.message()))?;
    let result = match answer {
        super::super::dialogue::DialogueReply::Raw(response, npc) => {
            let npc_reply = match npc {
                Some(context) => {
                    let shown = state.service.npc_voice.display_line(&context).await;
                    context.publish_typed_line(&state.service.table_log, &shown);
                    let _thinking = state.service.table_log.reply_pending(&context.speaker);
                    state
                        .service
                        .npc_voice
                        .converse(context, &state.service.table_log)
                        .await
                        .unwrap_or_default()
                }
                None => response.npc_reply.clone(),
            };
            Ok((
                axum::http::StatusCode::OK,
                Json(json!({
                    "status": "said",
                    "disposition": disposition_name(response.disposition),
                    "npc_reply": npc_reply,
                    "confirmation_required": response.confirmation_required,
                    "input_id":response.input_id.as_ref().and_then(|id|id.value.as_ref()).map(|bytes|crate::gameplay::journey::hex(bytes)),
                    "confirmation_token":crate::gameplay::journey::hex(&response.confirmation_token),
                    "revision": super::revision_json(response.revision.as_ref()),
                })),
            ))
        }
        _ => Err(failed("dialogue response binding invalid")),
    };
    super::super::actor::record_audit_line(
        "gameplay.exchange_timing",
        format!(
            "STORY exchange id={} total_ms={} status={}",
            super::super::journey::hex(&input),
            started.elapsed().as_millis(),
            if result.is_ok() {
                "completed"
            } else {
                "failed"
            },
        ),
    );
    result
}

fn disposition_name(disposition: i32) -> &'static str {
    match rpc::DialogueDisposition::try_from(disposition) {
        Ok(rpc::DialogueDisposition::Action) => "action",
        Ok(rpc::DialogueDisposition::Question) => "question",
        Ok(rpc::DialogueDisposition::Social) => "social",
        Ok(rpc::DialogueDisposition::PlanOnly) => "plan-only",
        Ok(rpc::DialogueDisposition::Meta) => "meta",
        Ok(rpc::DialogueDisposition::Joke) => "joke",
        Ok(rpc::DialogueDisposition::Clarify) => "clarify",
        Ok(rpc::DialogueDisposition::Rejected) => "rejected",
        _ => "unknown",
    }
}

#[cfg(test)]
mod offer_tests {
    use super::*;

    fn player_view(offers: Vec<rpc::GameplayActionOffer>) -> rpc::ViewMessage {
        rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    offers,
                    ..Default::default()
                },
            )),
        }
    }

    fn offer(label: &str) -> rpc::GameplayActionOffer {
        rpc::GameplayActionOffer {
            offer_id: format!("offer-{label}"),
            action_kind: rpc::GameplayActionKind::EndTurn as i32,
            label: label.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn player_offers_projects_labels_groups_and_destinations() {
        let view = player_view(vec![offer("End turn")]);
        let offers = player_offers(&view).expect("player view reads");
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].offer_id, "offer-End turn");
        assert_eq!(
            offers[0].action_kind,
            rpc::GameplayActionKind::EndTurn as i32
        );
    }

    #[test]
    fn player_offers_accepts_the_transient_empty_view_shape() {
        // Right after a commit the owner can serve a player view with no
        // offers yet; seat_offers re-reads that shape, so mapping it must not
        // fail here.
        let view = player_view(Vec::new());
        let offers = player_offers(&view).expect("empty player view reads");
        assert!(offers.is_empty());
    }

    #[test]
    fn player_offers_refuses_a_display_audience() {
        let view = rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView::default(),
            )),
        };
        assert!(player_offers(&view).is_err());
    }

    #[test]
    fn generic_choose_resolves_offered_choice_ids_and_labels() {
        let offer = Offer {
            offer_id: "rest-offer".to_owned(),
            action_kind: rpc::GameplayActionKind::ShortRest as i32,
            label: "Short Rest".to_owned(),
            groups: vec![rpc::JourneyGroup {
                group_id: "hit-point-die".to_owned(),
                options: vec![
                    rpc::JourneyOption {
                        option_id: "spend".to_owned(),
                        label: "Spend one Hit Point Die".to_owned(),
                        ..Default::default()
                    },
                    rpc::JourneyOption {
                        option_id: "keep".to_owned(),
                        label: "Keep it".to_owned(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            destinations: Vec::new(),
        };
        assert_eq!(
            unique_offer_match(std::slice::from_ref(&offer), "keep"),
            Some(0)
        );
        assert_eq!(
            unique_offer_match(std::slice::from_ref(&offer), "Spend one Hit Point Die"),
            Some(0)
        );
        let mut submit = rpc::SubmitActionRequest::default();
        fill_action_choices(
            &json!({"option": "Keep it"}),
            parse::PlayAction::Choose,
            &offer,
            &mut submit,
        )
        .unwrap();
        assert_eq!(submit.action_choices.len(), 1);
        assert_eq!(submit.action_choices[0].group_id, "hit-point-die");
        assert_eq!(submit.action_choices[0].option_id, "keep");
    }

    #[test]
    fn search_option_selects_the_offered_finding_srd_521_page_189() {
        // SRD 5.2.1 p. 189: Study uses an Intelligence check; the server derives the
        // finding's listed skill from the selected scene option.
        let offer = Offer {
            offer_id: "look-around".to_owned(),
            action_kind: rpc::GameplayActionKind::ExamineHarborSeal as i32,
            label: "Look around".to_owned(),
            groups: vec![rpc::JourneyGroup {
                group_id: "finding".to_owned(),
                label: "Look around".to_owned(),
                options: vec![rpc::JourneyOption {
                    option_id: "errand-truth".to_owned(),
                    label: "Errand Truth".to_owned(),
                    ..Default::default()
                }],
            }],
            destinations: Vec::new(),
        };
        let mut submit = rpc::SubmitActionRequest::default();

        fill_action_choices(
            &json!({"option": "errand-truth"}),
            parse::PlayAction::Search,
            &offer,
            &mut submit,
        )
        .unwrap();

        assert_eq!(
            submit.action_choices,
            [rpc::JourneyChoice {
                group_id: "finding".to_owned(),
                option_id: "errand-truth".to_owned(),
            }]
        );
    }

    #[test]
    fn stabilisation_target_selects_the_named_companion() {
        // The target is a dying 0 HP creature under SRD 5.2.1 p18 or p97.
        let options = vec![rpc::JourneyOption {
            option_id: "stabilise:hero-2".to_owned(),
            label: "Stabilise Sigrun with the Healer's Kit (9 uses left)".to_owned(),
            ..Default::default()
        }];

        assert_eq!(
            target_option(&json!({"target": "Sigrun"}), &options).unwrap(),
            "stabilise:hero-2"
        );
        assert!(target_option(&json!({}), &options).is_err());
    }
}

#[cfg(test)]
mod explicit_choice_tests {
    use super::*;

    #[test]
    fn any_offer_can_take_explicit_values_for_every_group() {
        let offer = Offer {
            offer_id: "offer".into(),
            action_kind: rpc::GameplayActionKind::GreatswordAttack as i32,
            label: "Attack".into(),
            destinations: Vec::new(),
            groups: vec![rpc::JourneyGroup {
                group_id: "weapon-mastery".into(),
                label: "Mastery".into(),
                options: ["yes", "no"]
                    .iter()
                    .map(|id| rpc::JourneyOption {
                        option_id: (*id).into(),
                        label: (*id).into(),
                        ..Default::default()
                    })
                    .collect(),
            }],
        };
        let mut submit = rpc::SubmitActionRequest::default();
        fill_action_choices(
            &json!({"option":"offer", "choices":{"weapon-mastery":"yes"}}),
            parse::PlayAction::Choose,
            &offer,
            &mut submit,
        )
        .unwrap();
        assert_eq!(submit.action_choices[0].option_id, "yes");
    }
}

#[cfg(test)]
mod creation_selection_tests {
    use super::*;
    #[test]
    fn creation_overrides_the_selected_menu_and_keeps_other_required_groups() {
        let choices =
            selected_creation_choices(&json!({"choices":{"alignment":"neutral-good"}})).unwrap();
        assert_eq!(
            choices
                .iter()
                .filter(|choice| choice.group_id == "alignment")
                .count(),
            1
        );
        assert_eq!(
            choices
                .iter()
                .find(|choice| choice.group_id == "alignment")
                .unwrap()
                .option_id,
            "neutral-good"
        );
        assert_eq!(choices.len(), creation_choices().len());
        assert!(choice_values(Some(&json!({"alignment":5}))).is_err());
    }
    #[test]
    fn identity_menu_choices_keep_the_selected_presentation_and_pronouns() {
        let identity = selected_identity(
            &json!({"identity_choices":{"presentation":"feminine","pronouns":"she-her"}}),
        )
        .unwrap()
        .unwrap();
        let appearance = identity.appearance.unwrap();
        assert_eq!(appearance.presentation, "feminine");
        assert_eq!(appearance.pronouns, "she-her");
        assert!(selected_identity(&json!({"identity":{},"identity_choices":{}})).is_err());
    }
}
