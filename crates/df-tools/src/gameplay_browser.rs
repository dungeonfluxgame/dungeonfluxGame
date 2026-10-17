//! Thin server-projected room, creation, dialogue and combat clients.
mod dialogue_recovery;
mod hero_creator;
mod layout;
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
mod live_notebook;
mod live_presence;
mod live_rules;
mod live_screens;
mod live_sfx;
mod live_stage;
mod live_theme;
mod live_tv;
mod live_voice;
mod phone_controls;
mod presentation_timing;
mod public_scene_assets;
mod seat_recovery;
mod talk_control;
mod view_delivery;

use df_client::{
    connection::{ConnectionGeneration, RpcConnection},
    connection_views::ConnectionViewAcceptance,
    revisions::ViewAcceptance,
};
use df_protocol::common as rpc;
use df_rpc_bridge::{BrowserChannel, BrowserConnection, RPC_MESSAGE_BYTES};
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
    reclaim_request: Option<rpc::ReclaimRoomRequest>,
    away_heroes: Vec<rpc::AwayHero>,
    seat_poll_abort: Option<AbortHandle>,
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
    creation: Option<hero_creator::HeroCreator>,
    creation_preview: Option<(Rc<()>, AbortHandle)>,
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
    /// Public preparation response survives reconnect redraws until the new page is loaded.
    preparing_adventure: bool,
    /// Times the page has confirmed an unconfirmed choice by itself since it was sent.
    auto_confirms: u8,
    feedback_generation: u64,
    /// The page's listener for returning to the front (see [`attach_visibility`]).
    visibility: Option<VisibilityWatch>,
}

/// The document listener that reconnects when a hidden page comes back to the front.
struct VisibilityWatch {
    document: Document,
    callback: Closure<dyn FnMut(web_sys::Event)>,
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
        presentation_timing::disconnect(self);
        cancel_creation_preview(self);
        seat_recovery::stop(self);
        if let Some(abort) = self.watch_abort.take() {
            abort.abort();
        }
        if let Some(connection) = self.connection.take() {
            connection.close();
        }
        clear_callbacks(self);
        if let Some(watch) = self.visibility.take() {
            let _removed = watch.document.remove_event_listener_with_callback(
                "visibilitychange",
                watch.callback.as_ref().unchecked_ref(),
            );
        }
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
const ACTION_REFUSAL_FEEDBACK: &str = "The game could not accept that. Choose again.";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubmitFailure {
    Refused,
    ServerLost,
    Unknown,
}
fn classify_submit_failure(code: tonic::Code) -> SubmitFailure {
    match code {
        tonic::Code::InvalidArgument
        | tonic::Code::FailedPrecondition
        | tonic::Code::PermissionDenied
        | tonic::Code::ResourceExhausted => SubmitFailure::Refused,
        tonic::Code::DataLoss => SubmitFailure::ServerLost,
        _ => SubmitFailure::Unknown,
    }
}
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
    activate: impl FnMut(Rc<RefCell<Client>>) + 'static,
) -> Result<Element, JsValue> {
    let element = node(&client.borrow().document, parent, "button", "", label)?;
    bind_click(client, &element, activate)?;
    Ok(element)
}
fn bind_click(
    client: &Rc<RefCell<Client>>,
    element: &Element,
    mut activate: impl FnMut(Rc<RefCell<Client>>) + 'static,
) -> Result<(), JsValue> {
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
    Ok(())
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
    presentation_timing::disconnect(client);
    cancel_creation_preview(client);
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
    cancel_creation_preview(client);
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
    live_screens::render_join(client)?;
    seat_recovery::ensure_poll(client)?;
    phone_controls::bind(client)?;
    presentation_timing::paint(client);
    Ok(())
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
        state.away_heroes.clear();
        state.reclaim_request = None;
        retire_transport(&mut state);
        let reset = clear_view_scope(&mut state);
        state.credential.clear();
        state.session = None;
        state.run = None;
        state.binding = None;
        state.join_request = None;
        cleared.and(reset)
    };
    let cleared = cleared.and(seat_recovery::clear(client));
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
    let different_reclaim_room = {
        let state = client.borrow();
        state.room_code != code && state.reclaim_request.is_some()
    };
    if different_reclaim_room && seat_recovery::clear(client).is_err() {
        feedback(
            &client.borrow(),
            "This browser could not clear the earlier request. Try again.",
        );
        return false;
    }
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
                operation_id: match state.reclaim_request.as_ref() {
                    Some(reclaim) => reclaim.operation_id.clone(),
                    None => Some(operation()?),
                },
                join_secret: random::<32>()?,
            });
        }
        let body = state.join_request.clone().ok_or("Room proof unavailable")?;
        retire_transport(&mut state);
        state.busy = true;
        status(&state, "Joining your companions…");
        Ok((body, state.generation, state.reclaim_request.clone()))
    })();
    let (body, generation, reclaim) = match prepared {
        Ok(value) => value,
        Err(error) => {
            feedback(&client.borrow(), &error);
            return;
        }
    };
    wasm_bindgen_futures::spawn_local(async move {
        let reclaiming = reclaim.is_some();
        let run = async {
            if let Some(reclaim) = reclaim {
                return seat_recovery::call(reclaim)
                    .await
                    .map(|reply| rpc::JoinRoomResponse {
                        outcome: reply.outcome.map(|outcome| match outcome {
                            rpc::reclaim_room_response::Outcome::Joined(joined) => {
                                rpc::join_room_response::Outcome::Joined(joined)
                            }
                            rpc::reclaim_room_response::Outcome::Refused(_) => {
                                rpc::join_room_response::Outcome::Refused(rpc::JoinRefusal {
                                    code: rpc::RejectionCode::InvalidSelection as i32,
                                })
                            }
                        }),
                    });
            }
            let url = crate::browser::tunnel_url("/gameplay/tunnel")?;
            let (connection, channel) = BrowserConnection::connect(&url)
                .await
                .map_err(|_| ROOM_CONNECTION_UNAVAILABLE.to_owned())?;
            if let Err(error) = recover_adventure(&client, channel.clone()).await {
                connection.close();
                return Err(error);
            }
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
                        seat_recovery::stop(&mut state);
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
                    let recovery_saved = seat_recovery::clear(&client);
                    connect(client.clone());
                    if saved.is_err() || recovery_saved.is_err() {
                        feedback(
                            &client.borrow(),
                            "This browser cannot remember your game, so a reload would start over.",
                        );
                    }
                }
                Some(rpc::join_room_response::Outcome::Refused(refusal)) => {
                    if reclaiming {
                        client.borrow_mut().join_request = None;
                        if seat_recovery::clear(&client).is_err() {
                            feedback(
                                &client.borrow(),
                                "This browser could not clear the request. Try again.",
                            );
                            return;
                        }
                        let _rendered = render_join(&client);
                        feedback(&client.borrow(), seat_recovery::REFUSED);
                        return;
                    }
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
                        if error == NEW_ADVENTURE {
                            NEW_ADVENTURE
                        } else {
                            "The room did not answer yet. Trying again."
                        },
                    );
                    schedule_join_retry(&client, error == NEW_ADVENTURE);
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
    if error == NEW_ADVENTURE {
        return true;
    }
    if error == SERVER_LOST || *retries >= crate::reconnect::JOIN_RETRIES {
        return false;
    }
    *retries += 1;
    true
}

/// Repeats the pending join after a backoff while the page shows "Reconnecting…".
fn schedule_join_retry(client: &Rc<RefCell<Client>>, preparing: bool) {
    let (token, wait) = {
        let mut state = client.borrow_mut();
        state.reconnect.token += 1;
        let jitter = (js_sys::Math::random() * 1000.0) as u32;
        let wait = crate::reconnect::delay(state.reconnect.attempt, jitter);
        state.reconnect.attempt = state.reconnect.attempt.saturating_add(1);
        show_notice(
            &state,
            Some(if preparing {
                NEW_ADVENTURE
            } else {
                RECONNECTING
            }),
        );
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
/// Listens for the page returning to the front. A page that was hidden has most likely lost its
/// socket while it slept, so it reconnects at once instead of waiting out a retry backoff.
fn attach_visibility(document: &Document, client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let hidden_since: Rc<std::cell::Cell<Option<f64>>> = Rc::new(std::cell::Cell::new(None));
    let weak = Rc::downgrade(client);
    let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
        let Some(client) = weak.upgrade() else {
            return;
        };
        let document_hidden = client.borrow().document.hidden();
        if document_hidden {
            hidden_since.set(Some(js_sys::Date::now()));
        } else if hidden_since.take().is_some() {
            reconnect_now(&client);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    document
        .add_event_listener_with_callback("visibilitychange", callback.as_ref().unchecked_ref())?;
    client.borrow_mut().visibility = Some(VisibilityWatch {
        document: document.clone(),
        callback,
    });
    Ok(())
}

/// Starts the game connection now, without waiting out a backoff. A retry already scheduled is
/// cancelled by the connect itself.
fn reconnect_now(client: &Rc<RefCell<Client>>) {
    // A page without a saved seat is on the join screen; returning to it must not redraw that.
    if client.borrow().credential.is_empty() {
        return;
    }
    client.borrow_mut().reconnect.attempt = 0;
    connect(client.clone());
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
            if let Err(error) = recover_adventure(&owned, channel.clone()).await {
                connection.close();
                return Err(error);
            }
            // The owner's view carries the combat section (reachable cells, legal targets, budget),
            // which exceeds 8 KiB in a three-enemy fight; RPC_MESSAGE_BYTES is the bridge's RPC ceiling.
            let mut sessions = SessionClient::new(channel.clone())
                .max_decoding_message_size(RPC_MESSAGE_BYTES)
                .max_encoding_message_size(RPC_MESSAGE_BYTES);
            let timing_channel = channel.clone();
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
            presentation_timing::connect(&owned, timing_channel);
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
            while let Some(view) = stream.message().await.map_err(|status| {
                if status.code() == tonic::Code::PermissionDenied {
                    SAVED_GAME_REFUSED
                } else {
                    "Connection interrupted; reconnect to recover"
                }
            })? {
                if owned.borrow().generation != generation {
                    return Ok(());
                }
                // A view that cannot be drawn is reported (redraw_current logs why) and the stream
                // stays open: dropping it would reconnect, receive the same view and fail again.
                if receive_view(&owned, &view_generation, scope, view).is_err() {
                    status(&owned.borrow(), "Server view could not be displayed");
                }
            }
            Ok(())
        };
        let result = Abortable::new(run, registration).await;
        if owned.borrow().generation == generation {
            let refused = matches!(&result, Ok(Err(error)) if error == SAVED_GAME_REFUSED);
            if owned.borrow().role == "player" && refused {
                forget_saved_game(&owned, "That hero is being played on another phone.");
                return;
            }
            presentation_timing::disconnect(&owned.borrow());
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

const NEW_ADVENTURE: &str = "A new adventure is being prepared...";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdventureRecovery {
    Current,
    Preparing,
    NewGame,
}

fn classify_adventure_recovery(
    current: &rpc::CurrentAdventureResponse,
    served_run: Option<&str>,
    room: &str,
) -> AdventureRecovery {
    if current.preparing {
        AdventureRecovery::Preparing
    } else if served_run
        .is_some_and(|run| !current.run_identity.is_empty() && run != current.run_identity)
        || current.room_code != room
    {
        AdventureRecovery::NewGame
    } else {
        AdventureRecovery::Current
    }
}

async fn recover_adventure(
    client: &Rc<RefCell<Client>>,
    channel: BrowserChannel,
) -> Result<(), String> {
    let mut request = tonic::Request::new(rpc::CurrentAdventureRequest {});
    request.set_timeout(request_timeout());
    let current = RoomClient::new(channel)
        .max_decoding_message_size(8192)
        .max_encoding_message_size(8192)
        .current_adventure(request)
        .await
        .map_err(|_| "Current adventure unavailable".to_owned())?
        .into_inner();
    let recovery = {
        let state = client.borrow();
        classify_adventure_recovery(&current, state.served_run.as_deref(), &state.room_code)
    };
    match recovery {
        AdventureRecovery::Current => {
            client.borrow_mut().preparing_adventure = false;
            Ok(())
        }
        AdventureRecovery::Preparing => {
            client.borrow_mut().preparing_adventure = true;
            show_notice(&client.borrow(), Some(NEW_ADVENTURE));
            Err(NEW_ADVENTURE.to_owned())
        }
        AdventureRecovery::NewGame => {
            let room = clean_room_code(&current.room_code).ok_or("New adventure room invalid")?;
            let page = if client.borrow().role == "player" {
                format!("/gameplay/player?room={room}")
            } else {
                "/gameplay/display".to_owned()
            };
            show_notice(&client.borrow(), Some(NEW_ADVENTURE));
            web_sys::window()
                .ok_or("Window absent")?
                .location()
                .replace(&page)
                .map_err(|_| "New adventure page unavailable")?;
            Err(NEW_ADVENTURE.to_owned())
        }
    }
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
        let preparing = state.preparing_adventure
            || state
                .views
                .as_ref()
                .and_then(|views| views.current())
                .is_some_and(|view| {
                    let journey = match &view.audience {
                        Some(rpc::view_message::Audience::Player(player)) => {
                            player.journey.as_ref()
                        }
                        Some(rpc::view_message::Audience::Display(display)) => {
                            display.journey.as_ref()
                        }
                        _ => None,
                    };
                    journey.is_some_and(|journey| journey.preparing_adventure)
                })
            || state.last_request.as_ref().is_some_and(|request| {
                request.action_kind == rpc::GameplayActionKind::StartNewAdventure as i32
                    && (!state.last_confirmed || state.last_accepted)
            });
        show_notice(
            &state,
            Some(if preparing {
                NEW_ADVENTURE
            } else if lost {
                SERVER_LOST
            } else {
                RECONNECTING
            }),
        );
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
            if let Some(creator) = client.borrow().creation.as_ref() {
                creator.set_pending(false);
                creator.show_error(&error);
            }
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
                .map_err(|status| match classify_submit_failure(status.code()) {
                    SubmitFailure::Refused => ACTION_REFUSAL_FEEDBACK,
                    SubmitFailure::ServerLost => SERVER_LOST,
                    SubmitFailure::Unknown
                        if body.action_kind
                            == rpc::GameplayActionKind::StartNewAdventure as i32 =>
                    {
                        NEW_ADVENTURE
                    }
                    SubmitFailure::Unknown => {
                        "Outcome unknown. Confirm the same operation or reconnect."
                    }
                }),
            Err(_) => Err(ACTION_REFUSAL_FEEDBACK),
        };
        if client.borrow().generation != generation {
            return;
        }
        let message = {
            let mut state = client.borrow_mut();
            state.busy = false;
            let message = complete_action_response(&mut state, &body, result);
            if message == ACTION_REFUSAL_FEEDBACK
                && body.character.is_some()
                && let Some(creator) = state.creation.as_ref()
            {
                creator.set_pending(false);
                creator.show_error("Your hero could not be saved. Your details are still here.");
            }
            message
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
                        Some(rpc::decision_receipt::Outcome::Accepted(accepted)) => {
                            if body.action_kind == rpc::GameplayActionKind::GreatswordAttack as i32
                                && state.first_attack.is_none()
                            {
                                state.first_attack = Some(body.clone());
                            }
                            if body.action_kind == rpc::GameplayActionKind::StartNewAdventure as i32
                            {
                                NEW_ADVENTURE.to_owned()
                            } else if body.action_kind
                                == rpc::GameplayActionKind::SetTableSetting as i32
                            {
                                "Table setting saved.".to_owned()
                            } else if body.action_kind
                                == rpc::GameplayActionKind::ExamineHarborSeal as i32
                            {
                                accepted.check.as_ref().map_or_else(
                                    || {
                                        "The observation was saved without a check receipt."
                                            .to_owned()
                                    },
                                    |check| observation_roll_line(check),
                                )
                            } else if receipt.replayed {
                                "Original saved outcome confirmed. No rolls or damage were repeated.".to_owned()
                            } else {
                                "Your choice is saved to the shared adventure.".to_owned()
                            }
                        }
                        Some(rpc::decision_receipt::Outcome::Rejected(_)) => {
                            state.last_request = None;
                            ACTION_REFUSAL_FEEDBACK.to_owned()
                        }
                        None => "The server returned no decision outcome.".to_owned(),
                    }
                }
            }
            Some(rpc::submit_action_response::Outcome::OperationObservation(_)) => {
                "The operation requires recovery; retain the same input.".to_owned()
            }
            None => "The server returned no operation outcome.".to_owned(),
        },
        Err(error) => {
            if error == ACTION_REFUSAL_FEEDBACK {
                state.last_request = None;
                state.last_confirmed = true;
                state.last_accepted = false;
                state.auto_confirms = 0;
            }
            error.to_owned()
        }
    }
}

fn observation_roll_line(check: &rpc::AbilityCheckResult) -> String {
    let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
    let Some(skill) = parts.get(2).copied() else {
        return "The observation roll could not be displayed.".to_owned();
    };
    let (ability, title) = match skill {
        "perception" => ("Wisdom", "Perception"),
        "insight" => ("Wisdom", "Insight"),
        "survival" => ("Wisdom", "Survival"),
        "medicine" => ("Wisdom", "Medicine"),
        "investigation" => ("Intelligence", "Investigation"),
        "arcana" => ("Intelligence", "Arcana"),
        "history" => ("Intelligence", "History"),
        "nature" => ("Intelligence", "Nature"),
        "religion" => ("Intelligence", "Religion"),
        "stealth" => ("Dexterity", "Stealth"),
        _ => return "The observation roll could not be displayed.".to_owned(),
    };
    let Ok(proficiency) = i32::try_from(check.proficiency_bonus) else {
        return "The observation roll could not be displayed.".to_owned();
    };
    let modifier = check.ability_modifier + proficiency;
    format!(
        "{ability} ({title}) {} + {modifier} = {}.",
        check.die, check.total
    )
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
fn cancel_creation_preview(client: &mut Client) {
    if let Some((_, abort)) = client.creation_preview.take() {
        abort.abort();
    }
}

fn creation(
    client: &Rc<RefCell<Client>>,
    surface: &Element,
    offer: &rpc::CharacterCreationOffer,
    action: &rpc::GameplayActionOffer,
) -> Result<(), JsValue> {
    let mut state = client.borrow_mut();
    state.creation_offer = Some(action.clone());
    if let Some(existing) = state.creation.as_ref() {
        let accepted = state.last_accepted
            && state.last_request.as_ref().is_some_and(|request| {
                request.offer_id == action.offer_id
                    && request.action_kind == rpc::GameplayActionKind::CreateCharacter as i32
            });
        existing.set_pending(
            state.busy || accepted || (!state.last_confirmed && state.last_request.is_some()),
        );
        if existing.root().parent_element().as_ref() != Some(surface) {
            surface.append_child(existing.root())?;
        }
        return Ok(());
    }
    let Some(identity) = offer.default_identity.as_ref() else {
        let document = state.document.clone();
        drop(state);
        surface.set_text_content(None);
        node(&document, surface, "h1", "", "Make your hero")?;
        node(
            &document,
            surface,
            "p",
            "hero-unavailable",
            "The table could not load the hero creator. Your seat is saved. Try reconnecting.",
        )?;
        button(client, surface, "Reconnect", connect)?;
        return Ok(());
    };
    let initial = hero_creator::IdentityDraft::from_rpc(identity)?;
    let document = state.document.clone();
    drop(state);
    let mut shown = offer.clone();
    for group in &mut shown.groups {
        group.label = creation_group_label(&group.group_id, &group.label);
        for option in &mut group.options {
            option.label = creation_option_label(&group.group_id, &option.label);
            option.description = creation_option_description(&group.group_id, &option.description);
        }
    }
    let weak = Rc::downgrade(client);
    let created = hero_creator::HeroCreator::create(
        &document,
        surface,
        &shown,
        &offer.identity_groups,
        &initial,
        move |event| {
            let Some(client) = weak.upgrade() else {
                return;
            };
            match event {
                hero_creator::CreationEvent::Preview {
                    draft_version,
                    appearance_text,
                    backstory_text,
                    current,
                } => preview_creation(
                    &client,
                    draft_version,
                    appearance_text,
                    backstory_text,
                    current,
                ),
                hero_creator::CreationEvent::Confirm(draft) => {
                    cancel_creation_preview(&mut client.borrow_mut());
                    let prepared = (|| {
                        let offered = client.borrow().creation_offer.clone().ok_or_else(|| {
                            "Your hero options changed. Review them again.".to_owned()
                        })?;
                        let selection = draft
                            .selection()
                            .map_err(|_| "Choose all your hero's details.".to_owned())?;
                        prepare(
                            &client,
                            &offered,
                            Some(selection),
                            false,
                            false,
                            rpc::HarborDestination::Unspecified as i32,
                        )
                    })();
                    match prepared {
                        Ok(body) => send(client, body, false),
                        Err(error) => {
                            if let Some(creator) = client.borrow().creation.as_ref() {
                                creator.set_pending(false);
                                creator.show_error(&error);
                            }
                        }
                    }
                }
            }
        },
    )?;
    client.borrow_mut().creation = Some(created);
    Ok(())
}

fn preview_creation(
    client: &Rc<RefCell<Client>>,
    draft_version: u64,
    appearance_text: String,
    backstory_text: String,
    current: hero_creator::HeroDraft,
) {
    let prepared = (|| -> Result<_, String> {
        let mut state = client.borrow_mut();
        cancel_creation_preview(&mut state);
        let action = state
            .action
            .clone()
            .ok_or("Reconnect to read your description.")?;
        let body = rpc::PreviewHeroIdentityRequest {
            client_binding_id: state.binding.clone(),
            session_id: state.session.clone(),
            run_id: state.run.clone(),
            observed_revision: state.revision,
            appearance_text,
            backstory_text,
            current_identity: Some(
                current
                    .identity
                    .to_rpc()
                    .map_err(|_| "Choose all your hero's details.")?,
            ),
        };
        let (abort, registration) = AbortHandle::new_pair();
        let token = Rc::new(());
        state.creation_preview = Some((token.clone(), abort));
        if let Some(creator) = state.creation.as_ref() {
            creator.show_error("Reading your description?");
        }
        Ok((
            action,
            body,
            state.credential.clone(),
            state.generation,
            token,
            registration,
        ))
    })();
    let (mut action, body, credential, generation, token, registration) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            if let Some(creator) = client.borrow().creation.as_ref() {
                creator.show_error(&error);
            }
            return;
        }
    };
    let revision = body.observed_revision;
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        let read = async {
            let mut request =
                request(body, &credential).map_err(|_| "Reconnect to read your description.")?;
            request.set_timeout(std::time::Duration::from_secs(8));
            action
                .preview_hero_identity(request)
                .await
                .map(|response| response.into_inner())
                .map_err(
                    |_| "Your description could not be read. Try again or choose your details.",
                )
        };
        let result = Abortable::new(read, registration).await;
        let Some(client) = weak.upgrade() else {
            return;
        };
        let mut state = client.borrow_mut();
        if state.generation != generation
            || !state
                .creation_preview
                .as_ref()
                .is_some_and(|(active, _)| Rc::ptr_eq(active, &token))
        {
            return;
        }
        state.creation_preview.take();
        let Some(creator) = state.creation.as_ref() else {
            return;
        };
        if state.revision != revision {
            creator.show_error("Your hero options changed. Read your description again.");
            return;
        }
        match result {
            Ok(Ok(response)) => {
                let identity = response
                    .proposed_identity
                    .as_ref()
                    .ok_or_else(|| JsValue::from_str("Hero details were not returned."))
                    .and_then(hero_creator::IdentityDraft::from_rpc);
                match identity {
                    Ok(identity) => {
                        if !creator.apply_preview(draft_version, &identity, &response.warnings) {
                            creator.show_error("Your description changed. Read it again.");
                        }
                    }
                    Err(_) => creator.show_error("Your description could not be read. Try again."),
                }
            }
            Ok(Err(error)) => creator.show_error(error),
            Err(_) => creator.show_error("Reading stopped. Your description is still editable."),
        }
    });
}

#[cfg(test)]
fn creation_group_description(group: &rpc::JourneyGroup) -> String {
    let [option] = group.options.as_slice() else {
        return "Choose one.".to_owned();
    };
    let label = creation_option_label(&group.group_id, &option.label);
    let description = creation_option_description(&group.group_id, &option.description);
    if description.is_empty() {
        label
    } else {
        format!("{label} · {description}")
    }
}
fn creation_group_label(group_id: &str, label: &str) -> String {
    if group_id == "allied-ties" {
        return "When turns are tied".to_owned();
    }
    live_combat::display_label(label)
}

fn creation_option_label(group_id: &str, label: &str) -> String {
    if group_id == "build-package" {
        if label.starts_with("Stalwart ·") {
            return "Stalwart · Chain mail, greatsword, flail, 8 javelins".to_owned();
        }
        if label.starts_with("Vanguard ·") {
            return "Vanguard · Chain mail, greatsword, flail, 8 javelins".to_owned();
        }
        if label.contains("Fighter B") || label.starts_with("Longbow and Archery") {
            return "Archer · Studded leather armor, scimitar, shortsword, longbow, 20 arrows"
                .to_owned();
        }
    }
    if group_id == "equipment" && label == "Fighter A + Soldier A" {
        return "Chain mail, greatsword, flail, 8 javelins".to_owned();
    }
    if group_id == "allied-ties" {
        return live_combat::display_label(&label.replace("the Bandit", "the enemy"));
    }
    if group_id == "portrait" {
        // The pack names the looks by slot; the form names them by what the picture shows.
        return match label {
            "Greatsword hero" => "Red beard, greatsword".to_owned(),
            "Second hero" => "Braided hair, greatsword".to_owned(),
            other => live_combat::display_label(other),
        };
    }
    live_combat::display_label(label)
}

fn creation_option_description(group_id: &str, description: &str) -> String {
    if group_id == "allied-ties" {
        // The pack's wording names one adventure's enemy; the rule is the same for any.
        description.replace("the Bandit", "the enemy")
    } else {
        description.to_owned()
    }
}

pub(super) fn action_identity(offer: &rpc::GameplayActionOffer) -> String {
    let mut groups = offer
        .input_groups
        .iter()
        .map(|group| group.group_id.as_str())
        .collect::<Vec<_>>();
    groups.sort_unstable();
    let groups = groups
        .into_iter()
        .map(|group_id| format!("{}:{group_id}", group_id.len()))
        .collect::<String>();
    // The label keeps two offers of one kind in the same view (two places to go) apart.
    format!("{}:{groups}:{}", offer.action_kind, offer.label)
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
    let identity = action_identity(&offer);
    let children = parent.query_selector_all(":scope > .offered-action")?;
    let existing = (0..children.length())
        .filter_map(|index| children.item(index))
        .map(|element| element.unchecked_into::<Element>())
        .find(|element| {
            element.get_attribute("data-action-identity").as_deref() == Some(&identity)
        });
    if let Some(wrapper) = existing {
        wrapper.set_attribute("data-offer-id", &offer.offer_id)?;
        let mut inputs = Vec::new();
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
                    inputs.push((group.group_id.clone(), select));
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
            let destination = wrapper
                .query_selector(":scope > label[data-destination] > select")?
                .and_then(|select| select.dyn_into::<HtmlSelectElement>().ok());
            bind_action_offer(client, &button, offer, inputs, destination)?;
        }
        return Ok(());
    }

    let wrapper = node(&document, parent, "div", "offered-action", "")?;
    wrapper.set_attribute("data-offer-id", &offer.offer_id)?;
    wrapper.set_attribute("data-action-identity", &identity)?;
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
    let button = node(&document, &wrapper, "button", "", &offer.label)?;
    bind_action_offer(client, &button, offer.clone(), inputs, destination)?;
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

fn bind_action_offer(
    client: &Rc<RefCell<Client>>,
    button: &Element,
    offer: rpc::GameplayActionOffer,
    inputs: Vec<(String, HtmlSelectElement)>,
    destination: Option<HtmlSelectElement>,
) -> Result<(), JsValue> {
    // `render` retires every listener first, so a kept control is bound exactly once here.
    bind_click(client, button, move |client| {
        activate_action_offer(client, &offer, &inputs, &destination);
    })
}

fn activate_action_offer(
    client: Rc<RefCell<Client>>,
    offered: &rpc::GameplayActionOffer,
    inputs: &[(String, HtmlSelectElement)],
    destination: &Option<HtmlSelectElement>,
) {
    let mut savage = false;
    let mut graze = false;
    let mut action_choices = Vec::new();
    let tactical_offer = offered
        .input_groups
        .iter()
        .any(|group| group.group_id == "tactical-choice");
    for (key, input) in inputs {
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
        if key == "finding"
            && offered.action_kind == rpc::GameplayActionKind::ExamineHarborSeal as i32
        {
            if !offered.input_groups.iter().any(|group| {
                group.group_id == *key
                    && group.options.iter().any(|option| option.option_id == value)
            }) {
                feedback(
                    &client.borrow(),
                    "Choose something the scene offers to examine.",
                );
                return;
            }
            action_choices.push(rpc::JourneyChoice {
                group_id: key.clone(),
                option_id: value,
            });
            continue;
        }
        if tactical_offer {
            let Some(choice) = tactical_action_choice(offered, key, &value) else {
                feedback(&client.borrow(), "Choose an offered attack approach.");
                return;
            };
            action_choices.push(choice);
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
    let selected = if let Some(select) = destination {
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
    match prepare(&client, offered, None, savage, graze, selected) {
        Ok(mut body) => {
            body.action_choices = action_choices;
            send(client, body, false);
        }
        Err(error) => feedback(&client.borrow(), &error),
    }
}

fn tactical_action_choice(
    offer: &rpc::GameplayActionOffer,
    group_id: &str,
    option_id: &str,
) -> Option<rpc::JourneyChoice> {
    if !matches!(option_id, "yes" | "no")
        || !offer
            .input_groups
            .iter()
            .any(|group| group.group_id == "tactical-choice")
        || !offer.input_groups.iter().any(|group| {
            group.group_id == group_id
                && group
                    .options
                    .iter()
                    .any(|option| option.option_id == option_id)
        })
    {
        return None;
    }
    Some(rpc::JourneyChoice {
        group_id: group_id.to_owned(),
        option_id: option_id.to_owned(),
    })
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
        ) && views.accept_art_update(generation, scope, &view, |shown, new| {
            crate::layout_contract::merge_layout(shown, new)
                | crate::live_art::merge_art(shown, new)
        }) {
            drop(state);
            if let Some(rpc::view_message::Audience::Display(display)) = &view.audience {
                layout::apply(client, &display.layout_json, display.layout_revision)?;
            }
            redraw_current(client)?;
            return live_health::phone_sync(client);
        }
        accepted
    };
    if matches!(
        acceptance,
        ConnectionViewAcceptance::View(ViewAcceptance::Applied | ViewAcceptance::Duplicate { .. })
    ) && let Some(rpc::view_message::Audience::Display(display)) = &view.audience
    {
        layout::apply(client, &display.layout_json, display.layout_revision)?;
    }
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
            let waiting_for_sound = client
                .borrow()
                .live
                .tv
                .as_ref()
                .is_some_and(|stage| stage.root().has_attribute("data-sound-gate"));
            if waiting_for_sound {
                redraw_current(client)?;
            } else {
                live_motion::on_view(client)?;
            }
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

fn is_dead_hero(journey: &rpc::JourneyView) -> bool {
    let Some(own) = journey.own_character.as_ref() else {
        return false;
    };
    own.facts
        .iter()
        .any(|fact| fact.label == "Condition" && fact.value.eq_ignore_ascii_case("Dead"))
        || journey.combat.as_ref().is_some_and(|combat| {
            combat
                .death_saves
                .iter()
                .any(|save| save.dead && save.name == own.name)
        })
}

fn dead_hero_notice(journey: &rpc::JourneyView) -> Option<String> {
    is_dead_hero(journey).then(|| {
        format!(
            "{} has died. You can make a new hero to join at the next story scene. Until then, you can watch the table.",
            journey.own_character.as_ref().map_or("Your hero", |own| &own.name)
        )
    })
}

fn player_offers_for(
    journey: &rpc::JourneyView,
    offers: &[rpc::GameplayActionOffer],
) -> Vec<rpc::GameplayActionOffer> {
    if is_dead_hero(journey) {
        offers
            .iter()
            .filter(|offer| offer.action_kind == rpc::GameplayActionKind::CreateCharacter as i32)
            .cloned()
            .collect()
    } else {
        offers.to_vec()
    }
}

fn render_dead_hero_notice(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    journey: &rpc::JourneyView,
    offers: &[rpc::GameplayActionOffer],
) -> Result<(), JsValue> {
    if let Some(previous) = parent.query_selector(":scope > .dead-hero-notice")? {
        previous.remove();
    }
    let Some(message) = dead_hero_notice(journey) else {
        return Ok(());
    };
    let document = client.borrow().document.clone();
    let notice = node(&document, parent, "section", "dead-hero-notice", "")?;
    notice.set_attribute("role", "status")?;
    notice.set_attribute("aria-live", "polite")?;
    node(&document, &notice, "p", "dead-hero-copy", &message)?;
    if journey.creation.is_none() {
        if let Some(offer) = offers
            .iter()
            .find(|offer| offer.action_kind == rpc::GameplayActionKind::CreateCharacter as i32)
        {
            let mut offer = offer.clone();
            offer.label = "Make a new hero".to_owned();
            action_button(client, &notice, offer)?;
        }
    }
    Ok(())
}

/// Only admitted snapshots or a local redraw of the retained snapshot reach rendering.
fn render(client: &Rc<RefCell<Client>>, view: rpc::ViewMessage) -> Result<(), JsValue> {
    let display_layout = match &view.audience {
        Some(rpc::view_message::Audience::Display(display)) => {
            Some((display.layout_json.clone(), display.layout_revision))
        }
        _ => None,
    };
    // A redraw rebuilds the page; it must not move the reader's place (feed, composer, sheet).
    let root = client.borrow().root.clone();
    let place = ScrollPlace::remember(&root);
    render_view(client, view)?;
    if let Some((json, revision)) = display_layout {
        layout::apply(client, &json, revision)?;
    }
    phone_controls::bind(client)?;
    presentation_timing::paint(client);
    place.restore(&root);
    Ok(())
}

/// Where the page and each scrolled element were before a redraw. An element is matched after
/// the redraw by its tag, class and its position among elements with that tag and class.
struct ScrollPlace {
    page: (f64, f64),
    elements: Vec<(String, usize, i32)>,
}

impl ScrollPlace {
    fn remember(root: &Element) -> Self {
        let page = web_sys::window()
            .map(|window| {
                (
                    window.scroll_x().unwrap_or(0.0),
                    window.scroll_y().unwrap_or(0.0),
                )
            })
            .unwrap_or((0.0, 0.0));
        let mut elements = Vec::new();
        for (key, index, element) in scroll_keys(root) {
            let top = element.scroll_top();
            if top > 0 {
                elements.push((key, index, top));
            }
        }
        Self { page, elements }
    }

    fn restore(&self, root: &Element) {
        for (key, index, element) in scroll_keys(root) {
            if let Some((_, _, top)) = self
                .elements
                .iter()
                .find(|(saved, saved_index, _)| *saved == key && *saved_index == index)
            {
                element.set_scroll_top(*top);
            }
        }
        if let Some(window) = web_sys::window() {
            window.scroll_to_with_x_and_y(self.page.0, self.page.1);
        }
    }
}

/// Every element under `root` with its key (tag and class), its index among the elements sharing
/// that key, and the element itself.
fn scroll_keys(root: &Element) -> Vec<(String, usize, Element)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut keyed = Vec::new();
    let Ok(all) = root.query_selector_all("*") else {
        return keyed;
    };
    for position in 0..all.length() {
        let Some(node) = all.item(position) else {
            continue;
        };
        let Ok(element) = node.dyn_into::<Element>() else {
            continue;
        };
        // The `class` attribute, never `className`: on an SVG element `className` is an
        // SVGAnimatedString, which the string binding cannot take and which aborted the client.
        let classes = element.get_attribute("class").unwrap_or_default();
        let key = format!("{}.{classes}", element.tag_name());
        let count = counts.entry(key.clone()).or_insert(0);
        keyed.push((key, *count, element));
        *count += 1;
    }
    keyed
}

fn render_view(client: &Rc<RefCell<Client>>, view: rpc::ViewMessage) -> Result<(), JsValue> {
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
    let dead_hero = role == "player" && is_dead_hero(&journey);
    if let Some(own) = journey.own_character.as_ref() {
        let notebook = live_notebook::view(&journey)?;
        live_notebook::render(client, &notebook, &own.name)?;
    }
    if journey.preparing_adventure {
        let surface = root
            .query_selector(".view")?
            .ok_or_else(|| JsValue::from_str("view absent"))?;
        live_screens::release(client, &surface)?;
        live_combat::release(client)?;
        // The whole screen between two adventures: a drawn night scene with the same
        // sentence in it as the notice (`live_preparing.html`, shared with the holding page).
        surface.set_inner_html(include_str!("gameplay_browser/live_preparing.html"));
        show_notice(&client.borrow(), Some(NEW_ADVENTURE));
        return Ok(());
    }
    if journey.party.len() > 2 || offers.len() > 12 {
        return Err(JsValue::from_str("server view exceeds bound"));
    }
    let offers = if role == "player" {
        player_offers_for(&journey, &offers)
    } else {
        offers
    };
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
        if dead_hero {
            render_dead_hero_notice(client, &surface, &journey, &offers)?;
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
    cancel_creation_preview(&mut client.borrow_mut());
    client.borrow_mut().creation_offer = None;
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
        // A role token or art not yet ready draws the neutral stage; only a deleted painting is
        // reported, and the view is still shown.
        if public_scene_assets::resolve(&shown.scene_asset).is_err()
            && live_screens::asset_url(&shown.scene_asset).is_none()
            && !crate::presentation::scene_art_admitted(&shown.scene_asset)
        {
            status(&client.borrow(), "The scene art is unavailable; showing the stage.");
        }
        surface.set_attribute("data-destination", &shown.destination.to_string())?;
        live_combat::release(client)?;
        live_screens::render(
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
        )?;
        if dead_hero {
            render_dead_hero_notice(client, &surface, &journey, &offers)?;
        }
        return Ok(());
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
        if dead_hero {
            render_dead_hero_notice(client, &surface, &journey, &offers)?;
        }
        return Ok(());
    }
    live_screens::release(client, &surface)?;
    live_combat::release(client)?;
    // The base scene art never refuses the view: a role token, live path or reviewed photo the
    // delivery cannot fetch leaves its fallback stage, and only a deleted painting is reported.
    if !crate::presentation::scene_art_admitted(&scene.scene_asset) {
        status(&client.borrow(), "The scene art is unavailable; showing the stage.");
    }
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
    if dead_hero {
        render_dead_hero_notice(client, &body, &journey, &offers)?;
    }
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
        let sheet = crate::player_sheet::player_sheet_view(sheet);
        live_combat::sheet::summary(client, &body, &sheet)?;
    }

    dialogue_recovery::mount(client, &body, &offers)?;
    let actions = node(&document, &body, "div", "scene-actions", "")?;
    if !dead_hero {
        for offer in offers {
            action_button(client, &actions, offer)?;
        }
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
        unknown_choice: has_unconfirmed_saved_choice(
            state.last_request.is_some(),
            state.last_confirmed,
            state.busy,
        ),
    }
}
fn has_unconfirmed_saved_choice(last_request: bool, last_confirmed: bool, busy: bool) -> bool {
    last_request && !last_confirmed && !busy
}
fn confirm_saved_choice(client: &Rc<RefCell<Client>>, parent: &Element) -> Result<(), JsValue> {
    button(client, parent, "Send my last choice again", |client| {
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
        root: root.clone(),
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
                value: Some(match root.get_attribute("data-run-id") {
                    Some(value) if value.len() == 32 && value.is_ascii() => value
                        .as_bytes()
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| {
                            std::str::from_utf8(pair)
                                .ok()
                                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                        })
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| JsValue::from_str("display run invalid"))?,
                    _ => vec![0x42; 16],
                }),
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
        reclaim_request: None,
        away_heroes: Vec::new(),
        seat_poll_abort: None,
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
        creation_preview: None,
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
        preparing_adventure: false,
        auto_confirms: 0,
        feedback_generation: 0,
        visibility: None,
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
        attach_visibility(&document, &client)?;
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
            source_scene_key: String::new(),
            source_scene_variant: String::new(),
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
            plate_update_pending: false,
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
    #[test]
    fn missing_server_identity_metadata_keeps_a_readable_creation_page() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let surface = node(&document, &root, "div", "view", "").unwrap();
        let client = new_client(&document, root, "player").unwrap();
        creation(
            &client,
            &surface,
            &rpc::CharacterCreationOffer::default(),
            &rpc::GameplayActionOffer::default(),
        )
        .unwrap();
        assert!(
            surface
                .query_selector(".hero-unavailable")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap()
                .contains("Your seat is saved")
        );
        assert!(surface.query_selector("button").unwrap().is_some());
        assert!(client.borrow().creation.is_none());
    }

    #[test]
    fn phone_tabs_switch_sheets_after_callbacks_are_rebound() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let client = new_client(&document, root.clone(), "player").unwrap();
        phone_controls::bind(&client).unwrap();
        let tabs = root.query_selector_all(".live-tab").unwrap();
        let click = |index| {
            tabs.item(index)
                .unwrap()
                .dyn_into::<Element>()
                .unwrap()
                .dispatch_event(&web_sys::Event::new("click").unwrap())
                .unwrap();
        };
        click(2);
        assert_eq!(
            root.query_selector_all(".live-sheet[data-open]")
                .unwrap()
                .length(),
            1
        );
        click(3);
        assert_eq!(
            root.query_selector_all(".live-sheet[data-open]")
                .unwrap()
                .length(),
            1
        );
        assert!(
            !root
                .query_selector(".live-notebook-sheet")
                .unwrap()
                .unwrap()
                .has_attribute("data-open")
        );
        clear_callbacks(&mut client.borrow_mut());
        phone_controls::bind(&client).unwrap();
        click(2);
        assert_eq!(
            root.query_selector_all(".live-sheet[data-open]")
                .unwrap()
                .length(),
            1
        );
        click(0);
        assert_eq!(
            root.query_selector_all(".live-sheet[data-open]")
                .unwrap()
                .length(),
            0
        );
        for index in 0..tabs.length() {
            assert!(
                tabs.item(index)
                    .unwrap()
                    .dyn_into::<Element>()
                    .unwrap()
                    .has_attribute("aria-label")
            );
        }
    }

    #[test]
    fn live_creation_mounts_identity_controls_and_keeps_the_draft_after_redraw() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let surface = node(&document, &root, "div", "view", "").unwrap();
        let client = new_client(&document, root, "player").unwrap();
        client.borrow_mut().binding = Some(rpc::ClientBindingId {
            value: Some(vec![1; 16]),
        });
        client.borrow_mut().revision = Some(rpc::SessionRevision {
            epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
            sequence: Some(1),
        });
        let offer = rpc::CharacterCreationOffer {
            description: "Choose your hero".to_owned(),
            default_identity: Some(rpc::HeroIdentity {
                appearance: Some(rpc::HeroAppearance::default()),
                origin: "The hills".to_owned(),
                occupation: "A guard".to_owned(),
                motivation: "Protect home".to_owned(),
                backstory: "A guard from the hills protecting home".to_owned(),
                facts: vec![
                    "The hills".to_owned(),
                    "A guard".to_owned(),
                    "Protect home".to_owned(),
                ],
            }),
            ..Default::default()
        };
        let action = rpc::GameplayActionOffer::default();
        creation(&client, &surface, &offer, &action).unwrap();
        let creator = surface.query_selector(".hero-creator").unwrap().unwrap();
        let editor = creator
            .query_selector("textarea")
            .unwrap()
            .unwrap()
            .dyn_into::<web_sys::HtmlTextAreaElement>()
            .unwrap();
        editor.set_value("Maren");
        editor
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        clear_callbacks(&mut client.borrow_mut());
        creation(&client, &surface, &offer, &action).unwrap();
        assert!(
            creator.is_same_node(
                surface
                    .query_selector(".hero-creator")
                    .unwrap()
                    .as_ref()
                    .map(|node| node.as_ref())
            )
        );
        assert_eq!(editor.value(), "Maren");
        assert_eq!(
            surface
                .query_selector_all(".hero-creator")
                .unwrap()
                .length(),
            1
        );
    }

    use super::{
        ACTION_REFUSAL_FEEDBACK, ROOM_CONNECTION_UNAVAILABLE, SERVER_LOST, SubmitFailure,
        action_identity, classify_submit_failure, creation_group_description,
        creation_option_description, creation_option_label, dead_hero_notice, is_dead_hero,
        player_offers_for, record_join_retry, should_replace_feedback, tactical_action_choice,
    };

    #[test]
    fn new_adventure_preparation_does_not_exhaust_join_retries_or_become_session_lost() {
        let mut retries = crate::reconnect::JOIN_RETRIES;
        for _ in 0..10 {
            assert!(super::record_join_retry(super::NEW_ADVENTURE, &mut retries));
        }
        assert_eq!(retries, crate::reconnect::JOIN_RETRIES);
        assert!(!super::record_join_retry(SERVER_LOST, &mut retries));
    }

    #[test]
    fn new_game_recovery_redirects_to_the_server_room_instead_of_refusing_the_saved_seat() {
        use super::{AdventureRecovery, classify_adventure_recovery, rpc};
        let mut current = rpc::CurrentAdventureResponse {
            room_code: "NEWROOM".to_owned(),
            run_identity: "new-run".to_owned(),
            preparing: true,
        };
        assert_eq!(
            classify_adventure_recovery(&current, Some("old-run"), "OLDROOM"),
            AdventureRecovery::Preparing
        );
        current.preparing = false;
        assert_eq!(
            classify_adventure_recovery(&current, Some("old-run"), "OLDROOM"),
            AdventureRecovery::NewGame
        );
        assert_eq!(
            classify_adventure_recovery(&current, Some("new-run"), "NEWROOM"),
            AdventureRecovery::Current
        );
        current.room_code = "OLDROOM".to_owned();
        assert_eq!(
            classify_adventure_recovery(&current, Some("old-run"), "OLDROOM"),
            AdventureRecovery::NewGame
        );
    }

    #[test]
    fn melee_creation_packages_keep_their_names_and_show_the_scores_they_are_sent() {
        assert_eq!(
            creation_option_label("build-package", "Stalwart · Greatsword and Defense"),
            "Stalwart · Chain mail, greatsword, flail, 8 javelins"
        );
        // The scores come from the server, which owns them; the page shows them as sent.
        assert_eq!(
            creation_option_description("build-package", "STR 17 · DEX 13 · CON 15"),
            "STR 17 · DEX 13 · CON 15"
        );
        assert_eq!(
            creation_option_label("build-package", "Vanguard · Greatsword and Defense"),
            "Vanguard · Chain mail, greatsword, flail, 8 javelins"
        );
        assert_eq!(
            creation_option_label(
                "build-package",
                "Longbow and Archery · Fighter B + Soldier B"
            ),
            "Archer · Studded leather armor, scimitar, shortsword, longbow, 20 arrows"
        );
        assert_eq!(
            creation_option_label("portrait", "Second hero"),
            "Braided hair, greatsword"
        );
    }

    #[test]
    fn creation_copy_uses_player_language_and_names_starting_gear() {
        let pack = include_str!("../../../content/harbor-journey/pack.json");
        let mut wording = include_str!("gameplay/fixtures/creation-wording.txt").lines();
        let creation_description = wording.next().expect("creation description fixture");
        let equipment_label = wording.next().expect("equipment label fixture");
        let equipment_description = wording.next().expect("equipment description fixture");
        let tied_turns_label = wording.next().expect("tied turns label fixture");
        let tied_turns_description = wording.next().expect("tied turns description fixture");
        let stalwart_package = wording.next().expect("Stalwart package fixture");
        let vanguard_package = wording.next().expect("Vanguard package fixture");
        let archer_package = wording.next().expect("archer package fixture");
        assert!(wording.next().is_none(), "fixture has no extra lines");

        assert!(pack.contains(&format!("\"description\": \"{creation_description}\"")));
        assert!(pack.contains(&format!("\"label\": \"{equipment_label}\"")));
        assert!(pack.contains(&format!("\"description\": \"{equipment_description}\"")));
        assert!(pack.contains(&format!("\"label\": \"{tied_turns_label}\"")));
        assert!(pack.contains(&format!("\"description\": \"{tied_turns_description}\"")));
        assert_eq!(
            creation_option_label("equipment", "Fighter A + Soldier A"),
            equipment_label
        );
        assert_eq!(
            creation_option_label("build-package", "Stalwart · Greatsword and Defense"),
            stalwart_package
        );
        assert_eq!(
            creation_option_label("build-package", "Vanguard · Greatsword and Defense"),
            vanguard_package
        );
        assert_eq!(
            creation_option_label(
                "build-package",
                "Longbow and Archery · Fighter B + Soldier B"
            ),
            archer_package
        );
        for developer_copy in [
            "validated on the server",
            "Other lawful SRD combinations remain unsupported",
            "three masteries",
            "Initiative tie agreement",
            "Fighter A + Soldier A",
        ] {
            assert!(!pack.contains(developer_copy), "found {developer_copy:?}");
        }
    }
    #[test]
    fn single_creation_options_render_as_facts_and_real_choices_keep_a_prompt() {
        let single = super::rpc::JourneyGroup {
            group_id: "class".to_owned(),
            options: vec![super::rpc::JourneyOption {
                label: "Fighter · level 1".to_owned(),
                description: "Choose a Fighting Style and call on Second Wind.".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            creation_group_description(&single),
            "Fighter · level 1 · Choose a Fighting Style and call on Second Wind."
        );

        let no_description = super::rpc::JourneyGroup {
            group_id: "background".to_owned(),
            options: vec![super::rpc::JourneyOption {
                label: "Soldier".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(creation_group_description(&no_description), "Soldier");

        let choice = super::rpc::JourneyGroup {
            options: vec![super::rpc::JourneyOption::default(); 2],
            ..Default::default()
        };
        assert_eq!(creation_group_description(&choice), "Choose one.");
    }

    #[test]
    fn submit_status_classification_separates_refusals_from_unknown_outcomes() {
        for code in [
            tonic::Code::InvalidArgument,
            tonic::Code::FailedPrecondition,
            tonic::Code::PermissionDenied,
            tonic::Code::ResourceExhausted,
        ] {
            assert_eq!(classify_submit_failure(code), SubmitFailure::Refused);
        }
        assert_eq!(
            classify_submit_failure(tonic::Code::DataLoss),
            SubmitFailure::ServerLost
        );
        for code in [
            tonic::Code::Unavailable,
            tonic::Code::DeadlineExceeded,
            tonic::Code::Cancelled,
            tonic::Code::Internal,
        ] {
            assert_eq!(classify_submit_failure(code), SubmitFailure::Unknown);
        }
        assert_eq!(
            ACTION_REFUSAL_FEEDBACK,
            "The game could not accept that. Choose again."
        );
    }

    #[test]
    fn tactical_attack_inputs_are_forwarded_only_when_the_offer_contains_them() {
        let group = |group_id: &str, options: &[&str]| df_protocol::common::JourneyGroup {
            group_id: group_id.to_owned(),
            options: options
                .iter()
                .map(|option_id| df_protocol::common::JourneyOption {
                    option_id: (*option_id).to_owned(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let offer = df_protocol::common::GameplayActionOffer {
            input_groups: vec![
                group("tactical-choice", &[]),
                group("weapon-mastery", &["no", "yes"]),
                group("savage-attacker", &["no", "yes"]),
            ],
            ..Default::default()
        };

        assert_eq!(
            tactical_action_choice(&offer, "weapon-mastery", "yes")
                .map(|choice| (choice.group_id, choice.option_id)),
            Some(("weapon-mastery".to_owned(), "yes".to_owned()))
        );
        assert!(tactical_action_choice(&offer, "graze", "yes").is_none());
        assert!(tactical_action_choice(&offer, "weapon-mastery", "maybe").is_none());
    }

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

    #[test]
    fn action_identity_survives_offer_revision_but_tracks_action_and_inputs() {
        let first = df_protocol::common::GameplayActionOffer {
            offer_id: "checkpoint-1".to_owned(),
            action_kind: 13,
            input_groups: vec![df_protocol::common::JourneyGroup {
                group_id: "target".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let refreshed = df_protocol::common::GameplayActionOffer {
            offer_id: "checkpoint-2".to_owned(),
            ..first.clone()
        };
        let other_action = df_protocol::common::GameplayActionOffer {
            action_kind: 14,
            ..refreshed.clone()
        };
        let other_inputs = df_protocol::common::GameplayActionOffer {
            input_groups: vec![df_protocol::common::JourneyGroup {
                group_id: "destination".to_owned(),
                ..Default::default()
            }],
            ..refreshed.clone()
        };

        assert_eq!(action_identity(&first), action_identity(&refreshed));
        assert_ne!(action_identity(&first), action_identity(&other_action));
        assert_ne!(action_identity(&first), action_identity(&other_inputs));
        let other_label = df_protocol::common::GameplayActionOffer {
            label: "Go to the harbor".to_owned(),
            ..refreshed.clone()
        };
        assert_ne!(action_identity(&refreshed), action_identity(&other_label));
    }

    #[test]
    fn dead_hero_notice_is_plain_and_keeps_only_the_replacement_offer() {
        let mut journey = df_protocol::common::JourneyView {
            own_character: Some(df_protocol::common::CharacterSheet {
                name: "Sigrun".to_owned(),
                facts: vec![df_protocol::common::SheetFact {
                    label: "Condition".to_owned(),
                    value: "Dead".to_owned(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let offers = vec![
            df_protocol::common::GameplayActionOffer {
                action_kind: df_protocol::common::GameplayActionKind::ShortRest as i32,
                label: "Short Rest".to_owned(),
                ..Default::default()
            },
            df_protocol::common::GameplayActionOffer {
                action_kind: df_protocol::common::GameplayActionKind::CreateCharacter as i32,
                label: "Make a new hero".to_owned(),
                ..Default::default()
            },
        ];

        assert!(is_dead_hero(&journey));
        assert_eq!(
            dead_hero_notice(&journey).as_deref(),
            Some(
                "Sigrun has died. You can make a new hero to join at the next story scene. Until then, you can watch the table."
            )
        );
        assert_eq!(player_offers_for(&journey, &offers), [offers[1].clone()]);

        journey
            .own_character
            .as_mut()
            .expect("own hero present")
            .facts
            .clear();
        journey.combat = Some(df_protocol::common::CombatView {
            death_saves: vec![df_protocol::common::DeathSaveState {
                name: "Sigrun".to_owned(),
                dead: true,
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(is_dead_hero(&journey));
        assert_eq!(
            dead_hero_notice(&journey).as_deref(),
            Some(
                "Sigrun has died. You can make a new hero to join at the next story scene. Until then, you can watch the table."
            )
        );
    }

    #[test]
    fn a_stable_zero_hit_point_hero_is_not_mistaken_for_dead() {
        let journey = df_protocol::common::JourneyView {
            own_character: Some(df_protocol::common::CharacterSheet {
                name: "Sigrun".to_owned(),
                facts: vec![df_protocol::common::SheetFact {
                    label: "Condition".to_owned(),
                    value: "Stable at 0 HP".to_owned(),
                }],
                ..Default::default()
            }),
            combat: Some(df_protocol::common::CombatView {
                death_saves: vec![df_protocol::common::DeathSaveState {
                    name: "Sigrun".to_owned(),
                    stable: true,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(!is_dead_hero(&journey));
        assert!(dead_hero_notice(&journey).is_none());
    }
}
