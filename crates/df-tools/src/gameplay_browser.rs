//! Thin server-projected room, creation, dialogue and combat clients.
mod dialogue_recovery;
mod live_battlefield;
mod live_chrome;
mod live_combat;
mod live_dice;
mod live_exploration;
mod live_health;
mod live_home;
mod live_home_video;
mod live_lobby;
mod live_motion;
mod live_music;
mod live_presence;
mod live_rules;
mod live_screens;
mod live_sfx;
mod live_stage;
mod live_theme;
mod live_tv;
mod live_voice;
mod public_scene_assets;
mod talk_control;
mod view_delivery;

use df_client::{
    connection::{ConnectionGeneration, RpcConnection},
    connection_views::ConnectionViewAcceptance,
    revisions::ViewAcceptance,
};
use df_protocol::common as rpc;
use df_rpc_bridge::{BrowserChannel, BrowserConnection, RPC_MESSAGE_BYTES};
use df_ui::{
    CharacterAction, CharacterActionKind, CharacterGroup, CharacterLimits, CharacterOption,
    CharacterPhaseSurface, CharacterPhaseView, CharacterStatus,
};
use futures::future::{AbortHandle, Abortable};
use public_scene_assets::browser::PublicSceneDelivery;
use std::{cell::RefCell, rc::Rc};
use view_delivery::{GameplayViews, ViewRole, ViewScope};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Document, Element, HtmlSelectElement};

type ActionClient = rpc::action_service_client::ActionServiceClient<BrowserChannel>;
type SessionClient = rpc::session_service_client::SessionServiceClient<BrowserChannel>;
type RoomClient = rpc::room_service_client::RoomServiceClient<BrowserChannel>;
type Callback = (Element, Closure<dyn FnMut(web_sys::Event)>);
struct Client {
    document: Document,
    root: Element,
    role: &'static str,
    credential: String,
    room_code: String,
    session: Option<rpc::SessionId>,
    run: Option<rpc::RunId>,
    binding: Option<rpc::ClientBindingId>,
    generation: u64,
    revision: Option<rpc::SessionRevision>,
    views: Option<GameplayViews>,
    join_request: Option<rpc::JoinRoomRequest>,
    last_request: Option<rpc::SubmitActionRequest>,
    first_attack: Option<rpc::SubmitActionRequest>,
    last_confirmed: bool,
    /// The server accepted the last request (a rejection also confirms it, but changes nothing).
    last_accepted: bool,
    action: Option<ActionClient>,
    /// A clone of the watch's session client for one-way unary reports (the display's
    /// presentation report); set with the watch connection, cleared with it.
    sessions: Option<SessionClient>,
    connection: Option<RpcConnection<SessionClient>>,
    watch_abort: Option<AbortHandle>,
    callbacks: Vec<Callback>,
    creation: Option<CharacterPhaseSurface>,
    /// The creation offer of the view the form is drawn in: a confirm always sends this one,
    /// never the offer the form was first built with.
    creation_offer: Option<rpc::GameplayActionOffer>,
    live: live_screens::LiveScreens,
    chrome: live_chrome::Chrome,
    scene_delivery: Option<Rc<PublicSceneDelivery>>,
    dialogue: dialogue_recovery::DialogueRecovery,
    combat: live_combat::Live,
    busy: bool,
    /// Retries of a lost or refused game connection (see [`schedule_reconnect`]).
    reconnect: Reconnect,
    /// The server run this page was served for, when it says so. A saved seat from another run
    /// is discarded.
    served_run: Option<String>,
    /// Automatic repeats of the current join, which is idempotent.
    join_retries: u32,
    /// Times the page has confirmed an unconfirmed choice by itself since it was sent.
    auto_confirms: u8,
    feedback_generation: u64,
}

#[derive(Default)]
struct Reconnect {
    /// Retries made since the connection last delivered a view.
    attempt: u32,
    /// Identifies the retry that may still run; any newer connect attempt cancels older ones.
    token: u64,
}
impl Drop for Client {
    fn drop(&mut self) {
        if let Some(abort) = self.watch_abort.take() {
            abort.abort();
        }
        if let Some(connection) = self.connection.take() {
            connection.close();
        }
        clear_callbacks(self);
        if let Some(scene) = self.scene_delivery.take() {
            let _cleanup = scene.dispose();
        }
        if let Some(creation) = self.creation.take() {
            let _cleanup = creation.dispose();
        }
        let _cleanup = live_combat::dispose(self);
        let _cleanup = self.live.dispose();
        self.chrome.dispose();
    }
}
thread_local! {static CLIENTS:RefCell<Vec<Rc<RefCell<Client>>>>=const {RefCell::new(Vec::new())};}

/// Local-demo credential store: the player's room binding and scope ids live in this
/// tab's `sessionStorage` so a reload can resume the same hero. It is per tab on purpose:
/// two players sharing one browser each keep their own seat.
/// Real guest sign-in (df-auth) replaces this store later. Nothing here is game data;
/// the server still authenticates every request and may refuse the saved credential.
mod saved_player {
    use super::*;

    const VERSION: &str = "v2";

    pub(super) struct SavedPlayer {
        pub(super) credential: String,
        pub(super) session: rpc::SessionId,
        pub(super) run: rpc::RunId,
        pub(super) binding: rpc::ClientBindingId,
        /// The server run the seat was joined in, as that run published it ("" when it did not).
        pub(super) run_identity: String,
    }

    fn key(room_code: &str) -> String {
        format!("df.gameplay.{room_code}.player")
    }

    fn storage() -> Result<web_sys::Storage, JsValue> {
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("window absent"))?
            .session_storage()?
            .ok_or_else(|| JsValue::from_str("session storage unavailable"))
    }

    fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn decode_id(text: &str) -> Option<Vec<u8>> {
        if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        text.as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
            .collect()
    }

    fn parse(text: &str) -> Option<SavedPlayer> {
        let mut parts = text.split('.');
        let (version, credential, session, run, binding, run_identity) = (
            parts.next()?,
            parts.next()?,
            parts.next()?,
            parts.next()?,
            parts.next()?,
            parts.next()?,
        );
        // The server accepts only lowercase hexadecimal, 64 characters.
        if parts.next().is_some()
            || version != VERSION
            || credential.len() != 64
            || !credential
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return None;
        }
        Some(SavedPlayer {
            credential: credential.to_owned(),
            session: rpc::SessionId {
                value: Some(decode_id(session)?),
            },
            run: rpc::RunId {
                value: Some(decode_id(run)?),
            },
            binding: rpc::ClientBindingId {
                value: Some(decode_id(binding)?),
            },
            run_identity: run_identity.to_owned(),
        })
    }

    /// A malformed entry is removed and treated as absent; storage failures are returned.
    pub(super) fn load(room_code: &str) -> Result<Option<SavedPlayer>, JsValue> {
        let storage = storage()?;
        let key = key(room_code);
        let Some(text) = storage.get_item(&key)? else {
            return Ok(None);
        };
        let saved = parse(&text);
        if saved.is_none() {
            storage.remove_item(&key)?;
        }
        Ok(saved)
    }

    pub(super) fn save(
        room_code: &str,
        credential: &str,
        session: &rpc::SessionId,
        run: &rpc::RunId,
        binding: &rpc::ClientBindingId,
        run_identity: &str,
    ) -> Result<(), JsValue> {
        let id = |value: &Option<Vec<u8>>| encode(value.as_deref().unwrap_or_default());
        storage()?.set_item(
            &key(room_code),
            &format!(
                "{VERSION}.{credential}.{}.{}.{}.{run_identity}",
                id(&session.value),
                id(&run.value),
                id(&binding.value)
            ),
        )
    }

    /// Whether the seat was joined in the server run the page says it was served for.
    pub(super) fn is_from_run(saved: &SavedPlayer, served_run: &str) -> bool {
        saved.run_identity == served_run
    }

    pub(super) fn clear(room_code: &str) -> Result<(), JsValue> {
        storage()?.remove_item(&key(room_code))
    }
}
const SAVED_GAME_REFUSED: &str = "The server did not accept this saved game.";
const ACTION_REFUSAL_FEEDBACK: &str = "That did not work. Try again, or pick something else.";
fn should_replace_feedback(text: &str, action_refusal_active: bool) -> bool {
    !text.is_empty() || !action_refusal_active
}
fn node(
    document: &Document,
    parent: &Element,
    tag: &str,
    class: &str,
    text: &str,
) -> Result<Element, JsValue> {
    let node = document.create_element(tag)?;
    node.set_class_name(class);
    node.set_text_content(Some(text));
    parent.append_child(&node)?;
    Ok(node)
}
fn status(client: &Client, text: &str) {
    if let Ok(Some(node)) = client.root.query_selector(".connection") {
        node.set_text_content(Some(text));
    }
}
fn feedback(client: &Client, text: &str) {
    if let Ok(Some(node)) = client.root.query_selector(".feedback") {
        if !should_replace_feedback(text, node.has_attribute("data-action-refusal")) {
            return;
        }
        if !text.is_empty() {
            let _ = node.remove_attribute("data-action-refusal");
        }
        node.set_text_content(Some(text));
    }
}
fn action_refusal_feedback(client: &Rc<RefCell<Client>>) {
    let (token, root) = {
        let mut state = client.borrow_mut();
        state.feedback_generation = state.feedback_generation.wrapping_add(1);
        (state.feedback_generation, state.root.clone())
    };
    let Ok(Some(node)) = root.query_selector(".feedback") else {
        return;
    };
    let token = token.to_string();
    let _ = node.set_attribute("data-action-refusal", &token);
    node.set_text_content(Some(ACTION_REFUSAL_FEEDBACK));
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(3500).await;
        let Some(client) = weak.upgrade() else {
            return;
        };
        if let Ok(Some(node)) = client.borrow().root.query_selector(".feedback")
            && node.get_attribute("data-action-refusal").as_deref() == Some(&token)
        {
            let _ = node.remove_attribute("data-action-refusal");
            node.set_text_content(Some(""));
        }
    });
}
fn clear_callbacks(client: &mut Client) {
    for (element, callback) in std::mem::take(&mut client.callbacks) {
        if element
            .remove_event_listener_with_callback("click", callback.as_ref().unchecked_ref())
            .is_err()
        {
            status(client, "A retired input listener could not be removed.");
        }
    }
}
fn button(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    label: &str,
    mut activate: impl FnMut(Rc<RefCell<Client>>) + 'static,
) -> Result<Element, JsValue> {
    let element = node(&client.borrow().document, parent, "button", "", label)?;
    let weak = Rc::downgrade(client);
    let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
        if let Some(client) = weak.upgrade() {
            activate(client);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    element.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
    client
        .borrow_mut()
        .callbacks
        .push((element.clone(), callback));
    Ok(element)
}
fn random<const N: usize>() -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("Browser unavailable")?;
    let crypto = window.crypto().map_err(|_| "Input identity unavailable")?;
    let mut bytes = [0; N];
    crypto
        .get_random_values_with_u8_array(&mut bytes)
        .map_err(|_| "Input identity unavailable")?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err("Input identity unavailable".to_owned());
    }
    Ok(bytes.to_vec())
}
fn operation() -> Result<rpc::OperationId, String> {
    Ok(rpc::OperationId {
        value: Some(random::<16>()?),
    })
}
thread_local! {
    /// How long one request may take; the server publishes it with the page so that the browser
    /// always waits a little longer than the server does.
    static REQUEST_TIMEOUT: std::cell::Cell<std::time::Duration> =
        const { std::cell::Cell::new(std::time::Duration::from_secs(25)) };
}
fn request_timeout() -> std::time::Duration {
    REQUEST_TIMEOUT.with(std::cell::Cell::get)
}
fn request<T>(body: T, credential: &str) -> Result<tonic::Request<T>, String> {
    let mut request = tonic::Request::new(body);
    request.set_timeout(request_timeout());
    request.metadata_mut().insert(
        "x-df-local-binding",
        credential
            .parse()
            .map_err(|_| "Local binding unavailable")?,
    );
    Ok(request)
}
fn retire_transport(client: &mut Client) {
    client.dialogue.retire_transport();
    if let Some(scene) = &client.scene_delivery
        && scene.suspend().is_err()
    {
        status(
            client,
            "The optional scene illustration is using its fallback.",
        );
    }
    if let Some(abort) = client.watch_abort.take() {
        abort.abort();
    }
    if let Some(connection) = client.connection.take() {
        connection.close();
    }
    client.action.take();
    client.sessions.take();
    client.generation += 1;
    client.busy = false;
}
fn clear_view_scope(client: &mut Client) -> Result<(), JsValue> {
    client.dialogue.clear_scope();
    if let Some(scene) = &client.scene_delivery {
        scene
            .retire()
            .map_err(|_| JsValue::from_str("retired scene scope cleanup failed"))?;
    }
    client.views.take();
    client.revision = None;
    client.last_request = None;
    client.first_attack = None;
    client.last_confirmed = false;
    client.last_accepted = false;
    clear_callbacks(client);
    let combat_cleanup = live_combat::dispose(client);
    let cleanup = client
        .creation
        .take()
        .map(|creation| creation.dispose())
        .transpose();
    let live_cleanup = client.live.dispose();
    if let Some(surface) = client.root.query_selector(".view")? {
        surface.set_text_content(None);
        surface.remove_attribute("data-destination")?;
    }
    client.root.remove_attribute("data-phase")?;
    client.root.remove_attribute("data-last-receipt-sequence")?;
    client.root.remove_attribute("data-last-receipt-replayed")?;
    cleanup.map_err(|_| JsValue::from_str("retired character scope cleanup failed"))?;
    live_cleanup.map_err(|_| JsValue::from_str("retired live screen cleanup failed"))?;
    combat_cleanup.map_err(|_| JsValue::from_str("retired combat screen cleanup failed"))?;
    Ok(())
}
fn render_join(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    live_screens::render_join(client)
}
const LEAVE_LABEL: &str = "Leave this game on this device";
/// Shown while a saved player is reconnecting, and as the way out if that cannot finish.
fn render_resume(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (document, surface) = {
        let mut state = client.borrow_mut();
        clear_callbacks(&mut state);
        (
            state.document.clone(),
            state
                .root
                .query_selector(".view")?
                .ok_or_else(|| JsValue::from_str("view missing"))?,
        )
    };
    live_screens::release(client, &surface)?;
    surface.set_text_content(None);
    let body = node(&document, &surface, "div", "scene-copy", "")?;
    node(&document, &body, "div", "eyebrow", "Welcome back")?;
    node(&document, &body, "h2", "", "Rejoining your adventure")?;
    node(
        &document,
        &body,
        "p",
        "narration",
        "Reconnecting to the hero saved on this device.",
    )?;
    let tools = node(&document, &body, "div", "tools", "")?;
    button(client, &tools, "Try again", connect)?;
    button(client, &tools, LEAVE_LABEL, leave_saved_game)?;
    node(&document, &body, "p", "feedback", "")?;
    Ok(())
}
fn leave_saved_game(client: Rc<RefCell<Client>>) {
    forget_saved_game(
        &client,
        "You left the game on this device. Join again whenever you like.",
    );
}
/// Forgets this device's credential and returns to the join screen. The server keeps the
/// hero and the room membership; nothing is deleted there.
fn forget_saved_game(client: &Rc<RefCell<Client>>, reason: &str) {
    let cleared = {
        let mut state = client.borrow_mut();
        let cleared = saved_player::clear(&state.room_code);
        retire_transport(&mut state);
        let reset = clear_view_scope(&mut state);
        state.credential.clear();
        state.session = None;
        state.run = None;
        state.binding = None;
        state.join_request = None;
        cleared.and(reset)
    };
    let rendered = render_join(client);
    let state = client.borrow();
    status(&state, "Your adventure awaits.");
    feedback(
        &state,
        if cleared.is_err() || rendered.is_err() {
            "This browser could not fully forget the saved game. Clear this site's data, then reload."
        } else {
            reason
        },
    );
}
/// The longest room code the server accepts.
const MAXIMUM_ROOM_CODE: usize = 32;

/// A room code as the server compares it: letters and digits only, upper-cased.
fn clean_room_code(typed: &str) -> Option<String> {
    let code = typed.trim();
    (!code.is_empty()
        && code.len() <= MAXIMUM_ROOM_CODE
        && code.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    .then(|| code.to_ascii_uppercase())
}

/// Adopts the code typed in the join form for the join about to start. A different code means
/// a different room, so the join proof of the earlier one is dropped. Returns whether the
/// join may go ahead.
fn use_room_code(client: &Rc<RefCell<Client>>, typed: &str) -> bool {
    let Some(code) = clean_room_code(typed) else {
        feedback(
            &client.borrow(),
            "Enter the room code shown on the display: letters and digits only.",
        );
        return false;
    };
    let mut state = client.borrow_mut();
    if state.room_code != code {
        state.room_code = code;
        state.join_request = None;
    }
    state.join_retries = 0;
    state.reconnect.attempt = 0;
    true
}

const ROOM_CONNECTION_UNAVAILABLE: &str = "Room connection unavailable";
/// What the server says when its saved state is gone for good (`SESSION_LOST_MESSAGE`).
const SERVER_LOST: &str = "The game server lost its saved state and must be restarted";

fn join_room(client: Rc<RefCell<Client>>) {
    let prepared = (|| -> Result<_, String> {
        let mut state = client.borrow_mut();
        if state.busy {
            return Err("Waiting for the room…".to_owned());
        }
        if state.join_request.is_none() {
            state.join_request = Some(rpc::JoinRoomRequest {
                room_code: state.room_code.clone(),
                operation_id: Some(operation()?),
                join_secret: random::<32>()?,
            });
        }
        let body = state.join_request.clone().ok_or("Room proof unavailable")?;
        retire_transport(&mut state);
        state.busy = true;
        status(&state, "Joining your companions…");
        Ok((body, state.generation))
    })();
    let (body, generation) = match prepared {
        Ok(value) => value,
        Err(error) => {
            feedback(&client.borrow(), &error);
            return;
        }
    };
    wasm_bindgen_futures::spawn_local(async move {
        let run = async {
            let url = crate::browser::tunnel_url("/gameplay/tunnel")?;
            let (connection, channel) = BrowserConnection::connect(&url)
                .await
                .map_err(|_| ROOM_CONNECTION_UNAVAILABLE.to_owned())?;
            let mut rooms = RoomClient::new(channel)
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192);
            let mut request = tonic::Request::new(body);
            request.set_timeout(request_timeout());
            let reply = rooms
                .join(request)
                .await
                .map(|reply| reply.into_inner())
                .map_err(|status| {
                    if status.code() == tonic::Code::DataLoss {
                        SERVER_LOST.to_owned()
                    } else {
                        "Join outcome unknown. Confirm the same join; do not start a new one."
                            .to_owned()
                    }
                });
            connection.close();
            reply
        };
        let outcome = run.await;
        if client.borrow().generation != generation {
            return;
        }
        client.borrow_mut().busy = false;
        if outcome.is_ok() {
            let mut state = client.borrow_mut();
            state.join_retries = 0;
            state.reconnect.attempt = 0;
            show_notice(&state, None);
        }
        match outcome {
            Ok(reply) => match reply.outcome {
                Some(rpc::join_room_response::Outcome::Joined(joined)) => {
                    let original = client
                        .borrow()
                        .join_request
                        .as_ref()
                        .and_then(|request| request.operation_id.clone());
                    let valid_receipt = joined.receipt.as_ref().is_some_and(|receipt| {
                        receipt.operation_id == original
                            && receipt.session_id == joined.session_id
                            && receipt.run_id == joined.run_id
                            && matches!(
                                receipt.outcome,
                                Some(rpc::decision_receipt::Outcome::Accepted(_))
                            )
                            && receipt.revision.as_ref().is_some_and(|revision| {
                                revision.sequence.is_some() && revision.epoch.is_some()
                            })
                    });
                    let valid_scope = joined
                        .session_id
                        .as_ref()
                        .and_then(|id| id.value.as_ref())
                        .is_some_and(|bytes| bytes.len() == 16)
                        && joined
                            .run_id
                            .as_ref()
                            .and_then(|id| id.value.as_ref())
                            .is_some_and(|bytes| bytes.len() == 16);
                    if !valid_receipt
                        || !valid_scope
                        || joined.local_binding.len() != 64
                        || joined.session_id.is_none()
                        || joined.run_id.is_none()
                        || joined
                            .client_binding_id
                            .as_ref()
                            .and_then(|id| id.value.as_ref())
                            .is_none_or(|bytes| bytes.len() != 16)
                    {
                        feedback(
                            &client.borrow(),
                            "The joined scope is incomplete. Confirm the same join.",
                        );
                        return;
                    }
                    let saved = {
                        let mut state = client.borrow_mut();
                        state.credential = joined.local_binding;
                        state.session = joined.session_id;
                        state.run = joined.run_id;
                        state.binding = joined.client_binding_id;
                        match (&state.session, &state.run, &state.binding) {
                            (Some(session), Some(run), Some(binding)) => saved_player::save(
                                &state.room_code,
                                &state.credential,
                                session,
                                run,
                                binding,
                                state.served_run.as_deref().unwrap_or_default(),
                            ),
                            _ => Err(JsValue::from_str("joined scope absent")),
                        }
                    };
                    connect(client.clone());
                    if saved.is_err() {
                        feedback(
                            &client.borrow(),
                            "This browser cannot remember your game, so a reload would start over.",
                        );
                    }
                }
                Some(rpc::join_room_response::Outcome::Refused(refusal)) => {
                    feedback(
                        &client.borrow(),
                        match rpc::RejectionCode::try_from(refusal.code) {
                            Ok(rpc::RejectionCode::ResourceMissing) => {
                                "This two-player room is full."
                            }
                            Ok(rpc::RejectionCode::InvalidSelection) => {
                                "That room code is not available."
                            }
                            Ok(rpc::RejectionCode::StaleOffer) => {
                                "The adventure has already begun."
                            }
                            _ => "Room recovery required. Confirm the original join proof.",
                        },
                    );
                }
                None => feedback(
                    &client.borrow(),
                    "The server returned no join outcome. Confirm the same join.",
                ),
            },
            Err(error) => {
                // The join is idempotent (same operation and proof), so bounded retries are
                // safe for both an unavailable tunnel and an unknown RPC outcome.
                let retry = {
                    let mut state = client.borrow_mut();
                    record_join_retry(&error, &mut state.join_retries)
                };
                let _ = render_join(&client);
                if retry {
                    feedback(
                        &client.borrow(),
                        "The room did not answer yet. Trying again.",
                    );
                    schedule_join_retry(&client);
                } else {
                    feedback(
                        &client.borrow(),
                        if error == ROOM_CONNECTION_UNAVAILABLE {
                            "Could not connect to the room. Select Try again to retry."
                        } else {
                            &error
                        },
                    );
                    if error == SERVER_LOST {
                        show_notice(&client.borrow(), Some(SERVER_LOST));
                    }
                }
            }
        }
    });
}

fn record_join_retry(error: &str, retries: &mut u32) -> bool {
    if error == SERVER_LOST || *retries >= crate::reconnect::JOIN_RETRIES {
        return false;
    }
    *retries += 1;
    true
}

/// Repeats the pending join after a backoff while the page shows "Reconnecting…".
fn schedule_join_retry(client: &Rc<RefCell<Client>>) {
    let (token, wait) = {
        let mut state = client.borrow_mut();
        state.reconnect.token += 1;
        let jitter = (js_sys::Math::random() * 1000.0) as u32;
        let wait = crate::reconnect::delay(state.reconnect.attempt, jitter);
        state.reconnect.attempt = state.reconnect.attempt.saturating_add(1);
        show_notice(&state, Some(RECONNECTING));
        (state.reconnect.token, wait)
    };
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(
            u32::try_from(wait.as_millis()).unwrap_or(u32::MAX),
        )
        .await;
        let Some(client) = weak.upgrade() else {
            return;
        };
        let due = {
            let state = client.borrow();
            state.reconnect.token == token
                && state.credential.is_empty()
                && state.join_request.is_some()
                && !state.busy
        };
        if due {
            join_room(client);
        }
    });
}
fn connect(client: Rc<RefCell<Client>>) {
    if client.borrow().credential.is_empty() {
        let _ = render_join(&client);
        return;
    }
    let prepared = (|| -> Result<_, JsValue> {
        let mut state = client.borrow_mut();
        // A connect attempt of any origin replaces a retry that is still waiting.
        state.reconnect.token += 1;
        retire_transport(&mut state);
        let scope = ViewScope::from_wire(
            state.session.as_ref(),
            state.run.as_ref(),
            state.binding.as_ref(),
            if state.role == "player" {
                ViewRole::Player
            } else {
                ViewRole::Display
            },
        )
        .map_err(|_| JsValue::from_str("saved view scope invalid"))?;
        if state
            .views
            .as_ref()
            .is_some_and(|views| views.scope() != scope)
        {
            clear_view_scope(&mut state)?;
        }
        status(&state, "Connecting to your saved game…");
        Ok((
            state.generation,
            state.credential.clone(),
            state.session.clone(),
            state.run.clone(),
            state.binding.clone(),
            state.revision,
            scope,
        ))
    })();
    let (generation, credential, session, run, binding, observed, scope) = match prepared {
        Ok(prepared) => prepared,
        Err(_) => {
            status(&client.borrow(), "Saved view scope could not be recovered.");
            return;
        }
    };
    let (abort, registration) = AbortHandle::new_pair();
    client.borrow_mut().watch_abort = Some(abort);
    let owned = client.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let run = async {
            let url = crate::browser::tunnel_url("/gameplay/tunnel")?;
            let (connection, channel) = BrowserConnection::connect(&url)
                .await
                .map_err(|_| "Game connection unavailable")?;
            if owned.borrow().generation != generation {
                connection.close();
                return Ok::<(), String>(());
            }
            // The owner's view carries the combat section (reachable cells, legal targets, budget),
            // which exceeds 8 KiB in a three-enemy fight; RPC_MESSAGE_BYTES is the bridge's RPC ceiling.
            let mut sessions = SessionClient::new(channel.clone())
                .max_decoding_message_size(RPC_MESSAGE_BYTES)
                .max_encoding_message_size(RPC_MESSAGE_BYTES);
            let connection = RpcConnection::from_browser(sessions.clone(), connection);
            let view_generation = connection.generation();
            {
                let mut state = owned.borrow_mut();
                if let Some(views) = state.views.as_mut() {
                    if !views.reconnect(view_generation.clone()) {
                        return Err("Game connection retired before view recovery".to_owned());
                    }
                } else {
                    state.views = Some(GameplayViews::new(scope, view_generation.clone()));
                }
                state.action = Some(
                    ActionClient::new(channel.clone())
                        .max_decoding_message_size(RPC_MESSAGE_BYTES)
                        .max_encoding_message_size(RPC_MESSAGE_BYTES),
                );
                state.dialogue.bind(channel);
                state.sessions = Some(sessions.clone());
                state.connection = Some(connection);
            }
            // Reconnect refreshes local input/busy state from the retained snapshot.
            // A duplicate server frame never replaces that snapshot.
            redraw_current(&owned).map_err(|_| "Saved view could not be displayed")?;
            let body = rpc::WatchViewRequest {
                session_id: session,
                run_id: run,
                client_binding_id: binding,
                after_revision: observed,
            };
            let mut stream = sessions
                .watch(request(body, &credential)?)
                .await
                .map_err(|error| match error.code() {
                    tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
                        SAVED_GAME_REFUSED
                    }
                    tonic::Code::DataLoss => SERVER_LOST,
                    _ => "Saved game view unavailable",
                })?
                .into_inner();
            status(&owned.borrow(), "Connected · server owns the saved game");
            while let Some(view) = stream
                .message()
                .await
                .map_err(|_| "Connection interrupted; reconnect to recover")?
            {
                if owned.borrow().generation != generation {
                    return Ok(());
                }
                receive_view(&owned, &view_generation, scope, view)
                    .map_err(|_| "Server view could not be displayed")?;
            }
            Ok(())
        };
        let result = Abortable::new(run, registration).await;
        if owned.borrow().generation == generation {
            let refused = matches!(&result, Ok(Err(error)) if error == SAVED_GAME_REFUSED);
            if owned.borrow().role == "player" && refused {
                forget_saved_game(
                    &owned,
                    "Your saved game was not recognised by this server, so it was cleared. Join again to start over.",
                );
                return;
            }
            if let Some(connection) = owned.borrow_mut().connection.take() {
                connection.close();
            }
            owned.borrow_mut().action.take();
            owned.borrow_mut().sessions.take();
            owned.borrow_mut().dialogue.retire_transport();
            // Redraw so the recovery controls reflect the lost connection.
            let redrawn = redraw_current(&owned);
            status(
                &owned.borrow(),
                match (&result, redrawn.is_err()) {
                    (_, true) => "Saved view could not be displayed.",
                    (Ok(Ok(())), _) => "Game connection closed.",
                    (Ok(Err(error)), _) => error.as_str(),
                    (Err(_), _) => "Connection cancelled.",
                },
            );
            // A lost, refused or closed connection is retried, never left on a dead screen.
            // Only a connection this page retired itself (an aborted run) stays down.
            if result.is_ok() {
                // A server that says its saved state is gone is still asked again (a restart
                // brings it back), but the page says what is wrong instead of "Reconnecting…".
                let lost = matches!(&result, Ok(Err(error)) if error == SERVER_LOST);
                schedule_reconnect(&owned, refused, lost);
            }
        }
    });
}

const RECONNECTING: &str = "Reconnecting…";

/// Shows `text` as the page's connection notice, or removes the notice.
fn show_notice(client: &Client, text: Option<&str>) {
    let Ok(existing) = client.root.query_selector(".df-reconnecting") else {
        return;
    };
    match (text, existing) {
        (None, Some(notice)) => notice.remove(),
        (Some(text), Some(notice)) => notice.set_text_content(Some(text)),
        (Some(text), None) => {
            let Ok(notice) = client.document.create_element("div") else {
                return;
            };
            notice.set_class_name("df-reconnecting");
            let _role = notice.set_attribute("role", "status");
            let _style = notice.set_attribute("style", RECONNECTING_STYLE);
            notice.set_text_content(Some(text));
            let _added = client.root.append_child(&notice);
        }
        (None, None) => {}
    }
}

// Sized for a TV seen from across a room as well as for a phone.
const RECONNECTING_STYLE: &str = "position:fixed;left:50%;bottom:3vh;transform:translateX(-50%);\
z-index:2147483000;padding:.5em 1.4em;border-radius:999px;border:1px solid rgba(223,185,116,.7);\
background:rgba(8,15,23,.88);color:#efe6ce;font:clamp(20px,1.4vw,36px) \"Iowan Old Style\",\
\"Palatino Linotype\",Palatino,Georgia,serif;letter-spacing:.03em;pointer-events:none";

/// Tries the game connection again after a backoff, with a visible "Reconnecting…". A display
/// whose credential the server refuses (the server restarted since the page was served) reloads
/// itself to fetch a fresh one; a player's refused credential is handled by the caller.
fn schedule_reconnect(client: &Rc<RefCell<Client>>, refused: bool, lost: bool) {
    let (token, wait, display) = {
        let mut state = client.borrow_mut();
        state.reconnect.token += 1;
        let jitter = (js_sys::Math::random() * 1000.0) as u32;
        let wait = crate::reconnect::delay(state.reconnect.attempt, jitter);
        state.reconnect.attempt = state.reconnect.attempt.saturating_add(1);
        show_notice(&state, Some(if lost { SERVER_LOST } else { RECONNECTING }));
        (state.reconnect.token, wait, state.role == "display")
    };
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(
            u32::try_from(wait.as_millis()).unwrap_or(u32::MAX),
        )
        .await;
        let Some(client) = weak.upgrade() else {
            return;
        };
        let due = {
            let state = client.borrow();
            state.reconnect.token == token && state.connection.is_none()
        };
        if !due {
            return;
        }
        if display && refused {
            if let Some(window) = web_sys::window() {
                let _reloading = window.location().reload();
            }
            return;
        }
        connect(client);
    });
}
fn prepare(
    client: &Rc<RefCell<Client>>,
    offer: &rpc::GameplayActionOffer,
    character: Option<rpc::CharacterSelection>,
    savage: bool,
    graze: bool,
    destination: i32,
) -> Result<rpc::SubmitActionRequest, String> {
    let state = client.borrow();
    if state.busy || (!state.last_confirmed && state.last_request.is_some()) {
        return Err("Confirm the current operation before making a new choice.".to_owned());
    }
    Ok(rpc::SubmitActionRequest {
        session_id: state.session.clone(),
        run_id: state.run.clone(),
        observed_revision: state.revision,
        operation_id: Some(operation()?),
        offer_id: offer.offer_id.clone(),
        action_kind: offer.action_kind,
        destination,
        character,
        savage_attacker: savage,
        graze,
        action_choices: Vec::new(),
        table_setting: None,
    })
}
/// A seated player's request to show or hide the choice chips on the shared display. It names no
/// offer; the server checks the seat and the value.
fn table_setting(
    client: &Rc<RefCell<Client>>,
    choice_hints_hidden: bool,
) -> Result<rpc::SubmitActionRequest, String> {
    let state = client.borrow();
    if state.busy || (!state.last_confirmed && state.last_request.is_some()) {
        return Err("Confirm the current operation before making a new choice.".to_owned());
    }
    Ok(rpc::SubmitActionRequest {
        session_id: state.session.clone(),
        run_id: state.run.clone(),
        observed_revision: state.revision,
        operation_id: Some(operation()?),
        offer_id: String::new(),
        action_kind: rpc::GameplayActionKind::SetTableSetting as i32,
        destination: rpc::HarborDestination::Unspecified as i32,
        character: None,
        savage_attacker: false,
        graze: false,
        action_choices: Vec::new(),
        table_setting: Some(rpc::TableSettingChange {
            value: Some(rpc::table_setting_change::Value::ChoiceHintsHidden(
                choice_hints_hidden,
            )),
        }),
    })
}
fn send(client: Rc<RefCell<Client>>, body: rpc::SubmitActionRequest, replay: bool) {
    let prepared = (|| -> Result<_, String> {
        let mut state = client.borrow_mut();
        if state.busy {
            return Err("Waiting for the server…".to_owned());
        }
        let action = state
            .action
            .clone()
            .ok_or("Reconnect before sending input")?;
        if !replay {
            state.dialogue.discard_unconfirmed();
            state.last_request = Some(body.clone());
            state.last_confirmed = false;
            state.last_accepted = false;
            state.auto_confirms = 0;
        }
        state.busy = true;
        feedback(&state, "Waiting for the game server…");
        Ok((action, state.credential.clone(), state.generation))
    })();
    let (mut action, credential, generation) = match prepared {
        Ok(value) => value,
        Err(error) => {
            feedback(&client.borrow(), &error);
            return;
        }
    };
    wasm_bindgen_futures::spawn_local(async move {
        let result = match request(body.clone(), &credential) {
            Ok(request) => action
                .submit(request)
                .await
                .map(|response| response.into_inner())
                .map_err(|status| {
                    if status.code() == tonic::Code::DataLoss {
                        SERVER_LOST
                    } else {
                        "Outcome unknown. Confirm the same operation or reconnect."
                    }
                }),
            Err(_) => Err("Input binding unavailable"),
        };
        if client.borrow().generation != generation {
            return;
        }
        let message = {
            let mut state = client.borrow_mut();
            state.busy = false;
            complete_action_response(&mut state, &body, result)
        };
        if redraw_current(&client).is_err() {
            status(&client.borrow(), "Saved view could not be displayed.");
        }
        if message == ACTION_REFUSAL_FEEDBACK {
            action_refusal_feedback(&client);
        } else {
            feedback(&client.borrow(), &message);
        }
    });
}
fn complete_action_response(
    state: &mut Client,
    body: &rpc::SubmitActionRequest,
    result: Result<rpc::SubmitActionResponse, &str>,
) -> String {
    match result {
        Ok(response) => match response.outcome {
            Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) => {
                if receipt.operation_id != body.operation_id
                    || receipt.session_id != body.session_id
                    || receipt.run_id != body.run_id
                    || receipt.revision.is_none()
                {
                    "The receipt belongs to a different scope. Reconnect to recover.".to_owned()
                } else {
                    if state
                        .last_request
                        .as_ref()
                        .is_some_and(|saved| saved.operation_id == body.operation_id)
                    {
                        state.last_confirmed = receipt.outcome.is_some();
                        state.last_accepted = matches!(
                            receipt.outcome,
                            Some(rpc::decision_receipt::Outcome::Accepted(_))
                        );
                    }
                    if let Some(revision) = receipt.revision {
                        let _ = state.root.set_attribute(
                            "data-last-receipt-sequence",
                            &revision.sequence.unwrap_or(0).to_string(),
                        );
                    }
                    let _ = state.root.set_attribute(
                        "data-last-receipt-replayed",
                        if receipt.replayed { "true" } else { "false" },
                    );
                    match receipt.outcome {
                                Some(rpc::decision_receipt::Outcome::Accepted(_))=>{
                                    if body.action_kind==rpc::GameplayActionKind::GreatswordAttack as i32 && state.first_attack.is_none() {state.first_attack=Some(body.clone());}
                                    if body.action_kind==rpc::GameplayActionKind::SetTableSetting as i32 {"Table setting saved.".to_owned()}
                                    else if receipt.replayed {"Original saved outcome confirmed. No rolls or damage were repeated.".to_owned()} else {"Your choice is saved to the shared adventure.".to_owned()}
                                }
                        Some(rpc::decision_receipt::Outcome::Rejected(_)) => {
                            ACTION_REFUSAL_FEEDBACK.to_owned()
                        }
                                None=>"The server returned no decision outcome.".to_owned(),
                            }
                }
            }
            Some(rpc::submit_action_response::Outcome::OperationObservation(_)) => {
                "The operation requires recovery; retain the same input.".to_owned()
            }
            None => "The server returned no operation outcome.".to_owned(),
        },
        Err(error) => error.to_owned(),
    }
}
/// A choice whose outcome was unknown is confirmed by the page itself once the game answers
/// again (the first view after a failure), a few times at most. Confirming repeats the same
/// operation, which the server never applies twice, so it is what the player would click.
fn auto_confirm(client: &Rc<RefCell<Client>>) {
    let due = {
        let mut state = client.borrow_mut();
        let due = state.role == "player"
            && !state.busy
            && !state.last_confirmed
            && state.last_request.is_some()
            && state.action.is_some()
            && !state.dialogue.has_confirmation()
            && state.auto_confirms < 3;
        if due {
            state.auto_confirms += 1;
        }
        due
    };
    if due {
        replay(client.clone(), false);
    }
}

fn replay(client: Rc<RefCell<Client>>, first_attack: bool) {
    if !first_attack && client.borrow().dialogue.has_confirmation() {
        dialogue_recovery::retry(client);
        return;
    }
    let body = if first_attack {
        client.borrow().first_attack.clone()
    } else {
        client.borrow().last_request.clone()
    };
    if let Some(body) = body {
        send(client, body, true);
    }
}
fn character_render_error(
    client: &Rc<RefCell<Client>>,
    error: &df_ui::CharacterPhaseError,
) -> JsValue {
    let code = match error {
        df_ui::CharacterPhaseError::InvalidView(reason) => reason.to_string(),
        df_ui::CharacterPhaseError::Control(_) => "character-control".to_owned(),
        df_ui::CharacterPhaseError::Dom(_) => "character-dom".to_owned(),
        df_ui::CharacterPhaseError::UpdateCleanup { .. } => "character-update-cleanup".to_owned(),
    };
    let _ = client
        .borrow()
        .root
        .set_attribute("data-render-error", &code);
    JsValue::from_str("Character controls could not be displayed")
}
fn creation(
    client: &Rc<RefCell<Client>>,
    surface: &Element,
    offer: &rpc::CharacterCreationOffer,
    action: &rpc::GameplayActionOffer,
) -> Result<(), JsValue> {
    let state = client.borrow();
    let owner = state
        .binding
        .as_ref()
        .and_then(|binding| binding.value.as_ref())
        .map(|bytes| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .ok_or_else(|| JsValue::from_str("character owner absent"))?;
    let editable = true;
    let view=CharacterPhaseView {generation:state.revision.and_then(|revision|revision.epoch).and_then(|epoch|epoch.value).ok_or_else(||JsValue::from_str("creation epoch absent"))?,owner_key:owner,revision:state.revision.and_then(|revision|revision.sequence).ok_or_else(||JsValue::from_str("creation revision absent"))?,
        chapter:"Your hero".to_owned(),title:"Make your hero".to_owned(),description:"Pick one option in each group, then confirm.".to_owned(),
        connection:"Connected to your shared room".to_owned(),status:CharacterStatus::Editing,
        status_message:"Choose your options and confirm. Names may use letters, dashes, or underscores (up to 24 characters).".to_owned(),
        editable,name:String::new(),flavor:String::new(),appearance:None,portrait:None,
        groups:offer.groups.iter().map(|group|CharacterGroup {id:group.group_id.clone(),label:creation_group_label(&group.group_id, &group.label),
            description:(if group.options.len() == 1 { "Every hero in this adventure has this." } else { "Pick one." }).to_owned(),selected:(group.options.len() == 1).then(|| group.options[0].option_id.clone()),options:group.options.iter().map(|option|CharacterOption {
                id:option.option_id.clone(),label:creation_option_label(&group.group_id, &option.label),description:creation_option_description(&group.group_id, &option.description),
                enabled:true,availability:"Available in this room".to_owned(),portrait:None,
            }).collect()}).collect(),
        facts:vec![],
        actions:vec![CharacterAction {id:action.offer_id.clone(),kind:CharacterActionKind::SubmitDraft,
            label:"Confirm your hero".to_owned(),enabled:editable}],
    };
    drop(state);
    client.borrow_mut().creation_offer = Some(action.clone());
    if let Some(existing) = client.borrow().creation.as_ref() {
        existing
            .update(&view)
            .map_err(|error| character_render_error(client, &error))?;
        apply_option_requirements(client, existing, offer)?;
        return Ok(());
    }
    let weak = Rc::downgrade(client);
    let created = CharacterPhaseSurface::create(
        &client.borrow().document,
        "journey-character",
        &view,
        CharacterLimits {
            max_groups: 16,
            max_options: 128,
            max_facts: 32,
            max_actions: 4,
            max_text_bytes: 8192,
        },
        move |submission| {
            let Some(client) = weak.upgrade() else {
                return;
            };
            let Some(offered) = client.borrow().creation_offer.clone() else {
                return;
            };
            let selection = rpc::CharacterSelection {
                name: submission.name,
                choices: submission
                    .choices
                    .into_iter()
                    .map(|choice| rpc::JourneyChoice {
                        group_id: choice.group_id,
                        option_id: choice.option_id,
                    })
                    .collect(),
            };
            match prepare(
                &client,
                &offered,
                Some(selection),
                false,
                false,
                rpc::HarborDestination::Unspecified as i32,
            ) {
                Ok(body) => send(client, body, false),
                Err(error) => feedback(&client.borrow(), &error),
            }
        },
    )
    .map_err(|error| character_render_error(client, &error))?;
    // Flavor is not an admitted field in this bounded character contract.
    created.flavor_input().root().set_attribute("hidden", "")?;
    apply_option_requirements(client, &created, offer)?;
    surface.append_child(created.root())?;
    client.borrow_mut().creation = Some(created);
    Ok(())
}

fn creation_group_label(group_id: &str, label: &str) -> String {
    if group_id == "allied-ties" {
        return "When turns are tied".to_owned();
    }
    live_combat::display_label(label)
}

fn creation_option_label(group_id: &str, label: &str) -> String {
    if group_id == "build-package" {
        if label.contains("Fighter A") {
            return "Greatsword and Defense".to_owned();
        }
        if label.contains("Fighter B") {
            return "Longbow and Archery".to_owned();
        }
    }
    if group_id == "allied-ties" {
        return live_combat::display_label(&label.replace("the Bandit", "the enemy"));
    }
    live_combat::display_label(label)
}

fn creation_option_description(group_id: &str, description: &str) -> String {
    if matches!(group_id, "species" | "class") {
        String::new()
    } else if group_id == "allied-ties" {
        // The pack's wording names one adventure's enemy; the rule is the same for any.
        description.replace("the Bandit", "the enemy")
    } else {
        description.to_owned()
    }
}

fn player_sheet_fact_value(label: &str, value: &str) -> Option<String> {
    match label {
        "Supported rules" => None,
        "Starting equipment" => {
            let Some((equipment, coins)) = value.rsplit_once(" · ") else {
                return Some(value.to_owned());
            };
            let Some(amount) = coins.strip_suffix(" GP") else {
                return Some(value.to_owned());
            };
            if matches!(equipment, "Fighter A + Soldier A" | "Fighter B + Soldier B") {
                Some(format!("{amount} gp"))
            } else {
                Some(format!("{equipment} · {amount} gp"))
            }
        }
        "First language" | "Second language" | "Base language" | "First class skill"
        | "Second class skill" | "Alignment" => Some(live_combat::display_label(value)),
        _ if value == "Held" => Some("(in hand)".to_owned()),
        _ if value.starts_with("Dropped") => Some("(dropped)".to_owned()),
        _ => Some(value.to_owned()),
    }
}

fn player_sheet_view(sheet: &rpc::CharacterSheet) -> rpc::CharacterSheet {
    let mut view = sheet.clone();
    view.facts.retain_mut(|fact| {
        if let Some(value) = player_sheet_fact_value(&fact.label, &fact.value) {
            fact.value = value;
            true
        } else {
            false
        }
    });
    view
}

/// Lists an option only with the choices the server says it goes with (an appearance only with
/// the packages that grant the weapon it shows); the server refuses the others in any case.
fn apply_option_requirements(
    client: &Rc<RefCell<Client>>,
    surface: &CharacterPhaseSurface,
    offer: &rpc::CharacterCreationOffer,
) -> Result<(), JsValue> {
    for group in &offer.groups {
        for option in group
            .options
            .iter()
            .filter(|option| !option.requires_group.is_empty())
        {
            surface
                .require_option(
                    &group.group_id,
                    &option.option_id,
                    &option.requires_group,
                    &option.requires_options,
                )
                .map_err(|error| character_render_error(client, &error))?;
        }
    }
    Ok(())
}
fn action_button(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    offer: rpc::GameplayActionOffer,
) -> Result<(), JsValue> {
    let document = client.borrow().document.clone();
    if offer.input_groups.len() > 3 {
        return Err(JsValue::from_str("offered inputs exceed bound"));
    }
    if offer.destinations.len() > 4 {
        return Err(JsValue::from_str("offered destinations exceed bound"));
    }
    let children = parent.query_selector_all(":scope > .offered-action")?;
    let existing = (0..children.length())
        .filter_map(|index| children.item(index))
        .map(|element| element.unchecked_into::<Element>())
        .find(|element| element.get_attribute("data-offer-id").as_deref() == Some(&offer.offer_id));
    if let Some(wrapper) = existing {
        for group in &offer.input_groups {
            let labels = wrapper.query_selector_all(":scope > label[data-group-id]")?;
            let label = (0..labels.length())
                .filter_map(|index| labels.item(index))
                .map(|element| element.unchecked_into::<Element>())
                .find(|element| {
                    element.get_attribute("data-group-id").as_deref() == Some(&group.group_id)
                });
            if let Some(label) = label {
                if let Some(text) = label.query_selector(":scope > .input-label")? {
                    text.set_text_content(Some(&group.label));
                }
                if let Some(select) = label.query_selector(":scope > select")? {
                    let select = select.dyn_into::<HtmlSelectElement>()?;
                    let old_value = select.value();
                    select.set_text_content(None);
                    for option in &group.options {
                        let item = node(&document, &select, "option", "", &option.label)?;
                        item.set_attribute("value", &option.option_id)?;
                    }
                    if group
                        .options
                        .iter()
                        .any(|option| option.option_id == old_value)
                    {
                        select.set_value(&old_value);
                    }
                }
            }
        }
        if let Some(label) = wrapper.query_selector(":scope > label[data-destination]")? {
            if offer.destinations.is_empty() {
                label.remove();
            } else if let Some(select) = label.query_selector(":scope > select")? {
                let select = select.dyn_into::<HtmlSelectElement>()?;
                let old_value = select.value();
                select.set_text_content(None);
                for option in &offer.destinations {
                    let item = node(&document, &select, "option", "", &option.label)?;
                    item.set_attribute("value", &option.destination.to_string())?;
                }
                if offer
                    .destinations
                    .iter()
                    .any(|option| option.destination.to_string() == old_value)
                {
                    select.set_value(&old_value);
                }
            }
        }
        if let Some(button) = wrapper.query_selector(":scope > button.offered")? {
            button.set_text_content(Some(&offer.label));
            button.set_attribute("data-action-kind", &offer.action_kind.to_string())?;
            if client.borrow().busy
                || (!client.borrow().last_confirmed && client.borrow().last_request.is_some())
            {
                button.set_attribute("disabled", "")?;
            } else {
                button.remove_attribute("disabled")?;
            }
        }
        return Ok(());
    }

    let wrapper = node(&document, parent, "div", "offered-action", "")?;
    wrapper.set_attribute("data-offer-id", &offer.offer_id)?;
    let mut inputs = Vec::new();
    for group in &offer.input_groups {
        if group.options.is_empty() || group.options.len() > 256 {
            return Err(JsValue::from_str("unsupported offered input shape"));
        }
        let label = node(&document, &wrapper, "label", "attack-option", "")?;
        label.set_attribute("data-group-id", &group.group_id)?;
        node(&document, &label, "span", "input-label", &group.label)?;
        let select = node(&document, &label, "select", "", "")?.dyn_into::<HtmlSelectElement>()?;
        select.set_attribute("data-group-id", &group.group_id)?;
        for option in &group.options {
            let item = node(&document, &select, "option", "", &option.label)?;
            item.set_attribute("value", &option.option_id)?;
        }
        inputs.push((group.group_id.clone(), select));
    }
    let destination = if offer.destinations.is_empty() {
        None
    } else {
        let label = node(&document, &wrapper, "label", "attack-option", "")?;
        label.set_attribute("data-destination", "")?;
        node(&document, &label, "span", "input-label", "Destination")?;
        let select = node(&document, &label, "select", "", "")?.dyn_into::<HtmlSelectElement>()?;
        select.set_attribute("data-destination", "")?;
        for option in &offer.destinations {
            let item = node(&document, &select, "option", "", &option.label)?;
            item.set_attribute("value", &option.destination.to_string())?;
        }
        Some(select)
    };
    let offered = offer.clone();
    let button = button(client, &wrapper, &offer.label, move |client| {
        let mut savage = false;
        let mut graze = false;
        let mut action_choices = Vec::new();
        for (key, input) in &inputs {
            let value = input.value();
            if key == "tactical-choice" {
                if !offered.input_groups.iter().any(|group| {
                    group.group_id == *key
                        && group.options.iter().any(|option| option.option_id == value)
                }) {
                    feedback(&client.borrow(), "Choose an offered option.");
                    return;
                }
                action_choices.push(rpc::JourneyChoice {
                    group_id: key.clone(),
                    option_id: value,
                });
                continue;
            }
            if !matches!(value.as_str(), "yes" | "no") {
                feedback(&client.borrow(), "Choose an offered attack approach.");
                return;
            }
            match key.as_str() {
                "savage-attacker" => savage = value == "yes",
                "graze" => graze = value == "yes",
                _ => {
                    feedback(
                        &client.borrow(),
                        "The server offered an unsupported required input.",
                    );
                    return;
                }
            }
        }
        let selected = if let Some(select) = &destination {
            let Ok(selected) = select.value().parse::<i32>() else {
                feedback(&client.borrow(), "Choose an offered destination.");
                return;
            };
            if !offered
                .destinations
                .iter()
                .any(|option| option.destination == selected)
            {
                feedback(&client.borrow(), "Choose an offered destination.");
                return;
            }
            selected
        } else {
            rpc::HarborDestination::Unspecified as i32
        };
        match prepare(&client, &offered, None, savage, graze, selected) {
            Ok(mut body) => {
                body.action_choices = action_choices;
                send(client, body, false);
            }
            Err(error) => feedback(&client.borrow(), &error),
        }
    })?;
    button.set_class_name("offered");
    button.set_attribute("data-action-kind", &offer.action_kind.to_string())?;
    if client.borrow().busy
        || (!client.borrow().last_confirmed && client.borrow().last_request.is_some())
    {
        button.set_attribute("disabled", "")?;
    } else {
        button.remove_attribute("disabled")?;
    }
    Ok(())
}
fn receive_view(
    client: &Rc<RefCell<Client>>,
    generation: &ConnectionGeneration,
    scope: ViewScope,
    view: rpc::ViewMessage,
) -> Result<(), JsValue> {
    // PLAY-V09: the plate picture a display view names is fetched at once, usually while the
    // scene before it is still being spoken. A refused fetch only means the cut to the plate waits
    // for the picture to load.
    if let Some(rpc::view_message::Audience::Display(display)) = &view.audience
        && let Some(scene) = display.scene.as_ref()
        && let Some(url) = crate::presentation::imagery_url(&scene.live_plate)
        && let Some(tv) = client.borrow().live.tv.as_ref()
    {
        let _fetch_only_helps = tv.preload_plate(&url);
    }
    let acceptance = {
        let mut state = client.borrow_mut();
        let views = state
            .views
            .as_mut()
            .ok_or_else(|| JsValue::from_str("view owner absent"))?;
        let accepted = views
            .accept(generation, scope, view.clone())
            .map_err(|_| JsValue::from_str("server view scope or projection invalid"))?;
        // PLAY-L01: the display's view of an equal revision that differs only in generated art
        // replaces the shown one and redraws; nothing else is accepted at an equal revision.
        if matches!(
            accepted,
            ConnectionViewAcceptance::View(ViewAcceptance::Duplicate { .. })
        ) && views.accept_art_update(generation, scope, &view, crate::live_art::merge_art)
        {
            drop(state);
            redraw_current(client)?;
            return live_health::phone_sync(client);
        }
        accepted
    };
    if acceptance != ConnectionViewAcceptance::ObsoleteGeneration {
        // Any view (even one already held) proves the connection is good again.
        {
            let mut state = client.borrow_mut();
            state.reconnect.attempt = 0;
            show_notice(&state, None);
        }
        auto_confirm(client);
    }
    if acceptance == ConnectionViewAcceptance::View(ViewAcceptance::Applied) {
        let mut state = client.borrow_mut();
        state.revision = state
            .views
            .as_ref()
            .and_then(|views| views.current().and_then(|view| view.revision));
        drop(state);
        // A fresh server view supersedes the last message shown in the toast.
        feedback(&client.borrow(), "");
        if client.borrow().role == "display" {
            // The display draws through the presentation director, which orders the change.
            live_motion::on_view(client)?;
        } else {
            redraw_current(client)?;
            live_health::phone_sync(client)?;
            live_voice::observe(client);
            live_motion::on_player_view(client)?;
        }
        live_music::observe(client);
    }
    Ok(())
}

fn redraw_current(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let view = client
        .borrow()
        .views
        .as_ref()
        .and_then(|views| views.current().cloned());
    if let Some(view) = view
        && let Err(error) = render(client, view)
    {
        // A view that cannot be drawn must never fail silently into a blank screen.
        web_sys::console::error_2(&JsValue::from_str("gameplay view not drawn:"), &error);
        return Err(error);
    }
    Ok(())
}

/// Only admitted snapshots or a local redraw of the retained snapshot reach rendering.
fn render(client: &Rc<RefCell<Client>>, view: rpc::ViewMessage) -> Result<(), JsValue> {
    let (document, root, role) = {
        let mut state = client.borrow_mut();
        let document = state.document.clone();
        state.dialogue.capture_focus(&document)?;
        clear_callbacks(&mut state);
        (document, state.root.clone(), state.role)
    };
    let (narration, scene, journey, offers, clue) = match view.audience {
        Some(rpc::view_message::Audience::Player(player)) if role == "player" => (
            player.narration,
            player.scene,
            player.journey,
            player.offers,
            player.private_clue,
        ),
        Some(rpc::view_message::Audience::Display(display)) if role == "display" => (
            display.narration,
            display.scene,
            display.journey,
            Vec::new(),
            String::new(),
        ),
        _ => return Err(JsValue::from_str("server audience mismatch")),
    };
    let journey = journey.ok_or_else(|| JsValue::from_str("server journey missing"))?;
    if journey.party.len() > 2 || offers.len() > 12 {
        return Err(JsValue::from_str("server view exceeds bound"));
    }
    if role == "display" {
        // PLAY-H01: the battlefield draws the server's health variants of these combatants.
        live_health::publish_battlefield_variants(&journey, scene.as_ref());
    }
    let surface = root
        .query_selector(".view")?
        .ok_or_else(|| JsValue::from_str("view absent"))?;
    root.set_attribute("data-phase", &journey.phase.to_string())?;
    live_combat::sheet::sync(client)?;
    if let Some(creation_offer) = journey.creation.as_ref() {
        if let Some(scene) = &client.borrow().scene_delivery {
            scene
                .retire()
                .map_err(|_| JsValue::from_str("retired scene cleanup failed"))?;
        }
        let offered = offers
            .iter()
            .find(|offer| offer.action_kind == rpc::GameplayActionKind::CreateCharacter as i32)
            .ok_or_else(|| JsValue::from_str("character action missing"))?;
        live_screens::release(client, &surface)?;
        if client.borrow().creation.is_none() {
            surface.set_text_content(None);
        }
        creation(client, &surface, creation_offer, offered)?;
        dialogue_recovery::mount(client, &surface, &offers)?;
        // Recovery remains reachable while a creation submission is uncertain.
        // The feedback node is kept (and moved last) so its message survives redraws.
        let kept_feedback = surface.query_selector(".feedback")?;
        for stale in [".recovery-needed", ".trouble"] {
            if let Some(previous) = surface.query_selector(stale)? {
                previous.remove();
            }
        }
        recovery_prompt(client, &surface)?;
        match kept_feedback {
            Some(feedback) => {
                surface.append_child(&feedback)?;
            }
            None => {
                node(&document, &surface, "p", "feedback", "")?;
            }
        }
        recovery_details(client, &surface)?;
        return Ok(());
    }
    if let Some(creation) = client.borrow_mut().creation.take() {
        creation
            .dispose()
            .map_err(|_| JsValue::from_str("character retirement failed"))?;
    }
    let journey_phase = rpc::JourneyPhase::try_from(journey.phase)
        .map_err(|_| JsValue::from_str("unknown required phase"))?;
    if let Some(shown) = scene.as_ref()
        && live_screens::takes_over(journey_phase, &journey, shown)
    {
        // Generated adventures name their own art from the served pool; the screens fall back
        // to a built-in scene when a file is absent, so any well-formed local path is accepted.
        if public_scene_assets::resolve(&shown.scene_asset).is_err()
            && live_screens::asset_url(&shown.scene_asset).is_none()
        {
            return Err(JsValue::from_str("unknown required public scene asset"));
        }
        surface.set_attribute("data-destination", &shown.destination.to_string())?;
        live_combat::release(client)?;
        return live_screens::render(
            client,
            &surface,
            &live_screens::SceneInput {
                phase: journey_phase,
                narration: &narration,
                scene: shown,
                journey: &journey,
                offers: &offers,
                clue: &clue,
            },
        );
    }
    let scene = scene.ok_or_else(|| JsValue::from_str("server scene missing"))?;
    let live_view = live_combat::LiveView {
        surface: &surface,
        narration: &narration,
        scene: &scene,
        journey: &journey,
        offers: &offers,
    };
    if live_combat::present(client, &live_view)? {
        return Ok(());
    }
    live_screens::release(client, &surface)?;
    live_combat::release(client)?;
    public_scene_assets::resolve(&scene.scene_asset)
        .map_err(|_| JsValue::from_str("unknown required public scene asset"))?;
    let (delivery, scope, revision) = {
        let mut state = client.borrow_mut();
        let views = state
            .views
            .as_ref()
            .ok_or_else(|| JsValue::from_str("scene view owner absent"))?;
        let scope = views.scope().cache_scope();
        let revision = views
            .current_revision()
            .ok_or_else(|| JsValue::from_str("scene revision absent"))?;
        if state.scene_delivery.is_none() {
            surface.set_text_content(None);
            state.scene_delivery = Some(
                PublicSceneDelivery::new(&document, &surface)
                    .map_err(|error| JsValue::from_str(&error.to_string()))?,
            );
        }
        let delivery = state
            .scene_delivery
            .as_ref()
            .cloned()
            .ok_or_else(|| JsValue::from_str("campaign scene owner absent"))?;
        (delivery, scope, revision)
    };
    if !delivery.is_attached() {
        surface.set_text_content(None);
        delivery
            .attach()
            .map_err(|_| JsValue::from_str("scene fallback attachment failed"))?;
    }
    // Only GameplayViews' admitted canonical scope/revision reaches the byte producer.
    // Optional image failure leaves the existing server-projected inputs available.
    if delivery
        .update(scope, revision, &scene.scene_asset)
        .is_err()
        || delivery.take_failure().is_some()
    {
        status(
            &client.borrow(),
            "The scene illustration is using its fallback.",
        );
    }
    let (active, queued) = delivery.work_counts();
    surface.set_attribute("data-public-scene-active", &active.to_string())?;
    surface.set_attribute("data-public-scene-queued", &queued.to_string())?;
    surface.set_attribute("data-destination", &scene.destination.to_string())?;
    let body = if let Some(body) = surface.query_selector(".scene-copy")? {
        body.set_text_content(None);
        body
    } else {
        node(&document, &surface, "div", "scene-copy", "")?
    };
    node(
        &document,
        &body,
        "div",
        "eyebrow",
        if role == "display" {
            "Your shared adventure"
        } else {
            "Your story · your choices"
        },
    )?;
    node(
        &document,
        &body,
        if role == "display" { "h1" } else { "h2" },
        "",
        &scene.title,
    )?;
    node(&document, &body, "p", "narration", &narration)?;
    let phase = rpc::JourneyPhase::try_from(journey.phase)
        .map_err(|_| JsValue::from_str("unknown required phase"))?;
    if phase == rpc::JourneyPhase::Room {
        let share = node(&document, &body, "div", "room-share", "")?;
        node(&document, &share, "span", "scope", "Join this adventure")?;
        node(&document, &share, "div", "room-code", &journey.room_code)?;
        let link = node(&document, &share, "a", "join-link", "Open player client")?;
        if !journey.join_path.starts_with("/gameplay/player?room=") {
            return Err(JsValue::from_str("join route invalid"));
        }
        link.set_attribute("href", &journey.join_path)?;
        link.set_attribute("target", "_blank")?;
    }
    let party = node(&document, &body, "div", "party", "")?;
    for member in &journey.party {
        node(
            &document,
            &party,
            "div",
            "party-member",
            &format!(
                "{} · {}",
                member.name,
                if member.character_ready {
                    "Ready"
                } else {
                    "Creating"
                }
            ),
        )?;
    }
    if !journey.dialogue.is_empty() {
        let speech = node(&document, &body, "div", "dialogue", "")?;
        let portrait = node(&document, &speech, "img", "speaker-portrait", "")?;
        portrait.set_attribute("src", "/assets/concept-art/vell-avatar.webp")?;
        portrait.set_attribute("alt", "Concept portrait illustrating the speaker")?;
        node(&document, &speech, "span", "eyebrow", &journey.speaker)?;
        node(&document, &speech, "blockquote", "", &journey.dialogue)?;
    }
    if !clue.is_empty() {
        node(&document, &body, "p", "clue", &clue)?;
    }
    if let Some(sheet) = journey.own_character.as_ref() {
        let sheet = player_sheet_view(sheet);
        live_combat::sheet::summary(client, &body, &sheet)?;
    }

    dialogue_recovery::mount(client, &body, &offers)?;
    let actions = node(&document, &body, "div", "scene-actions", "")?;
    for offer in offers {
        action_button(client, &actions, offer)?;
    }
    recovery_prompt(client, &body)?;
    node(&document, &body, "p", "feedback", "")?;
    recovery_details(client, &body)?;
    Ok(())
}

/// What currently needs recovery: a lost connection, or a choice whose outcome is unknown
/// (sent, but no committed receipt seen yet and no request still in flight).
struct RecoveryNeeds {
    lost_connection: bool,
    unknown_choice: bool,
}
fn recovery_needs(state: &Client) -> RecoveryNeeds {
    RecoveryNeeds {
        lost_connection: state.connection.is_none(),
        unknown_choice: state.last_request.is_some() && !state.last_confirmed && !state.busy,
    }
}
fn confirm_saved_choice(client: &Rc<RefCell<Client>>, parent: &Element) -> Result<(), JsValue> {
    button(client, parent, "Confirm saved choice", |client| {
        replay(client, false)
    })?;
    Ok(())
}
/// Shows a recovery control prominently only while its condition holds. A dialogue
/// confirmation of unknown outcome has its own retry button inside the dialogue panel.
fn recovery_prompt(client: &Rc<RefCell<Client>>, _parent: &Element) -> Result<(), JsValue> {
    live_chrome::prompt(client)
}
/// The rarely used recovery controls live in the player's menu sheet, not in the view.
fn recovery_details(client: &Rc<RefCell<Client>>, _parent: &Element) -> Result<(), JsValue> {
    live_chrome::fill_menu(client)
}
fn new_client(
    document: &Document,
    root: Element,
    role: &'static str,
) -> Result<Rc<RefCell<Client>>, JsValue> {
    live_theme::install(document)?;
    let credential = root.get_attribute("data-local-binding").unwrap_or_default();
    root.remove_attribute("data-local-binding")?;
    if let Some(milliseconds) = root
        .get_attribute("data-request-timeout-ms")
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|milliseconds| (1_000..=600_000).contains(milliseconds))
    {
        REQUEST_TIMEOUT.with(|timeout| timeout.set(std::time::Duration::from_millis(milliseconds)));
    }
    let served_run = root.get_attribute("data-run").filter(|run| !run.is_empty());
    // The `?room=` link wins over the code the page was served with.
    let linked = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|query| {
            query
                .trim_start_matches('?')
                .split('&')
                .find_map(|pair| pair.strip_prefix("room=").and_then(clean_room_code))
        });
    let room_code = linked
        .or_else(|| {
            root.get_attribute("data-room")
                .and_then(|code| clean_room_code(&code))
        })
        .unwrap_or_else(|| "LANTERN".to_owned());
    let client = Rc::new(RefCell::new(Client {
        document: document.clone(),
        root,
        role,
        credential,
        room_code,
        session: if role == "display" {
            Some(rpc::SessionId {
                value: Some(vec![0x41; 16]),
            })
        } else {
            None
        },
        run: if role == "display" {
            Some(rpc::RunId {
                value: Some(vec![0x42; 16]),
            })
        } else {
            None
        },
        binding: if role == "display" {
            Some(rpc::ClientBindingId {
                value: Some(vec![0x72; 16]),
            })
        } else {
            None
        },
        generation: 0,
        revision: None,
        views: None,
        join_request: None,
        last_request: None,
        first_attack: None,
        last_confirmed: false,
        last_accepted: false,
        action: None,
        sessions: None,
        connection: None,
        watch_abort: None,
        callbacks: Vec::new(),
        creation: None,
        creation_offer: None,
        live: live_screens::LiveScreens::default(),
        chrome: live_chrome::Chrome::default(),
        scene_delivery: None,
        dialogue: dialogue_recovery::DialogueRecovery::default(),
        combat: live_combat::Live::default(),
        busy: false,
        reconnect: Reconnect::default(),
        served_run,
        join_retries: 0,
        auto_confirms: 0,
        feedback_generation: 0,
    }));
    live_chrome::mount(&client)?;
    Ok(client)
}

/// Resumes a credential saved on this device through the ordinary reconnect path.
fn resume_or_join(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let saved = saved_player::load(&client.borrow().room_code);
    let stale = match (&saved, client.borrow().served_run.as_deref()) {
        (Ok(Some(saved)), Some(served)) => !saved_player::is_from_run(saved, served),
        _ => false,
    };
    if stale {
        // The seat was joined in another server run; this server cannot know it.
        let cleared = saved_player::clear(&client.borrow().room_code);
        render_join(client)?;
        if cleared.is_err() {
            feedback(
                &client.borrow(),
                "This browser could not forget the earlier game. Clear this site's data, then reload.",
            );
        }
        return Ok(());
    }
    match saved {
        Ok(Some(saved)) => {
            {
                let mut state = client.borrow_mut();
                state.credential = saved.credential;
                state.session = Some(saved.session);
                state.run = Some(saved.run);
                state.binding = Some(saved.binding);
            }
            render_resume(client)?;
            connect(client.clone());
        }
        Ok(None) => render_join(client)?,
        Err(_) => {
            render_join(client)?;
            feedback(
                &client.borrow(),
                "This browser cannot remember your game, so a reload would start over.",
            );
        }
    }
    Ok(())
}

pub(super) fn start() -> Result<(), JsValue> {
    if CLIENTS.with(|clients| !clients.borrow().is_empty()) {
        return Err(JsValue::from_str("gameplay clients already mounted"));
    }
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("document absent"))?;
    // PLAY-F02 mount point: `#battle-preview` shows the 3D battlefield alone, with no server calls.
    if live_battlefield::preview_requested() {
        return live_battlefield::start_preview(&document);
    }
    for role in ["player", "display"] {
        let Some(root) = document.get_element_by_id(role) else {
            continue;
        };
        let client = new_client(&document, root, role)?;
        CLIENTS.with(|clients| clients.borrow_mut().push(client.clone()));
        if role == "player" {
            resume_or_join(&client)?;
        } else {
            live_tv::mount_splash(&document, &client.borrow().root)?;
            connect(client);
        }
    }
    Ok(())
}

// Development fixture reaches the same production Client, admission and render paths.
// It issues local connection generations, never credentials or a network game decision.
#[cfg(feature = "public-scene-delivery-fixture")]
pub mod fixture {
    use super::*;
    thread_local! {
        static CLIENT: RefCell<Option<Rc<RefCell<Client>>>> = const { RefCell::new(None) };
        static CONNECTION: RefCell<Option<RpcConnection<()>>> = const { RefCell::new(None) };
        static OLD: RefCell<Option<ConnectionGeneration>> = const { RefCell::new(None) };
        static CLOSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    fn client() -> Result<Rc<RefCell<Client>>, JsValue> {
        CLIENT
            .with(|client| client.borrow().as_ref().cloned())
            .ok_or_else(|| JsValue::from_str("fixture owner absent"))
    }
    fn scope(binding: u8) -> Result<ViewScope, JsValue> {
        ViewScope::from_wire(
            Some(&rpc::SessionId {
                value: Some(vec![0x41; 16]),
            }),
            Some(&rpc::RunId {
                value: Some(vec![0x42; 16]),
            }),
            Some(&rpc::ClientBindingId {
                value: Some(vec![binding; 16]),
            }),
            ViewRole::Player,
        )
        .map_err(|_| JsValue::from_str("fixture scope invalid"))
    }
    pub fn mount() -> Result<(), JsValue> {
        if CLOSED.with(std::cell::Cell::get) || CLIENT.with(|client| client.borrow().is_some()) {
            return Err(JsValue::from_str("fixture already mounted or disposed"));
        }
        // Referenced for the same module's real production entry point; not invoked here.
        let _production_entry: fn() -> Result<(), JsValue> = start;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document absent"))?;
        let root = document
            .get_element_by_id("player")
            .ok_or_else(|| JsValue::from_str("actual player host absent"))?;
        let client = new_client(&document, root, "player")?;
        let connection = RpcConnection::new(());
        client.borrow_mut().views = Some(GameplayViews::new(scope(0x71)?, connection.generation()));
        CLIENT.with(|slot| *slot.borrow_mut() = Some(client));
        CONNECTION.with(|slot| *slot.borrow_mut() = Some(connection));
        Ok(())
    }
    pub fn present(
        epoch: u32,
        sequence: u32,
        tavern: bool,
        display: bool,
        wrong_scope: bool,
        old_generation: bool,
    ) -> Result<(), JsValue> {
        let client = client()?;
        let generation = if old_generation {
            OLD.with(|slot| slot.borrow().clone())
        } else {
            CONNECTION.with(|slot| slot.borrow().as_ref().map(RpcConnection::generation))
        }
        .ok_or_else(|| JsValue::from_str("generation absent"))?;
        let current_scope = client
            .borrow()
            .views
            .as_ref()
            .map(GameplayViews::scope)
            .ok_or_else(|| JsValue::from_str("view scope absent"))?;
        let scene = rpc::GameplayScene {
            destination: rpc::HarborDestination::HarborInn as i32,
            title: if tavern {
                "The harbor inn"
            } else {
                "Greyhaven harbor"
            }
            .to_owned(),
            scene_asset: if tavern {
                "assets/concept-art/scene-tavern-barkeep-talk-rain.webp"
            } else {
                "assets/ui/scenes/mara-harbor-v4.png"
            }
            .to_owned(),
            speaker_portrait: String::new(),
        };
        let journey = rpc::JourneyView {
            phase: rpc::JourneyPhase::Opening as i32,
            ..Default::default()
        };
        let audience = if display {
            rpc::view_message::Audience::Display(rpc::DisplayGameplayView {
                narration: "Public narration".to_owned(),
                scene: Some(scene),
                journey: Some(journey),
                ..Default::default()
            })
        } else {
            rpc::view_message::Audience::Player(rpc::PlayerGameplayView {
                narration: "Accepted player narration".to_owned(),
                scene: Some(scene),
                journey: Some(journey),
                offers: vec![rpc::GameplayActionOffer {
                    offer_id: "fixture-public-offer".to_owned(),
                    action_kind: rpc::GameplayActionKind::ExamineHarborSeal as i32,
                    label: "Examine the seal".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            })
        };
        receive_view(
            &client,
            &generation,
            if wrong_scope {
                scope(0x73)?
            } else {
                current_scope
            },
            rpc::ViewMessage {
                revision: Some(rpc::SessionRevision {
                    epoch: Some(rpc::RecoveryEpoch {
                        value: Some(u64::from(epoch)),
                    }),
                    sequence: Some(u64::from(sequence)),
                }),
                audience: Some(audience),
            },
        )
    }
    pub fn reconnect() -> Result<(), JsValue> {
        let client = client()?;
        retire_transport(&mut client.borrow_mut());
        CONNECTION.with(|slot| {
            if let Some(old) = slot.borrow_mut().take() {
                OLD.with(|saved| *saved.borrow_mut() = Some(old.generation()));
                old.close();
            }
        });
        let connection = RpcConnection::new(());
        if !client
            .borrow_mut()
            .views
            .as_mut()
            .ok_or_else(|| JsValue::from_str("view owner absent"))?
            .reconnect(connection.generation())
        {
            return Err(JsValue::from_str("reconnect refused"));
        }
        CONNECTION.with(|slot| *slot.borrow_mut() = Some(connection));
        redraw_current(&client)
    }
    pub fn replace_scope(binding: u8) -> Result<(), JsValue> {
        let client = client()?;
        clear_view_scope(&mut client.borrow_mut())?;
        let generation = CONNECTION
            .with(|slot| slot.borrow().as_ref().map(RpcConnection::generation))
            .ok_or_else(|| JsValue::from_str("generation absent"))?;
        client.borrow_mut().views = Some(GameplayViews::new(scope(binding)?, generation));
        Ok(())
    }
    pub fn counts() -> Result<u32, JsValue> {
        let client = client()?;
        let state = client.borrow();
        let (active, queued) = state
            .scene_delivery
            .as_ref()
            .map_or((0, 0), |owner| owner.work_counts());
        Ok((active as u32) << 16 | queued as u32)
    }
    pub fn failure() -> Result<String, JsValue> {
        let client = client()?;
        let state = client.borrow();
        Ok(state
            .scene_delivery
            .as_ref()
            .and_then(|owner| owner.take_failure())
            .map_or_else(|| "none".to_owned(), |error| format!("{error:?}")))
    }
    pub fn dispose() -> Result<(), JsValue> {
        CLOSED.with(|closed| closed.set(true));
        let client = CLIENT
            .with(|slot| slot.borrow_mut().take())
            .ok_or_else(|| JsValue::from_str("fixture owner absent"))?;
        let mut state = client.borrow_mut();
        retire_transport(&mut state);
        clear_callbacks(&mut state);
        if let Some(owner) = state.scene_delivery.take() {
            owner
                .dispose()
                .map_err(|_| JsValue::from_str("scene disposal failed"))?;
        }
        state.views = None;
        CONNECTION.with(|slot| {
            if let Some(connection) = slot.borrow_mut().take() {
                connection.close();
            }
        });
        OLD.with(|slot| slot.borrow_mut().take());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ACTION_REFUSAL_FEEDBACK, ROOM_CONNECTION_UNAVAILABLE, SERVER_LOST, record_join_retry,
        should_replace_feedback,
    };

    #[test]
    fn rejected_action_feedback_survives_view_updates_until_replaced_or_expired() {
        assert_eq!(
            ACTION_REFUSAL_FEEDBACK,
            "That did not work. Try again, or pick something else."
        );
        assert!(!should_replace_feedback("", true));
        assert!(should_replace_feedback("", false));
        assert!(should_replace_feedback("A newer message", true));
    }

    #[test]
    fn unavailable_tunnel_retries_stop_at_the_configured_limit() {
        let mut retries = 0;
        for _ in 0..crate::reconnect::JOIN_RETRIES {
            assert!(record_join_retry(ROOM_CONNECTION_UNAVAILABLE, &mut retries));
        }
        assert_eq!(retries, crate::reconnect::JOIN_RETRIES);
        assert!(!record_join_retry(
            ROOM_CONNECTION_UNAVAILABLE,
            &mut retries
        ));
        assert_eq!(retries, crate::reconnect::JOIN_RETRIES);
    }

    #[test]
    fn lost_server_state_is_never_retried() {
        let mut retries = 0;
        assert!(!record_join_retry(SERVER_LOST, &mut retries));
        assert_eq!(retries, 0);
    }
}
