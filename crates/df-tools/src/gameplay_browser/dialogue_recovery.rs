//! Player-owned presentation of the native dialogue ticket and exact confirmation.
use super::*;
use crate::speech_capture::Interruption;
use web_sys::HtmlTextAreaElement;

#[derive(Default)]
pub(super) struct DialogueRecovery {
    channel: Option<rpc::dialogue_service_client::DialogueServiceClient<BrowserChannel>>,
    editor: Option<HtmlTextAreaElement>,
    send_button: Option<Element>,
    offers: Vec<rpc::GameplayActionOffer>,
    ticket: Option<(rpc::RawDialogueRequest, rpc::DialogueResponse)>,
    confirmation: Option<rpc::ConfirmDialogueRequest>,
    active: Option<(Rc<()>, AbortHandle)>,
    message: String,
    restore_focus: bool,
    selection: Option<(u32, u32)>,
    exchange: Option<Exchange>,
    /// What this player just sent while the speaker's reply is still being written.
    awaiting: Option<String>,
    /// Tap-to-talk speech input for this text box.
    pub(super) talk: super::talk_control::Talk,
    /// The editor's listeners for the speech-intent signal.
    intent_listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

/// The player's last line, its disposition, and any separate reply or action result. It is
/// shown only while the revision it answered is still current.
struct Exchange {
    revision: Option<rpc::SessionRevision>,
    you: String,
    disposition: Option<String>,
    speaker: String,
    portrait: String,
    reply: String,
}

/// How long a text box may stay unfocused (a redraw moves it and restores focus) before the
/// player counts as having left it.
const BLUR_GRACE_MS: u32 = 250;

/// A model-voiced reply is a short line; anything longer is not shown.
const MAXIMUM_REPLY_BYTES: usize = 1024;
/// The typed-dialogue request also waits for the speaker's reply, so it gets longer than the
/// ordinary five seconds.
const RAW_DIALOGUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The recovery revision as (epoch, sequence). Tuples compare epoch first, which is the
/// canonical recovery order. An absent component or a zero epoch cannot be ordered.
fn ordered_revision(revision: Option<&rpc::SessionRevision>) -> Option<(u64, u64)> {
    let revision = revision?;
    let epoch = revision.epoch.as_ref()?.value.filter(|epoch| *epoch != 0)?;
    Some((epoch, revision.sequence?))
}

/// A reply answers this input when it carries the same input identity and a revision at or after
/// the one the input observed. A turn that commits state advances the revision it answers at, so
/// the phone's own revision may already have moved on; an older or unidentified reply is refused.
fn answers_input(input: &rpc::RawDialogueRequest, reply: &rpc::DialogueResponse) -> bool {
    match (
        ordered_revision(input.observed_revision.as_ref()),
        ordered_revision(reply.revision.as_ref()),
    ) {
        (Some(observed), Some(answered)) => {
            reply.input_id == input.input_id && answered >= observed
        }
        _ => false,
    }
}

fn scope_change_notice(draft: &str) -> Option<&'static str> {
    (!draft.trim().is_empty()).then_some("Your draft is still here. Review it before sending.")
}

/// The text the player has in the composer right now. A scope notice speaks of that text only.
fn composer_draft(dialogue: &DialogueRecovery) -> String {
    dialogue
        .editor
        .as_ref()
        .map(HtmlTextAreaElement::value)
        .unwrap_or_default()
}

/// The notice for a request refused because the offer or scope moved on. It appears only while the
/// player has a draft that the change could affect: another player's turn is not the player's change.
fn scope_changed_notice(draft: &str) -> Option<&'static str> {
    (!draft.trim().is_empty()).then_some("The offer or scope changed. Your text remains editable.")
}

fn split_action_response(response: &str) -> (&str, &str) {
    response.find(". ").map_or((response, ""), |period| {
        (&response[..=period], &response[period + 2..])
    })
}

fn portrait_url(speaker: &str) -> &'static str {
    match speaker {
        "Courier" => "/assets/ui/generated/sprite-courier-documentary-photo",
        "Bandit" => "/assets/ui/generated/sprite-bandit-documentary-photo",
        "Vell" => "/assets/concept-art/vell-avatar.webp",
        _ => "/assets/concept-art/dm-avatar.webp",
    }
}

fn current_speaker(client: &Client) -> String {
    match client
        .views
        .as_ref()
        .and_then(|views| views.current())
        .and_then(|view| view.audience.as_ref())
    {
        Some(rpc::view_message::Audience::Player(player)) => player
            .journey
            .as_ref()
            .map(|journey| journey.speaker.clone())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn current_scene(client: &Client) -> Option<&rpc::GameplayScene> {
    client
        .views
        .as_ref()
        .and_then(|views| views.current())
        .and_then(|view| match view.audience.as_ref()? {
            rpc::view_message::Audience::Player(player) => player.scene.as_ref(),
            _ => None,
        })
}

fn reply_attribution(
    reply: &rpc::DialogueResponse,
    scene: Option<&rpc::GameplayScene>,
    selected_speaker: &str,
) -> (String, String) {
    let actor_id = reply.npc_actor_id.trim();
    let canonical_speaker = reply.npc_speaker.trim();
    if !actor_id.is_empty() && !canonical_speaker.is_empty() {
        let portrait = scene
            .and_then(|scene| {
                scene
                    .cast
                    .iter()
                    .find(|actor| actor.id == actor_id && actor.name == canonical_speaker)
            })
            .and_then(|actor| super::live_screens::asset_url(&actor.portrait))
            .unwrap_or_else(|| portrait_url(canonical_speaker).to_owned());
        return (canonical_speaker.to_owned(), portrait);
    }

    let speaker = selected_speaker.to_owned();
    let portrait = speaker_portrait_url(scene, &speaker);
    (speaker, portrait)
}

fn reply_status(kind: rpc::DialogueDisposition, reply_text: &str, speaker: &str) -> String {
    match kind {
        rpc::DialogueDisposition::Meta if !reply_text.is_empty() => reply_text.to_owned(),
        rpc::DialogueDisposition::Meta => "Noted.".to_owned(),
        rpc::DialogueDisposition::PlanOnly => "You say to the party.".to_owned(),
        rpc::DialogueDisposition::Question | rpc::DialogueDisposition::Social
            if !speaker.is_empty() =>
        {
            format!("You say to {speaker}.")
        }
        rpc::DialogueDisposition::Clarify => "Please rephrase that.".to_owned(),
        rpc::DialogueDisposition::Rejected => "That line could not be taken.".to_owned(),
        _ => "You say that to the table.".to_owned(),
    }
}

fn scene_portrait_url(scene: Option<&rpc::GameplayScene>) -> Option<String> {
    scene.and_then(|scene| super::live_screens::asset_url(&scene.speaker_portrait))
}

fn speaker_portrait_url(scene: Option<&rpc::GameplayScene>, speaker: &str) -> String {
    scene_portrait_url(scene).unwrap_or_else(|| portrait_url(speaker).to_owned())
}

fn composer_available(journey: Option<&rpc::JourneyView>) -> bool {
    journey.is_some_and(|journey| {
        !journey.adventure_ended
            && journey.creation.is_none()
            && journey.own_character.is_some()
            && !super::is_dead_hero(journey)
    })
}

impl DialogueRecovery {
    #[cfg(test)]
    pub(super) fn set_test_editor(&mut self, editor: HtmlTextAreaElement) {
        self.editor = Some(editor);
    }

    pub(super) fn editor(&self) -> Option<&HtmlTextAreaElement> {
        self.editor.as_ref()
    }

    pub(super) fn channel_clone(
        &self,
    ) -> Option<rpc::dialogue_service_client::DialogueServiceClient<BrowserChannel>> {
        self.channel.clone()
    }

    pub(super) fn capture_focus(&mut self, document: &Document) -> Result<(), JsValue> {
        self.restore_focus = false;
        self.selection = None;
        if let Some(editor) = &self.editor {
            let element: &Element = editor.as_ref();
            self.restore_focus = document.active_element().as_ref() == Some(element);
            if self.restore_focus
                && let (Some(start), Some(end)) =
                    (editor.selection_start()?, editor.selection_end()?)
            {
                self.selection = Some((start, end));
            }
        }
        Ok(())
    }

    pub(super) fn bind(&mut self, channel: BrowserChannel) {
        self.channel = Some(
            rpc::dialogue_service_client::DialogueServiceClient::new(channel)
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        );
    }

    pub(super) fn retire_transport(&mut self) {
        self.talk.interrupt_now(Interruption::ConnectionLost);
        // Disable Say it at once, before any redraw, so a press cannot land on a dead channel.
        if let Some(button) = &self.send_button {
            let _disabled = button.set_attribute("disabled", "");
        }
        if let Some((_, abort)) = self.active.take() {
            abort.abort();
        }
        self.channel = None;
        self.ticket = None;
        if self.confirmation.is_some() {
            self.message =
                "Confirmation outcome unknown. Reconnect, then retry the same confirmation."
                    .to_owned();
        }
    }

    pub(super) fn clear_scope(&mut self) {
        self.talk.interrupt_now(Interruption::SceneChanged);
        self.retire_transport();
        self.confirmation = None;
        let draft = self
            .editor
            .as_ref()
            .map(HtmlTextAreaElement::value)
            .unwrap_or_default();
        self.message = scope_change_notice(&draft).unwrap_or_default().to_owned();
    }

    pub(super) fn has_confirmation(&self) -> bool {
        self.confirmation.is_some()
    }

    pub(super) fn discard_unconfirmed(&mut self) {
        self.ticket = None;
    }
}

impl Drop for DialogueRecovery {
    fn drop(&mut self) {
        if let Some((_, abort)) = self.active.take() {
            abort.abort();
        }
    }
}

enum Call {
    Raw(rpc::RawDialogueRequest),
    Cancel(rpc::CancelDialogueRequest),
    Confirm(rpc::ConfirmDialogueRequest),
}

pub(super) fn submit_disabled(client: &Client) -> bool {
    // While the game connection is down the line cannot reach the table, so Say it waits.
    client.busy
        || client.connection.is_none()
        || client.dialogue.channel.is_none()
        || client.dialogue.confirmation.is_some()
        || client.dialogue.talk.is_busy()
}

pub(super) fn mount(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    offers: &[rpc::GameplayActionOffer],
) -> Result<(), JsValue> {
    if client.borrow().role != "player" {
        return Ok(());
    }
    let can_show_composer = client
        .borrow()
        .views
        .as_ref()
        .and_then(|views| views.current())
        .and_then(|view| view.audience.as_ref())
        .and_then(|audience| match audience {
            rpc::view_message::Audience::Player(player) => player.journey.as_ref(),
            _ => None,
        })
        .is_some_and(|journey| composer_available(Some(journey)));
    if !can_show_composer {
        if let Some(previous) = parent.query_selector(".dialogue-recovery")? {
            previous.remove();
        }
        return Ok(());
    }
    let (document, editor, focused, selection) = {
        let mut state = client.borrow_mut();
        let document = state.document.clone();
        let recovery = &mut state.dialogue;
        let editor = match recovery.editor.as_ref() {
            Some(editor) => editor.clone(),
            None => {
                let editor = document
                    .create_element("textarea")?
                    .dyn_into::<HtmlTextAreaElement>()?;
                editor.set_max_length(4096);
                editor.set_rows(3);
                editor.set_attribute("aria-label", "Your dialogue draft")?;
                editor.set_class_name("dialogue-draft");
                recovery.editor = Some(editor.clone());
                editor
            }
        };
        let focused = recovery.restore_focus;
        let selection = recovery.selection;
        (document, editor, focused, selection)
    };
    if client.borrow().dialogue.intent_listeners.is_empty() {
        listen_for_intent(client, &editor)?;
    }
    if let Some(previous) = parent.query_selector(".dialogue-recovery")? {
        previous.remove();
    }
    let panel = node(&document, parent, "section", "dialogue-recovery", "")?;
    node(&document, &panel, "h3", "", "Say what you mean")?;
    node(
        &document,
        &panel,
        "p",
        "scope",
        "Speak to someone in the scene, ask about what you see, or describe what your hero does.",
    )?;
    let revision = client.borrow().revision;
    {
        let state = client.borrow();
        if let Some(exchange) = state
            .dialogue
            .exchange
            .as_ref()
            .filter(|exchange| exchange.revision == revision)
        {
            let you: String = exchange.you.chars().take(300).collect();
            let thread = node(&document, &panel, "div", "npc-exchange", "")?;
            node(&document, &thread, "p", "npc-you", &format!("You: {you}"))?;
            if let Some(disposition) = &exchange.disposition {
                node(
                    &document,
                    &thread,
                    "p",
                    "npc-disposition npc-you",
                    disposition,
                )?;
            }
            if !exchange.reply.is_empty() {
                let bubble = node(&document, &thread, "div", "npc-bubble", "")?;
                let portrait = node(&document, &bubble, "img", "npc-bubble-portrait", "")?;
                portrait.set_attribute("src", &exchange.portrait)?;
                portrait.set_attribute("alt", "")?;
                let body = node(&document, &bubble, "div", "npc-bubble-body", "")?;
                let name = if exchange.speaker.is_empty() {
                    "Narrator"
                } else {
                    &exchange.speaker
                };
                node(&document, &body, "p", "npc-bubble-name", name)?;
                node(&document, &body, "p", "npc-bubble-reply", &exchange.reply)?;
            }
        }
    }
    thinking_row(client, &document, &panel)?;
    panel.append_child(&editor)?;
    super::talk_control::mount(client, &panel)?;
    let mut current = Vec::new();
    for offer in offers {
        if offer.offer_id.len() > 96 || offer.label.len() > 256 {
            return Err(JsValue::from_str("dialogue offer exceeds bound"));
        }
        // Additional selections stay in their existing typed controls, below this panel.
        if !offer.input_groups.is_empty()
            || !offer.destinations.is_empty()
            || matches!(
                rpc::GameplayActionKind::try_from(offer.action_kind),
                Ok(rpc::GameplayActionKind::CreateCharacter
                    | rpc::GameplayActionKind::GreatswordAttack)
            )
        {
            continue;
        }
        current.push(offer.clone());
    }
    {
        let mut state = client.borrow_mut();
        let revision = state.revision;
        let draft = composer_draft(&state.dialogue);
        let recovery = &mut state.dialogue;
        if recovery.confirmation.is_none()
            && recovery.ticket.as_ref().is_some_and(|(input, _)| {
                input.observed_revision != revision
                    || !current.iter().any(|offer| offer.offer_id == input.offer_id)
            })
        {
            // A revision moved by another player drops the stale ticket, but the notice is only
            // for a player whose own text is still waiting in the composer.
            recovery.ticket = None;
            recovery.message = if draft.trim().is_empty() {
                String::new()
            } else {
                "The offer changed. Your text remains editable; request current clarification."
                    .to_owned()
            };
        }
        recovery.offers = current;
    }
    if client.borrow().connection.is_none() {
        // The feed above is the last one received; the draft below is kept until the table is back.
        let notice = node(
            &document,
            &panel,
            "p",
            "dialogue-reconnecting",
            "Reconnecting to the table. Your words stay in the box; Say it works again when you are back.",
        )?;
        notice.set_attribute("role", "status")?;
    }
    let controls = node(&document, &panel, "div", "dialogue-controls", "")?;
    let raw = mount_send(client, &controls)?;
    let has_ticket = client.borrow().dialogue.ticket.is_some();
    let cancel = has_ticket
        .then(|| button(client, &controls, "Decline this choice", decline))
        .transpose()?;
    let confirm = has_ticket
        .then(|| button(client, &controls, "Confirm offered choice", confirm))
        .transpose()?;
    // Retry is a lost-response recovery path: it exists only while a confirmation's outcome
    // is unknown. Otherwise the main view's "Connection trouble?" section covers recovery.
    let retry = if client.borrow().dialogue.has_confirmation() {
        Some(button(
            client,
            &controls,
            "Retry the same confirmation",
            retry,
        )?)
    } else {
        None
    };
    update_typed_draft(&editor)?;
    let state = client.borrow();
    let recovery = &state.dialogue;
    raw.set_attribute("data-dialogue", "submit")?;
    if let Some(cancel) = &cancel {
        cancel.set_attribute("data-dialogue", "cancel")?;
    }
    if let Some(confirm) = &confirm {
        confirm.set_attribute("data-dialogue", "confirm")?;
    }
    if let Some(retry) = &retry {
        retry.set_attribute("data-dialogue", "retry")?;
    }
    for (element, disabled) in std::iter::once((&raw, submit_disabled(&state)))
        .chain(cancel.iter().map(|element| {
            (
                element,
                state.busy || recovery.channel.is_none() || recovery.confirmation.is_some(),
            )
        }))
        .chain(confirm.iter().map(|element| {
            (
                element,
                state.busy || recovery.channel.is_none() || recovery.confirmation.is_some(),
            )
        }))
        .chain(
            retry
                .as_ref()
                .map(|retry| (retry, state.busy || recovery.channel.is_none())),
        )
    {
        if disabled {
            element.set_attribute("disabled", "")?;
        } else {
            element.remove_attribute("disabled")?;
        }
    }
    if !recovery.message.is_empty() {
        let message = node(
            &document,
            &panel,
            "p",
            "dialogue-feedback",
            &recovery.message,
        )?;
        message.set_attribute("role", "status")?;
    }
    if focused {
        editor.focus()?;
        if let Some((start, end)) = selection {
            editor.set_selection_range(start, end)?;
        }
    }
    Ok(())
}

fn mount_send(client: &Rc<RefCell<Client>>, controls: &Element) -> Result<Element, JsValue> {
    let existing = client.borrow().dialogue.send_button.clone();
    let raw = match existing {
        Some(raw) => {
            controls.append_child(&raw)?;
            bind_click(client, &raw, super::talk_control::send)?;
            raw
        }
        None => button(client, controls, "Say it", super::talk_control::send)?,
    };
    raw.set_attribute("type", "button")?;
    client.borrow_mut().dialogue.send_button = Some(raw.clone());
    Ok(raw)
}

/// While a speaker writes a reply the phone shows who is thinking, with animated dots beside
/// their name, and this player's own words above it. Presentation only.
fn thinking_row(
    client: &Rc<RefCell<Client>>,
    document: &Document,
    panel: &Element,
) -> Result<(), JsValue> {
    let state = client.borrow();
    let replying = state
        .views
        .as_ref()
        .and_then(|views| views.current())
        .and_then(crate::presentation::replying_speaker);
    let Some(speaker) = replying.or_else(|| {
        state
            .dialogue
            .awaiting
            .as_ref()
            .map(|_| current_speaker(&state))
    }) else {
        return Ok(());
    };
    let thread = node(document, panel, "div", "npc-exchange", "")?;
    if let Some(you) = &state.dialogue.awaiting {
        let you: String = you.chars().take(300).collect();
        node(document, &thread, "p", "npc-you", &format!("You: {you}"))?;
    }
    let bubble = node(document, &thread, "div", "npc-bubble npc-thinking", "")?;
    bubble.set_attribute("data-thinking", "")?;
    let portrait = node(document, &bubble, "img", "npc-bubble-portrait", "")?;
    portrait.set_attribute("src", portrait_url(&speaker))?;
    portrait.set_attribute("alt", "")?;
    let body = node(document, &bubble, "div", "npc-bubble-body", "")?;
    let name = if speaker.is_empty() {
        "Narrator"
    } else {
        &speaker
    };
    let heading = node(document, &body, "p", "npc-bubble-name", name)?;
    heading.set_attribute("role", "status")?;
    heading.set_attribute("aria-label", &format!("{name} is thinking"))?;
    let dots = node(document, &heading, "span", "npc-dots", "")?;
    dots.set_attribute("aria-hidden", "true")?;
    for _ in 0..3 {
        node(document, &dots, "i", "", "")?;
    }
    Ok(())
}

/// Typing changes the emphasis of the existing controls without replacing the editor/button.
pub(super) fn update_typed_draft(editor: &HtmlTextAreaElement) -> Result<(), JsValue> {
    if let Some(panel) = editor.closest(".dialogue-recovery")? {
        if editor.value().trim().is_empty() {
            panel.remove_attribute("data-typed")?;
        } else {
            panel.set_attribute("data-typed", "")?;
        }
    }
    Ok(())
}

/// What the draft says about the player's intent: typing while there is text, otherwise nothing.
fn draft_intent(client: &Rc<RefCell<Client>>) -> live_presence::Intent {
    let typing = client
        .borrow()
        .dialogue
        .editor
        .as_ref()
        .is_some_and(|editor| !editor.value().trim().is_empty());
    if typing {
        live_presence::Intent::Typing
    } else {
        live_presence::Intent::Stopped
    }
}

/// Wires the speech-intent signal to the draft box (typing, blur) and the context control (table
/// talk shows no bubble). Only an ephemeral state leaves the phone; never the draft.
fn listen_for_intent(
    client: &Rc<RefCell<Client>>,
    editor: &HtmlTextAreaElement,
) -> Result<(), JsValue> {
    let mut listeners = Vec::new();
    let mut attach =
        |target: &web_sys::EventTarget,
         name: &str,
         react: fn(&Rc<RefCell<Client>>, &web_sys::Event) -> Option<live_presence::Intent>|
         -> Result<(), JsValue> {
            let weak = Rc::downgrade(client);
            let callback = Closure::wrap(Box::new(move |event: web_sys::Event| {
                if let Some(client) = weak.upgrade()
                    && let Some(intent) = react(&client, &event)
                {
                    live_presence::signal(&client, intent);
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            target.add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())?;
            listeners.push(callback);
            Ok(())
        };
    attach(editor.as_ref(), "input", |client, event| {
        client.borrow_mut().dialogue.message.clear();
        if let Some(editor) = client.borrow().dialogue.editor()
            && update_typed_draft(editor).is_err()
        {
            status(
                &client.borrow(),
                "Your draft controls could not be updated.",
            );
        }
        if let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
            && let Ok(Some(panel)) = target.closest(".dialogue-recovery")
            && let Ok(Some(message)) = panel.query_selector(".dialogue-feedback")
        {
            message.set_text_content(Some(""));
        }
        Some(draft_intent(client))
    })?;
    attach(editor.as_ref(), "blur", |client, event| {
        // Moving to the panel's own buttons (the send button) is not leaving the box.
        let inside = js_sys::Reflect::get(event, &JsValue::from_str("relatedTarget"))
            .ok()
            .and_then(|target| target.dyn_into::<Element>().ok())
            .is_some_and(|target| {
                target
                    .closest(".dialogue-recovery")
                    .is_ok_and(|panel| panel.is_some())
            });
        if inside {
            return Some(draft_intent(client));
        }
        // A redraw moves the box and gives focus back at once; only a box that stays unfocused
        // means the player left it.
        let weak = Rc::downgrade(client);
        wasm_bindgen_futures::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(BLUR_GRACE_MS).await;
            let Some(client) = weak.upgrade() else {
                return;
            };
            let focused = {
                let state = client.borrow();
                state.dialogue.editor.as_ref().is_some_and(|editor| {
                    let element: &Element = editor.as_ref();
                    state.document.active_element().as_ref() == Some(element)
                })
            };
            if !focused {
                live_presence::signal(&client, live_presence::Intent::Stopped);
            }
        });
        None
    })?;
    client.borrow_mut().dialogue.intent_listeners = listeners;
    Ok(())
}

pub(super) fn submit(client: Rc<RefCell<Client>>) {
    let prepared = (|| -> Result<_, String> {
        let state = client.borrow();
        if state.dialogue.confirmation.is_some()
            || (!state.last_confirmed && state.last_request.is_some())
        {
            return Err("Recover the original confirmed operation first.".to_owned());
        }
        let text = state
            .dialogue
            .editor
            .as_ref()
            .ok_or("Dialogue input absent")?
            .value();
        if text.trim().is_empty() || text.len() > 4096 {
            return Err("Enter between 1 and 4096 UTF-8 bytes.".to_owned());
        }
        Ok(rpc::RawDialogueRequest {
            session_id: state.session.clone(),
            run_id: state.run.clone(),
            observed_revision: state.revision,
            input_id: Some(operation()?),
            text,
            context: rpc::DialogueContext::Unspecified as i32,
            finality: rpc::DialogueFinality::Final as i32,
            offer_id: String::new(),
        })
    })();
    match prepared {
        Ok(body) => {
            // The loader stays on the hero while the line is cleaned and committed.
            live_presence::signal(&client, live_presence::Intent::Submitted);
            dispatch(client, Call::Raw(body));
        }
        Err(message) => show(&client, &message),
    }
}

/// Sends a notebook command. Returns false when nothing was sent, so the caller keeps its text.
pub(super) fn submit_notebook_command(client: Rc<RefCell<Client>>, command: String) -> bool {
    let prepared = (|| -> Result<_, String> {
        let state = client.borrow();
        if state.role != "player" {
            return Err("A player seat is required.".to_owned());
        }
        if state.busy
            || state.dialogue.confirmation.is_some()
            || (!state.last_confirmed && state.last_request.is_some())
        {
            return Err("Finish the current choice before changing notebook notes.".to_owned());
        }
        if command.trim().is_empty() || command.len() > 4096 + "write down ".len() {
            return Err("Notebook command is outside its text bound.".to_owned());
        }
        Ok(rpc::RawDialogueRequest {
            session_id: state.session.clone(),
            run_id: state.run.clone(),
            observed_revision: state.revision.clone(),
            input_id: Some(operation()?),
            text: command,
            context: rpc::DialogueContext::Meta as i32,
            finality: rpc::DialogueFinality::Final as i32,
            offer_id: String::new(),
        })
    })();
    match prepared {
        Ok(request) => match try_dispatch(client.clone(), Call::Raw(request)) {
            Ok(()) => true,
            Err(message) => {
                show(&client, &message);
                false
            }
        },
        Err(message) => {
            show(&client, &message);
            false
        }
    }
}

fn decline(client: Rc<RefCell<Client>>) {
    let body = {
        let state = client.borrow();
        if state.dialogue.confirmation.is_some() {
            None
        } else {
            state
                .dialogue
                .ticket
                .as_ref()
                .map(|(input, reply)| rpc::CancelDialogueRequest {
                    input_id: input.input_id.clone(),
                    confirmation_token: reply.confirmation_token.clone(),
                })
        }
    };
    match body {
        Some(body) => dispatch(client, Call::Cancel(body)),
        None => show(
            &client,
            "Only an unconfirmed current choice can be declined.",
        ),
    }
}

fn confirm(client: Rc<RefCell<Client>>) {
    let prepared = (|| -> Result<_, String> {
        let state = client.borrow();
        let (input, ticket) = state
            .dialogue
            .ticket
            .as_ref()
            .ok_or("Request clarification first.")?;
        if input.observed_revision != state.revision {
            return Err("That clarification is stale; request a current one.".to_owned());
        }
        let offer = state
            .dialogue
            .offers
            .iter()
            .find(|offer| offer.offer_id == input.offer_id)
            .cloned()
            .ok_or("The selected offer ended.")?;
        let input_id = input.input_id.clone();
        let token = ticket.confirmation_token.clone();
        drop(state);
        let action = prepare(&client, &offer, None, false, false, 0)?;
        if action.operation_id == input_id {
            return Err("Separate confirmation identity unavailable.".to_owned());
        }
        Ok(rpc::ConfirmDialogueRequest {
            input_id,
            confirmation_token: token,
            action: Some(action),
        })
    })();
    match prepared {
        Ok(body) => dispatch(client, Call::Confirm(body)),
        Err(message) => show(&client, &message),
    }
}

pub(super) fn retry(client: Rc<RefCell<Client>>) {
    let body = client.borrow().dialogue.confirmation.clone();
    match body {
        Some(body) => dispatch(client, Call::Confirm(body)),
        None => show(&client, "No uncertain dialogue confirmation is retained."),
    }
}

fn show(client: &Rc<RefCell<Client>>, message: &str) {
    client.borrow_mut().dialogue.message = message.to_owned();
    if redraw_current(client).is_err() {
        status(
            &client.borrow(),
            "Dialogue controls could not be displayed.",
        );
    }
}

fn dispatch(client: Rc<RefCell<Client>>, call: Call) {
    if let Err(message) = try_dispatch(client.clone(), call) {
        show(&client, &message);
    }
}

/// Starts the call. A refusal names why nothing was sent, so the caller keeps its draft.
fn try_dispatch(client: Rc<RefCell<Client>>, call: Call) -> Result<(), String> {
    let prepared = (|| -> Result<_, String> {
        let mut state = client.borrow_mut();
        if state.busy {
            return Err("Waiting for the server…".to_owned());
        }
        let channel = state
            .dialogue
            .channel
            .clone()
            .ok_or("Reconnect before sending dialogue.")?;
        let credential = state.credential.clone();
        let generation = state.generation;
        if let Call::Raw(body) = &call {
            state.dialogue.ticket = None;
            state.dialogue.exchange = None;
            state.dialogue.awaiting = Some(body.text.clone());
        }
        if let Call::Confirm(body) = &call {
            let action = body.action.clone().ok_or("Separate typed action absent")?;
            if let Some(original) = &state.dialogue.confirmation
                && original != body
            {
                return Err("Retry the exact original confirmation.".to_owned());
            }
            state.last_request = Some(action);
            state.last_confirmed = false;
            state.last_accepted = false;
            state.dialogue.confirmation = Some(body.clone());
        }
        let instance = Rc::new(());
        let (abort, registration) = AbortHandle::new_pair();
        if let Some((_, old)) = state.dialogue.active.replace((instance.clone(), abort)) {
            old.abort();
        }
        state.busy = true;
        state.dialogue.message = "Sending…".to_owned();
        Ok((channel, credential, generation, instance, registration))
    })();
    let (mut channel, credential, generation, instance, registration) = prepared?;
    let weak = Rc::downgrade(&client);
    let _ = redraw_current(&client);
    wasm_bindgen_futures::spawn_local(async move {
        let run =
            async {
                match &call {
                    Call::Raw(body) => {
                        let mut raw = request(body.clone(), &credential).map_err(|_| {
                            tonic::Status::unauthenticated("input binding unavailable")
                        })?;
                        raw.set_timeout(RAW_DIALOGUE_TIMEOUT);
                        channel
                            .submit(raw)
                            .await
                            .map(|reply| (Some(reply.into_inner()), None))
                    }
                    Call::Cancel(body) => channel
                        .cancel(request(body.clone(), &credential).map_err(|_| {
                            tonic::Status::unauthenticated("input binding unavailable")
                        })?)
                        .await
                        .map(|reply| (Some(reply.into_inner()), None)),
                    Call::Confirm(body) => channel
                        .confirm(request(body.clone(), &credential).map_err(|_| {
                            tonic::Status::unauthenticated("input binding unavailable")
                        })?)
                        .await
                        .map(|reply| (None, Some(reply.into_inner()))),
                }
            };
        let outcome = Abortable::new(run, registration).await;
        let Some(client) = weak.upgrade() else {
            return;
        };
        {
            let mut state = client.borrow_mut();
            if state.generation != generation
                || !state
                    .dialogue
                    .active
                    .as_ref()
                    .is_some_and(|(current, _)| Rc::ptr_eq(current, &instance))
            {
                return;
            }
            state.dialogue.active.take();
            state.dialogue.awaiting = None;
            state.busy = false;
            if matches!(call, Call::Raw(_)) {
                // Whatever came back, the hero is no longer composing.
                let weak = Rc::downgrade(&client);
                wasm_bindgen_futures::spawn_local(async move {
                    if let Some(client) = weak.upgrade() {
                        live_presence::signal(&client, live_presence::Intent::Stopped);
                    }
                });
            }
            let message = match outcome {
                Ok(Ok((Some(reply), None))) => match &call {
                    Call::Raw(input)
                        if reply.input_id == input.input_id
                            && reply.revision.is_some()
                            && reply.revision != input.observed_revision
                            && reply.disposition == rpc::DialogueDisposition::Action as i32
                            && !reply.confirmation_required
                            && reply.confirmation_token.is_empty()
                            && !reply.npc_reply.is_empty()
                            && reply.npc_reply.len() <= MAXIMUM_REPLY_BYTES =>
                    {
                        state.dialogue.ticket = None;
                        let (disposition, result) = split_action_response(&reply.npc_reply);
                        state.dialogue.exchange = Some(Exchange {
                            revision: reply.revision.clone(),
                            you: input.text.clone(),
                            disposition: Some(disposition.to_owned()),
                            speaker: "Narrator".to_owned(),
                            portrait: portrait_url("Narrator").to_owned(),
                            reply: result.to_owned(),
                        });
                        if let Some(editor) = &state.dialogue.editor {
                            editor.set_value("");
                        }
                        String::new()
                    }
                    Call::Raw(input) if answers_input(input, &reply) => {
                        match rpc::DialogueDisposition::try_from(reply.disposition) {
                            Ok(rpc::DialogueDisposition::Clarify)
                                if reply.confirmation_required
                                    && reply.confirmation_token.len() == 32
                                    && !input.offer_id.is_empty() =>
                            {
                                // The offer was read at the revision that answered, so the ticket
                                // is bound to that revision rather than the one the input saw.
                                let mut bound = input.clone();
                                bound.observed_revision = reply.revision;
                                state.dialogue.ticket = Some((bound, reply));
                                "Clarification required. Decline, or explicitly confirm the selected offered action.".to_owned()
                            }
                            Ok(kind)
                                if !reply.confirmation_required
                                    && reply.confirmation_token.is_empty()
                                    && kind != rpc::DialogueDisposition::Action
                                    && kind != rpc::DialogueDisposition::Unspecified =>
                            {
                                state.dialogue.ticket = None;
                                let selected_speaker = current_speaker(&state);
                                let (speaker, portrait) = reply_attribution(
                                    &reply,
                                    current_scene(&state),
                                    &selected_speaker,
                                );
                                if !reply.npc_reply.is_empty()
                                    && reply.npc_reply.len() <= MAXIMUM_REPLY_BYTES
                                {
                                    state.dialogue.exchange = Some(Exchange {
                                        revision: reply.revision,
                                        you: input.text.clone(),
                                        disposition: None,
                                        speaker: speaker.clone(),
                                        portrait,
                                        reply: reply.npc_reply.clone(),
                                    });
                                    if let Some(editor) = &state.dialogue.editor {
                                        editor.set_value("");
                                    }
                                } else {
                                    state.dialogue.exchange = None;
                                }
                                if let Some(editor) = &state.dialogue.editor {
                                    editor.set_value("");
                                }
                                reply_status(kind, &reply.npc_reply, &speaker)
                            }
                            _ => {
                                "The dialogue response is invalid. Retain your draft and reconnect."
                                    .to_owned()
                            }
                        }
                    }
                    Call::Cancel(input)
                        if reply.input_id == input.input_id
                            && reply.disposition == rpc::DialogueDisposition::Rejected as i32
                            && !reply.confirmation_required
                            && reply.confirmation_token.is_empty()
                            && reply.revision.is_some() =>
                    {
                        state.dialogue.ticket = None;
                        "Choice declined by the native owner. Your draft remains editable; no action was confirmed.".to_owned()
                    }
                    _ => "The response does not match this dialogue scope. Reconnect.".to_owned(),
                },
                Ok(Ok((None, Some(reply)))) => {
                    if let Call::Confirm(input) = &call {
                        if let Some(action) = &input.action {
                            let message = complete_action_response(&mut state, action, Ok(reply));
                            if state.last_confirmed {
                                state.dialogue.confirmation = None;
                                state.dialogue.ticket = None;
                            }
                            message
                        } else {
                            "The retained typed action is absent. Reconnect.".to_owned()
                        }
                    } else {
                        "The native response kind is invalid. Reconnect.".to_owned()
                    }
                }
                Ok(Err(error)) => {
                    // A failed confirmation can already have committed. Keep its exact input.
                    if state.dialogue.confirmation.is_some() {
                        "Confirmation outcome unknown. Reconnect, then retry the same confirmation."
                            .to_owned()
                    } else {
                        match error.code() {
                            tonic::Code::FailedPrecondition => {
                                scope_changed_notice(&composer_draft(&state.dialogue))
                                    .unwrap_or_default()
                                    .to_owned()
                            }
                            tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
                                "Dialogue authorization is unavailable. Reconnect.".to_owned()
                            }
                            tonic::Code::InvalidArgument | tonic::Code::ResourceExhausted => {
                                "The native owner refused this input. Edit the bounded draft."
                                    .to_owned()
                            }
                            _ => "Dialogue is unavailable. Your draft remains editable.".to_owned(),
                        }
                    }
                }
                Err(_) => {
                    "The dialogue request was cancelled. Your draft remains editable.".to_owned()
                }
                _ => "The native dialogue response is absent. Reconnect.".to_owned(),
            };
            state.dialogue.message = message;
        }
        if redraw_current(&client).is_err() {
            status(
                &client.borrow(),
                "Dialogue controls could not be displayed.",
            );
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_same_send_button_remains_callable_after_page_callbacks_are_cleared() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let client = new_client(&document, root.clone(), "player").unwrap();
        let controls = node(&document, &root, "div", "dialogue-controls", "").unwrap();
        let first = mount_send(&client, &controls).unwrap();
        clear_callbacks(&mut client.borrow_mut());
        let second = mount_send(&client, &controls).unwrap();
        assert!(first.is_same_node(Some(second.as_ref())));
        assert_eq!(controls.query_selector_all("button").unwrap().length(), 1);
        second
            .dispatch_event(&web_sys::Event::new("click").unwrap())
            .unwrap();
        assert!(!client.borrow().dialogue.message.is_empty());
    }

    #[test]
    fn typing_promotes_the_same_composer_and_clearing_restores_voice_emphasis() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let panel = document.create_element("section").unwrap();
        panel.set_class_name("dialogue-recovery");
        let editor = node(&document, &panel, "textarea", "dialogue-draft", "")
            .unwrap()
            .dyn_into::<HtmlTextAreaElement>()
            .unwrap();
        let send = node(&document, &panel, "button", "", "Say it").unwrap();
        editor.set_value("What happened here?");
        update_typed_draft(&editor).unwrap();
        assert!(panel.has_attribute("data-typed"));
        assert_eq!(editor.value(), "What happened here?");
        assert!(
            send.is_same_node(
                panel
                    .query_selector("button")
                    .unwrap()
                    .as_ref()
                    .map(|node| node.as_ref())
            )
        );
        editor.set_value("");
        update_typed_draft(&editor).unwrap();
        assert!(!panel.has_attribute("data-typed"));
        assert_eq!(panel.query_selector_all("textarea").unwrap().length(), 1);
    }

    use super::{
        composer_available, reply_attribution, reply_status, scope_change_notice,
        scope_changed_notice, speaker_portrait_url, split_action_response,
    };

    #[test]
    fn scope_change_notice_is_absent_for_an_empty_draft() {
        assert_eq!(scope_change_notice(""), None);
        assert_eq!(scope_change_notice(" \t\n"), None);
    }

    /// Wasm-only: another player's turn moves the scope while this player's composer is empty.
    /// Arrange an empty composer; act by reading its draft as the refusal handler does; assert no
    /// notice. Then type a draft and assert the notice names it.
    #[test]
    fn a_scope_change_stays_silent_while_this_players_composer_is_empty() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let client = new_client(&document, root.clone(), "player").unwrap();
        let editor = node(&document, &root, "textarea", "dialogue-draft", "")
            .unwrap()
            .dyn_into::<HtmlTextAreaElement>()
            .unwrap();
        client.borrow_mut().dialogue.set_test_editor(editor.clone());
        assert_eq!(
            scope_changed_notice(&composer_draft(&client.borrow().dialogue)),
            None
        );
        editor.set_value("I look behind the counter.");
        assert_eq!(
            scope_changed_notice(&composer_draft(&client.borrow().dialogue)),
            Some("The offer or scope changed. Your text remains editable.")
        );
    }

    #[test]
    fn scope_changed_notice_is_absent_for_a_blank_draft() {
        assert_eq!(scope_changed_notice(""), None);
        assert_eq!(scope_changed_notice(" \t\n"), None);
    }

    #[test]
    fn scope_change_notice_keeps_a_nonempty_draft_reviewable() {
        assert_eq!(
            scope_change_notice("I look behind the counter."),
            Some("Your draft is still here. Review it before sending.")
        );
    }

    #[test]
    fn action_response_separates_attempt_from_roll_and_result() {
        let response = "You search behind the counter. Brakka searches the counter: Wisdom (Perception) 16 + 3 = 19. You find the ledger.";

        let (attempt, result) = split_action_response(response);

        assert_eq!(attempt, "You search behind the counter.");
        assert_eq!(
            result,
            "Brakka searches the counter: Wisdom (Perception) 16 + 3 = 19. You find the ledger."
        );
    }

    #[test]
    fn composer_is_hidden_during_creation_and_after_the_journey_ends() {
        let mut journey = super::rpc::JourneyView {
            own_character: Some(Default::default()),
            ..Default::default()
        };

        assert!(!composer_available(None));
        journey.own_character = None;
        assert!(!composer_available(Some(&journey)));
        journey.own_character = Some(Default::default());
        assert!(composer_available(Some(&journey)));
        journey
            .own_character
            .as_mut()
            .expect("own hero present")
            .facts
            .push(super::rpc::SheetFact {
                label: "Condition".to_owned(),
                value: "Dead".to_owned(),
            });
        assert!(!composer_available(Some(&journey)));
        journey
            .own_character
            .as_mut()
            .expect("own hero present")
            .facts
            .clear();
        assert!(composer_available(Some(&journey)));
        journey.adventure_ended = true;
        assert!(!composer_available(Some(&journey)));
    }

    #[test]
    fn reply_portrait_prefers_the_current_scene_and_keeps_named_fallback() {
        let scene = super::rpc::GameplayScene {
            speaker_portrait: "assets/ui/generated/patron.webp".to_owned(),
            ..Default::default()
        };

        assert_eq!(
            speaker_portrait_url(Some(&scene), "Unlisted patron"),
            "/assets/ui/generated/patron.webp"
        );
        assert_eq!(
            speaker_portrait_url(None, "Courier"),
            "/assets/ui/generated/sprite-courier-documentary-photo"
        );
    }

    #[test]
    fn reply_attribution_uses_the_canonical_responding_actor() {
        let scene = super::rpc::GameplayScene {
            speaker_portrait: "/assets/concept-art/marek-avatar.webp".to_owned(),
            cast: vec![
                super::rpc::SceneActor {
                    id: "merchant".to_owned(),
                    name: "Marek".to_owned(),
                    portrait: "/assets/concept-art/marek-avatar.webp".to_owned(),
                    ..Default::default()
                },
                super::rpc::SceneActor {
                    id: "courier".to_owned(),
                    name: "Corin".to_owned(),
                    portrait: "/assets/concept-art/corin-avatar.webp".to_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let reply = super::rpc::DialogueResponse {
            npc_actor_id: "courier".to_owned(),
            npc_speaker: "Corin".to_owned(),
            ..Default::default()
        };

        let (speaker, portrait) = reply_attribution(&reply, Some(&scene), "Marek");

        assert_eq!(speaker, "Corin");
        assert_eq!(portrait, "/assets/concept-art/corin-avatar.webp");
        assert_eq!(
            reply_status(super::rpc::DialogueDisposition::Social, "", &speaker),
            "You say to Corin."
        );
    }

    #[test]
    fn reply_attribution_uses_selected_speaker_for_legacy_responses() {
        let scene = super::rpc::GameplayScene {
            speaker_portrait: "/assets/concept-art/marek-avatar.webp".to_owned(),
            ..Default::default()
        };

        let (speaker, portrait) = reply_attribution(
            &super::rpc::DialogueResponse::default(),
            Some(&scene),
            "Marek",
        );

        assert_eq!(speaker, "Marek");
        assert_eq!(portrait, "/assets/concept-art/marek-avatar.webp");
    }

    use super::*;

    fn revision(epoch: u64, sequence: u64) -> super::rpc::SessionRevision {
        super::rpc::SessionRevision {
            epoch: Some(super::rpc::RecoveryEpoch { value: Some(epoch) }),
            sequence: Some(sequence),
        }
    }

    fn operation(byte: u8) -> super::rpc::OperationId {
        super::rpc::OperationId {
            value: Some(vec![byte; 16]),
        }
    }

    fn raw_input(byte: u8, observed: u64) -> super::rpc::RawDialogueRequest {
        super::rpc::RawDialogueRequest {
            input_id: Some(operation(byte)),
            observed_revision: Some(revision(1, observed)),
            ..Default::default()
        }
    }

    fn reply_at(
        byte: u8,
        answered: Option<super::rpc::SessionRevision>,
    ) -> super::rpc::DialogueResponse {
        super::rpc::DialogueResponse {
            input_id: Some(operation(byte)),
            revision: answered,
            ..Default::default()
        }
    }

    #[test]
    fn a_reply_to_the_same_input_at_a_newer_revision_is_accepted() {
        // Arrange: the hero's line observed revision 4; the turn committed a merchant quote.
        let input = raw_input(7, 4);
        let committed = reply_at(7, Some(revision(1, 5)));

        // Act and assert: the observed revision and a later one of the same epoch both answer it.
        assert!(answers_input(&input, &committed));
        assert!(answers_input(&input, &reply_at(7, Some(revision(1, 4)))));
        assert!(answers_input(&input, &reply_at(7, Some(revision(2, 0)))));
    }

    #[test]
    fn a_reply_to_a_different_input_is_refused_at_any_revision() {
        let input = raw_input(7, 4);

        assert!(!answers_input(&input, &reply_at(8, Some(revision(1, 5)))));
        assert!(!answers_input(&input, &reply_at(8, Some(revision(1, 4)))));
    }

    #[test]
    fn a_reply_at_an_older_revision_is_refused() {
        let input = raw_input(7, 4);

        assert!(!answers_input(&input, &reply_at(7, Some(revision(1, 3)))));
        assert!(!answers_input(&input, &reply_at(7, Some(revision(0, 9)))));
    }

    #[test]
    fn a_reply_without_a_revision_is_refused() {
        let input = raw_input(7, 4);

        assert!(!answers_input(&input, &reply_at(7, None)));
    }
}
