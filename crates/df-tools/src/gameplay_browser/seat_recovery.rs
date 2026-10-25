//! Room-code-only seat recovery. The client displays server offers and retains retry proof.
use super::*;

pub(super) const REFUSED: &str = "That hero is being played on another phone.";
pub(super) const ROOM_MOVED: &str = "This game has ended or moved. Scan the QR code on the TV, or enter its new room code to join the current game.";
/// A retained proof field is at most 256 bytes, so at most 512 hex characters.
const MAXIMUM_ENCODED_HEX_CHARACTERS: usize = 512;
const SEAT_POLL_INTERVAL_MS: u32 = 1_000;

pub(super) fn wrong_room(answer: &rpc::ReclaimRoomResponse) -> bool {
    matches!(
        answer.outcome,
        Some(rpc::reclaim_room_response::Outcome::Refused(code))
            if code == rpc::SeatReclaimRefusal::SeatRoomCodeWrong as i32
    )
}

fn storage() -> Result<web_sys::Storage, JsValue> {
    web_sys::window()
        .ok_or_else(|| JsValue::from_str("window absent"))?
        .session_storage()?
        .ok_or_else(|| JsValue::from_str("session storage unavailable"))
}
fn key(code: &str) -> String {
    format!("df.gameplay.{code}.reclaim")
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn decode(encoded_hex: &str) -> Option<Vec<u8>> {
    if encoded_hex.len() > MAXIMUM_ENCODED_HEX_CHARACTERS || !encoded_hex.len().is_multiple_of(2) {
        return None;
    }
    encoded_hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}
fn encode(request: &rpc::ReclaimRoomRequest) -> String {
    let operation = request
        .operation_id
        .as_ref()
        .and_then(|id| id.value.as_deref())
        .unwrap_or_default();
    format!(
        "v1.{}.{}.{}",
        request.room_code,
        hex(operation),
        hex(&request.seat_token)
    )
}
fn parse(encoded_request: &str, room_code: &str) -> Option<rpc::ReclaimRoomRequest> {
    let mut encoded_fields = encoded_request.split('.');
    if encoded_fields.next()? != "v1" || encoded_fields.next()? != room_code {
        return None;
    }
    let operation_id_bytes = decode(encoded_fields.next()?)?;
    let seat_token_bytes = decode(encoded_fields.next()?)?;
    if encoded_fields.next().is_some()
        || operation_id_bytes.len() != 16
        || seat_token_bytes.len() != 32
    {
        return None;
    }
    Some(rpc::ReclaimRoomRequest {
        room_code: room_code.to_owned(),
        operation_id: Some(rpc::OperationId {
            value: Some(operation_id_bytes),
        }),
        seat_token: seat_token_bytes,
    })
}
fn load(code: &str) -> Result<Option<rpc::ReclaimRoomRequest>, JsValue> {
    let storage = storage()?;
    let Some(text) = storage.get_item(&key(code))? else {
        return Ok(None);
    };
    let request = parse(&text, code);
    if request.is_none() {
        storage.remove_item(&key(code))?;
    }
    Ok(request)
}

pub(super) fn stop(state: &mut Client) {
    if let Some(abort) = state.seat_poll_abort.take() {
        abort.abort();
    }
}
pub(super) fn clear(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let mut state = client.borrow_mut();
    storage()?.remove_item(&key(&state.room_code))?;
    state.reclaim_request = None;
    state.away_heroes.clear();
    Ok(())
}

pub(super) async fn call(
    request: rpc::ReclaimRoomRequest,
) -> Result<rpc::ReclaimRoomResponse, String> {
    let url = super::tunnel_url()?;
    let (connection, channel) = BrowserConnection::connect(&url)
        .await
        .map_err(|_| ROOM_CONNECTION_UNAVAILABLE.to_owned())?;
    let mut rooms = RoomClient::new(channel)
        .max_decoding_message_size(8192)
        .max_encoding_message_size(8192);
    let mut request = tonic::Request::new(request);
    request.set_timeout(request_timeout());
    let reply = rooms
        .reclaim(request)
        .await
        .map(|reply| reply.into_inner())
        .map_err(|_| "The room did not answer yet. Confirm the same request.".to_owned());
    connection.close();
    reply
}

pub(super) fn take_back(client: Rc<RefCell<Client>>, token: Vec<u8>) {
    let prepared = (|| -> Result<(), JsValue> {
        let mut state = client.borrow_mut();
        if state.busy {
            return Ok(());
        }
        if state.reclaim_request.is_none() {
            // An uncertain ordinary join must be reconciled before another identity admission.
            if state.join_request.is_some() {
                feedback(&state, "Confirm your earlier join first.");
                return Ok(());
            }
            let request = rpc::ReclaimRoomRequest {
                room_code: state.room_code.clone(),
                operation_id: Some(operation().map_err(|error| JsValue::from_str(&error))?),
                seat_token: token,
            };
            storage()?.set_item(&key(&state.room_code), &encode(&request))?;
            state.reclaim_request = Some(request);
        }
        Ok(())
    })();
    if prepared.is_err() {
        feedback(
            &client.borrow(),
            "This browser could not save the request. Try again.",
        );
        return;
    }
    if client.borrow().reclaim_request.is_some() {
        join_room(client);
    }
}

pub(super) fn ensure_poll(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    {
        let mut state = client.borrow_mut();
        if !state.credential.is_empty() || state.seat_poll_abort.is_some() {
            return Ok(());
        }
        if state.reclaim_request.is_none() {
            state.reclaim_request = load(&state.room_code)?;
        }
        let (abort, registration) = AbortHandle::new_pair();
        state.seat_poll_abort = Some(abort);
        let weak = Rc::downgrade(client);
        wasm_bindgen_futures::spawn_local(async move {
            let run = async {
                loop {
                    let Some(client) = weak.upgrade() else {
                        break;
                    };
                    let (code, generation, is_busy) = {
                        let state = client.borrow();
                        if !state.credential.is_empty() {
                            break;
                        }
                        let typed = state
                            .live
                            .lobby
                            .as_ref()
                            .map(|screen| screen.invitation_value());
                        let code =
                            if state.join_request.is_some() || state.reclaim_request.is_some() {
                                Some(state.room_code.clone())
                            } else {
                                typed.as_deref().and_then(clean_room_code)
                            };
                        (code, state.generation, state.busy)
                    };
                    let now_ms = js_sys::Date::now() as u64;
                    if let Some(code) = code.filter(|code| {
                        !is_busy && client.borrow_mut().seat_poll.ready(code, now_ms)
                    }) {
                        let answer = call(rpc::ReclaimRoomRequest {
                            room_code: code.clone(),
                            operation_id: None,
                            seat_token: Vec::new(),
                        })
                        .await;
                        if client.borrow().generation == generation
                            && client.borrow().credential.is_empty()
                        {
                            let outcome = match &answer {
                                Ok(answer) if wrong_room(answer) => {
                                    super::phone_reconnect::PollOutcome::WrongRoom
                                }
                                Ok(answer) if answer.outcome.is_none() => {
                                    super::phone_reconnect::PollOutcome::Available
                                }
                                Ok(_) => super::phone_reconnect::PollOutcome::Refused,
                                Err(_) => super::phone_reconnect::PollOutcome::Failed,
                            };
                            client.borrow_mut().seat_poll.observe(
                                &code,
                                js_sys::Date::now() as u64,
                                outcome,
                            );
                            match answer {
                                Ok(answer) => {
                                    let moved = wrong_room(&answer);
                                    let heroes = if answer.outcome.is_none() {
                                        answer.away_heroes
                                    } else {
                                        Vec::new()
                                    };
                                    let changed = {
                                        let mut state = client.borrow_mut();
                                        let changed =
                                            state.away_heroes != heroes || state.room_code != code;
                                        state.room_code = code.clone();
                                        state.away_heroes = heroes;
                                        changed
                                    };
                                    if changed && render_join(&client).is_err() {
                                        feedback(
                                            &client.borrow(),
                                            "The available heroes could not be displayed. Try again.",
                                        );
                                    }
                                    if moved {
                                        feedback(&client.borrow(), ROOM_MOVED);
                                    }
                                }
                                Err(_) => feedback(
                                    &client.borrow(),
                                    "The room could not be reached. Trying again.",
                                ),
                            }
                        }
                    }
                    drop(client);
                    gloo_timers::future::TimeoutFuture::new(SEAT_POLL_INTERVAL_MS).await;
                }
            };
            let _cancelled = Abortable::new(run, registration).await;
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_reclaim_proof_preserves_operation_and_opaque_token() {
        let request = rpc::ReclaimRoomRequest {
            room_code: "ROOM".to_owned(),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![1; 16]),
            }),
            seat_token: vec![2; 32],
        };
        assert_eq!(parse(&encode(&request), "ROOM").unwrap(), request);
        assert!(decode("xyz").is_none());
        assert!(decode(&"a".repeat(514)).is_none());
    }
}
