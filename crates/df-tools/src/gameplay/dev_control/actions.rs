//! Game actions for the control surface: every command plays through the
//! session owner's admission, validation, rules and commit path as a seated
//! player. Nothing here writes game state directly.

use axum::{extract::State, http::HeaderMap, response::Json};
use df_protocol::common as rpc;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::oneshot;

use super::{
    Outcome, Seats, Shared, authorized, bad_request, failed, owner_submit_action, seat_view,
};
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
        Some(rpc::join_room_response::Outcome::Refused(refused)) => {
            let joined_here = state.seats.lock().await.seats[index].member.is_some();
            Ok((
                axum::http::StatusCode::OK,
                Json(json!({
                    "status": "refused",
                    "code": super::rejection_name(refused.code),
                    "reason": join_refusal_reason(refused.code, index, joined_here),
                })),
            ))
        }
        None => Err(failed("empty join answer")),
    }
}

/// Says in plain words why the room refused a join to this seat. A seat this control
/// did not join while the room is full is held by a phone, and the refusal names the
/// one way to take it back.
fn join_refusal_reason(code: i32, index: usize, joined_here: bool) -> String {
    let seat = index + 1;
    match rpc::RejectionCode::try_from(code) {
        Ok(rpc::RejectionCode::ResourceMissing) if joined_here => format!(
            "seat {seat} is already joined through this control, and the room has no free place for another hero"
        ),
        Ok(rpc::RejectionCode::ResourceMissing) => format!(
            "seat {seat} is held by a phone; use dev reclaim {seat} once it has been away 30 s"
        ),
        Ok(rpc::RejectionCode::StaleOffer) => {
            "the room has left its join phase, so no hero can join it now".to_owned()
        }
        Ok(rpc::RejectionCode::InvalidSelection) => {
            "the room code does not match this room".to_owned()
        }
        Ok(rpc::RejectionCode::RecoveryRequired) => {
            "the room is recovering from an unresolved commit; retry the join after it recovers"
                .to_owned()
        }
        Ok(rpc::RejectionCode::OperationConflict) => {
            "this join conflicts with an earlier join operation; retry the join".to_owned()
        }
        Ok(rpc::RejectionCode::OperationExpired) => {
            "this join operation expired; retry the join".to_owned()
        }
        _ => format!(
            "the room refused the join ({})",
            super::rejection_name(code)
        ),
    }
}

/// `POST /dev/reclaim {"seat":"1"}` uses the room-code-only phone admission path.
/// It reads no player view first: doing so would keep the abandoned credential active.
/// A seat this control joined is taken back by its own hero. A seat a phone joined is
/// taken back by the one away hero that no other control seat holds. Both follow the
/// owner's away report, so the 30 s rule is the one the phones' "Take this hero back" uses.
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
    let pending = state.seats.lock().await.seats[index].reclaim;
    let pending = match pending {
        Some(pending) => pending,
        None => {
            let away = away_heroes(&state).await?;
            let token = {
                let seats = state.seats.lock().await;
                reclaim_token(index, &seats, &away)
            };
            let token = match token {
                Ok(token) => token,
                Err(reason) => return reclaim_refusal(reason),
            };
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
        Err(reason) => reclaim_refusal(format!("{reason:?}")),
    }
}

/// The owner's away report: each hero whose phone has been silent for 30 s, with the
/// token a reclaim presents. The owner decides who is away; the control only asks.
async fn away_heroes(
    state: &Shared,
) -> Result<Vec<super::super::room::AwayHero>, (axum::http::StatusCode, Json<Value>)> {
    let (reply, wait) = oneshot::channel();
    state
        .service
        .actor
        .try_submit(super::super::actor::Call::AwaySeats {
            room_code: super::super::journey::room_code(),
            reply,
        })
        .map_err(|_| failed("room actor queue full"))?;
    tokio::time::timeout(super::super::timing::active().reply, wait)
        .await
        .map_err(|_| failed("away seat read deadline"))?
        .map_err(|_| failed("room owner stopped"))?
        .map_err(|status| failed(status.message()))
}

/// The reclaim token for one control seat. A seat this control joined names its own
/// hero; when that hero is not away the token stays zero and the owner refuses it. A seat
/// it did not join is held by a phone, so the candidate is the one away hero that the
/// other control seat does not hold. Zero or several candidates are refused, not guessed.
fn reclaim_token(
    index: usize,
    seats: &Seats,
    away: &[super::super::room::AwayHero],
) -> Result<[u8; 32], String> {
    if let Some(member) = seats.seats[index].member.as_deref() {
        return Ok(away
            .iter()
            .find(|hero| hero.member.as_slice() == member)
            .map_or([0; 32], |hero| hero.token));
    }
    let other = seats.seats[1 - index].member.as_deref();
    let candidates: Vec<_> = away
        .iter()
        .filter(|hero| Some(hero.member.as_slice()) != other)
        .collect();
    match candidates.as_slice() {
        [hero] => Ok(hero.token),
        [] => Err(format!(
            "seat {} is held by a phone, and no hero has been away for 30 s to take back; the phone must stay silent for 30 s first",
            index + 1
        )),
        _ => Err(format!(
            "several heroes are away and this control cannot tell which one holds seat {}; nothing was reclaimed",
            index + 1
        )),
    }
}

/// A refusal that changed no seat.
fn reclaim_refusal(reason: String) -> Outcome {
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "refused", "reason": reason,
            "message": "No seat changed. Reclaim is accepted only for an away seat after 30 seconds; an active phone is never displaced."
        })),
    ))
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
    seat: usize,
) -> Result<Vec<rpc::JourneyChoice>, (axum::http::StatusCode, Json<Value>)> {
    let offer =
        super::super::wire::creation_offer().map_err(|_| failed("creation offer unavailable"))?;
    let overrides = choice_values(body.get("choices"))?;
    if overrides.iter().any(|choice| {
        !offer
            .groups
            .iter()
            .any(|group| group.group_id == choice.group_id)
    }) {
        return Err(bad_request("choice group is not currently offered"));
    }
    let mut choices = Vec::with_capacity(offer.groups.len());
    for group in &offer.groups {
        let requested = overrides
            .iter()
            .find(|choice| choice.group_id == group.group_id);
        let available = group
            .options
            .iter()
            .filter(|option| creation_dependencies_met(option, &choices))
            .filter(|option| {
                !duplicates_creation_choice(&group.group_id, &option.option_id, &choices)
            })
            .collect::<Vec<_>>();
        let selected = if let Some(requested) = requested {
            group
                .options
                .iter()
                .find(|option| option.option_id == requested.option_id)
                .filter(|option| creation_dependencies_met(option, &choices))
                .ok_or_else(|| {
                    bad_request("choice is not currently offered or its dependency is unmet")
                })?
        } else {
            let index = default_creation_option_index(&group.group_id, seat, available.len());
            available
                .get(index)
                .or_else(|| available.first())
                .copied()
                .ok_or_else(|| bad_request("creation group has no valid default"))?
        };
        if duplicates_creation_choice(&group.group_id, &selected.option_id, &choices) {
            return Err(bad_request("language and skill choices must be distinct"));
        }
        choices.push(rpc::JourneyChoice {
            group_id: group.group_id.clone(),
            option_id: selected.option_id.clone(),
        });
    }
    Ok(choices)
}

fn creation_dependencies_met(option: &rpc::JourneyOption, selected: &[rpc::JourneyChoice]) -> bool {
    if option.requires_group.is_empty() {
        return true;
    }
    selected
        .iter()
        .find(|choice| choice.group_id == option.requires_group)
        .is_some_and(|choice| {
            option.requires_options.is_empty()
                || option.requires_options.contains(&choice.option_id)
        })
}

fn duplicates_creation_choice(group: &str, option: &str, selected: &[rpc::JourneyChoice]) -> bool {
    let category = match group {
        "language-1" | "language-2" => "language",
        "skill-1" | "skill-2" => "skill",
        _ => return false,
    };
    selected.iter().any(|choice| {
        choice.option_id == option
            && match category {
                "language" => matches!(choice.group_id.as_str(), "language-1" | "language-2"),
                "skill" => matches!(choice.group_id.as_str(), "skill-1" | "skill-2"),
                _ => false,
            }
    })
}

fn default_creation_option_index(group: &str, seat: usize, option_count: usize) -> usize {
    if option_count <= 1 {
        return 0;
    }
    let seat_offset = seat.saturating_sub(1);
    if matches!(group, "build-package" | "portrait") {
        return seat_offset.min(option_count - 1);
    }
    if !matches!(group, "language-1" | "language-2" | "skill-1" | "skill-2") {
        return 0;
    }
    (seat_offset.saturating_mul(2)).min(option_count - 1)
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
        super::seat_credential(&seats, index)?
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
            choices: selected_creation_choices(&body, index + 1)?,
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
    let credential = {
        let seats = state.seats.lock().await;
        super::seat_credential(&seats, index)?
    };
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
        let credential = {
            let seats = state.seats.lock().await;
            super::seat_credential(&seats, index)?
        };
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
        super::seat_credential(&seats, index)?
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
    // A scene assault goes through the one entry point that starts a scene fight, so the
    // bystanders' stances and the consequences are the same as for a typed attack.
    if action_kind == rpc::GameplayActionKind::SceneAssault {
        return super::owner_scene_assault(&state, credential, submit).await;
    }
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
    let credential = {
        let seats = state.seats.lock().await;
        super::seat_credential(&seats, index)?
    };
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
        parse::PlayAction::StabiliseWithHealersKit
            | parse::PlayAction::StabiliseMedicine
            | parse::PlayAction::SceneAssault
    ) {
        target_option(body, &group.options)?
    } else if action == parse::PlayAction::HelpAttack {
        help_attack_option(body, &group.options)?
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
    if action == parse::PlayAction::Attack
        && let Some(value) = body.get("knockout")
    {
        let text = value
            .as_str()
            .ok_or_else(|| bad_request("knockout must be yes or no"))?;
        let option_id = parse::parse_knockout_choice(text).map_err(bad_request)?;
        if !offer
            .groups
            .iter()
            .any(|group| group.group_id == "knock-out")
        {
            return Err(bad_request(
                "knock-out choice is not offered for this attack",
            ));
        }
        submit.action_choices.push(rpc::JourneyChoice {
            group_id: "knock-out".to_owned(),
            option_id: option_id.to_owned(),
        });
    }
    Ok(())
}

fn target_option(
    body: &Value,
    options: &[rpc::JourneyOption],
) -> Result<String, (axum::http::StatusCode, Json<Value>)> {
    let target = body
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("this action needs {\"target\": \"<name>\"}"))?;
    let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
    unique_label_match(&labels, options, target)
}

/// Picks the offered Help (Attack) pair whose label names both the ally and the foe.
/// Zero or several matching pairs are refused rather than guessed.
fn help_attack_option(
    body: &Value,
    options: &[rpc::JourneyOption],
) -> Result<String, (axum::http::StatusCode, Json<Value>)> {
    let usage = || bad_request("help-attack needs {\"ally\": \"<hero>\", \"target\": \"<foe>\"}");
    let ally = body.get("ally").and_then(Value::as_str).ok_or_else(usage)?;
    let target = body
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(usage)?;
    let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
    let matches: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, label)| parse::names_match(label, ally) && parse::names_match(label, target))
        .map(|(index, _)| index)
        .collect();
    match matches.as_slice() {
        [index] => Ok(options[*index].option_id.clone()),
        [] => Err((
            axum::http::StatusCode::OK,
            Json(json!({
                "status": "unavailable",
                "reason": format!("no offered help names both {ally:?} and {target:?}"),
                "options": labels,
            })),
        )),
        _ => Err((
            axum::http::StatusCode::OK,
            Json(json!({
                "status": "ambiguous",
                "reason": format!("{ally:?} and {target:?} match several help offers; be specific"),
                "options": matches.iter().map(|index| labels[*index]).collect::<Vec<_>>(),
            })),
        )),
    }
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
/// Omitted context uses the normal classifier/execution path. With providers off the authored fallback answers. A clarification ticket
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
        .unwrap_or("unspecified")
    {
        "unspecified" => rpc::DialogueContext::Unspecified,
        "question" => rpc::DialogueContext::Question,
        "social" => rpc::DialogueContext::Social,
        "plan" => rpc::DialogueContext::PlanOnly,
        "meta" => rpc::DialogueContext::Meta,
        "joke" => rpc::DialogueContext::Joke,
        "clarify" => rpc::DialogueContext::Clarify,
        _ => {
            return Err(bad_request(
                "context must be unspecified, question, social, plan, meta, joke or clarify",
            ));
        }
    };
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, hero).map_err(bad_request)?
    };
    let credential = {
        let seats = state.seats.lock().await;
        super::seat_credential(&seats, index)?
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
    let super::super::dialogue::DialogueReply::Raw(response, npc) = answer else {
        return Err(failed("dialogue response binding invalid"));
    };
    let completion = state
        .service
        .complete_dialogue_response(credential, response, npc)
        .await
        .map_err(|status| failed(status.message()))?;
    let response = completion.response;
    let npc_outcome = completion.npc_outcome;
    let npc_reply = response.npc_reply.clone();
    let result = Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": match npc_outcome {
                Some(super::super::npc_reply::ExchangeOutcome::Withheld) => "withheld",
                Some(super::super::npc_reply::ExchangeOutcome::Unavailable) => "unavailable",
                Some(super::super::npc_reply::ExchangeOutcome::Spoken) | None => "said",
            },
            "disposition": disposition_name(response.disposition),
            "npc_reply": npc_reply,
            "npc_outcome": npc_outcome.map(|outcome| outcome.label()).unwrap_or("no_reply"),
            "npc_fallback": completion.npc_fallback,
            "npc_event_proposal": completion.npc_event_proposal,
            "npc_event_result": completion.npc_event_result,
            "npc_actor_id": response.npc_actor_id,
            "npc_speaker": response.npc_speaker,
            "confirmation_required": response.confirmation_required,
            "input_id":response.input_id.as_ref().and_then(|id|id.value.as_ref()).map(|bytes|crate::gameplay::journey::hex(bytes)),
            "confirmation_token":crate::gameplay::journey::hex(&response.confirmation_token),
            "revision": super::revision_json(response.revision.as_ref()),
        })),
    ));
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

    fn help_option(id: &str, label: &str) -> rpc::JourneyOption {
        rpc::JourneyOption {
            option_id: id.to_owned(),
            label: label.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn help_attack_names_pick_the_one_offered_ally_and_foe_pair() {
        let options = [
            help_option("2-5", "Help Brynn attack Goblin"),
            help_option("3-5", "Help Neris attack Goblin"),
            help_option("2-6", "Help Brynn attack Wolf"),
        ];
        let pick = |body: Value| help_attack_option(&body, &options).map_err(|(_, json)| json.0);
        assert_eq!(
            pick(json!({"ally": "Neris", "target": "Goblin"})).expect("one pair matches"),
            "3-5"
        );
        // Ambiguous or unoffered pairs are refused, never guessed.
        let ambiguous = pick(json!({"ally": "Brynn", "target": "o"})).expect_err("two pairs match");
        assert_eq!(ambiguous["status"], "ambiguous");
        let absent = pick(json!({"ally": "Neris", "target": "Wolf"})).expect_err("no such pair");
        assert_eq!(absent["status"], "unavailable");
        assert!(pick(json!({"ally": "Brynn"})).is_err());
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

    #[test]
    fn attack_submits_knockout_choice_only_when_given() {
        let offer = Offer {
            offer_id: "attack".into(),
            action_kind: rpc::GameplayActionKind::GreatswordAttack as i32,
            label: "Attack".into(),
            destinations: Vec::new(),
            groups: vec![
                rpc::JourneyGroup {
                    group_id: "tactical-choice".into(),
                    options: vec![rpc::JourneyOption {
                        option_id: "attack-foe-greataxe".into(),
                        label: "Attack Foe with Greataxe".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                rpc::JourneyGroup {
                    group_id: "knock-out".into(),
                    options: ["yes", "no"]
                        .into_iter()
                        .map(|option_id| rpc::JourneyOption {
                            option_id: option_id.into(),
                            label: option_id.into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
            ],
        };

        for (body, expected) in [
            (json!({"target":"Foe", "weapon":"Greataxe"}), None),
            (
                json!({"target":"Foe", "weapon":"Greataxe", "knockout":"no"}),
                Some("no"),
            ),
            (
                json!({"target":"Foe", "weapon":"Greataxe", "knockout":"yes"}),
                Some("yes"),
            ),
        ] {
            let mut submit = rpc::SubmitActionRequest::default();
            fill_action_choices(&body, parse::PlayAction::Attack, &offer, &mut submit).unwrap();
            assert_eq!(
                submit
                    .action_choices
                    .iter()
                    .find(|choice| choice.group_id == "knock-out")
                    .map(|choice| choice.option_id.as_str()),
                expected
            );
        }

        let mut submit = rpc::SubmitActionRequest::default();
        assert!(
            fill_action_choices(
                &json!({"target":"Foe", "weapon":"Greataxe", "knockout":"lethal"}),
                parse::PlayAction::Attack,
                &offer,
                &mut submit,
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod creation_selection_tests {
    use super::*;
    #[test]
    fn creation_overrides_the_selected_menu_and_keeps_other_required_groups() {
        let choices =
            selected_creation_choices(&json!({"choices":{"alignment":"neutral-good"}}), 1).unwrap();
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
        assert_eq!(
            choices.len(),
            super::super::super::wire::creation_offer()
                .unwrap()
                .groups
                .len()
        );
        assert!(choice_values(Some(&json!({"alignment":5}))).is_err());
    }

    #[test]
    fn creation_defaults_include_accepted_portraits_and_validate_for_both_seats() {
        let offer = super::super::super::wire::creation_offer().unwrap();
        let first = selected_creation_choices(&json!({}), 1).unwrap();
        let second = selected_creation_choices(&json!({}), 2).unwrap();
        assert_eq!(first.len(), offer.groups.len());
        assert_eq!(second.len(), offer.groups.len());
        assert_eq!(
            first
                .iter()
                .map(|choice| choice.group_id.as_str())
                .collect::<Vec<_>>(),
            offer
                .groups
                .iter()
                .map(|group| group.group_id.as_str())
                .collect::<Vec<_>>()
        );
        for stale_group in ["array", "equipment", "style-masteries"] {
            if !offer
                .groups
                .iter()
                .any(|group| group.group_id == stale_group)
            {
                assert!(!first.iter().any(|choice| choice.group_id == stale_group));
            }
        }
        for choices in [&first, &second] {
            let borrowed = choices
                .iter()
                .filter(|choice| {
                    crate::gameplay::hero_identity::is_character_pack_validation_choice(
                        &choice.group_id,
                    )
                })
                .map(|choice| (choice.group_id.as_str(), choice.option_id.as_str()))
                .collect::<Vec<_>>();
            crate::gameplay::pack::validate_character("Brakka", &borrowed)
                .expect("the current creation offer has valid defaults");
            let portrait = choices
                .iter()
                .find(|choice| choice.group_id == "portrait")
                .expect("null-identity creation needs an offered portrait");
            assert!(
                crate::gameplay::hero_identity::accepted_visual_dossier(&portrait.option_id)
                    .is_some(),
                "{} is an accepted quickstart portrait",
                portrait.option_id
            );
        }
        for (group_a, group_b) in [("language-1", "language-2"), ("skill-1", "skill-2")] {
            fn selected<'a>(choices: &'a [rpc::JourneyChoice], group: &str) -> Option<&'a str> {
                choices
                    .iter()
                    .find(|choice| choice.group_id == group)
                    .map(|choice| choice.option_id.as_str())
            }
            assert_ne!(selected(&first, group_a), selected(&first, group_b));
            assert_ne!(selected(&second, group_a), selected(&second, group_b));
        }
        if let (Some(first_package), Some(second_package)) = (
            first
                .iter()
                .find(|choice| choice.group_id == "build-package"),
            second
                .iter()
                .find(|choice| choice.group_id == "build-package"),
        ) {
            assert_ne!(first_package.option_id, second_package.option_id);
        }
    }

    #[test]
    fn non_tactical_shop_creation_offers_and_submits_accepted_hero_portraits() {
        let loaded = crate::gameplay::pack::load_from_bytes(
            include_bytes!("../../../../../content/merchant-shop/pack.json"),
            include_bytes!("../../../../../content/merchant-shop/tuning.json"),
        )
        .expect("merchant shop pack loads");
        crate::gameplay::pack::with_test_pack(loaded, || {
            let offer = super::super::super::wire::creation_offer().expect("creation offer");
            let portrait = offer
                .groups
                .iter()
                .find(|group| group.group_id == "portrait")
                .expect("quickstart portrait group is offered");
            assert!(
                offer
                    .groups
                    .iter()
                    .all(|group| group.group_id != "build-package")
            );
            assert_eq!(portrait.options.len(), 2);
            assert!(
                portrait
                    .options
                    .iter()
                    .all(|option| option.requires_group.is_empty())
            );

            for (seat, expected) in [(1, "portrait-hero-1"), (2, "portrait-hero-2")] {
                let choices = selected_creation_choices(&json!({}), seat).unwrap();
                let selected = choices
                    .iter()
                    .find(|choice| choice.group_id == "portrait")
                    .expect("null identity receives an accepted portrait");
                assert_eq!(selected.option_id, expected);
                let pack_choices = choices
                    .iter()
                    .filter(|choice| {
                        crate::gameplay::hero_identity::is_character_pack_validation_choice(
                            &choice.group_id,
                        )
                    })
                    .map(|choice| (choice.group_id.as_str(), choice.option_id.as_str()))
                    .collect::<Vec<_>>();
                crate::gameplay::pack::validate_character("Brakka", &pack_choices)
                    .expect("portrait metadata remains validated with the build");
            }
        });
    }

    #[test]
    fn creation_rejects_a_group_that_is_no_longer_offered() {
        let answer = selected_creation_choices(
            &json!({
                "choices": {"array": "stalwart"}
            }),
            1,
        );
        if super::super::super::wire::creation_offer()
            .unwrap()
            .groups
            .iter()
            .all(|group| group.group_id != "array")
        {
            assert!(answer.is_err());
        }
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

#[cfg(test)]
mod phone_seat_tests {
    use super::super::Seat;
    use super::*;
    use crate::gameplay::room::AwayHero;

    fn bound_seat(member: u8) -> Seat {
        Seat {
            reclaim: None,
            credential: Some([member; 32]),
            member: Some(vec![member; 16]),
            hero: None,
        }
    }

    fn away(member: u8, token: u8) -> AwayHero {
        AwayHero {
            member: [member; 16],
            name: format!("Hero {member}"),
            token: [token; 32],
        }
    }

    fn seats_with_first(first: Seat) -> Seats {
        Seats {
            session_id: None,
            run_id: None,
            seats: [first, Seat::empty()],
        }
    }

    #[test]
    fn refused_join_to_a_phone_held_seat_names_the_reclaim_and_the_30_second_rule() {
        let code = rpc::RejectionCode::ResourceMissing as i32;

        let reason = join_refusal_reason(code, 1, false);

        assert_eq!(
            reason,
            "seat 2 is held by a phone; use dev reclaim 2 once it has been away 30 s"
        );
    }

    #[test]
    fn refused_join_to_a_seat_this_control_already_joined_says_so() {
        let code = rpc::RejectionCode::ResourceMissing as i32;

        let reason = join_refusal_reason(code, 0, true);

        assert!(reason.contains("seat 1 is already joined through this control"));
        assert!(!reason.contains("reclaim"));
    }

    #[test]
    fn every_refused_join_carries_a_readable_reason_not_a_bare_code() {
        for code in [
            rpc::RejectionCode::StaleOffer,
            rpc::RejectionCode::InvalidSelection,
            rpc::RejectionCode::RecoveryRequired,
            rpc::RejectionCode::OperationConflict,
            rpc::RejectionCode::OperationExpired,
        ] {
            let reason = join_refusal_reason(code as i32, 0, false);
            assert!(
                reason.contains(' '),
                "{code:?} reason is readable: {reason}"
            );
            assert_ne!(reason, super::super::rejection_name(code as i32));
        }
        assert!(join_refusal_reason(i32::MAX, 0, false).contains("refused the join"));
    }

    #[test]
    fn reclaim_of_a_phone_held_seat_takes_the_one_away_hero_no_other_seat_holds() {
        // Seat 1 is this control's hero (member 1). The phone holds seat 2 (member 2).
        let seats = seats_with_first(bound_seat(1));
        let away = [away(1, 0xa1), away(2, 0xb2)];

        let token = reclaim_token(1, &seats, &away).expect("the phone's away hero is found");

        assert_eq!(token, [0xb2; 32]);
    }

    #[test]
    fn reclaim_of_a_phone_held_seat_is_refused_while_the_phone_is_active() {
        // The owner reports no away hero while the phone has been active in the last 30 s.
        let seats = seats_with_first(bound_seat(1));
        let away = [away(1, 0xa1)];

        let reason = reclaim_token(1, &seats, &away).expect_err("no phone hero is away");

        assert!(reason.contains("held by a phone"));
        assert!(reason.contains("30 s"));
    }

    #[test]
    fn reclaim_of_a_phone_held_seat_refuses_when_several_heroes_could_be_it() {
        // Neither hero is this control's, so the seat cannot be matched to one of them.
        let seats = seats_with_first(bound_seat(1));
        let away = [away(2, 0xb2), away(3, 0xc3)];

        let reason = reclaim_token(1, &seats, &away).expect_err("two candidates are refused");

        assert!(reason.contains("several heroes are away"));
    }

    #[test]
    fn reclaim_of_a_seat_this_control_joined_uses_its_own_hero_token() {
        // Seat 2 is bound to member 2; another away phone hero (member 3) must not be chosen.
        let seats = Seats {
            session_id: None,
            run_id: None,
            seats: [Seat::empty(), bound_seat(2)],
        };
        let away = [away(3, 0xc3), away(2, 0xb2)];

        let token = reclaim_token(1, &seats, &away).expect("the bound hero is away");

        assert_eq!(token, [0xb2; 32]);
    }

    #[test]
    fn reclaim_of_a_bound_seat_that_is_not_away_keeps_the_zero_token_for_the_owner_to_refuse() {
        let seats = seats_with_first(bound_seat(1));

        let token = reclaim_token(0, &seats, &[away(2, 0xb2)]).expect("owner decides");

        assert_eq!(token, [0; 32]);
    }
}
